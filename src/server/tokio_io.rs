//! Async transport over the published local-owner / Store compatibility APIs.

#[cfg(test)]
#[path = "tokio_io_tests.rs"]
mod tests;

use super::*;
use crate::engine::{ServerServices, ShardExecutor};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener as TokioListener, TcpStream as TokioStream};
use tokio::sync::{Semaphore, mpsc, oneshot};
use tokio::task::JoinSet;

const ROUTE_CAPACITY: usize = 32;
const COMMAND_QUANTUM: usize = 64;
const OUTPUT_QUANTUM: usize = 64 * 1024;
const MAX_READ: usize = 16 * 1024;
const POLL_TICK: Duration = Duration::from_millis(10);
const MAINTENANCE_TICK: Duration = Duration::from_millis(100);

type Controls = Arc<Mutex<HashMap<u64, TcpStream>>>;
type Tasks = JoinSet<io::Result<()>>;
type ColdJob = Box<dyn FnOnce() + Send>;

struct Accepted {
    stream: TcpStream,
    session: Session,
    pubsub: Receiver<Vec<u8>>,
    registration: ClientRegistration,
}

struct ClientRegistration {
    id: u64,
    store: Arc<ServerServices>,
    controls: Controls,
}

impl Drop for ClientRegistration {
    fn drop(&mut self) {
        self.store.pubsub_remove_client(self.id);
        self.store.unregister_client(self.id);
        if let Ok(mut controls) = self.controls.lock() {
            controls.remove(&self.id);
        }
    }
}

struct Lane {
    sender: mpsc::Sender<Accepted>,
    thread: JoinHandle<io::Result<()>>,
}

// Tokio catches task panics. Publish fail-stop while unwinding, before another
// local task can observe Data after a partially executed owner turn.
struct FailureStop {
    stopping: Arc<AtomicBool>,
    controls: Controls,
}
impl Drop for FailureStop {
    fn drop(&mut self) {
        if thread::panicking() {
            self.stopping.store(true, Ordering::Release);
            close_controls(&self.controls);
        }
    }
}

struct Context {
    config: Config,
    store: Arc<Store>,
    stopping: Arc<AtomicBool>,
    controls: Controls,
    cold: ColdExecutor,
    task_limit: usize,
}

impl Context {
    fn failure_guard(&self) -> FailureStop {
        FailureStop {
            stopping: Arc::clone(&self.stopping),
            controls: Arc::clone(&self.controls),
        }
    }

    fn stopped(&self) -> bool {
        self.stopping.load(Ordering::Acquire) || self.store.shutdown_requested()
    }

    fn stop(&self) {
        self.stopping.store(true, Ordering::Release);
        close_controls(&self.controls);
    }

    fn spawn(
        self: &Arc<Self>,
        tasks: &mut Tasks,
        accepted: Accepted,
        executor: Option<Rc<ShardExecutor>>,
    ) {
        let context = Arc::clone(self);
        tasks.spawn_local(async move {
            let _failure = context.failure_guard();
            let _registration = accepted.registration;
            let stream = TokioStream::from_std(accepted.stream)?;
            serve_connection(
                stream,
                accepted.session,
                accepted.pubsub,
                &context,
                executor,
            )
            .await
        });
    }
}

fn close_controls(controls: &Controls) {
    if let Ok(controls) = controls.lock() {
        for control in controls.values() {
            let _ = control.shutdown(Shutdown::Both);
        }
    }
}

// One cold executor, not Tokio's unbounded blocking-task queue. Its single
// permit is acquired BEFORE copying a request or moving session/output state,
// and is owned by the job until completion even if its receiver disappears.
// This keeps ACL/paused disk-capable commands off network runtimes without
// introducing a hop for ordinary commands or changing Session's epilogue.
struct ColdExecutor {
    sender: mpsc::Sender<ColdJob>,
    permit: Arc<Semaphore>,
}

enum ColdOperation<'a> {
    Command(&'a [u8]),
    Paused,
    Blocked,
    Maintenance,
}

