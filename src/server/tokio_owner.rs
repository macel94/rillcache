//! Bounded asynchronous transport turns. Only owned Session/buffers cross a
//! thread; one runtime-local ShardExecutor still owns every keyspace operation.

use super::*;

#[cfg(test)]
#[path = "tokio_owner_tests.rs"]
mod tests;

// Moving a hot Session must not construct a throwaway Session/new global client
// ID on every batch. None exists only while its owned transport turn is awaited.
pub(super) struct SessionSlot {
    session: Option<Session>,
    stop_reply: Option<StopReply>,
}
impl SessionSlot {
    pub(super) fn new(session: Session) -> Self {
        Self {
            session: Some(session),
            stop_reply: None,
        }
    }
    pub(super) fn take(&mut self) -> Session {
        self.session.take().expect("session already in flight")
    }
    pub(super) fn restore(&mut self, session: Session) {
        self.session = Some(session);
    }
    pub(super) fn hold_stop_reply(&mut self, guard: StopReply) {
        self.stop_reply = Some(guard);
    }
    pub(super) fn reply_timeout(&self, configured: Duration) -> Duration {
        if self.stop_reply.is_some() {
            POLL_TICK.min(configured)
        } else {
            configured
        }
    }
}
impl std::ops::Deref for SessionSlot {
    type Target = Session;
    fn deref(&self) -> &Session {
        self.session
            .as_ref()
            .expect("session turn is owned by dispatcher")
    }
}
impl std::ops::DerefMut for SessionSlot {
    fn deref_mut(&mut self) -> &mut Session {
        self.session
            .as_mut()
            .expect("session turn is owned by dispatcher")
    }
}

pub(super) const CAPACITY: usize = TOKIO_OWNER_TURN_CAPACITY;
pub(super) type Job = Box<dyn FnOnce(&ShardExecutor) + Send>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    Read,
    Buffered,
    Close,
    ProtocolError,
    Cold(usize),
    Stopped,
}

#[derive(Clone, Copy)]
pub(super) enum Operation {
    Batch,
    Blocked,
}

pub(super) struct Sender {
    queue: mpsc::Sender<Job>,
    pub(super) permit: Arc<Semaphore>,
}

pub(super) fn channel() -> (Sender, mpsc::Receiver<Job>) {
    let (queue, receive) = mpsc::channel(CAPACITY);
    (
        Sender {
            queue,
            permit: Arc::new(Semaphore::new(CAPACITY)),
        },
        receive,
    )
}

impl Sender {
    pub(super) async fn run(
        &self,
        session: &mut SessionSlot,
        input: &mut Vec<u8>,
        output: &mut Vec<u8>,
        context: &Context,
        operation: Operation,
    ) -> io::Result<Outcome> {
        // Admission precedes ownership transfer; no per-command frame copy.
        let permit = Arc::clone(&self.permit)
            .acquire_owned()
            .await
            .map_err(|_| io::Error::other("async owner admission closed"))?;
        if context.stopped() {
            return Err(io::ErrorKind::Interrupted.into());
        }
        let mut session_owned = session.take();
        let mut input_owned = std::mem::take(input);
        let mut output_owned = std::mem::take(output);
        let limits = context.config.limits;
        let store = Arc::clone(&context.store);
        let stopping = Arc::clone(&context.stopping);
        let stop_replies = Arc::clone(&context.stop_replies);
        let (send, receive) = oneshot::channel();
        let job: Job = Box::new(move |executor| {
            let _permit = permit; // survives a cancelled response waiter
            if send.is_closed() {
                // Revoked before execution: release connection state/credit,
                // never run a queued data mutation after its waiter vanished.
                let id = session_owned.id();
                store.pubsub_remove_client(id);
                store.unregister_client(id);
                return;
            }
            let outcome = if stopping.load(Ordering::Acquire) || store.shutdown_requested() {
                Outcome::Stopped
            } else {
                match operation {
                    Operation::Batch => batch(
                        &mut session_owned,
                        &mut input_owned,
                        &mut output_owned,
                        limits,
                        executor,
                        &stopping,
                    ),
                    Operation::Blocked => {
                        if let Some(reply) = session_owned.poll_blocked_on(executor) {
                            output_owned.extend_from_slice(&reply);
                        }
                        Outcome::Read
                    }
                }
            };
            // Executed on runtime zero: publish ownership before its accept
            // task can observe SHUTDOWN and begin closing socket controls.
            let stop_reply = (outcome == Outcome::Close && store.shutdown_requested())
                .then(|| StopReply::new(&stop_replies));
            let id = session_owned.id();
            if send
                .send((
                    session_owned,
                    input_owned,
                    output_owned,
                    outcome,
                    stop_reply,
                ))
                .is_err()
            {
                // A late SUBSCRIBE/MONITOR result cannot leave registration
                // behind after its socket waiter/guard has disappeared.
                store.pubsub_remove_client(id);
                store.unregister_client(id);
            }
        });
        if self.queue.try_send(job).is_err() {
            context.stop();
            return Err(io::Error::other(
                "async owner stopped or admission invariant failed",
            ));
        }
        match receive.await {
            Ok((returned, bytes, replies, outcome, stop_reply)) => {
                session.restore(returned);
                if let Some(guard) = stop_reply {
                    session.hold_stop_reply(guard);
                }
                *input = bytes;
                *output = replies;
                Ok(outcome)
            }
            Err(_) => {
                context.stop();
                Err(io::Error::other("async owner task failed"))
            }
        }
    }
}

pub(super) async fn drive(
    mut receive: mpsc::Receiver<Job>,
    executor: Rc<ShardExecutor>,
    context: Arc<Context>,
    finished: Arc<AtomicBool>,
) {
    let _failure = context.failure_guard();
    while !finished.load(Ordering::Acquire) {
        if context.failed.load(Ordering::Acquire) {
            return;
        }
        match tokio::time::timeout(POLL_TICK, receive.recv()).await {
            Ok(Some(job)) => {
                job(executor.as_ref());
                // A ready mailbox must not monopolize runtime zero's sockets.
                tokio::task::yield_now().await;
            }
            Ok(None) => return,
            Err(_) => {}
        }
    }
}

pub(super) fn batch(
    session: &mut Session,
    input: &mut Vec<u8>,
    output: &mut Vec<u8>,
    limits: Limits,
    executor: &ShardExecutor,
    stopping: &AtomicBool,
) -> Outcome {
    let mut consumed = 0;
    let mut outcome = Outcome::Buffered;
    for _ in 0..COMMAND_QUANTUM {
        if stopping.load(Ordering::Acquire) {
            outcome = Outcome::Stopped;
            break;
        }
        let mut cold = false;
        match dispatch_request(&input[consumed..], limits, |args| {
            cold = cold_command(args);
            if cold || args.is_empty() {
                Action::Continue
            } else {
                session.execute_on(executor, args, output)
            }
        }) {
            Ok(Some((action, used))) => {
                if cold {
                    outcome = Outcome::Cold(used);
                    break;
                }
                consumed += used;
                if action == Action::Close {
                    outcome = Outcome::Close;
                    break;
                }
                if session.is_blocked() || session.is_paused() || output.len() >= OUTPUT_QUANTUM {
                    break;
                }
            }
            Ok(None) => {
                outcome = Outcome::Read;
                break;
            }
            Err(error) => {
                let _ = write!(output, "-ERR Protocol error: {error}\r\n");
                outcome = Outcome::ProtocolError;
                break;
            }
        }
    }
    if consumed == input.len() {
        input.clear();
    } else if consumed > 0 {
        input.drain(..consumed);
    }
    outcome
}
