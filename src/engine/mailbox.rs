#![allow(dead_code)] // multi-owner internals; diagnostics wiring is incremental
//! Bounded cross-owner admission. Message slots, retained input bytes and reply
//! bytes are separate resources; no argument copy is made before admission.
//!
//! One budget counts bytes and in-flight messages together under one lock, so a
//! caller takes both atomically and never deadlocks between a byte and a message
//! waiter. Admission is offered in two equivalent forms: owner runtimes await a
//! credit, while the synchronous socket adapter blocks on a condition variable
//! outside any runtime. Credit is released exactly when the retained work is
//! consumed or discarded.
use std::ops::Deref;
use std::sync::{Arc, Condvar, Mutex};
use tokio::sync::{Notify, mpsc, oneshot};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MailboxError {
    Closed,
    TooLarge,
    InvalidCapacity,
    ReplyTooLarge,
}
impl std::fmt::Display for MailboxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "owner mailbox: {self:?}")
    }
}
impl std::error::Error for MailboxError {}

struct BudgetState {
    bytes: usize,
    messages: usize,
    closed: bool,
}
struct BudgetInner {
    state: Mutex<BudgetState>,
    released: Condvar,
    async_wake: Notify,
    bytes_capacity: usize,
    messages_capacity: usize,
}
/// `take` runs behind `&self`; this keeps the single `Arc` clone explicit.
fn self_arc(inner: &BudgetInner) -> Arc<BudgetInner> {
    let _ = inner;
    unreachable!("budget take clones through the stored Arc")
}

impl BudgetInner {
    fn release(&self, bytes: usize, messages: usize) {
        if let Ok(mut state) = self.state.lock() {
            state.bytes = state.bytes.saturating_add(bytes);
            state.messages = state.messages.saturating_add(messages);
        }
        self.released.notify_all();
        self.async_wake.notify_waiters();
    }
    fn close(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.closed = true;
        }
        self.released.notify_all();
        self.async_wake.notify_waiters();
    }
    fn take(&self, bytes: usize) -> Option<Credit> {
        let mut state = self.state.lock().ok()?;
        if state.closed || state.bytes < bytes {
            return None;
        }
        // A lane reserves one message; a reply budget does not count messages.
        let messages = usize::from(self.messages_capacity > 0);
        if self.messages_capacity > 0 && state.messages < messages {
            return None;
        }
        state.bytes -= bytes;
        state.messages -= messages;
        Some(Credit {
            inner: Arc::clone(&self_arc(self)),
            bytes,
            messages,
        })
    }
    fn available_bytes(&self) -> usize {
        self.state.lock().map(|state| state.bytes).unwrap_or(0)
    }
    fn available_messages(&self) -> usize {
        self.state.lock().map(|state| state.messages).unwrap_or(0)
    }
    fn is_closed(&self) -> bool {
        self.state.lock().map(|state| state.closed).unwrap_or(true)
    }
}

