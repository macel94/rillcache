//! Default Unix plain-TCP readiness loop. Mio uses epoll on Linux and kqueue
//! where supported. Edge-triggered readiness is retained in the runnable queue
//! until WouldBlock; bounded passes never rely on receiving a second edge.

use super::pool::{Client, LocalPoll};
use super::*;
use crate::engine::{BridgeInbox, Execution, ServerServices, StoreHandle};
use mio::unix::SourceFd;
use mio::{Events, Interest, Poll, Token, Waker};
use std::collections::{HashMap, HashSet, VecDeque};
use std::os::fd::AsRawFd;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender};

const LISTENER: Token = Token(0);
const WAKER: Token = Token(usize::MAX);
const ACCEPT_QUANTUM: usize = 64;
const RUN_QUANTUM: usize = 256;
const STOP_TICK: Duration = Duration::from_millis(10);
const ASYNC_TICK: Duration = Duration::from_millis(1);
const MAINTENANCE_TICK: Duration = Duration::from_millis(100);
/// After a productive pass the next request for a client we just answered is
/// usually already in flight. One short timed wait lets the kernel absorb it,
/// which avoids a sleep/wake cycle per command at pipeline 1. It replaces a
/// blocking wait rather than adding one, and it only runs when the runnable
/// queue is empty, so an idle server still parks for the full stop tick.
const FAST_WAIT: Duration = Duration::from_micros(8);

/// An index is reusable, but its full, non-recycled selector token is not.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct ReadyKey {
    slot: usize,
    token: Token,
}

#[derive(Debug, PartialEq, Eq)]
enum Served {
    Runnable(bool),
    Closed,
    Migrated,
}

enum Pass {
    Progress { made: bool, retry_before: bool },
    Closed,
    Migrate,
}

struct Connection {
    token: Token,
    /// Taken only when the connection migrates to the owner reactor.
    client: Option<Client>,
    store: Arc<ServerServices>,
    queued: bool,
    asynchronous: bool,
    writable: bool,
    read_closed: bool,
}

impl Connection {
    fn client(&self) -> &Client {
        self.client.as_ref().expect("active connection client")
    }

    #[cfg(test)]
    fn client_mut(&mut self) -> &mut Client {
        self.client.as_mut().expect("active connection client")
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        if let Some(client) = &self.client {
            self.store.pubsub_remove_client(client.session.id());
            self.store.unregister_client(client.session.id());
        }
    }
}

struct Reactor<E: Execution> {
    poll: Poll,
    listener: TcpListener,
    // Only kernel-event routing hashes tokens. Productive turns use indexed
    // slots directly; holes are recycled rather than growing with connection age.
    tokens: HashMap<Token, usize>,
    clients: Vec<Option<Connection>>,
    free_slots: Vec<usize>,
    ready: VecDeque<ReadyKey>,
    asynchronous: HashSet<ReadyKey>,
    next_token: usize,
    accept_ready: bool,
    config: Config,
    store: Rc<E>,
    inbox: Option<BridgeInbox>,
    admitted: Arc<AtomicUsize>,
    stopping: Arc<AtomicBool>,
    /// Only the owner/first reactor runs journal/expiry/rewrite maintenance.
    maintenance: bool,
    /// Worker reactors stop before data commands and send the connection here.
    worker: bool,
    migrate: Option<Sender<Client>>,
    incoming: Option<Receiver<Client>>,
    waker: Option<Arc<Waker>>,
}

impl<E: Execution> Drop for Reactor<E> {
    fn drop(&mut self) {
        // Cloned administration handles can keep an open-file description alive.
        // Deregister before closing any registered socket, including on errors.
        for connection in self.clients.iter().flatten() {
            let fd = connection.client().stream.as_raw_fd();
            let _ = self.poll.registry().deregister(&mut SourceFd(&fd));
        }
        self.clients.clear();
    }
}

impl<E: Execution> Reactor<E> {
    fn enqueue(&mut self, key: ReadyKey) {
        if let Some(connection) = self.clients.get_mut(key.slot).and_then(Option::as_mut)
            && connection.token == key.token
            && !connection.queued
        {
            connection.queued = true;
            self.ready.push_back(key);
        }
    }

