//! Managed thread-per-client I/O, or an opt-in bounded nonblocking worker pool.

use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, sync_channel};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::engine::{Action, Auth, EvictionPolicy, FsyncPolicy, NotifyFlags, Store};
use crate::protocol::{Limits, dispatch_request};
use crate::session::Session;
use std::path::PathBuf;

mod handoff;
mod pool;

/// Busy clients may briefly lend I/O to an existing client thread on one CPU.
pub const CLIENT_HANDOFF_IDLE_US: u64 = 50;
/// Continuous traffic cannot retain a coordinator/admission slot indefinitely.
pub const CLIENT_HANDOFF_MAX_EPOCH_MS: u64 = 100;
/// v2 requires recent reply activity before creating a new handoff epoch.
pub const CLIENT_HANDOFF_POLICY_VERSION: u64 = 2;

/// Fixed workers keep polling briefly after progress, then return to idle parking.
/// This is bounded active polling, not readiness notification or free CPU work.
pub const FIXED_WORKER_ACTIVE_POLL_US: u64 = 50;
/// A cooperative hint between bounded passes, not a hard real-time deadline.
pub const FIXED_WORKER_RUN_QUANTUM_US: u64 = 100;
/// v2 starts a fresh run interval after a voluntary yield/park returns.
pub const FIXED_WORKER_RUN_POLICY_VERSION: u64 = 2;

/// Immutable bind-time scheduling decisions. Capacity is a hint, not a CPU budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SchedulingPolicy {
    pub active_poll_us: u64,
    pub run_quantum_us: u64,
    pub single_reply_yield: bool,
    pub client_handoff_idle_us: u64,
}
impl SchedulingPolicy {
    /// Version of this binary's multi-lane run policy; zero means no run budget.
    pub fn run_policy_version(self) -> u64 {
        if self.run_quantum_us == 0 {
            0
        } else {
            FIXED_WORKER_RUN_POLICY_VERSION
        }
    }

    /// Version of the captured client I/O policy; zero means disabled.
    pub fn client_handoff_policy_version(self) -> u64 {
        if self.client_handoff_idle_us == 0 {
            0
        } else {
            CLIENT_HANDOFF_POLICY_VERSION
        }
    }

    /// Captured policy's maximum epoch; checked between bounded passes.
    pub fn client_handoff_max_epoch_ms(self) -> u64 {
        if self.client_handoff_idle_us == 0 {
            0
        } else {
            CLIENT_HANDOFF_MAX_EPOCH_MS
        }
    }

    fn new(workers: Option<usize>, capacity: Option<usize>) -> Self {
        Self {
            active_poll_us: if workers.is_some() {
                FIXED_WORKER_ACTIVE_POLL_US
            } else {
                0
            },
            run_quantum_us: if workers.is_some_and(|n| n > 1) {
                FIXED_WORKER_RUN_QUANTUM_US
            } else {
                0
            },
            single_reply_yield: workers.is_none() && capacity == Some(1),
            client_handoff_idle_us: if workers.is_none() && capacity == Some(1) {
                CLIENT_HANDOFF_IDLE_US
            } else {
                0
            },
        }
    }
    fn yield_reply(self, commands: usize, permit: &Permit) -> bool {
        self.single_reply_yield && commands == 1 && permit.has_peers()
    }
}

// Also serves as a scheduling hint in blocking mode; admission there remains
// bounded by managed workers. Construct before spawn so failure drops the permit.
struct Permit(Arc<AtomicUsize>);
impl Permit {
    fn acquire(counter: &Arc<AtomicUsize>) -> Self {
        counter.fetch_add(1, Ordering::AcqRel);
        Self(Arc::clone(counter))
    }
    fn has_peers(&self) -> bool {
        self.0.load(Ordering::Relaxed) > 1
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Release);
    }
}