#[derive(Clone)]
pub(crate) struct ByteBudget {
    inner: Arc<BudgetInner>,
}
impl ByteBudget {
    pub(crate) fn new(capacity: usize) -> Result<Self, MailboxError> {
        Self::with_messages(capacity, 0)
    }
    pub(crate) fn with_messages(
        bytes_capacity: usize,
        messages_capacity: usize,
    ) -> Result<Self, MailboxError> {
        if bytes_capacity == 0 || bytes_capacity > u32::MAX as usize {
            return Err(MailboxError::InvalidCapacity);
        }
        if messages_capacity > u32::MAX as usize {
            return Err(MailboxError::InvalidCapacity);
        }
        Ok(Self {
            inner: Arc::new(BudgetInner {
                state: Mutex::new(BudgetState {
                    bytes: bytes_capacity,
                    messages: messages_capacity,
                    closed: false,
                }),
                released: Condvar::new(),
                async_wake: Notify::new(),
                bytes_capacity,
                messages_capacity,
            }),
        })
    }
    fn take(&self, bytes: usize) -> Option<Credit> {
        let mut state = self.inner.state.lock().ok()?;
        if state.closed || state.bytes < bytes {
            return None;
        }
        let messages = usize::from(self.inner.messages_capacity > 0);
        if self.inner.messages_capacity > 0 && state.messages < messages {
            return None;
        }
        state.bytes -= bytes;
        state.messages -= messages;
        Some(Credit {
            inner: Arc::clone(&self.inner),
            bytes,
            messages,
        })
    }
    /// Nonblocking form for the readiness socket loop: a full lane reports
    /// `None` so the caller can keep servicing other connections instead of
    /// parking the whole reactor on one admission.
    pub(crate) fn try_reserve(&self, bytes: usize) -> Result<Option<Credit>, MailboxError> {
        if bytes > self.inner.bytes_capacity {
            return Err(MailboxError::TooLarge);
        }
        let mut state = self.inner.state.lock().map_err(|_| MailboxError::Closed)?;
        if state.closed {
            return Err(MailboxError::Closed);
        }
        let messages = usize::from(self.inner.messages_capacity > 0);
        if state.bytes < bytes || state.messages < messages {
            return Ok(None);
        }
        state.bytes -= bytes;
        state.messages -= messages;
        Ok(Some(Credit {
            inner: Arc::clone(&self.inner),
            bytes,
            messages,
        }))
    }
    pub(crate) async fn reserve(&self, bytes: usize) -> Result<Credit, MailboxError> {
        if bytes > self.inner.bytes_capacity {
            return Err(MailboxError::TooLarge);
        }
        loop {
            if let Some(credit) = self.take(bytes) {
                return Ok(credit);
            }
            // Register before the next check so a release cannot be missed.
            let released = self.inner.async_wake.notified();
            if self.inner.is_closed() {
                return Err(MailboxError::Closed);
            }
            released.await;
        }
    }
    /// Blocking admission for the synchronous socket adapter. Never called from
    /// an owner runtime; the owner runtime awaits [`Self::reserve`] instead.
    pub(crate) fn reserve_blocking(&self, bytes: usize) -> Result<Credit, MailboxError> {
        if bytes > self.inner.bytes_capacity {
            return Err(MailboxError::TooLarge);
        }
        let mut state = self.inner.state.lock().map_err(|_| MailboxError::Closed)?;
        loop {
            if state.closed {
                return Err(MailboxError::Closed);
            }
            let messages = usize::from(self.inner.messages_capacity > 0);
            if state.bytes >= bytes && state.messages >= messages {
                state.bytes -= bytes;
                state.messages -= messages;
                return Ok(Credit {
                    inner: Arc::clone(&self.inner),
                    bytes,
                    messages,
                });
            }
            state = self
                .inner
                .released
                .wait(state)
                .map_err(|_| MailboxError::Closed)?;
        }
    }
    pub(crate) fn close(&self) {
        self.inner.close();
    }
    #[cfg(test)]
    fn available(&self) -> usize {
        self.inner.available_bytes()
    }
    #[cfg(test)]
    fn slots(&self) -> usize {
        self.inner.available_messages()
    }
}

pub(crate) type ByteCredit = Credit;

pub(crate) struct Credit {
    inner: Arc<BudgetInner>,
    bytes: usize,
    messages: usize,
}
impl Credit {
    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }
}
impl Drop for Credit {
    fn drop(&mut self) {
        self.inner.release(self.bytes, self.messages);
    }
}

pub(crate) struct Envelope<T: Send> {
    value: T,
    _input: Credit,
}
impl<T: Send> Envelope<T> {
    /// The callback executes in a synchronous owner turn. Input credit remains
    /// charged until it finishes; no unaccounted `into_inner` escape is exposed.
    pub(crate) fn dispatch<R>(self, operation: impl FnOnce(T) -> R) -> R {
        let Self { value, _input } = self;
        let result = operation(value);
        drop(_input);
        result
    }
}