impl ColdExecutor {
    async fn run(
        &self,
        session: &mut Session,
        output: &mut Vec<u8>,
        context: &Context,
        operation: ColdOperation<'_>,
    ) -> io::Result<Option<Action>> {
        let permit = Arc::clone(&self.permit)
            .acquire_owned()
            .await
            .map_err(|_| io::Error::other("cold executor stopped"))?;
        if context.stopped() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "server stopping",
            ));
        }
        let frame = match operation {
            ColdOperation::Command(frame) => Some(frame.to_vec()),
            ColdOperation::Paused | ColdOperation::Blocked | ColdOperation::Maintenance => None,
        };
        let blocked = matches!(operation, ColdOperation::Blocked);
        let maintenance = matches!(operation, ColdOperation::Maintenance);
        let mut owned_session = std::mem::take(session);
        let mut owned_output = std::mem::take(output);
        let store = Arc::clone(&context.store);
        let limits = context.config.limits;
        let (sender, receiver) = oneshot::channel();
        let job = Box::new(move || {
            let _permit = permit;
            let action = if let Some(frame) = frame {
                dispatch_request(&frame, limits, |args| {
                    owned_session.execute(&store, args, &mut owned_output)
                })
                .expect("previously validated cold frame")
                .map(|(action, _)| action)
            } else if maintenance {
                store.purge_expired(128);
                None
            } else if blocked {
                owned_session.poll_blocked(&store).map(|reply| {
                    owned_output.extend_from_slice(&reply);
                    Action::Continue
                })
            } else {
                owned_session.poll_paused(&store).map(|(reply, action)| {
                    owned_output.extend_from_slice(&reply);
                    action
                })
            };
            let _ = sender.send((owned_session, owned_output, action));
        });
        if self.sender.send(job).await.is_err() {
            context.stop();
            return Err(io::Error::other("cold executor exited unexpectedly"));
        }
        match receiver.await {
            Ok((returned_session, returned_output, action)) => {
                *session = returned_session;
                *output = returned_output;
                Ok(action)
            }
            Err(_) => {
                context.stop();
                Err(io::Error::other("cold command panicked"))
            }
        }
    }
}

pub(super) fn serve(
    listener: TcpListener,
    config: Config,
    store: Arc<Store>,
    stopping: Arc<AtomicBool>,
    workers: usize,
) -> io::Result<()> {
    // max_clients bounds active task sets as well as registered/queued clients.
    // Channel capacity alone would not bound an eagerly drained JoinSet.
    let task_limit = config.max_clients;
    serve_with_task_limit(listener, config, store, stopping, workers, task_limit)
}

fn serve_with_task_limit(
    listener: TcpListener,
    config: Config,
    store: Arc<Store>,
    stopping: Arc<AtomicBool>,
    workers: usize,
    task_limit: usize,
) -> io::Result<()> {
    let cores = core_affinity::get_core_ids().unwrap_or_default();
    let workers = workers.clamp(1, 64);
    let controls = Arc::new(Mutex::new(HashMap::new()));
    // Compatibility modes have one published synchronous Store owner. Start it
    // outside any async runtime; all later bridge calls use the cold service.
    if workers > 1 || config.tls_bind_addr.is_some() {
        store.ensure_external_owner();
    }
    let (sender, mut receiver) = mpsc::channel::<ColdJob>(1);
    let cold_failure = FailureStop {
        stopping: Arc::clone(&stopping),
        controls: Arc::clone(&controls),
    };
    let cold_thread = thread::Builder::new()
        .name("rillcache-cold-io".into())
        .spawn(move || {
            let _failure = cold_failure;
            while let Some(job) = receiver.blocking_recv() {
                job();
            }
        })?;
    let context = Arc::new(Context {
        config,
        store,
        stopping,
        controls,
        cold: ColdExecutor {
            sender,
            permit: Arc::new(Semaphore::new(1)),
        },
        task_limit: task_limit.max(1),
    });
    // Runtime zero accepts AND serves clients. Exactly N network runtime
    // threads, plus one separately accounted cold-service thread; no N+1
    // accept-runtime topology. Affinity never wraps or duplicates logical IDs.
    let mut lanes = Vec::with_capacity(workers - 1);
    let result = (|| {
        for worker_index in 1..workers {
            let (sender, receiver) = mpsc::channel(ROUTE_CAPACITY);
            let worker_context = Arc::clone(&context);
            let core = cores.get(worker_index).copied();
            let thread = thread::Builder::new()
                .name(format!("rillcache-tokio-{worker_index}"))
                .spawn(move || {
                    pin_remote_worker(worker_index, core);
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()?;
                    tokio::task::LocalSet::new()
                        .block_on(&runtime, worker(receiver, worker_context))
                })?;
            lanes.push(Lane { sender, thread });
        }
        // Runtime zero can be the CLI process leader. Its affinity belongs to
        // the launcher: narrowing it changes the externally observed cpuset
        // contract (and also changes this calling thread after run returns).
        // Only remote network threads attempt per-worker affinity.
        let _ = writeln!(
            io::stderr().lock(),
            "rillcache: Tokio runtime 0 preserves launcher affinity"
        );
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        tokio::task::LocalSet::new().block_on(
            &runtime,
            accept_with_storage(listener, Arc::clone(&context), &lanes),
        )
    })();
    context.stop();
    let mut result = result;
    // Even startup failure joins every already-started worker and the service.
    for lane in lanes {
        drop(lane.sender);
        result = result.and(join_result(lane.thread));
    }
    drop(context); // close the cold queue only after every client has joined
    result.and(join(cold_thread))
}