    fn remove(&mut self, key: ReadyKey) {
        let Some(slot) = self.clients.get_mut(key.slot) else {
            return;
        };
        if slot
            .as_ref()
            .is_none_or(|connection| connection.token != key.token)
        {
            return; // A stale queued reference must not close the recycled slot.
        }
        let connection = slot.take().expect("validated occupied connection slot");
        self.tokens.remove(&key.token);
        self.asynchronous.remove(&key);
        self.free_slots.push(key.slot);
        let fd = connection.client().stream.as_raw_fd();
        let _ = self.poll.registry().deregister(&mut SourceFd(&fd));
    }

    fn accept(&mut self) -> io::Result<()> {
        for _ in 0..ACCEPT_QUANTUM {
            let (mut stream, peer) = match self.listener.accept() {
                Ok(accepted) => accepted,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    self.accept_ready = false;
                    return Ok(());
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            };
            stream.set_nonblocking(true)?;
            stream.set_nodelay(true)?;
            self.store.record_connection();
            if self.store.connected_clients() >= self.store.max_clients() {
                self.store.record_rejected_connection();
                let _ = stream.write(b"-ERR max number of clients reached\r\n");
                let _ = stream.shutdown(Shutdown::Both);
                continue;
            }
            let token = Token(self.next_token);
            self.next_token = self
                .next_token
                .checked_add(1)
                .ok_or_else(|| io::Error::other("readiness token space exhausted"))?;
            let control = stream.try_clone()?;
            let (sender, receiver) = sync_channel(Store::pubsub_queue_capacity());
            let mut session = Session::new();
            session.set_peer(peer);
            session.set_pubsub_sender(sender);
            let fd = stream.as_raw_fd();
            self.poll
                .registry()
                .register(&mut SourceFd(&fd), token, Interest::READABLE)?;
            self.store
                .register_client(session.id(), peer, stream.local_addr()?, control);
            let connection = Connection {
                token,
                client: Some(Client {
                    stream,
                    session,
                    input: Vec::with_capacity(8192.min(self.config.limits.max_frame_bytes)),
                    output: Vec::with_capacity(8192),
                    sent: 0,
                    closing: false,
                    eof: false,
                    retry_io: false,
                    activity: Instant::now(),
                    _permit: Some(Permit::acquire(&self.admitted)),
                    shutdown_on_drop: true,
                    pubsub: receiver,
                }),
                store: self.store.services_handle(),
                queued: false,
                asynchronous: false,
                writable: false,
                read_closed: false,
            };
            let slot = match self.free_slots.pop() {
                Some(slot) => {
                    debug_assert!(self.clients[slot].is_none());
                    self.clients[slot] = Some(connection);
                    slot
                }
                None => {
                    self.clients.push(Some(connection));
                    self.clients.len() - 1
                }
            };
            self.tokens.insert(token, slot);
            self.enqueue(ReadyKey { slot, token });
        }
        // Keep accept readiness after a bounded batch, even without another edge.
        Ok(())
    }

