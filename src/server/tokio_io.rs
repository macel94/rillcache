//! Default plain-TCP path: bounded per-runtime routing and async socket tasks.

use super::*;
use crate::session::Session;
use std::collections::HashMap;
use std::sync::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener as TokioListener, TcpStream as TokioStream};
use tokio::sync::mpsc;
use tokio::task::JoinSet;

const ROUTE_CAPACITY: usize = 32;
const COMMAND_QUANTUM: usize = 64;
const OUTPUT_QUANTUM: usize = 64 * 1024;
const MAX_READ: usize = 16 * 1024;
const POLL_TICK: Duration = Duration::from_millis(10);
const MAINTENANCE_TICK: Duration = Duration::from_millis(100);

struct Accepted {
    stream: std::net::TcpStream,
    session: Session,
    pubsub: Receiver<Vec<u8>>,
    registration: ClientRegistration,
}

struct ClientRegistration {
    id: u64,
    store: Arc<Store>,
    controls: Arc<Mutex<HashMap<u64, TcpStream>>>,
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

pub(super) fn serve(
    listener: TcpListener,
    config: Config,
    store: Arc<Store>,
    stopping: Arc<AtomicBool>,
    workers: usize,
) -> io::Result<()> {
    let cores = core_affinity::get_core_ids().unwrap_or_default();
    let workers = workers.clamp(1, 64);
    let controls = Arc::new(Mutex::new(HashMap::new()));
    let mut lanes: Vec<Lane> = Vec::with_capacity(workers);
    for index in 0..workers {
        let (sender, receiver) = mpsc::channel(ROUTE_CAPACITY);
        let worker_config = config.clone();
        let worker_store = Arc::clone(&store);
        let worker_stop = Arc::clone(&stopping);
        let core = cores.get(index % cores.len().max(1)).copied();
        let thread = match thread::Builder::new()
            .name(format!("rillcache-tokio-{index}"))
            .spawn(move || {
                if let Some(core) = core {
                    // Affinity is best-effort; failure leaves the OS scheduler in control.
                    let _ = core_affinity::set_for_current(core);
                }
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?;
                runtime.block_on(worker(receiver, worker_config, worker_store, worker_stop))
            }) {
            Ok(thread) => thread,
            Err(error) => {
                stopping.store(true, Ordering::Release);
                for lane in lanes.drain(..) {
                    drop(lane.sender);
                    let _ = lane.thread.join();
                }
                return Err(error);
            }
        };
        lanes.push(Lane { sender, thread });
    }

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            stopping.store(true, Ordering::Release);
            for lane in lanes {
                drop(lane.sender);
                let _ = lane.thread.join();
            }
            return Err(error);
        }
    };
    let result = runtime.block_on(accept_loop(
        listener,
        Arc::clone(&store),
        Arc::clone(&controls),
        Arc::clone(&stopping),
        &lanes,
    ));
    stopping.store(true, Ordering::Release);
    if let Ok(controls) = controls.lock() {
        for control in controls.values() {
            let _ = control.shutdown(Shutdown::Both);
        }
    }
    let mut result = result;
    for lane in lanes {
        drop(lane.sender);
        match lane.thread.join() {
            Err(error) => {
                result = result.and(Err(io::Error::other(format!(
                    "Tokio network worker panicked: {error:?}"
                ))));
            }
            Ok(Err(error)) => result = result.and(Err(error)),
            Ok(Ok(())) => {}
        }
    }
    result
}

async fn accept_loop(
    listener: TcpListener,
    store: Arc<Store>,
    controls: Arc<Mutex<HashMap<u64, TcpStream>>>,
    stopping: Arc<AtomicBool>,
    lanes: &[Lane],
) -> io::Result<()> {
    let listener = TokioListener::from_std(listener)?;
    let mut next_lane = 0usize;
    let mut maintenance = Instant::now();
    while !stopping.load(Ordering::Acquire) && !store.shutdown_requested() {
        if lanes.iter().any(|lane| lane.thread.is_finished()) {
            return Err(io::Error::other("Tokio network worker exited unexpectedly"));
        }
        if maintenance.elapsed() >= MAINTENANCE_TICK {
            let _ = store.journal_sync();
            store.purge_expired(128);
            store.auto_rewrite_if_due();
            maintenance = Instant::now();
        }
        let accepted = tokio::time::timeout(POLL_TICK, listener.accept()).await;
        let (stream, peer) = match accepted {
            Err(_) => continue,
            Ok(Ok(pair)) => pair,
            Ok(Err(error)) if error.kind() == io::ErrorKind::Interrupted => continue,
            Ok(Err(error)) => return Err(error),
        };
        store.record_connection();
        if store.connected_clients() >= store.max_clients() {
            store.record_rejected_connection();
            let mut stream = stream;
            let _ = tokio::time::timeout(
                POLL_TICK,
                stream.write_all(b"-ERR max number of clients reached\r\n"),
            )
            .await;
            let _ = stream.shutdown().await;
            continue;
        }
        let stream = stream;
        stream.set_nodelay(true)?;
        let local = stream.local_addr()?;
        let std_stream = stream.into_std()?;
        let control = std_stream.try_clone()?;
        let (sender, pubsub) = sync_channel(Store::pubsub_queue_capacity());
        let mut session = Session::new();
        session.set_peer(peer);
        session.set_pubsub_sender(sender);
        let id = session.id();
        store.register_client(id, peer, local, control.try_clone()?);
        controls
            .lock()
            .map_err(|_| io::Error::other("Tokio client registry poisoned"))?
            .insert(id, control);
        let accepted = Accepted {
            stream: std_stream,
            session,
            pubsub,
            registration: ClientRegistration {
                id,
                store: Arc::clone(&store),
                controls: Arc::clone(&controls),
            },
        };

        // A full lane applies bounded backpressure to acceptance rather than
        // creating detached/unbounded connection tasks. Rotate before waiting.
        let mut pending = Some(accepted);
        let mut routed = false;
        for offset in 0..lanes.len() {
            let index = (next_lane + offset) % lanes.len();
            match lanes[index]
                .sender
                .try_send(pending.take().expect("pending connection"))
            {
                Ok(()) => {
                    next_lane = (index + 1) % lanes.len();
                    routed = true;
                    break;
                }
                Err(mpsc::error::TrySendError::Full(value)) => pending = Some(value),
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    return Err(io::Error::other("Tokio network worker exited unexpectedly"));
                }
            }
        }
        if !routed {
            let index = next_lane % lanes.len();
            lanes[index]
                .sender
                .send(pending.expect("pending connection"))
                .await
                .map_err(|_| io::Error::other("Tokio network worker stopped during routing"))?;
            next_lane = (index + 1) % lanes.len();
        }
    }
    Ok(())
}

