//! Linux completion TCP on one local socket/data owner. Kernel buffers are
//! owned Vecs; no keyspace borrow crosses an I/O await or another thread.

#[cfg(test)]
#[path = "compio_io_tests.rs"]
mod tests;

use super::*;
use crate::engine::{BridgeInbox, ShardExecutor};
use compio::buf::{BufResult, IntoInner, Slice};
use compio::io::{AsyncRead, AsyncWrite};
use compio::net::{TcpListener as CompletionListener, TcpStream as CompletionStream};
use compio::runtime::{CancelToken, FutureExt, JoinHandle as Task, Runtime};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::future::{Future, poll_fn};
use std::os::fd::AsFd;
use std::pin::{Pin, pin};
use std::rc::Rc;
use std::sync::{Mutex, mpsc};
use std::task::Poll;
use tokio::sync::{Semaphore, oneshot};

const MAX_READ: usize = 16 * 1024;
const OUTPUT_QUANTUM: usize = 64 * 1024;
const COMMAND_QUANTUM: usize = 64;
const CONTROL_TICK: Duration = Duration::from_millis(1);
const STOP_TICK: Duration = Duration::from_millis(10);
const MAINTENANCE_TICK: Duration = Duration::from_millis(100);
type Controls = Arc<Mutex<HashMap<u64, TcpStream>>>;
type ColdJob = Box<dyn FnOnce() + Send>;

pub(super) fn available() -> bool {
    // A temporary ring can leave a delayed kernel task notification on its
    // creating thread, interrupting that caller's unrelated blocking socket IO.
    // Isolate capability detection in a joined startup-only thread. The actual
    // transport runtime is created and used on its socket/data owner below.
    thread::Builder::new()
        .name("rillcache-uring-probe".into())
        .spawn(|| Runtime::new().is_ok_and(|runtime| runtime.driver_type().is_iouring()))
        .is_ok_and(|probe| probe.join().unwrap_or(false))
}

fn close_controls(controls: &Controls) {
    for control in controls
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .values()
    {
        let _ = control.shutdown(Shutdown::Both);
    }
}

struct FailureStop {
    stopping: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
    controls: Controls,
}
impl Drop for FailureStop {
    fn drop(&mut self) {
        if thread::panicking() {
            self.failed.store(true, Ordering::Release);
            self.stopping.store(true, Ordering::Release);
            close_controls(&self.controls);
        }
    }
}

struct Shared {
    config: Config,
    store: Arc<Store>,
    stopping: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
    controls: Controls,
    cold: mpsc::SyncSender<ColdJob>,
    cold_permit: Arc<Semaphore>,
}
impl Shared {
    fn stopped(&self) -> bool {
        self.stopping.load(Ordering::Acquire) || self.store.shutdown_requested()
    }
    fn stop(&self) {
        self.stopping.store(true, Ordering::Release);
        close_controls(&self.controls);
    }
    fn fail(&self) {
        self.failed.store(true, Ordering::Release);
        self.stop();
    }
    fn guard(&self) -> FailureStop {
        FailureStop {
            stopping: Arc::clone(&self.stopping),
            failed: Arc::clone(&self.failed),
            controls: Arc::clone(&self.controls),
        }
    }
    fn shutdown(&self, id: u64, how: Shutdown) {
        if let Some(control) = self
            .controls
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&id)
        {
            let _ = control.shutdown(how);
        }
    }

    // Tokio's semaphore/oneshot futures are runtime-independent. Only this
    // bounded cold path moves Session/output, never the local executor or Data.
    async fn cold(
        &self,
        session: &mut Session,
        output: &mut Vec<u8>,
        operation: Cold<'_>,
    ) -> io::Result<Option<Action>> {
        let permit = Arc::clone(&self.cold_permit)
            .acquire_owned()
            .await
            .map_err(|_| io::Error::other("completion cold service closed"))?;
        if self.stopped() {
            return Err(io::ErrorKind::Interrupted.into());
        }
        let frame = match operation {
            Cold::Command(frame) => Some(frame.to_vec()),
            Cold::Paused => None,
        };
        let mut owned_session = std::mem::take(session);
        let mut owned_output = std::mem::take(output);
        let store = Arc::clone(&self.store);
        let limits = self.config.limits;
        let (send, receive) = oneshot::channel();
        let job = Box::new(move || {
            let _permit = permit;
            let action = if let Some(frame) = frame {
                dispatch_request(&frame, limits, |args| {
                    owned_session.execute(&store, args, &mut owned_output)
                })
                .expect("validated cold frame")
                .map(|(action, _)| action)
            } else {
                owned_session.poll_paused(&store).map(|(reply, action)| {
                    owned_output.extend_from_slice(&reply);
                    action
                })
            };
            let _ = send.send((owned_session, owned_output, action));
        });
        if self.cold.try_send(job).is_err() {
            self.fail();
            return Err(io::Error::other("completion cold service unavailable"));
        }
        match receive.await {
            Ok((returned, bytes, action)) => {
                *session = returned;
                *output = bytes;
                Ok(action)
            }
            Err(_) => {
                self.fail();
                Err(io::Error::other("completion cold command failed"))
            }
        }
    }
}