pub(crate) struct LaneSender<T: Send> {
    sender: mpsc::UnboundedSender<Envelope<T>>,
    bytes: ByteBudget,
}
impl<T: Send> Clone for LaneSender<T> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            bytes: self.bytes.clone(),
        }
    }
}
pub(crate) struct LaneReceiver<T: Send> {
    receiver: mpsc::UnboundedReceiver<Envelope<T>>,
    bytes: ByteBudget,
}
pub(crate) struct Admission<T: Send> {
    sender: mpsc::UnboundedSender<Envelope<T>>,
    credit: Credit,
}
impl<T: Send> Admission<T> {
    /// Construct retained work only after both byte and message admission.
    pub(crate) fn send_with(self, prepare: impl FnOnce() -> T) {
        let value = prepare();
        let _ = self.sender.send(Envelope {
            value,
            _input: self.credit,
        });
    }
}
pub(crate) fn lane<T: Send>(
    messages: usize,
    bytes: usize,
) -> Result<(LaneSender<T>, LaneReceiver<T>), MailboxError> {
    if messages == 0 {
        return Err(MailboxError::InvalidCapacity);
    }
    let budget = ByteBudget::with_messages(bytes, messages)?;
    let (sender, receiver) = mpsc::unbounded_channel();
    Ok((
        LaneSender {
            sender,
            bytes: budget.clone(),
        },
        LaneReceiver {
            receiver,
            bytes: budget,
        },
    ))
}
impl<T: Send> LaneSender<T> {
    #[cfg(test)]
    pub(crate) async fn reserve(&self, bytes: usize) -> Result<Admission<T>, MailboxError> {
        if bytes > self.bytes.inner.bytes_capacity {
            return Err(MailboxError::TooLarge);
        }
        let credit = self.bytes.reserve(bytes).await?;
        Ok(Admission {
            sender: self.sender.clone(),
            credit,
        })
    }

    /// Synchronous form of [`Self::reserve`] for socket threads and the Store
    /// compatibility bridge. Bytes and the message slot are taken atomically.
    pub(crate) fn reserve_blocking(&self, bytes: usize) -> Result<Admission<T>, MailboxError> {
        let credit = self.bytes.reserve_blocking(bytes)?;
        Ok(Admission {
            sender: self.sender.clone(),
            credit,
        })
    }

    /// Nonblocking admission used by deferred owner batches. `None` means the
    /// lane is full, not closed; the caller retains the work and retries.
    pub(crate) fn try_reserve(&self, bytes: usize) -> Result<Option<Admission<T>>, MailboxError> {
        Ok(self.bytes.try_reserve(bytes)?.map(|credit| Admission {
            sender: self.sender.clone(),
            credit,
        }))
    }

    #[cfg(test)]
    fn slots(&self) -> usize {
        self.bytes.slots()
    }
}
impl<T: Send> LaneSender<T> {
    /// Close admission for new work; admitted messages are still delivered.
    pub(crate) fn close(&self) {
        self.bytes.close();
    }
}