fn pin_remote_worker(index: usize, core: Option<core_affinity::CoreId>) {
    debug_assert!(index > 0, "runtime zero preserves its launcher's affinity");
    let applied = core.is_some_and(core_affinity::set_for_current);
    // Diagnostics must not crash a server whose launcher's stderr pipe closed.
    let _ = writeln!(
        io::stderr().lock(),
        "rillcache: Tokio runtime {index} affinity cpu={:?} applied={applied}",
        core.map(|core| core.id)
    );
}

fn reap(tasks: &mut Tasks) -> io::Result<()> {
    let mut result = Ok(());
    while let Some(task) = tasks.try_join_next() {
        if let Err(error) = task {
            result = result.and(Err(io::Error::other(format!(
                "Tokio client task failed: {error}"
            ))));
        }
        // Network/protocol io::Errors are connection-local; task panics are not.
    }
    result
}

async fn join_tasks(tasks: &mut Tasks) -> io::Result<()> {
    let mut result = Ok(());
    while let Some(task) = tasks.join_next().await {
        if let Err(error) = task {
            result = result.and(Err(io::Error::other(format!(
                "Tokio client task failed: {error}"
            ))));
        }
    }
    result
}

async fn accept_with_storage(
    listener: TcpListener,
    context: Arc<Context>,
    lanes: &[Lane],
) -> io::Result<()> {
    // Install !Send Data on the actual socket thread. Mixed TLS or multiple
    // network runtimes use the compatibility owner, never several Data copies.
    let local = if lanes.is_empty() && context.config.tls_bind_addr.is_none() {
        context.store.install_local()
    } else {
        None
    };
    let (executor, driver) = if let Some((executor, inbox)) = local {
        let executor = Rc::new(executor);
        let owner = Rc::clone(&executor);
        let finished = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&finished);
        let failure_context = Arc::clone(&context);
        let driver = tokio::task::spawn_local(async move {
            let _failure = failure_context.failure_guard();
            // Cold paused-command resumption can use the external bridge.
            // Service it on this same owner without blocking a network task,
            // including while client tasks are being joined on shutdown.
            while !stop.load(Ordering::Acquire) {
                inbox.drain(owner.as_ref());
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            inbox.drain(owner.as_ref());
        });
        (Some(executor), Some((finished, driver)))
    } else {
        (None, None)
    };
    let mut result = accept_loop(listener, Arc::clone(&context), lanes, executor).await;
    if let Some((finished, driver)) = driver {
        // A task panic/cancellation can drop its reply receiver while a cold
        // job still owns session state. Keep the local bridge/Data alive until
        // that job releases its permit, not merely until socket tasks join.
        let drained = context.cold.permit.acquire().await;
        if drained.is_err() {
            result = result.and(Err(io::Error::other(
                "cold service admission closed during drain",
            )));
        }
        finished.store(true, Ordering::Release);
        if let Err(error) = driver.await {
            result = result.and(Err(io::Error::other(format!(
                "local Store bridge task failed: {error}"
            ))));
        }
    }
    result
}