enum Cold<'a> {
    Command(&'a [u8]),
    Paused,
}

struct ClientState {
    activity: Cell<Instant>,
    writing: Cell<Option<Instant>>,
    asynchronous: Cell<bool>,
    closed: Cell<bool>,
}
struct Local {
    shared: Arc<Shared>,
    clients: RefCell<HashMap<u64, Rc<ClientState>>>,
}
struct Registration {
    id: u64,
    local: Rc<Local>,
}
impl Drop for Registration {
    fn drop(&mut self) {
        let shared = &self.local.shared;
        if let Some(control) = shared
            .controls
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&self.id)
        {
            let _ = control.shutdown(Shutdown::Both);
        }
        shared.store.pubsub_remove_client(self.id);
        shared.store.unregister_client(self.id);
        self.local.clients.borrow_mut().remove(&self.id);
    }
}

async fn yield_turn() {
    let mut yielded = false;
    poll_fn(|cx| {
        if yielded {
            Poll::Ready(())
        } else {
            yielded = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    })
    .await
}

// A control tick borrows a pending completion future; it never owns/drops it.
// In particular, cancelling an accept on every tick can discard a connection
// already accepted by the kernel but not yet delivered to this executor.
async fn ready_or_tick<F: Future>(mut operation: Pin<&mut F>, tick: Duration) -> Option<F::Output> {
    let mut timer = pin!(compio::time::sleep(tick));
    poll_fn(|cx| {
        if let Poll::Ready(value) = operation.as_mut().poll(cx) {
            return Poll::Ready(Some(value));
        }
        timer.as_mut().poll(cx).map(|_| None)
    })
    .await
}

async fn control_loop(
    local: Rc<Local>,
    executor: Rc<ShardExecutor>,
    mut inbox: Option<BridgeInbox>,
    finished: Rc<Cell<bool>>,
) {
    let _failure = local.shared.guard();
    let mut maintenance = Instant::now();
    let mut stopped = false;
    while !finished.get() {
        if local.shared.failed.load(Ordering::Acquire) {
            inbox.take();
        }
        if let Some(inbox) = &inbox {
            inbox.drain(executor.as_ref());
        }
        if local.shared.stopped() {
            // Protocol SHUTDOWN must flush already-produced pipeline replies
            // before accept-loop cleanup closes all controls (no new stop ACK).
            if local.shared.stopping.load(Ordering::Acquire) && !stopped {
                local.shared.stop();
                stopped = true;
            }
        } else {
            let now = Instant::now();
            for (&id, state) in local.clients.borrow().iter() {
                // Waiting subscriptions/blocked/paused sessions retain the
                // existing timeout exemption, but an actual stalled output
                // still has the configured total write deadline.
                if state.writing.get().is_none() && state.asynchronous.get() {
                    continue;
                }
                let since = state.writing.get().unwrap_or_else(|| state.activity.get());
                if now.duration_since(since) >= local.shared.config.idle_timeout
                    && !state.closed.replace(true)
                {
                    local.shared.shutdown(id, Shutdown::Both);
                }
            }
            if now.duration_since(maintenance) >= MAINTENANCE_TICK {
                executor.purge_expired(128);
                maintenance = now;
            }
        }
        compio::time::sleep(CONTROL_TICK).await;
    }
    if let Some(inbox) = inbox {
        inbox.drain(executor.as_ref());
    }
}

async fn reap(tasks: &mut Vec<Task<io::Result<()>>>) -> io::Result<()> {
    let mut index = 0;
    while index < tasks.len() {
        if tasks[index].is_finished() {
            // Per-connection network errors are isolated; task panic/cancellation
            // is a server failure, already published by the unwind guard.
            let _ = tasks.swap_remove(index).await.map_err(io::Error::from)?;
        } else {
            index += 1;
        }
    }
    Ok(())
}

pub(super) fn serve(
    listener: TcpListener,
    config: Config,
    store: Arc<Store>,
    stopping: Arc<AtomicBool>,
) -> io::Result<()> {
    serve_with_cold_service(listener, config, store, stopping, |receive| {
        while let Ok(job) = receive.recv() {
            job();
        }
    })
}

fn serve_with_cold_service<F>(
    listener: TcpListener,
    config: Config,
    store: Arc<Store>,
    stopping: Arc<AtomicBool>,
    service: F,
) -> io::Result<()>
where
    F: FnOnce(mpsc::Receiver<ColdJob>) + Send + 'static,
{
    let runtime = Runtime::new()?;
    if !runtime.driver_type().is_iouring() {
        return Err(io::Error::other("completion driver is not io_uring"));
    }
    listener.set_nonblocking(false)?;
    let _ = writeln!(
        io::stderr().lock(),
        "rillcache: plain TCP compio io_uring; one local socket/data owner"
    );
    let (send, receive) = mpsc::sync_channel::<ColdJob>(1);
    let shared = Arc::new(Shared {
        config,
        store,
        stopping,
        failed: Arc::new(AtomicBool::new(false)),
        controls: Arc::new(Mutex::new(HashMap::new())),
        cold: send,
        cold_permit: Arc::new(Semaphore::new(1)),
    });
    let failure = shared.guard();
    let cold = thread::Builder::new()
        .name("rillcache-uring-cold".into())
        .spawn(move || {
            let _failure = failure;
            service(receive);
        })?;
    let result = runtime.block_on(async {
        let _failure = shared.guard();
        let (executor, inbox) = shared
            .store
            .install_local()
            .ok_or_else(|| io::Error::other("completion transport needs local owner"))?;
        let executor = Rc::new(executor);
        let local = Rc::new(Local {
            shared: Arc::clone(&shared),
            clients: RefCell::new(HashMap::new()),
        });
        let finished = Rc::new(Cell::new(false));
        let driver = compio::runtime::spawn(control_loop(
            Rc::clone(&local),
            Rc::clone(&executor),
            Some(inbox),
            Rc::clone(&finished),
        ));
        let result = accept_loop(listener, Rc::clone(&local), Rc::clone(&executor), &driver).await;
        local.shared.stop();
        // A cancelled cold receiver may leave a job borrowing the Store bridge.
        // Keep the driver/executor alive until that admitted job releases credit.
        let drained = shared.cold_permit.acquire().await;
        finished.set(true);
        let joined = driver.await.map_err(io::Error::from);
        let drained = drained
            .map(|_| ())
            .map_err(|_| io::Error::other("cold service drain failed"));
        result.and(drained).and(joined)
    });
    shared.stop();
    drop(runtime); // kernel ring closes before any retained operation buffers free
    drop(shared); // close cold admission only after clients/bridge have joined
    result.and(join(cold))
}

async fn accept_loop(
    listener: TcpListener,
    local: Rc<Local>,
    executor: Rc<ShardExecutor>,
    driver: &Task<()>,
) -> io::Result<()> {
    let listener = CompletionListener::from_std(listener)?;
    let shared = &local.shared;
    let mut tasks = Vec::new();
    let result = async {
        'admission: loop {
            let mut accepting = pin!(listener.accept());
            let (stream, peer) = loop {
                reap(&mut tasks).await?;
                if driver.is_finished() {
                    return Err(io::Error::other("completion control task stopped"));
                }
                if shared.failed.load(Ordering::Acquire) {
                    return Err(io::Error::other("completion task failed"));
                }
                if shared.stopped() {
                    return Ok(());
                }
                match ready_or_tick(accepting.as_mut(), STOP_TICK).await {
                    None => continue,
                    Some(Ok(accepted)) => break accepted,
                    Some(Err(error)) if error.kind() == io::ErrorKind::Interrupted => {
                        continue 'admission;
                    }
                    Some(Err(error)) => return Err(error),
                }
            };
            shared.store.record_connection();
            stream.set_nodelay(true)?;
            if shared.store.connected_clients().max(tasks.len()) >= shared.store.max_clients() {
                shared.store.record_rejected_connection();
                // Rejection is not admitted to the task set; only one such
                // bounded accept/reply operation exists at a time.
                let mut socket = &stream;
                let _ = compio::time::timeout(
                    STOP_TICK,
                    socket.write(b"-ERR max number of clients reached\r\n".to_vec()),
                )
                .await;
                let _ = stream.close().await;
                continue;
            }
            let control = TcpStream::from(stream.as_fd().try_clone_to_owned()?);
            let (sender, pubsub) = sync_channel(Store::pubsub_queue_capacity());
            let mut session = Session::new();
            session.set_peer(peer);
            session.set_pubsub_sender(sender);
            let id = session.id();
            let registration = Registration {
                id,
                local: Rc::clone(&local),
            };
            shared
                .store
                .register_client(id, peer, stream.local_addr()?, control.try_clone()?);
            shared
                .controls
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .insert(id, control);
            let state = Rc::new(ClientState {
                activity: Cell::new(Instant::now()),
                writing: Cell::new(None),
                asynchronous: Cell::new(false),
                closed: Cell::new(false),
            });
            local.clients.borrow_mut().insert(id, Rc::clone(&state));
            let client = Rc::clone(&local);
            let owner = Rc::clone(&executor);
            tasks.push(compio::runtime::spawn(async move {
                let _failure = client.shared.guard();
                let _registration = registration;
                let result =
                    serve_connection(&stream, session, pubsub, &client, owner, state).await;
                client.shared.shutdown(id, Shutdown::Both);
                let closed = stream.close().await;
                result.and(closed)
            }));
            if tasks.len().is_multiple_of(COMMAND_QUANTUM) {
                yield_turn().await;
            }
        }
    }
    .await;
    if shared.store.shutdown_requested() && !shared.stopping.load(Ordering::Acquire) {
        let began = Instant::now();
        while !shared.failed.load(Ordering::Acquire)
            && began.elapsed() < STOP_TICK
            && local
                .clients
                .borrow()
                .values()
                .any(|state| state.writing.get().is_some())
        {
            compio::time::sleep(CONTROL_TICK).await;
        }
    }
    shared.stop();
    // Close/cancel socket operations before awaiting handles; never detach or
    // drop pending client tasks as a substitute for joined shutdown.
    let mut result = result;
    for task in tasks {
        result = result.and(task.await.map(|_| ()).map_err(io::Error::from));
    }
    result.and(listener.close().await)
}