#[derive(Clone)]
pub struct Config {
    pub bind_addr: SocketAddr,
    pub max_clients: usize,
    /// None: one blocking worker per client. Some(1..=64): fixed I/O/execution lanes.
    pub worker_threads: Option<usize>,
    /// Key and value bytes only, not allocator, map, thread or network overhead.
    pub max_data_bytes: usize,
    pub limits: Limits,
    pub idle_timeout: Duration,
    /// Default-user password. `None` or empty disables authentication.
    /// Never include this value in Debug output or logs.
    pub requirepass: Option<Vec<u8>>,
    /// Optional Redis-style ACL file. Mutually exclusive with `requirepass`.
    pub aclfile: Option<PathBuf>,
    /// Enable the append-only command journal.
    pub appendonly: bool,
    /// Journal basename inside `dir`; no path separators.
    pub appendfilename: String,
    /// Journal fsync policy.
    pub appendfsync: FsyncPolicy,
    /// Directory containing the journal.
    pub dir: PathBuf,
    /// Policy used when payload admission would otherwise fail.
    pub maxmemory_policy: EvictionPolicy,
    /// Keyspace/keyevent notification classes; empty disables publication.
    pub notify_keyspace_events: NotifyFlags,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Config")
            .field("bind_addr", &self.bind_addr)
            .field("max_clients", &self.max_clients)
            .field("worker_threads", &self.worker_threads)
            .field("max_data_bytes", &self.max_data_bytes)
            .field("limits", &self.limits)
            .field("idle_timeout", &self.idle_timeout)
            .field(
                "requirepass",
                &self.requirepass.as_ref().map(|_| "<redacted>"),
            )
            .field("aclfile", &self.aclfile)
            .field("appendonly", &self.appendonly)
            .field("appendfilename", &self.appendfilename)
            .field("appendfsync", &self.appendfsync)
            .field("dir", &self.dir)
            .field("maxmemory_policy", &self.maxmemory_policy)
            .field(
                "notify_keyspace_events",
                &self.notify_keyspace_events.as_string(),
            )
            .finish()
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bind_addr: SocketAddr::from(([127, 0, 0, 1], 6379)),
            max_clients: 128,
            worker_threads: None,
            max_data_bytes: 64 * 1024 * 1024,
            limits: Limits::default(),
            idle_timeout: Duration::from_secs(30),
            requirepass: None,
            aclfile: None,
            appendonly: false,
            appendfilename: "appendonly.aof".into(),
            appendfsync: FsyncPolicy::EverySec,
            dir: PathBuf::from("."),
            maxmemory_policy: EvictionPolicy::NoEviction,
            notify_keyspace_events: NotifyFlags::default(),
        }
    }
}

impl Config {
    fn validate(&self) -> io::Result<()> {
        if self
            .worker_threads
            .is_some_and(|count| !(1..=64).contains(&count))
            || self.max_clients == 0
            || self.max_data_bytes == 0
            || self.limits.max_arguments == 0
            || self.limits.max_bulk_bytes == 0
            || self.limits.max_frame_bytes < 8
            || self.limits.max_bulk_bytes > self.limits.max_frame_bytes
            || self.limits.max_frame_bytes > 512 * 1024 * 1024
            || self.idle_timeout.is_zero()
            || self
                .requirepass
                .as_ref()
                .is_some_and(|password| password.is_empty() || password.len() > 1024)
            || (self.requirepass.is_some() && self.aclfile.is_some())
            || self.appendfilename.is_empty()
            || self.appendfilename == "."
            || self.appendfilename == ".."
            || self.appendfilename.contains(['/', '\\'])
            || self.dir.as_os_str().is_empty()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "limits and timeout must be positive; frame limit must be at most 512 MiB, bulk limit must fit within frame limit, requirepass must be 1..=1024 bytes, requirepass and aclfile are mutually exclusive, and the journal needs a basename without path separators in a nonempty directory",
            ));
        }
        Ok(())
    }
}

pub struct Server {
    listener: TcpListener,
    config: Config,
    policy: SchedulingPolicy,
    store: Store,
}

impl Config {
    /// Load the immutable bind-time ACL before accepting clients.
    fn load_auth(&self) -> io::Result<Auth> {
        match &self.aclfile {
            Some(path) => Auth::from_acl_file(path).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("cannot load ACL file {}: {error:?}", path.display()),
                )
            }),
            None => Ok(Auth::from_password(self.requirepass.as_deref())),
        }
    }
}

impl Config {
    fn journal_path(&self) -> PathBuf {
        self.dir.join(&self.appendfilename)
    }
}