    fn service(&mut self, key: ReadyKey, chunk: &mut [u8]) -> io::Result<Served> {
        let pass = {
            let Some(connection) = self
                .clients
                .get_mut(key.slot)
                .and_then(Option::as_mut)
                .filter(|connection| connection.token == key.token)
            else {
                return Ok(Served::Closed);
            };
            connection.queued = false;
            let client = connection
                .client
                .as_mut()
                .expect("active connection client");
            client.retry_io = false;
            // A peer FIN may arrive behind queued pipeline bytes while a command
            // is deferred. Confirm EOF through bounded reads, not the hint.
            if !self.worker
                && connection.read_closed
                && (client.session.is_blocked() || client.session.is_paused())
                && !client.input.is_empty()
                && !client.closing
            {
                let received = client.receive(self.config.limits, chunk)?;
                if client.eof {
                    return Ok(Served::Closed);
                }
                if received {
                    Pass::Progress {
                        made: true,
                        retry_before: false,
                    }
                } else {
                    Pass::Progress {
                        made: false,
                        retry_before: false,
                    }
                }
            } else if self.worker {
                match client.poll_local(self.store.as_ref(), &self.config, chunk)? {
                    LocalPoll::Closed => Pass::Closed,
                    LocalPoll::Migrate => Pass::Migrate,
                    LocalPoll::Progress(made) => Pass::Progress {
                        made,
                        retry_before: false,
                    },
                }
            } else {
                let retry_before = client.retry_io;
                match client.poll(self.store.as_ref(), &self.config, chunk)? {
                    None => Pass::Closed,
                    Some(made) => Pass::Progress { made, retry_before },
                }
            }
        };
        match pass {
            Pass::Closed => Ok(Served::Closed),
            Pass::Migrate => {
                self.detach(key);
                Ok(Served::Migrated)
            }
            Pass::Progress { made, retry_before } => {
                let Some(connection) = self
                    .clients
                    .get_mut(key.slot)
                    .and_then(Option::as_mut)
                    .filter(|connection| connection.token == key.token)
                else {
                    return Ok(Served::Closed);
                };
                let client = connection
                    .client
                    .as_mut()
                    .expect("active connection client");
                let needs_async = client.session.is_blocked()
                    || client.session.is_paused()
                    || client.session.is_subscribed()
                    || client.session.is_monitoring();
                if needs_async != connection.asynchronous {
                    if needs_async {
                        self.asynchronous.insert(key);
                    } else {
                        self.asynchronous.remove(&key);
                    }
                    connection.asynchronous = needs_async;
                }
                let writable = !client.output.is_empty();
                if writable != connection.writable {
                    let interest = if writable {
                        Interest::READABLE.add(Interest::WRITABLE)
                    } else {
                        Interest::READABLE
                    };
                    let fd = client.stream.as_raw_fd();
                    self.poll
                        .registry()
                        .reregister(&mut SourceFd(&fd), key.token, interest)?;
                    connection.writable = writable;
                }
                let runnable = made || retry_before || client.retry_io;
                if runnable {
                    connection.queued = true;
                    self.ready.push_back(key);
                }
                Ok(Served::Runnable(runnable))
            }
        }
    }

    fn detach(&mut self, key: ReadyKey) {
        let Some(mut connection) = self.clients[key.slot].take() else {
            return;
        };
        self.tokens.remove(&key.token);
        self.asynchronous.remove(&key);
        self.free_slots.push(key.slot);
        let Some(client) = connection.client.take() else {
            return;
        };
        let fd = client.stream.as_raw_fd();
        let _ = self.poll.registry().deregister(&mut SourceFd(&fd));
        if let Some(sender) = &self.migrate {
            if let Err(error) = sender.send(client) {
                let client = error.0;
                self.store.pubsub_remove_client(client.session.id());
                self.store.unregister_client(client.session.id());
                return;
            }
            if let Some(waker) = &self.waker {
                let _ = waker.wake();
            }
        } else {
            self.store.pubsub_remove_client(client.session.id());
            self.store.unregister_client(client.session.id());
        }
    }

    fn attach(&mut self, client: Client) -> io::Result<()> {
        let token = Token(self.next_token);
        self.next_token = self
            .next_token
            .checked_add(1)
            .ok_or_else(|| io::Error::other("readiness token space exhausted"))?;
        let fd = client.stream.as_raw_fd();
        if let Err(error) =
            self.poll
                .registry()
                .register(&mut SourceFd(&fd), token, Interest::READABLE)
        {
            self.store.pubsub_remove_client(client.session.id());
            self.store.unregister_client(client.session.id());
            return Err(error);
        }
        let connection = Connection {
            token,
            client: Some(client),
            store: self.store.services_handle(),
            queued: false,
            asynchronous: false,
            writable: false,
            read_closed: false,
        };
        let slot = match self.free_slots.pop() {
            Some(slot) => {
                self.clients[slot] = Some(connection);
                slot
            }
            None => {
                self.clients.push(Some(connection));
                self.clients.len() - 1
            }
        };
        self.tokens.insert(token, slot);
        self.enqueue(ReadyKey { slot, token });
        Ok(())
    }

    /// Turn readiness events into queued work. Tokens are validated against the
    /// live connection, so a recycled slot never services a stale event.
    fn ingest(&mut self, events: &Events) {
        for event in events {
            let token = event.token();
            if token == WAKER {
                continue;
            }
            if token == LISTENER {
                self.accept_ready = true;
            } else if let Some(&slot) = self.tokens.get(&token) {
                if let Some(connection) = self
                    .clients
                    .get_mut(slot)
                    .and_then(Option::as_mut)
                    .filter(|connection| connection.token == token)
                {
                    connection.read_closed |= event.is_read_closed();
                }
                self.enqueue(ReadyKey { slot, token });
            }
        }
    }