async fn accept_loop(
    listener: TcpListener,
    context: Arc<Context>,
    lanes: &[Lane],
    executor: Option<Rc<ShardExecutor>>,
) -> io::Result<()> {
    let listener = TokioListener::from_std(listener)?;
    let mut tasks = Tasks::new();
    let mut pending = None;
    let mut next_lane = 0;
    let mut turns = 0usize;
    let mut maintenance = Instant::now();
    let result = async {
        loop {
            reap(&mut tasks)?;
            if context.stopped() {
                return Ok(());
            }
            if lanes.iter().any(|lane| lane.thread.is_finished()) {
                return Err(io::Error::other("Tokio network worker exited unexpectedly"));
            }
            if maintenance.elapsed() >= MAINTENANCE_TICK {
                // Journal/audit configurations use the proven fallback. No
                // sync/rewrite or other filesystem maintenance runs here.
                if let Some(executor) = &executor {
                    executor.purge_expired(128);
                } else {
                    context
                        .cold
                        .run(
                            &mut Session::new(),
                            &mut Vec::new(),
                            &context,
                            ColdOperation::Maintenance,
                        )
                        .await?;
                }
                maintenance = Instant::now();
            }
            turns += 1;
            if turns.is_multiple_of(COMMAND_QUANTUM) {
                tokio::task::yield_now().await;
            }
            if let Some(accepted) = pending.take() {
                // Try each lane once. Retain ownership on saturation, service
                // maintenance/stop each tick, and NEVER drop a pending client
                // just because another task finished. Do not await join_next
                // while sockets are open: idle clients might never finish.
                let count = lanes.len() + 1;
                pending = Some(accepted);
                for offset in 0..count {
                    let index = (next_lane + offset) % count;
                    if index == 0 {
                        if tasks.len() >= context.task_limit {
                            continue;
                        }
                        context.spawn(
                            &mut tasks,
                            pending.take().expect("pending client"),
                            executor.clone(),
                        );
                    } else {
                        match lanes[index - 1]
                            .sender
                            .try_send(pending.take().expect("pending client"))
                        {
                            Ok(()) => {}
                            Err(mpsc::error::TrySendError::Full(value)) => {
                                pending = Some(value);
                                continue;
                            }
                            Err(mpsc::error::TrySendError::Closed(_)) => {
                                return Err(io::Error::other("Tokio network worker stopped"));
                            }
                        }
                    }
                    next_lane = (index + 1) % count;
                    break;
                }
                if pending.is_some() {
                    tokio::time::sleep(POLL_TICK).await;
                }
                continue;
            }
            let (mut stream, peer) = match tokio::time::timeout(POLL_TICK, listener.accept()).await
            {
                Err(_) => continue,
                Ok(Ok(pair)) => pair,
                Ok(Err(error)) if error.kind() == io::ErrorKind::Interrupted => continue,
                Ok(Err(error)) => return Err(error),
            };
            context.store.record_connection();
            if context.store.connected_clients() >= context.store.max_clients() {
                context.store.record_rejected_connection();
                let _ = tokio::time::timeout(
                    POLL_TICK,
                    stream.write_all(b"-ERR max number of clients reached\r\n"),
                )
                .await;
                continue;
            }
            stream.set_nodelay(true)?;
            let local = stream.local_addr()?;
            let stream = stream.into_std()?;
            let control = stream.try_clone()?;
            let (sender, pubsub) = sync_channel(Store::pubsub_queue_capacity());
            let mut session = Session::new();
            session.set_peer(peer);
            session.set_pubsub_sender(sender);
            let id = session.id();
            // Construct the guard before either registry insertion so every
            // error path unregisters and releases its control descriptor.
            let registration = ClientRegistration {
                id,
                store: context.store.services_handle(),
                controls: Arc::clone(&context.controls),
            };
            context
                .store
                .register_client(id, peer, local, control.try_clone()?);
            context
                .controls
                .lock()
                .map_err(|_| io::Error::other("Tokio client registry poisoned"))?
                .insert(id, control);
            pending = Some(Accepted {
                stream,
                session,
                pubsub,
                registration,
            });
        }
    }
    .await;
    // Stop and close BEFORE joining: read/write/paused/blocked tasks, and remote
    // lanes saturated with admitted clients, must all observe cancellation.
    context.stop();
    drop(pending);
    drop(listener);
    result.and(join_tasks(&mut tasks).await)
}