async fn worker(
    mut receiver: mpsc::Receiver<Accepted>,
    config: Config,
    store: Arc<Store>,
    stopping: Arc<AtomicBool>,
) -> io::Result<()> {
    let mut tasks = JoinSet::new();
    while let Some(accepted) = receiver.recv().await {
        while let Some(result) = tasks.try_join_next() {
            if let Err(error) = result
                && error.is_panic()
            {
                return Err(io::Error::other("Tokio client task panicked"));
            }
        }
        let client_store = Arc::clone(&store);
        let client_config = config.clone();
        let client_stop = Arc::clone(&stopping);
        tasks.spawn(async move {
            let _registration = accepted.registration;
            let stream = TokioStream::from_std(accepted.stream)?;
            serve_connection(
                stream,
                accepted.session,
                accepted.pubsub,
                &client_store,
                &client_config,
                &client_stop,
            )
            .await
        });
    }
    while let Some(result) = tasks.join_next().await {
        if let Err(error) = result
            && error.is_panic()
        {
            return Err(io::Error::other("Tokio client task panicked"));
        }
    }
    Ok(())
}

async fn close_after_reply(stream: &mut TokioStream, reply: &[u8]) -> io::Result<()> {
    stream.write_all(reply).await?;
    stream.shutdown().await?;
    let mut discard = [0u8; MAX_READ];
    loop {
        match tokio::time::timeout(POLL_TICK, stream.read(&mut discard)).await {
            Ok(Ok(0)) | Err(_) => return Ok(()),
            Ok(Ok(_)) => {}
            Ok(Err(error)) if error.kind() == io::ErrorKind::Interrupted => {}
            Ok(Err(error)) => return Err(error),
        }
    }
}

async fn serve_connection(
    mut stream: TokioStream,
    mut session: Session,
    pubsub: Receiver<Vec<u8>>,
    store: &Store,
    config: &Config,
    stopping: &AtomicBool,
) -> io::Result<()> {
    let _ = stream.set_nodelay(true);
    let mut input = Vec::with_capacity(8192.min(config.limits.max_frame_bytes));
    let mut output = Vec::with_capacity(8192);
    let mut chunk = [0u8; MAX_READ];
    let mut activity = Instant::now();
    loop {
        if stopping.load(Ordering::Acquire) || store.shutdown_requested() {
            return Ok(());
        }
        let mut pushed = 0;
        while pushed < 64 && output.len() < OUTPUT_QUANTUM {
            match pubsub.try_recv() {
                Ok(frame) => {
                    output.extend_from_slice(&frame);
                    pushed += 1;
                }
                Err(_) => break,
            }
        }
        if session.is_blocked()
            && let Some(reply) = session.poll_blocked(store)
        {
            output.extend_from_slice(&reply);
        }
        if session.is_paused()
            && let Some((reply, action)) = session.poll_paused(store)
        {
            output.extend_from_slice(&reply);
            if action == Action::Close {
                if !output.is_empty() {
                    stream.write_all(&output).await?;
                }
                return Ok(());
            }
        }
        if !output.is_empty() {
            stream.write_all(&output).await?;
            output.clear();
            activity = Instant::now();
        }

        if !session.is_blocked() && !session.is_paused() {
            let mut consumed = 0usize;
            let mut need_input = false;
            for _ in 0..COMMAND_QUANTUM {
                match dispatch_request(&input[consumed..], config.limits, |args| {
                    if args.is_empty() {
                        Action::Continue
                    } else {
                        session.execute(store, args, &mut output)
                    }
                }) {
                    Ok(Some((action, used))) => {
                        consumed += used;
                        if action == Action::Close {
                            stream.write_all(&output).await?;
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
                        close_after_reply(&mut stream, &output).await?;
                        return Ok(());
                    }
                }
            }
            if consumed > 0 {
                input.drain(..consumed);
            }
            if !output.is_empty() {
                stream.write_all(&output).await?;
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
        match tokio::time::timeout(wait, stream.read(&mut chunk[..room])).await {
            Err(_) if asynchronous => continue,
            Err(_) => return Ok(()),
            Ok(Ok(0)) => {
                if !input.is_empty() {
                    stream
                        .write_all(b"-ERR Protocol error: unexpected end of stream\r\n")
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