impl<T: Send> LaneReceiver<T> {
    pub(crate) fn poll_recv(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Envelope<T>>> {
        self.receiver.poll_recv(cx)
    }
    pub(crate) fn close(&mut self) {
        self.receiver.close();
        self.bytes.close();
    }
}
impl<T: Send> Drop for LaneReceiver<T> {
    fn drop(&mut self) {
        self.bytes.close();
    }
}

/// Separate lanes ensure saturated/fenced DATA cannot consume CONTROL capacity.
pub(crate) struct OwnerSender<D: Send, C: Send> {
    pub(crate) data: LaneSender<D>,
    pub(crate) control: LaneSender<C>,
}
impl<D: Send, C: Send> Clone for OwnerSender<D, C> {
    fn clone(&self) -> Self {
        Self {
            data: self.data.clone(),
            control: self.control.clone(),
        }
    }
}
pub(crate) struct OwnerInbox<D: Send, C: Send> {
    data: LaneReceiver<D>,
    control: LaneReceiver<C>,
    data_closed: bool,
    control_closed: bool,
}
pub(crate) enum Incoming<D: Send, C: Send> {
    Data(Envelope<D>),
    Control(Envelope<C>),
}
pub(crate) type MailboxParts<D, C> = (OwnerSender<D, C>, OwnerInbox<D, C>);

pub(crate) fn owner_mailbox<D: Send, C: Send>(
    data_messages: usize,
    data_bytes: usize,
    control_messages: usize,
    control_bytes: usize,
) -> Result<MailboxParts<D, C>, MailboxError> {
    let (data, data_receiver) = lane(data_messages, data_bytes)?;
    let (control, control_receiver) = lane(control_messages, control_bytes)?;
    Ok((
        OwnerSender { data, control },
        OwnerInbox {
            data: data_receiver,
            control: control_receiver,
            data_closed: false,
            control_closed: false,
        },
    ))
}
impl<D: Send, C: Send> OwnerInbox<D, C> {
    pub(crate) async fn next(&mut self, data_enabled: bool) -> Option<Incoming<D, C>> {
        std::future::poll_fn(|cx| {
            use std::task::Poll;
            if !self.control_closed {
                match self.control.poll_recv(cx) {
                    Poll::Ready(Some(message)) => {
                        return Poll::Ready(Some(Incoming::Control(message)));
                    }
                    Poll::Ready(None) => self.control_closed = true,
                    Poll::Pending => {}
                }
            }
            if data_enabled && !self.data_closed {
                match self.data.poll_recv(cx) {
                    Poll::Ready(Some(message)) => {
                        return Poll::Ready(Some(Incoming::Data(message)));
                    }
                    Poll::Ready(None) => self.data_closed = true,
                    Poll::Pending => {}
                }
            }
            if self.control_closed && (self.data_closed || !data_enabled) {
                Poll::Ready(None)
            } else {
                Poll::Pending
            }
        })
        .await
    }
    pub(crate) fn close_data(&mut self) {
        self.data.close();
    }
}

/// The charge is retained while the owned result remains in the reply slot or
/// caller's reply guard. Implementations must include retained heap capacity.
pub(crate) trait RetainedBytes: Send {
    fn retained_bytes(&self) -> Option<usize>;
}
impl RetainedBytes for Vec<u8> {
    fn retained_bytes(&self) -> Option<usize> {
        Some(self.capacity())
    }
}
impl RetainedBytes for () {
    fn retained_bytes(&self) -> Option<usize> {
        Some(0)
    }
}
impl RetainedBytes for bool {
    fn retained_bytes(&self) -> Option<usize> {
        Some(0)
    }
}
impl RetainedBytes for usize {
    fn retained_bytes(&self) -> Option<usize> {
        Some(0)
    }
}
impl RetainedBytes for u64 {
    fn retained_bytes(&self) -> Option<usize> {
        Some(0)
    }
}
impl<T: RetainedBytes, E: RetainedBytes> RetainedBytes for Result<T, E> {
    fn retained_bytes(&self) -> Option<usize> {
        match self {
            Ok(value) => value.retained_bytes(),
            Err(error) => error.retained_bytes(),
        }
    }
}
impl RetainedBytes for super::XReadGroupOutcome {
    fn retained_bytes(&self) -> Option<usize> {
        Some(0)
    }
}
impl RetainedBytes for super::blocking::PopOutcome {
    fn retained_bytes(&self) -> Option<usize> {
        Some(0)
    }
}
impl RetainedBytes for super::collection_commands::ListPopOutcome {
    fn retained_bytes(&self) -> Option<usize> {
        Some(0)
    }
}
impl RetainedBytes for super::collection_commands::ListMoveOutcome {
    fn retained_bytes(&self) -> Option<usize> {
        Some(0)
    }
}
impl RetainedBytes for super::collection_commands::ZSetPopOutcome {
    fn retained_bytes(&self) -> Option<usize> {
        Some(0)
    }
}
impl RetainedBytes for super::streams::XReadOutcome {
    fn retained_bytes(&self) -> Option<usize> {
        Some(0)
    }
}
impl RetainedBytes for (u64, u64) {
    fn retained_bytes(&self) -> Option<usize> {
        Some(0)
    }
}
impl RetainedBytes for &'static [u8] {
    fn retained_bytes(&self) -> Option<usize> {
        Some(0)
    }
}
impl RetainedBytes for (Vec<super::data::StreamId>, super::streams::XReadOutcome) {
    fn retained_bytes(&self) -> Option<usize> {
        Some(0)
    }
}
impl<T: RetainedBytes> RetainedBytes for Option<T> {
    fn retained_bytes(&self) -> Option<usize> {
        self.as_ref().and_then(RetainedBytes::retained_bytes)
    }
}
impl RetainedBytes for std::io::Error {
    fn retained_bytes(&self) -> Option<usize> {
        Some(0)
    }
}
impl RetainedBytes for MailboxError {
    fn retained_bytes(&self) -> Option<usize> {
        Some(0)
    }
}