async fn worker(mut receiver: mpsc::Receiver<Accepted>, context: Arc<Context>) -> io::Result<()> {
    let mut tasks = Tasks::new();
    let result = async {
        loop {
            reap(&mut tasks)?;
            if context.stopped() {
                return Ok(());
            }
            if tasks.len() >= context.task_limit {
                // Capacity includes active tasks, not just the routing mailbox.
                // Poll stop even if every active client is idle/backpressured.
                tokio::time::sleep(POLL_TICK).await;
                continue;
            }
            match tokio::time::timeout(POLL_TICK, receiver.recv()).await {
                Ok(Some(accepted)) => context.spawn(&mut tasks, accepted, None),
                Ok(None) => return Ok(()),
                Err(_) => {}
            }
        }
    }
    .await;
    context.stop();
    drop(receiver); // drop queued registrations before joining active tasks
    result.and(join_tasks(&mut tasks).await)
}

async fn write_reply(stream: &mut TokioStream, reply: &[u8], timeout: Duration) -> io::Result<()> {
    tokio::time::timeout(timeout, async {
        for (index, chunk) in reply.chunks(OUTPUT_QUANTUM).enumerate() {
            if index > 0 {
                tokio::task::yield_now().await;
            }
            stream.write_all(chunk).await?;
        }
        Ok(())
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "client write timeout"))?
}