async fn write_owned(
    stream: &CompletionStream,
    output: &mut Vec<u8>,
    state: &ClientState,
) -> io::Result<()> {
    state.writing.set(Some(Instant::now()));
    let mut buffer = std::mem::take(output);
    let mut sent = 0;
    let mut result = Ok(());
    while sent < buffer.len() {
        let end = buffer.len().min(sent + OUTPUT_QUANTUM);
        let mut socket = stream;
        let BufResult(written, returned) = socket
            .write(compio::buf::IoBuf::slice(buffer, sent..end))
            .await;
        buffer = returned.into_inner();
        let size = match written {
            Ok(0) => {
                result = Err(io::ErrorKind::WriteZero.into());
                break;
            }
            Ok(size) => size,
            Err(error) => {
                result = Err(error);
                break;
            }
        };
        sent += size;
        state.activity.set(Instant::now());
        if sent < buffer.len() {
            yield_turn().await;
        }
    }
    state.writing.set(None);
    *output = buffer;
    if result.is_ok() {
        output.clear();
    }
    result
}

async fn close_after_reply(
    stream: &CompletionStream,
    output: &mut Vec<u8>,
    local: &Local,
    state: &ClientState,
    id: u64,
) -> io::Result<()> {
    write_owned(stream, output, state).await?;
    local.shared.shutdown(id, Shutdown::Write);
    // Only discard buffers are used with a timeout: cancellation can retain an
    // owned buffer until CQE, so the normal input buffer must NEVER be dropped
    // on an asynchronous tick. The final close awaits retained kernel refs.
    let _ = compio::time::timeout(STOP_TICK, async {
        let mut discard = Vec::with_capacity(MAX_READ);
        loop {
            let mut socket = stream;
            let BufResult(result, returned) = socket.read(discard).await;
            discard = returned;
            if result? == 0 {
                return Ok::<(), io::Error>(());
            }
            discard.clear();
        }
    })
    .await;
    Ok(())
}