    fn run(&mut self) -> io::Result<()> {
        let mut events = Events::with_capacity(1024);
        let mut chunk = [0; 16 * 1024];
        let mut maintenance = Instant::now();
        let mut asynchronous = maintenance;
        while !self.stopping.load(Ordering::Acquire) && !self.store.shutdown_requested() {
            if let Some(inbox) = &self.inbox {
                inbox.drain(self.store.as_ref());
            }
            while let Some(client) = self.incoming.as_ref().and_then(|rx| rx.try_recv().ok()) {
                self.attach(client)?;
            }
            let now = Instant::now();
            if now.duration_since(maintenance) >= MAINTENANCE_TICK {
                if self.maintenance {
                    let _ = self.store.journal_sync();
                    self.store.purge_expired(128);
                    self.store.auto_rewrite_if_due();
                }
                let expired: Vec<_> = self
                    .clients
                    .iter()
                    .enumerate()
                    .filter_map(|(slot, entry)| {
                        entry
                            .as_ref()
                            .filter(|connection| {
                                connection.client().activity.elapsed() >= self.config.idle_timeout
                            })
                            .map(|connection| ReadyKey {
                                slot,
                                token: connection.token,
                            })
                    })
                    .collect();
                for key in expired {
                    self.enqueue(key);
                }
                maintenance = now;
            }
            if !self.asynchronous.is_empty() && now.duration_since(asynchronous) >= ASYNC_TICK {
                let keys: Vec<_> = self.asynchronous.iter().copied().collect();
                for key in keys {
                    self.enqueue(key);
                }
                asynchronous = now;
            }
            // The listener is polled on every pass: a queued connection or a new
            // client must never wait behind a sustained pipeline. A zero timeout
            // keeps that pass non-blocking while work is already queued.
            let timeout = if self.accept_ready || !self.ready.is_empty() {
                Duration::ZERO
            } else if self.asynchronous.is_empty() {
                STOP_TICK
            } else {
                ASYNC_TICK
            };
            match self.poll.poll(&mut events, Some(timeout)) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
            self.ingest(&events);
            if self.accept_ready {
                self.accept()?;
            }
            let mut serviced = 0usize;
            for _ in 0..RUN_QUANTUM {
                let Some(key) = self.ready.pop_front() else {
                    break;
                };
                match self.service(key, &mut chunk) {
                    Ok(Served::Runnable(_)) => serviced += 1,
                    Ok(Served::Closed) | Err(_) => self.remove(key),
                    Ok(Served::Migrated) => {}
                }
            }
            if serviced > 0 && self.ready.is_empty() && !self.accept_ready {
                match self.poll.poll(&mut events, Some(FAST_WAIT)) {
                    Ok(()) => self.ingest(&events),
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    Err(error) => return Err(error),
                }
            }
        }
        Ok(())
    }
}

pub(super) fn serve(
    listener: TcpListener,
    config: Config,
    store: Arc<Store>,
    stopping: Arc<AtomicBool>,
) -> io::Result<()> {
    if let Some((executor, inbox)) = store.install_local() {
        serve_with(
            listener,
            config,
            Rc::new(executor),
            Some(inbox),
            stopping,
            Arc::new(AtomicUsize::new(0)),
            true,
        )
    } else {
        serve_with(
            listener,
            config,
            Rc::new(StoreHandle(store)),
            None,
            stopping,
            Arc::new(AtomicUsize::new(0)),
            true,
        )
    }
}