pub(crate) struct ReplyValue<T: RetainedBytes> {
    value: T,
    _credit: Credit,
}
impl<T: RetainedBytes> Deref for ReplyValue<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}
impl<T: RetainedBytes> ReplyValue<T> {
    pub(crate) fn into_inner(self) -> T {
        let Self { value, _credit } = self;
        value
    }
}
pub(crate) struct ReplySender<T: RetainedBytes> {
    sender: oneshot::Sender<Result<ReplyValue<T>, MailboxError>>,
    credit: Credit,
}
pub(crate) struct ReplyReceiver<T: RetainedBytes> {
    receiver: oneshot::Receiver<Result<ReplyValue<T>, MailboxError>>,
}
pub(crate) fn reply_channel<T: RetainedBytes>(
    credit: Credit,
) -> (ReplySender<T>, ReplyReceiver<T>) {
    let (sender, receiver) = oneshot::channel();
    (ReplySender { sender, credit }, ReplyReceiver { receiver })
}
impl<T: RetainedBytes> ReplySender<T> {
    pub(crate) fn is_closed(&self) -> bool {
        self.sender.is_closed()
    }
    pub(crate) fn send(self, value: T) -> Result<(), MailboxError> {
        let valid = value
            .retained_bytes()
            .is_some_and(|bytes| bytes <= self.credit.bytes());
        if !valid {
            let _ = self.sender.send(Err(MailboxError::ReplyTooLarge));
            return Err(MailboxError::ReplyTooLarge);
        }
        self.sender
            .send(Ok(ReplyValue {
                value,
                _credit: self.credit,
            }))
            .map_err(|_| MailboxError::Closed)
    }
}
impl<T: RetainedBytes> ReplyReceiver<T> {
    #[cfg(test)]
    pub(crate) async fn recv(self) -> Result<ReplyValue<T>, MailboxError> {
        self.receiver.await.map_err(|_| MailboxError::Closed)?
    }
    /// Blocking form for the synchronous socket adapter. Never called from an
    /// owner runtime task; those await [`Self::recv`].
    pub(crate) fn blocking_recv(self) -> Result<ReplyValue<T>, MailboxError> {
        self.receiver
            .blocking_recv()
            .map_err(|_| MailboxError::Closed)?
    }
    /// Nonblocking readiness check for a deferred owner batch. The reply credit
    /// is released when the returned value is consumed or dropped.
    pub(crate) fn try_recv(&mut self) -> Result<Option<ReplyValue<T>>, MailboxError> {
        match self.receiver.try_recv() {
            Ok(Ok(value)) => Ok(Some(value)),
            Ok(Err(error)) => Err(error),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => Ok(None),
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => Err(MailboxError::Closed),
        }
    }
}

#[cfg(test)]
mod tests;