impl Server {
    pub fn bind(config: Config) -> io::Result<Self> {
        config.validate()?;
        let auth = config.load_auth()?;
        let store = if config.appendonly {
            Store::with_auth_and_journal(
                config.max_data_bytes,
                auth,
                &config.journal_path(),
                config.appendfsync,
                config.limits,
            )?
        } else {
            Store::with_auth(config.max_data_bytes, auth)
        };
        store.set_eviction_policy(config.maxmemory_policy);
        store.set_notify_flags(config.notify_keyspace_events);
        let listener = TcpListener::bind(config.bind_addr)?;
        listener.set_nonblocking(true)?;
        let capacity = thread::available_parallelism().ok().map(usize::from);
        let policy = SchedulingPolicy::new(config.worker_threads, capacity);
        Ok(Self {
            listener,
            config,
            policy,
            store,
        })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Run until an OS-level process termination or an unrecoverable server error.
    /// The standard library has no portable signal-handler API.
    pub fn run(self) -> io::Result<()> {
        self.serve(Arc::new(AtomicBool::new(false)))
    }

    /// Run in a managed background thread, useful for tests and local benchmarks.
    pub fn spawn(self) -> io::Result<ServerHandle> {
        let addr = self.local_addr()?;
        let stopping = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&stopping);
        let policy = self.policy;
        let thread = thread::Builder::new()
            .name("rillcache-accept".into())
            .spawn(move || self.serve(stop))?;
        Ok(ServerHandle {
            addr,
            stopping,
            policy,
            thread: Some(thread),
        })
    }

    fn serve(self, stopping: Arc<AtomicBool>) -> io::Result<()> {
        if self.config.worker_threads.is_some() {
            return pool::serve(
                self.listener,
                self.config,
                stopping,
                self.policy,
                Arc::new(self.store),
            );
        }
        let store = Arc::new(self.store);
        let admitted = Arc::new(AtomicUsize::new(0));
        let sharing = (self.policy.client_handoff_idle_us > 0).then(|| {
            Arc::new(handoff::Sharing::new(
                self.config.clone(),
                Arc::clone(&stopping),
                self.policy.client_handoff_idle_us,
            ))
        });
        let mut workers: Vec<Worker> = Vec::new();
        let mut maintenance = Instant::now();
        let result = (|| {
            while !stopping.load(Ordering::Acquire) {
                if maintenance.elapsed() >= Duration::from_millis(100) {
                    let _ = store.journal_sync();
                    store.purge_expired(128);
                    maintenance = Instant::now();
                }
                let mut index = 0;
                while index < workers.len() {
                    if workers[index].thread.is_finished() {
                        join(workers.swap_remove(index).thread)?;
                    } else {
                        index += 1;
                    }
                }
                let (mut stream, peer) = match self.listener.accept() {
                    Ok(connection) => connection,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1));
                        continue;
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => return Err(error),
                };
                stream.set_nonblocking(false)?;
                stream.set_nodelay(true)?;
                store.record_connection();
                stream.set_read_timeout(Some(self.config.idle_timeout))?;
                stream.set_write_timeout(Some(self.config.idle_timeout))?;
                if workers.len() >= self.config.max_clients {
                    store.record_rejected_connection();
                    let _ = stream.write_all(b"-ERR max number of clients reached\r\n");
                    let _ = stream.shutdown(Shutdown::Both);
                    continue;
                }
                let control = stream.try_clone()?;
                let store = Arc::clone(&store);
                let limits = self.config.limits;
                let idle_timeout = self.config.idle_timeout;
                let policy = self.policy;
                let permit = Permit::acquire(&admitted);
                let sharing = sharing.clone();
                let (sender, receiver) = sync_channel(Store::pubsub_queue_capacity());
                let mut session = Session::new();
                session.set_pubsub_sender(sender);
                store.register_client(session.id(), peer, control.try_clone()?);
                let thread = thread::Builder::new()
                    .name("rillcache-client".into())
                    .stack_size(256 * 1024)
                    .spawn(move || {
                        // Disconnects, idle timeouts and malformed input affect only this client.
                        let context = ClientContext {
                            limits,
                            idle_timeout,
                            policy,
                            permit: &permit,
                            sharing: sharing.as_deref(),
                        };
                        let _ = serve_client(&mut stream, session, &store, &context, receiver);
                        let _ = stream.shutdown(Shutdown::Both);
                    })?;
                workers.push(Worker { control, thread });
            }
            Ok(())
        })();
        // Wake blocked readers/writers before joining; no detached workers survive shutdown.
        for worker in &workers {
            let _ = worker.control.shutdown(Shutdown::Both);
        }
        let mut result = result;
        for worker in workers {
            if let Err(error) = join(worker.thread) {
                result = result.and(Err(error));
            }
        }
        result
    }
}