fn asynchronous(session: &Session) -> bool {
    session.is_subscribed()
        || session.is_monitoring()
        || session.is_blocked()
        || session.is_paused()
}

async fn observe(
    session: &mut Session,
    output: &mut Vec<u8>,
    pubsub: &Receiver<Vec<u8>>,
    local: &Local,
    executor: &ShardExecutor,
) -> io::Result<Option<Action>> {
    for _ in 0..COMMAND_QUANTUM {
        match pubsub.try_recv() {
            Ok(frame) => {
                output.extend_from_slice(&frame);
                if output.len() >= OUTPUT_QUANTUM {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    if session.is_blocked()
        && let Some(reply) = session.poll_blocked_on(executor)
    {
        output.extend_from_slice(&reply);
    }
    if session.is_paused() {
        local.shared.cold(session, output, Cold::Paused).await
    } else {
        Ok(None)
    }
}

enum Next {
    Read(io::Result<usize>, Vec<u8>),
    Buffered(Vec<u8>),
    Closed,
}

#[allow(clippy::too_many_arguments)]
async fn wait_read<F>(
    future: F,
    cancel: Option<CancelToken>,
    stream: &CompletionStream,
    session: &mut Session,
    output: &mut Vec<u8>,
    pubsub: &Receiver<Vec<u8>>,
    local: &Local,
    executor: &ShardExecutor,
    state: &ClientState,
    id: u64,
) -> io::Result<Next>
where
    F: Future<Output = BufResult<usize, Slice<Vec<u8>>>>,
{
    let mut read = pin!(future);
    loop {
        state.asynchronous.set(asynchronous(session));
        if !asynchronous(session) {
            let BufResult(result, returned) = read.await;
            return Ok(Next::Read(result, returned.into_inner()));
        }
        let event = ready_or_tick(read.as_mut(), CONTROL_TICK).await;
        if let Some(BufResult(result, returned)) = event {
            return Ok(Next::Read(result, returned.into_inner()));
        }
        if local.shared.stopped() || state.closed.get() {
            local.shared.shutdown(id, Shutdown::Both);
            let _ = read.await;
            return Ok(Next::Closed);
        }
        let action = observe(session, output, pubsub, local, executor).await?;
        state.asynchronous.set(asynchronous(session));
        if !output.is_empty() {
            write_owned(stream, output, state).await?;
        }
        if action == Some(Action::Close) {
            local.shared.shutdown(id, Shutdown::Both);
            let _ = read.await;
            return Ok(Next::Closed);
        }
        if !asynchronous(session)
            && let Some(token) = cancel.as_ref()
        {
            // Only a bounded single-read token is used, never a connection-long
            // token retaining weak operation keys for millions of requests.
            token.clone().cancel();
            let BufResult(result, returned) = read.await;
            return Ok(match result {
                Err(error) if error.raw_os_error() == Some(125) => {
                    Next::Buffered(returned.into_inner())
                } // Linux ECANCELED
                result => Next::Read(result, returned.into_inner()),
            });
        }
    }
}

async fn serve_connection(
    stream: &CompletionStream,
    mut session: Session,
    pubsub: Receiver<Vec<u8>>,
    local: &Local,
    executor: Rc<ShardExecutor>,
    state: Rc<ClientState>,
) -> io::Result<()> {
    let shared = &local.shared;
    let id = session.id();
    let limits = shared.config.limits;
    let mut input = Vec::with_capacity(8192.min(limits.max_frame_bytes));
    let mut output = Vec::with_capacity(8192);
    loop {
        if shared.stopped() || state.closed.get() {
            return Ok(());
        }
        let action = observe(&mut session, &mut output, &pubsub, local, executor.as_ref()).await?;
        if !output.is_empty() {
            write_owned(stream, &mut output, &state).await?;
        }
        if action == Some(Action::Close) {
            return Ok(());
        }
        if !session.is_blocked() && !session.is_paused() {
            let mut consumed = 0;
            let mut need_input = false;
            for _ in 0..COMMAND_QUANTUM {
                if shared.stopped() {
                    return Ok(());
                }
                let mut cold = false;
                match dispatch_request(&input[consumed..], limits, |args| {
                    cold = args
                        .first()
                        .is_some_and(|name| name.eq_ignore_ascii_case(b"ACL"));
                    if cold || args.is_empty() {
                        Action::Continue
                    } else {
                        session.execute_on(executor.as_ref(), args, &mut output)
                    }
                }) {
                    Ok(Some((mut action, used))) => {
                        if cold {
                            action = shared
                                .cold(
                                    &mut session,
                                    &mut output,
                                    Cold::Command(&input[consumed..consumed + used]),
                                )
                                .await?
                                .expect("complete cold command");
                        }
                        consumed += used;
                        if action == Action::Close {
                            write_owned(stream, &mut output, &state).await?;
                            return Ok(());
                        }
                        if session.is_blocked()
                            || session.is_paused()
                            || output.len() >= OUTPUT_QUANTUM
                        {
                            break;
                        }
                    }
                    Ok(None) => {
                        need_input = true;
                        break;
                    }
                    Err(error) => {
                        write!(output, "-ERR Protocol error: {error}\r\n")?;
                        return close_after_reply(stream, &mut output, local, &state, id).await;
                    }
                }
            }
            if consumed == input.len() {
                input.clear();
            } else if consumed > 0 {
                input.drain(..consumed);
            }
            if !output.is_empty() {
                write_owned(stream, &mut output, &state).await?;
            }
            if !need_input && !input.is_empty() && !session.is_blocked() && !session.is_paused() {
                yield_turn().await;
                continue;
            }
        }
        let room = limits
            .max_frame_bytes
            .saturating_sub(input.len())
            .min(MAX_READ);
        if room == 0 {
            output.extend_from_slice(b"-ERR Protocol error: request frame limit exceeded\r\n");
            return close_after_reply(stream, &mut output, local, &state, id).await;
        }
        let begin = input.len();
        input.reserve(room);
        let mut socket = stream;
        let future = socket.read(compio::buf::IoBuf::slice(input, begin..begin + room));
        let next = if asynchronous(&session) && begin > 0 {
            let cancel = CancelToken::new();
            wait_read(
                future.with_cancel(cancel.clone()),
                Some(cancel),
                stream,
                &mut session,
                &mut output,
                &pubsub,
                local,
                executor.as_ref(),
                &state,
                id,
            )
            .await?
        } else {
            wait_read(
                future,
                None,
                stream,
                &mut session,
                &mut output,
                &pubsub,
                local,
                executor.as_ref(),
                &state,
                id,
            )
            .await?
        };
        match next {
            Next::Closed => return Ok(()),
            Next::Buffered(returned) => input = returned,
            Next::Read(result, returned) => {
                input = returned;
                match result {
                    Ok(0) => {
                        if !input.is_empty() && !session.is_blocked() && !session.is_paused() {
                            output.extend_from_slice(
                                b"-ERR Protocol error: unexpected end of stream\r\n",
                            );
                            write_owned(stream, &mut output, &state).await?;
                        }
                        return Ok(());
                    }
                    Ok(_) => state.activity.set(Instant::now()),
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    Err(error) => return Err(error),
                }
            }
        }
    }
}
