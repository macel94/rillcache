//! Optional bounded I/O/execution lanes using only safe standard-library APIs.
//! Nonblocking round-robin polling is portable, not an epoll/kqueue substitute.

use super::*;
use crate::engine::{BridgeInbox, Execution, StoreHandle};
use std::sync::atomic::AtomicUsize;
use std::sync::mpsc::{self, Receiver, Sender, sync_channel};
use std::time::Instant;

use crate::session::Session;

const QUANTUM: usize = 64;
const OUTPUT_CHUNK: usize = 64 * 1024;

struct PollWindow {
    until: Instant,
    run_since: Instant,
}
impl PollWindow {
    fn new(now: Instant) -> Self {
        Self {
            until: now,
            run_since: now,
        }
    }
    fn active(&mut self, now: Instant, progress: bool) -> bool {
        if progress {
            self.until = now + Duration::from_micros(FIXED_WORKER_ACTIVE_POLL_US);
        }
        now < self.until
    }
    fn yield_due(&mut self, now: Instant, budget_us: u64, peers: bool) -> bool {
        if budget_us == 0
            || !peers
            || now.duration_since(self.run_since) < Duration::from_micros(budget_us)
        {
            return false;
        }
        self.run_since = now;
        true
    }
    fn resumed(&mut self, now: Instant) {
        self.run_since = now;
    }
}

pub(super) struct Client {
    pub(super) stream: TcpStream,
    pub(super) session: Session,
    pub(super) input: Vec<u8>,
    pub(super) output: Vec<u8>,
    pub(super) sent: usize,
    pub(super) closing: bool,
    pub(super) eof: bool,
    /// EINTR is not WouldBlock: readiness owners must retain their runnable token.
    pub(super) retry_io: bool,
    pub(super) activity: Instant,
    pub(super) _permit: Option<Permit>,
    pub(super) shutdown_on_drop: bool,
    pub(super) pubsub: Receiver<Vec<u8>>,
}

impl Drop for Client {
    fn drop(&mut self) {
        // Shut down before close even with unread input, as in blocking mode.
        if self.shutdown_on_drop {
            let _ = self.stream.shutdown(Shutdown::Both);
        }
    }
}