struct Worker {
    control: TcpStream,
    thread: JoinHandle<()>,
}

fn join(thread: JoinHandle<()>) -> io::Result<()> {
    thread
        .join()
        .map_err(|_| io::Error::other("client worker panicked"))
}

pub struct ServerHandle {
    addr: SocketAddr,
    stopping: Arc<AtomicBool>,
    policy: SchedulingPolicy,
    thread: Option<JoinHandle<io::Result<()>>>,
}

impl ServerHandle {
    /// The policy captured at bind time and used by these workers.
    pub fn scheduling_policy(&self) -> SchedulingPolicy {
        self.policy
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn shutdown(mut self) -> io::Result<()> {
        self.stop()
    }

    fn stop(&mut self) -> io::Result<()> {
        self.stopping.store(true, Ordering::Release);
        match self.thread.take() {
            Some(thread) => thread
                .join()
                .map_err(|_| io::Error::other("accept worker panicked"))?,
            None => Ok(()),
        }
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

struct ClientContext<'a> {
    limits: Limits,
    idle_timeout: Duration,
    policy: SchedulingPolicy,
    permit: &'a Permit,
    sharing: Option<&'a handoff::Sharing>,
}

fn serve_client(
    stream: &mut TcpStream,
    mut session: Session,
    store: &Store,
    context: &ClientContext<'_>,
    receiver: Receiver<Vec<u8>>,
) -> io::Result<()> {
    let ClientContext {
        limits,
        idle_timeout,
        policy,
        permit,
        sharing,
    } = context;
    let limits = *limits;
    let idle_timeout = *idle_timeout;
    let policy = *policy;
    let client_id = session.id();
    let mut input = Vec::with_capacity(8192.min(limits.max_frame_bytes));
    let mut output = Vec::with_capacity(8192);
    let mut chunk = [0u8; 16 * 1024];
    let result = (|| {
        let mut subscriber_timeout = false;
        loop {
            // Subscribers poll their bounded message queue and the socket. The short
            // read timeout is a delivery-latency bound, not a busy-poll guarantee.
            let polling = session.is_subscribed() || session.is_blocked() || session.is_paused();
            if polling != subscriber_timeout {
                stream.set_read_timeout(Some(if polling {
                    Duration::from_millis(10)
                } else {
                    idle_timeout
                }))?;
                subscriber_timeout = polling;
            }
            let mut pushed = 0;
            while pushed < 64 {
                match receiver.try_recv() {
                    Ok(frame) => {
                        stream.write_all(&frame)?;
                        pushed += 1;
                    }
                    Err(_) => break,
                }
            }
            if session.is_blocked() {
                if let Some(reply) = session.poll_blocked(store) {
                    output.extend_from_slice(&reply);
                }
                if !output.is_empty() {
                    stream.write_all(&output)?;
                    output.clear();
                }
                if session.is_blocked() {
                    let room = (limits.max_frame_bytes - input.len()).min(chunk.len());
                    if room > 0 {
                        match stream.read(&mut chunk[..room]) {
                            Ok(0) => return Ok(()),
                            Ok(size) => input.extend_from_slice(&chunk[..size]),
                            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                            Err(error)
                                if matches!(
                                    error.kind(),
                                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                                ) => {}
                            Err(error) => return Err(error),
                        }
                    } else {
                        thread::sleep(Duration::from_millis(1));
                    }
                    continue;
                }
            }
            if session.is_paused() {
                if let Some((reply, action)) = session.poll_paused(store) {
                    output.extend_from_slice(&reply);
                    if !output.is_empty() {
                        stream.write_all(&output)?;
                        output.clear();
                    }
                    if action == Action::Close {
                        return Ok(());
                    }
                }
                if session.is_paused() || session.is_blocked() {
                    let room = (limits.max_frame_bytes - input.len()).min(chunk.len());
                    if room > 0 {
                        match stream.read(&mut chunk[..room]) {
                            Ok(0) => return Ok(()),
                            Ok(size) => input.extend_from_slice(&chunk[..size]),
                            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                            Err(error)
                                if matches!(
                                    error.kind(),
                                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                                ) => {}
                            Err(error) => return Err(error),
                        }
                    } else {
                        thread::sleep(Duration::from_millis(1));
                    }
                    continue;
                }
            }
            let mut consumed = 0;
            let mut commands = 0;
            loop {
                let (action, used) = match dispatch_request(&input[consumed..], limits, |args| {
                    if args.is_empty() {
                        Action::Continue // Redis ignores an empty inline line.
                    } else {
                        session.execute(store, args, &mut output)
                    }
                }) {
                    Ok(Some(result)) => result,
                    Ok(None) => break,
                    Err(error) => {
                        write!(output, "-ERR Protocol error: {error}\r\n")?;
                        stream.write_all(&output)?;
                        return Ok(());
                    }
                };
                consumed += used;
                commands += 1;
                if output.len() >= 64 * 1024 || action == Action::Close {
                    stream.write_all(&output)?;
                    output.clear();
                    if action != Action::Close && policy.yield_reply(commands, permit) {
                        thread::yield_now();
                    }
                    commands = 0;
                }
                if action == Action::Close {
                    return Ok(());
                }
                if session.is_blocked() || session.is_paused() {
                    break;
                }
            }
            if consumed == input.len() {
                input.clear();
            } else if consumed > 0 {
                input.drain(..consumed);
            }
            if !output.is_empty() {
                stream.write_all(&output)?;
                output.clear();
                if let Some(sharing) = sharing.as_ref().filter(|sharing| {
                    !session.is_subscribed() && permit.has_peers() && sharing.should_lend()
                }) {
                    if sharing.exchange(stream, &mut input, &mut output, &mut session, store)? {
                        return Ok(());
                    }
                    // A lease may return ready/partial input. Parse before reading again.
                    continue;
                } else if policy.yield_reply(commands, permit) {
                    thread::yield_now();
                }
            }
            let room = (limits.max_frame_bytes - input.len()).min(chunk.len());
            let polling = session.is_subscribed() || session.is_blocked() || session.is_paused();
            if polling != subscriber_timeout {
                stream.set_read_timeout(Some(if polling {
                    Duration::from_millis(10)
                } else {
                    idle_timeout
                }))?;
                subscriber_timeout = polling;
            }
            if room == 0 {
                stream.write_all(b"-ERR Protocol error: request frame limit exceeded\r\n")?;
                return Ok(());
            }
            match stream.read(&mut chunk[..room]) {
                Ok(0) => {
                    if !input.is_empty() {
                        stream.write_all(b"-ERR Protocol error: unexpected end of stream\r\n")?;
                    }
                    return Ok(());
                }
                Ok(size) => input.extend_from_slice(&chunk[..size]),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) && (session.is_subscribed()
                        || session.is_blocked()
                        || session.is_paused()) =>
                {
                    continue;
                }
                Err(error) => return Err(error),
            }
        }
    })();
    store.pubsub_remove_client(client_id);
    store.unregister_client(client_id);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheduling_hints_are_guarded_and_permits_release_on_drop() {
        let one_cpu = SchedulingPolicy::new(None, Some(1));
        let many_cpu = SchedulingPolicy::new(None, Some(4));
        assert!(!SchedulingPolicy::new(None, None).single_reply_yield);
        assert_eq!(SchedulingPolicy::new(None, None).client_handoff_idle_us, 0);
        assert_eq!(many_cpu.client_handoff_idle_us, 0);
        assert_eq!(one_cpu.client_handoff_idle_us, CLIENT_HANDOFF_IDLE_US);
        assert_eq!(one_cpu.client_handoff_policy_version(), 2);
        assert_eq!(many_cpu.client_handoff_policy_version(), 0);
        assert_eq!((one_cpu.active_poll_us, one_cpu.run_quantum_us), (0, 0));
        let counter = Arc::new(AtomicUsize::new(0));
        let first = Permit::acquire(&counter);
        assert!(!one_cpu.yield_reply(1, &first));
        {
            let _second = Permit::acquire(&counter);
            assert!(one_cpu.yield_reply(1, &first));
            for commands in [0, 2, 16, 64] {
                assert!(!one_cpu.yield_reply(commands, &first));
            }
            assert!(!many_cpu.yield_reply(1, &first));
        }
        assert_eq!(counter.load(Ordering::Acquire), 1);
        assert!(!one_cpu.yield_reply(1, &first));
        drop(first);
        assert_eq!(counter.load(Ordering::Acquire), 0);
        for workers in [1, 4, 64] {
            let policy = SchedulingPolicy::new(Some(workers), Some(1));
            assert!(!policy.single_reply_yield);
            assert_eq!(policy.client_handoff_idle_us, 0);
            assert_eq!(policy.client_handoff_policy_version(), 0);
            assert_eq!(policy.active_poll_us, FIXED_WORKER_ACTIVE_POLL_US);
            assert_eq!(
                policy.run_quantum_us,
                if workers > 1 {
                    FIXED_WORKER_RUN_QUANTUM_US
                } else {
                    0
                }
            );
        }
    }

    #[test]
    fn handle_reports_the_immutable_policy_used_by_its_workers() {
        let server = Server::bind(Config {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            worker_threads: Some(4),
            ..Config::default()
        })
        .unwrap();
        let expected = server.policy;
        let handle = server.spawn().unwrap();
        assert_eq!(handle.scheduling_policy(), expected);
        let mut copy = handle.scheduling_policy();
        copy.run_quantum_us = 0;
        assert_ne!(copy, handle.scheduling_policy());
        handle.shutdown().unwrap();
    }

    #[test]
    fn constrained_reply_hints_preserve_wire_order_quit_and_shutdown() {
        let mut server = Server::bind(Config {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            ..Config::default()
        })
        .unwrap();
        server.policy = SchedulingPolicy::new(None, Some(1));
        let handle = server.spawn().unwrap();
        assert!(handle.scheduling_policy().single_reply_yield);
        let mut first = TcpStream::connect(handle.local_addr()).unwrap();
        let mut second = TcpStream::connect(handle.local_addr()).unwrap();
        for stream in [&mut first, &mut second] {
            stream.set_nodelay(true).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            stream.write_all(b"*1\r\n$4\r\nPING\r\n").unwrap();
            let mut reply = [0; 7];
            stream.read_exact(&mut reply).unwrap();
            assert_eq!(&reply, b"+PONG\r\n");
        }
        first
            .write_all(&b"*1\r\n$4\r\nPING\r\n".repeat(16))
            .unwrap();
        let mut replies = vec![0; 7 * 16];
        first.read_exact(&mut replies).unwrap();
        assert_eq!(replies, b"+PONG\r\n".repeat(16));
        first
            .write_all(b"*1\r\n$4\r\nQUIT\r\n*3\r\n$3\r\nSET\r\n$1\r\nk\r\n$1\r\nv\r\n")
            .unwrap();
        let mut reply = Vec::new();
        first.read_to_end(&mut reply).unwrap();
        assert_eq!(reply, b"+OK\r\n");
        second.write_all(b"*2\r\n$3\r\nGET\r\n$1\r\nk\r\n").unwrap();
        let mut reply = [0; 5];
        second.read_exact(&mut reply).unwrap();
        assert_eq!(&reply, b"$-1\r\n");
        handle.shutdown().unwrap();
        assert_eq!(second.read(&mut [0]).unwrap(), 0);
    }

    #[test]
    fn rejects_invalid_configuration_before_binding() {
        for config in [
            Config {
                worker_threads: Some(0),
                ..Config::default()
            },
            Config {
                worker_threads: Some(65),
                ..Config::default()
            },
            Config {
                max_clients: 0,
                ..Config::default()
            },
            Config {
                max_data_bytes: 0,
                ..Config::default()
            },
            Config {
                idle_timeout: Duration::ZERO,
                ..Config::default()
            },
            Config {
                limits: Limits {
                    max_arguments: 0,
                    ..Limits::default()
                },
                ..Config::default()
            },
            Config {
                limits: Limits {
                    max_frame_bytes: 7,
                    ..Limits::default()
                },
                ..Config::default()
            },
            Config {
                limits: Limits {
                    max_frame_bytes: 512 * 1024 * 1024 + 1,
                    ..Limits::default()
                },
                ..Config::default()
            },
            Config {
                limits: Limits {
                    max_bulk_bytes: 4 * 1024 * 1024,
                    max_frame_bytes: 2 * 1024 * 1024,
                    ..Limits::default()
                },
                ..Config::default()
            },
            Config {
                requirepass: Some(Vec::new()),
                ..Config::default()
            },
            Config {
                requirepass: Some(vec![b'x'; 1025]),
                ..Config::default()
            },
        ] {
            assert_eq!(
                Server::bind(config).err().unwrap().kind(),
                io::ErrorKind::InvalidInput
            );
        }
    }
}