/// Owner-local data reactor plus connection-local worker reactors. Workers
/// execute only bare PING/ECHO/QUIT; the first data command migrates the
/// connection to the owner reactor, which keeps the published local executor.
pub(super) fn serve_shared(
    listener: TcpListener,
    config: Config,
    store: Arc<Store>,
    stopping: Arc<AtomicBool>,
    workers: usize,
) -> io::Result<()> {
    debug_assert!(workers > 1);
    let Some((executor, inbox)) = store.install_local() else {
        return serve(listener, config, store, stopping);
    };
    let poll = Poll::new()?;
    let fd = listener.as_raw_fd();
    poll.registry()
        .register(&mut SourceFd(&fd), LISTENER, Interest::READABLE)?;
    let waker = Arc::new(Waker::new(poll.registry(), WAKER)?);
    let admitted = Arc::new(AtomicUsize::new(0));
    let (migrate, incoming) = std::sync::mpsc::channel::<Client>();
    let mut handles = Vec::with_capacity(workers - 1);
    for index in 1..workers {
        let listener = listener.try_clone()?;
        let config = config.clone();
        let store = Arc::clone(&store);
        let stop = Arc::clone(&stopping);
        let admitted = Arc::clone(&admitted);
        let migrate = migrate.clone();
        let waker = Arc::clone(&waker);
        let handle = thread::Builder::new()
            .name(format!("rillcache-worker-{index}"))
            .spawn(move || {
                let result = serve_worker(
                    listener,
                    config,
                    store,
                    stop.clone(),
                    admitted,
                    migrate,
                    waker,
                );
                if result.is_err() {
                    stop.store(true, Ordering::Release);
                }
                result
            })?;
        handles.push(handle);
    }
    drop(migrate);
    let mut reactor = Reactor {
        poll,
        listener,
        tokens: HashMap::new(),
        clients: Vec::new(),
        free_slots: Vec::new(),
        ready: VecDeque::new(),
        asynchronous: HashSet::new(),
        next_token: 1,
        accept_ready: false,
        config,
        store: Rc::new(executor),
        inbox: Some(inbox),
        admitted,
        stopping: Arc::clone(&stopping),
        maintenance: true,
        worker: false,
        migrate: None,
        incoming: Some(incoming),
        waker: None,
    };
    let result = reactor.run();
    stopping.store(true, Ordering::Release);
    let mut first_error = result.err();
    for handle in handles {
        match join_result(handle) {
            Ok(()) => {}
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn serve_worker(
    listener: TcpListener,
    config: Config,
    store: Arc<Store>,
    stopping: Arc<AtomicBool>,
    admitted: Arc<AtomicUsize>,
    migrate: Sender<Client>,
    waker: Arc<Waker>,
) -> io::Result<()> {
    let poll = Poll::new()?;
    let fd = listener.as_raw_fd();
    poll.registry()
        .register(&mut SourceFd(&fd), LISTENER, Interest::READABLE)?;
    Reactor {
        poll,
        listener,
        tokens: HashMap::new(),
        clients: Vec::new(),
        free_slots: Vec::new(),
        ready: VecDeque::new(),
        asynchronous: HashSet::new(),
        next_token: 1,
        accept_ready: false,
        config,
        store: Rc::new(StoreHandle(store)),
        inbox: None,
        admitted,
        stopping,
        maintenance: false,
        worker: true,
        migrate: Some(migrate),
        incoming: None,
        waker: Some(waker),
    }
    .run()
}

fn serve_with<E: Execution>(
    listener: TcpListener,
    config: Config,
    store: Rc<E>,
    inbox: Option<BridgeInbox>,
    stopping: Arc<AtomicBool>,
    admitted: Arc<AtomicUsize>,
    maintenance: bool,
) -> io::Result<()> {
    let poll = Poll::new()?;
    let fd = listener.as_raw_fd();
    poll.registry()
        .register(&mut SourceFd(&fd), LISTENER, Interest::READABLE)?;
    Reactor {
        poll,
        listener,
        tokens: HashMap::new(),
        clients: Vec::new(),
        free_slots: Vec::new(),
        ready: VecDeque::new(),
        asynchronous: HashSet::new(),
        next_token: 1,
        accept_ready: false,
        config,
        store,
        inbox,
        admitted,
        stopping,
        maintenance,
        worker: false,
        migrate: None,
        incoming: None,
        waker: None,
    }
    .run()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Reactor<crate::engine::ShardExecutor> {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let config = Config::default();
        let store = Rc::new(crate::engine::ShardExecutor::new(config.max_data_bytes));
        store.set_max_clients(128);
        Reactor {
            poll: Poll::new().unwrap(),
            listener,
            config,
            store,
            inbox: None,
            tokens: HashMap::new(),
            clients: Vec::new(),
            free_slots: Vec::new(),
            ready: VecDeque::new(),
            asynchronous: HashSet::new(),
            next_token: 1,
            accept_ready: true,
            admitted: Arc::new(AtomicUsize::new(0)),
            stopping: Arc::new(AtomicBool::new(false)),
            maintenance: true,
            worker: false,
            migrate: None,
            incoming: None,
            waker: None,
        }
    }

    #[test]
    fn bounded_accept_retains_readiness_until_the_kernel_queue_is_drained() {
        let mut reactor = fixture();
        let peers: Vec<_> = (0..95)
            .map(|_| TcpStream::connect(reactor.listener.local_addr().unwrap()).unwrap())
            .collect();
        reactor.accept().unwrap();
        assert_eq!(reactor.tokens.len(), ACCEPT_QUANTUM);
        assert!(reactor.accept_ready, "a second edge is not guaranteed");
        reactor.accept().unwrap();
        assert_eq!(reactor.tokens.len(), peers.len());
        assert!(!reactor.accept_ready);
        assert_eq!(reactor.ready.len(), peers.len());
        let store = reactor.store.services();
        drop(reactor);
        assert_eq!(
            store.connected_clients(),
            0,
            "teardown unregisters every client"
        );
    }

    #[test]
    fn runnable_queue_deduplicates_and_ignores_stale_tokens() {
        let mut reactor = fixture();
        let _first = TcpStream::connect(reactor.listener.local_addr().unwrap()).unwrap();
        reactor.accept().unwrap();
        let old = ReadyKey {
            slot: 0,
            token: Token(1),
        };
        for _ in 0..100 {
            reactor.enqueue(old);
        }
        assert_eq!(reactor.ready.len(), 1);
        reactor.remove(old);
        let _second = TcpStream::connect(reactor.listener.local_addr().unwrap()).unwrap();
        reactor.accept().unwrap();
        let new = ReadyKey {
            slot: 0,
            token: Token(2),
        };
        assert_eq!(reactor.clients.len(), 1, "reuse the same bounded slot");
        assert_eq!(reactor.tokens.get(&Token(2)), Some(&0));
        assert!(!reactor.tokens.contains_key(&Token(1)));
        reactor.enqueue(old);
        assert_eq!(
            reactor.ready.iter().copied().collect::<Vec<_>>(),
            vec![old, new]
        );
        assert_eq!(
            reactor.service(old, &mut [0; 1024]).unwrap(),
            Served::Closed
        );
        reactor.remove(old); // Stale cleanup must not close the new occupant.
        assert_eq!(reactor.clients[0].as_ref().unwrap().token, new.token);
    }

    #[test]
    fn productive_turn_owns_exactly_one_runnable_continuation() {
        let mut reactor = fixture();
        let _peer = TcpStream::connect(reactor.listener.local_addr().unwrap()).unwrap();
        reactor.accept().unwrap();
        let key = reactor.ready.pop_front().unwrap();
        reactor.clients[key.slot]
            .as_mut()
            .unwrap()
            .client_mut()
            .input
            .extend_from_slice(b"PING\r\n");
        assert_eq!(
            reactor.service(key, &mut [0; 1024]).unwrap(),
            Served::Runnable(true)
        );
        assert!(reactor.clients[key.slot].as_ref().unwrap().queued);
        assert_eq!(reactor.ready.iter().copied().collect::<Vec<_>>(), vec![key]);
        reactor.enqueue(key);
        assert_eq!(reactor.ready.len(), 1);
        assert!(reactor.asynchronous.is_empty());
    }

    #[test]
    fn closed_slots_do_not_grow_with_total_connection_count() {
        let mut reactor = fixture();
        for _ in 0..128 {
            let _peer = TcpStream::connect(reactor.listener.local_addr().unwrap()).unwrap();
            reactor.accept().unwrap();
            let key = reactor.ready.pop_front().unwrap();
            reactor.remove(key);
            assert_eq!(reactor.clients.len(), 1);
            assert_eq!(reactor.free_slots, vec![0]);
            assert!(reactor.tokens.is_empty());
            assert!(reactor.asynchronous.is_empty());
            assert_eq!(reactor.store.connected_clients(), 0);
        }
    }
}