async fn close_after_reply(
    stream: &mut TokioStream,
    reply: &[u8],
    timeout: Duration,
) -> io::Result<()> {
    write_reply(stream, reply, timeout).await?;
    stream.shutdown().await?;
    // A single total drain deadline, not a fresh deadline per incoming chunk.
    let _ = tokio::time::timeout(POLL_TICK, async {
        let mut discard = [0u8; MAX_READ];
        loop {
            match stream.read(&mut discard).await {
                Ok(0) => return Ok(()),
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
    })
    .await;
    Ok(())
}

// ACL GENPASS reads /dev/urandom without holding a data/auth lock. ACL-file
// configurations use the fallback because SAVE/LOAD hold auth during disk I/O.
// Session::queue rejects ACL inside MULTI; EXEC therefore stays synchronous and
// cannot perform ACL disk I/O. Paused resumption also uses the cold executor.
fn cold_command(args: &[&[u8]]) -> bool {
    args.first()
        .is_some_and(|name| name.eq_ignore_ascii_case(b"ACL"))
}

async fn serve_connection(
    mut stream: TokioStream,
    mut session: Session,
    pubsub: Receiver<Vec<u8>>,
    context: &Context,
    executor: Option<Rc<ShardExecutor>>,
) -> io::Result<()> {
    let config = &context.config;
    let mut input = Vec::with_capacity(8192.min(config.limits.max_frame_bytes));
    let mut output = Vec::with_capacity(8192);
    let mut chunk = [0u8; MAX_READ];
    let mut activity = Instant::now();
    loop {
        if context.stopped() {
            return Ok(());
        }
        let mut pushed = 0;
        while pushed < COMMAND_QUANTUM && output.len() < OUTPUT_QUANTUM {
            match pubsub.try_recv() {
                Ok(frame) => {
                    output.extend_from_slice(&frame);
                    pushed += 1;
                }
                Err(_) => break,
            }
        }
        if session.is_blocked() {
            if let Some(executor) = &executor {
                if let Some(reply) = session.poll_blocked_on(executor.as_ref()) {
                    output.extend_from_slice(&reply);
                }
            } else {
                context
                    .cold
                    .run(&mut session, &mut output, context, ColdOperation::Blocked)
                    .await?;
            }
        }
        if session.is_paused()
            && let Some(Action::Close) = context
                .cold
                .run(&mut session, &mut output, context, ColdOperation::Paused)
                .await?
        {
            write_reply(&mut stream, &output, config.idle_timeout).await?;
            return Ok(());
        }
        if !output.is_empty() {
            write_reply(&mut stream, &output, config.idle_timeout).await?;
            output.clear();
            activity = Instant::now();
            if context.stopped() {
                return Ok(());
            }
        }

        if !session.is_blocked() && !session.is_paused() {
            let mut consumed = 0usize;
            let mut need_input = false;
            for _ in 0..COMMAND_QUANTUM {
                let mut cold = false;
                match dispatch_request(&input[consumed..], config.limits, |args| {
                    cold = executor.is_none() || cold_command(args);
                    if cold || args.is_empty() {
                        Action::Continue
                    } else {
                        session.execute_on(
                            executor.as_ref().expect("local executor").as_ref(),
                            args,
                            &mut output,
                        )
                    }
                }) {
                    Ok(Some((mut action, used))) => {
                        if cold {
                            action = context
                                .cold
                                .run(
                                    &mut session,
                                    &mut output,
                                    context,
                                    ColdOperation::Command(&input[consumed..consumed + used]),
                                )
                                .await?
                                .expect("complete cold command");
                            if context.stopped() {
                                return Ok(());
                            }
                        }
                        consumed += used;
                        if action == Action::Close {
                            write_reply(&mut stream, &output, config.idle_timeout).await?;
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
                        close_after_reply(&mut stream, &output, config.idle_timeout).await?;
                        return Ok(());
                    }
                }
            }
            if consumed > 0 {
                input.drain(..consumed);
            }
            // No new dispatch/read while reply bytes are backpressured. The
            // 64 KiB batch bound can be exceeded by ONE existing atomic reply;
            // it is not a dishonest hard maximum or a change to wire semantics.
            if !output.is_empty() {
                write_reply(&mut stream, &output, config.idle_timeout).await?;
                output.clear();
                activity = Instant::now();
            }
            if !need_input && !input.is_empty() && !session.is_blocked() && !session.is_paused() {
                tokio::task::yield_now().await;
                continue;
            }
        }

        let room = config
            .limits
            .max_frame_bytes
            .saturating_sub(input.len())
            .min(chunk.len());
        if room == 0 {
            close_after_reply(
                &mut stream,
                b"-ERR Protocol error: request frame limit exceeded\r\n",
                config.idle_timeout,
            )
            .await?;
            return Ok(());
        }
        let asynchronous = session.is_subscribed()
            || session.is_monitoring()
            || session.is_blocked()
            || session.is_paused();
        let wait = if asynchronous {
            POLL_TICK.min(config.idle_timeout)
        } else {
            config.idle_timeout.saturating_sub(activity.elapsed())
        };
        if wait.is_zero() {
            return Ok(());
        }
        let read = stream.read(&mut chunk[..room]);
        #[cfg(test)]
        let read = tests::observe_read_wait(read);
        match tokio::time::timeout(wait, read).await {
            Err(_) if asynchronous => continue,
            Err(_) => return Ok(()),
            Ok(Ok(0)) => {
                if !input.is_empty() {
                    write_reply(
                        &mut stream,
                        b"-ERR Protocol error: unexpected end of stream\r\n",
                        config.idle_timeout,
                    )
                    .await?;
                }
                return Ok(());
            }
            Ok(Ok(size)) => {
                input.extend_from_slice(&chunk[..size]);
                activity = Instant::now();
            }
            Ok(Err(error)) if error.kind() == io::ErrorKind::Interrupted => continue,
            Ok(Err(error)) => return Err(error),
        }
    }
}