impl Client {
    fn flush(&mut self) -> io::Result<bool> {
        if self.sent == self.output.len() {
            self.output.clear();
            self.sent = 0;
            return Ok(false);
        }
        let end = self.output.len().min(self.sent + OUTPUT_CHUNK);
        match self.stream.write(&self.output[self.sent..end]) {
            Ok(0) => Err(io::ErrorKind::WriteZero.into()),
            Ok(size) => {
                self.sent += size;
                self.activity = Instant::now();
                if self.sent == self.output.len() {
                    self.output.clear();
                    self.sent = 0;
                }
                Ok(true)
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                self.retry_io |= error.kind() == io::ErrorKind::Interrupted;
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }

    fn open(&self, progress: bool, timeout: Duration) -> Option<bool> {
        (progress || self.activity.elapsed() < timeout).then_some(progress)
    }

    /// At most one bounded socket read per client/pass.
    pub(super) fn receive(&mut self, limits: Limits, chunk: &mut [u8]) -> io::Result<bool> {
        let room = (limits.max_frame_bytes - self.input.len()).min(chunk.len());
        if room == 0 {
            self.output
                .extend_from_slice(b"-ERR Protocol error: request frame limit exceeded\r\n");
            self.closing = true;
            return Ok(true);
        }
        match self.stream.read(&mut chunk[..room]) {
            Ok(0) => {
                self.eof = true;
                Ok(true)
            }
            Ok(size) => {
                self.input.extend_from_slice(&chunk[..size]);
                self.activity = Instant::now();
                Ok(true)
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                self.retry_io |= error.kind() == io::ErrorKind::Interrupted;
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }

    /// None means closed; Some indicates whether this pass made progress.
    pub(super) fn poll<E: Execution>(
        &mut self,
        store: &E,
        config: &Config,
        chunk: &mut [u8],
    ) -> io::Result<Option<bool>> {
        // Like blocking I/O, service already-ready input/output before declaring
        // inactivity; scheduler delay must not discard buffered commands.
        self.retry_io = false;
        let mut progress = self.flush()?;
        if self.closing {
            return Ok(if self.output.is_empty() {
                None
            } else {
                self.open(progress, config.idle_timeout)
            });
        }
        if self.session.is_subscribed() || self.session.is_monitoring() {
            let mut pushed = 0;
            while pushed < 64 {
                match self.pubsub.try_recv() {
                    Ok(frame) => {
                        self.output.extend_from_slice(&frame);
                        pushed += 1;
                        if self.output.len() >= OUTPUT_CHUNK {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            if pushed > 0 {
                progress |= self.flush()?;
            }
        }
        if self.session.is_blocked() {
            if let Some(reply) = self.session.poll_blocked_on(store) {
                self.output.extend_from_slice(&reply);
                progress = true;
            }
            if !self.output.is_empty() {
                progress |= self.flush()?;
            }
            if self.session.is_blocked() {
                if self.eof {
                    return Ok(None);
                }
                if self.input.is_empty() {
                    let received = self.receive(config.limits, chunk)?;
                    progress |= received;
                    if self.eof {
                        return Ok(None);
                    }
                }
                return Ok(Some(progress));
            }
        }
        if self.session.is_paused() {
            if let Some((reply, action)) = self.session.poll_paused_on(store) {
                self.output.extend_from_slice(&reply);
                progress = true;
                if action == Action::Close {
                    self.closing = true;
                }
            }
            if self.session.is_paused() || self.session.is_blocked() {
                if self.eof {
                    return Ok(None);
                }
                if self.input.is_empty() {
                    let received = self.receive(config.limits, chunk)?;
                    progress |= received;
                    if self.eof {
                        return Ok(None);
                    }
                }
                return Ok(Some(progress));
            }
        }
        if !self.output.is_empty() {
            return Ok(self.open(progress, config.idle_timeout)); // Backpressure.
        }
        if self.closing {
            return Ok(None);
        }
        let read_first = self.input.is_empty() && !self.eof;
        if read_first {
            progress |= self.receive(config.limits, chunk)?;
            if self.input.is_empty() || self.closing {
                return Ok(self.open(progress, config.idle_timeout));
            }
        }
        let mut consumed = 0;
        let mut need_input = false;
        for _ in 0..QUANTUM {
            match dispatch_request(&self.input[consumed..], config.limits, |args| {
                if args.is_empty() {
                    Action::Continue // Redis ignores an empty inline line.
                } else {
                    self.session.execute_on(store, args, &mut self.output)
                }
            }) {
                Ok(Some((action, used))) => {
                    consumed += used;
                    progress = true;
                    if action == Action::Close {
                        self.closing = true;
                        break;
                    }
                    if self.session.is_blocked() || self.session.is_paused() {
                        break;
                    }
                    // At most one bounded reply can take us beyond this soft limit.
                    if self.output.len() >= OUTPUT_CHUNK {
                        break;
                    }
                }
                Ok(None) => {
                    need_input = true;
                    break;
                }
                Err(error) => {
                    write!(self.output, "-ERR Protocol error: {error}\r\n")?;
                    self.closing = true;
                    progress = true;
                    break;
                }
            }
        }
        if consumed == self.input.len() {
            self.input.clear();
        } else if consumed > 0 {
            self.input.drain(..consumed);
        }
        progress |= self.flush()?;
        if self.closing || !self.output.is_empty() || !need_input {
            return Ok(self.open(progress, config.idle_timeout));
        }
        if self.eof {
            if self.input.is_empty() {
                return Ok(None);
            }
            self.output
                .extend_from_slice(b"-ERR Protocol error: unexpected end of stream\r\n");
            self.closing = true;
            return Ok(Some(true));
        }
        if !read_first {
            progress |= self.receive(config.limits, chunk)?;
        }
        Ok(self.open(progress, config.idle_timeout))
    }
}

struct Lane {
    sender: Sender<Client>,
    thread: JoinHandle<()>,
}

pub(super) fn serve(
    listener: TcpListener,
    config: Config,
    stopping: Arc<AtomicBool>,
    policy: SchedulingPolicy,
    store: Arc<Store>,
) -> io::Result<()> {
    let count = config.worker_threads.expect("fixed worker configuration");
    let admitted = Arc::new(AtomicUsize::new(0));
    let mut lanes: Vec<Lane> = Vec::new();
    let result = (|| {
        for index in 0..count {
            // The shared admission counter bounds the sum of all channel queues,
            // active clients and in-flight transfers by max_clients.
            let (sender, receiver) = mpsc::channel::<Client>();
            let store = Arc::clone(&store);
            let admitted = Arc::clone(&admitted);
            let stop = Arc::clone(&stopping);
            let config = config.clone();
            let reaper = index == 0;
            let thread = thread::Builder::new()
                .name(format!("rillcache-io-{index}"))
                .spawn(move || {
                    if count == 1
                        && let Some((executor, inbox)) = store.install_local()
                    {
                        run_lane(
                            executor,
                            Some(inbox),
                            receiver,
                            config,
                            stop,
                            admitted,
                            policy,
                            reaper,
                        );
                        return;
                    }
                    run_lane(
                        StoreHandle(store),
                        None,
                        receiver,
                        config,
                        stop,
                        admitted,
                        policy,
                        reaper,
                    );
                    // Dropping clients and queued transfers closes every socket and permit.
                })?;
            lanes.push(Lane { sender, thread });
        }
        let mut next = 0;
        while !stopping.load(Ordering::Acquire) {
            if store.shutdown_requested() {
                stopping.store(true, Ordering::Release);
                break;
            }
            if lanes.iter().any(|lane| lane.thread.is_finished()) {
                // A worker may observe the protocol shutdown request and exit
                // before this accept loop publishes the stopping flag.
                if stopping.load(Ordering::Acquire) || store.shutdown_requested() {
                    stopping.store(true, Ordering::Release);
                    break;
                }
                return Err(io::Error::other("I/O worker exited unexpectedly"));
            }
            let (mut stream, peer) = match listener.accept() {
                Ok(connection) => connection,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(1));
                    continue;
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            };
            stream.set_nodelay(true)?;
            store.record_connection();
            if admitted.load(Ordering::Acquire) >= store.max_clients() {
                store.record_rejected_connection();
                stream.set_write_timeout(Some(config.idle_timeout))?;
                let _ = stream.write_all(b"-ERR max number of clients reached\r\n");
                let _ = stream.shutdown(Shutdown::Both);
                continue;
            }
            stream.set_nonblocking(true)?;
            let control = stream.try_clone()?;
            let (sender, receiver) = sync_channel(Store::pubsub_queue_capacity());
            let mut session = Session::new();
            session.set_peer(peer);
            session.set_pubsub_sender(sender);
            store.register_client(session.id(), peer, stream.local_addr()?, control);
            let client = Client {
                stream,
                session,
                input: Vec::with_capacity(8192.min(config.limits.max_frame_bytes)),
                output: Vec::with_capacity(8192),
                sent: 0,
                closing: false,
                eof: false,
                retry_io: false,
                activity: Instant::now(),
                _permit: Some(Permit::acquire(&admitted)),
                shutdown_on_drop: true,
                pubsub: receiver,
            };
            if lanes[next].sender.send(client).is_err() {
                // Shutdown may race an accept already in flight. Closed receiver
                // after a stop request is cancellation, not a worker failure.
                if stopping.load(Ordering::Acquire) {
                    break;
                }
                return Err(io::Error::other("I/O worker unavailable"));
            }
            lanes[next].thread.thread().unpark();
            next = (next + 1) % count;
        }
        Ok(())
    })();
    stopping.store(true, Ordering::Release);
    for lane in &lanes {
        lane.thread.thread().unpark();
    }
    let mut result = result;
    for lane in lanes {
        if let Err(error) = join(lane.thread) {
            result = result.and(Err(error));
        }
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn run_lane<E: Execution>(
    store: E,
    inbox: Option<BridgeInbox>,
    receiver: Receiver<Client>,
    config: Config,
    stop: Arc<AtomicBool>,
    admitted: Arc<AtomicUsize>,
    policy: SchedulingPolicy,
    reaper: bool,
) {
    let mut clients: Vec<Client> = Vec::new();
    let mut chunk = [0; 16 * 1024];
    let mut maintenance = Instant::now();
    let mut polling = PollWindow::new(maintenance);
    while !stop.load(Ordering::Acquire) && !store.shutdown_requested() {
        if let Some(inbox) = &inbox {
            inbox.drain(&store);
        }
        if reaper && maintenance.elapsed() >= Duration::from_millis(100) {
            let _ = store.journal_sync();
            store.purge_expired(128);
            store.auto_rewrite_if_due();
            maintenance = Instant::now();
        }
        let mut progress = false;
        while let Ok(client) = receiver.try_recv() {
            clients.push(client);
            progress = true;
        }
        let mut index = 0;
        while index < clients.len() {
            match clients[index].poll(&store, &config, &mut chunk) {
                Ok(Some(made_progress)) => {
                    progress |= made_progress;
                    index += 1;
                }
                _ => {
                    let id = clients[index].session.id();
                    store.pubsub_remove_client(id);
                    store.unregister_client(id);
                    clients.swap_remove(index);
                }
            }
        }
        // Replies in flight are not an idle connection. Give the
        // next client request a short bounded window to arrive,
        // without rearming that window on empty/spurious scans.
        let now = Instant::now();
        if polling.active(now, progress) {
            if polling.yield_due(
                now,
                policy.run_quantum_us,
                admitted.load(Ordering::Relaxed) > 1,
            ) {
                thread::yield_now();
                polling.resumed(Instant::now());
            } else if !progress {
                std::hint::spin_loop();
            }
        } else {
            thread::park_timeout(Duration::from_millis(1));
            polling.resumed(Instant::now());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_polling_expires_and_empty_passes_never_rearm_it() {
        let now = Instant::now();
        let mut window = PollWindow::new(now);
        assert!(!window.active(now, false));
        assert!(window.active(now, true));
        let budget = Duration::from_micros(FIXED_WORKER_ACTIVE_POLL_US);
        assert!(window.active(now + budget / 2, false));
        assert!(!window.active(now + budget, false));
        assert!(!window.active(now + budget * 2, false));
        assert!(!window.active(now + Duration::from_secs(10), false));
        assert!(window.active(now + Duration::from_secs(10), true));
        assert!(!window.active(now + Duration::from_secs(10) + budget, false));
    }

    #[test]
    fn continuous_progress_never_rearms_the_cooperative_run_budget() {
        let now = Instant::now();
        let budget = FIXED_WORKER_RUN_QUANTUM_US;
        let mut window = PollWindow::new(now);
        for elapsed in 0..budget {
            let t = now + Duration::from_micros(elapsed);
            assert!(window.active(t, true));
            assert!(!window.yield_due(t, budget, true));
        }
        let due = now + Duration::from_micros(budget);
        assert!(window.active(due, true));
        assert!(window.yield_due(due, budget, true));
        assert!(!window.yield_due(due, budget, true));
        let later = due + Duration::from_secs(1);
        assert!(!window.yield_due(later, 0, true));
        assert!(!window.yield_due(later, budget, false));
        window.resumed(later);
        assert!(!window.yield_due(later, budget, true));
        assert!(window.yield_due(later + Duration::from_micros(budget), budget, true));
    }

    #[test]
    fn explicit_yield_wait_is_not_charged_to_the_next_run_budget() {
        let now = Instant::now();
        let budget = FIXED_WORKER_RUN_QUANTUM_US;
        let mut window = PollWindow::new(now);
        let due = now + Duration::from_micros(budget);
        assert!(window.active(due, true));
        assert!(window.yield_due(due, budget, true));
        let returned = due + Duration::from_secs(1);
        window.resumed(returned);
        assert!(!window.yield_due(returned, budget, true));
        assert!(
            !window.active(returned, false),
            "resumption must not rearm active polling"
        );
        assert!(!window.yield_due(returned + Duration::from_micros(budget - 1), budget, true));
        assert!(window.yield_due(returned + Duration::from_micros(budget), budget, true));
    }

    fn test_client(input: Vec<u8>) -> (Client, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        peer.set_nodelay(true).unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let (stream, _) = listener.accept().unwrap();
        stream.set_nonblocking(true).unwrap();
        (
            Client {
                stream,
                session: Session::new(),
                input,
                output: Vec::new(),
                sent: 0,
                closing: false,
                eof: false,
                retry_io: false,
                activity: Instant::now(),
                _permit: Some(Permit(Arc::new(AtomicUsize::new(1)))),
                shutdown_on_drop: true,
                pubsub: std::sync::mpsc::sync_channel(1).1,
            },
            peer,
        )
    }

    #[test]
    fn a_fresh_complete_read_executes_in_the_same_pass_exactly_once() {
        let (mut client, mut peer) = test_client(Vec::new());
        let request = b"*2\r\n$4\r\nINCR\r\n$7\r\ncounter\r\n";
        peer.write_all(request).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut queued = [0; 128];
        loop {
            if client
                .stream
                .peek(&mut queued)
                .is_ok_and(|n| n == request.len())
            {
                break;
            }
            assert!(Instant::now() < deadline);
            thread::yield_now();
        }
        let store = Store::new(1024);
        let config = Config::default();
        assert_eq!(
            client.poll(&store, &config, &mut [0; 1024]).unwrap(),
            Some(true)
        );
        let mut reply = [0; 4];
        peer.read_exact(&mut reply).unwrap();
        assert_eq!(&reply, b":1\r\n");
        assert!(client.input.is_empty());
        assert_eq!(
            client.poll(&store, &config, &mut [0; 1024]).unwrap(),
            Some(false)
        );
        let mut output = Vec::new();
        store.execute(&[b"GET", b"counter"], &mut output);
        assert_eq!(output, b"$1\r\n1\r\n");
    }

    #[test]
    fn ready_input_still_obeys_the_command_quantum() {
        let request = b"*2\r\n$4\r\nINCR\r\n$7\r\ncounter\r\n";
        let (mut client, mut peer) = test_client(request.repeat(QUANTUM + 1));
        let store = Store::new(1024);
        let config = Config::default();
        assert_eq!(
            client.poll(&store, &config, &mut [0; 1024]).unwrap(),
            Some(true)
        );
        assert_eq!(client.input, request);
        let expected: Vec<u8> = (1..=QUANTUM)
            .flat_map(|n| format!(":{n}\r\n").into_bytes())
            .collect();
        let mut reply = vec![0; expected.len()];
        peer.read_exact(&mut reply).unwrap();
        assert_eq!(reply, expected);
        assert_eq!(
            client.poll(&store, &config, &mut [0; 1024]).unwrap(),
            Some(true)
        );
        let expected = format!(":{}\r\n", QUANTUM + 1);
        let mut reply = vec![0; expected.len()];
        peer.read_exact(&mut reply).unwrap();
        assert_eq!(reply, expected.as_bytes());
        assert!(client.input.is_empty());
    }

    #[test]
    fn buffered_work_is_not_dropped_because_the_worker_was_descheduled() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let (stream, _) = listener.accept().unwrap();
        stream.set_nonblocking(true).unwrap();
        let mut client = Client {
            stream,
            session: Session::new(),
            input: b"*1\r\n$4\r\nPING\r\n".to_vec(),
            output: Vec::new(),
            sent: 0,
            closing: false,
            eof: false,
            retry_io: false,
            activity: Instant::now() - Duration::from_secs(2),
            _permit: Some(Permit(Arc::new(AtomicUsize::new(1)))),
            shutdown_on_drop: true,
            pubsub: std::sync::mpsc::sync_channel(1).1,
        };
        let config = Config {
            idle_timeout: Duration::from_secs(1),
            ..Config::default()
        };
        assert_eq!(
            client
                .poll(&Store::new(100), &config, &mut [0; 128])
                .unwrap(),
            Some(true)
        );
        let mut reply = [0; 7];
        peer.read_exact(&mut reply).unwrap();
        assert_eq!(&reply, b"+PONG\r\n");
        client.activity = Instant::now() - Duration::from_secs(2);
        assert_eq!(
            client
                .poll(&Store::new(100), &config, &mut [0; 128])
                .unwrap(),
            None
        );
    }
}
