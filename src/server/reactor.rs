//! Default Unix plain-TCP readiness loop. Mio uses epoll on Linux and kqueue
//! where supported. Edge-triggered readiness is retained in the runnable queue
//! until WouldBlock; bounded passes never rely on receiving a second edge.

use super::pool::Client;
use super::*;
use mio::unix::SourceFd;
use mio::{Events, Interest, Poll, Token};
use std::collections::{HashMap, HashSet, VecDeque};
use std::os::fd::AsRawFd;

const LISTENER: Token = Token(0);
const ACCEPT_QUANTUM: usize = 64;
const RUN_QUANTUM: usize = 256;
const STOP_TICK: Duration = Duration::from_millis(10);
const ASYNC_TICK: Duration = Duration::from_millis(1);
const MAINTENANCE_TICK: Duration = Duration::from_millis(100);

struct Connection {
    client: Client,
    store: Arc<Store>,
    queued: bool,
    writable: bool,
    read_closed: bool,
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.store.pubsub_remove_client(self.client.session.id());
        self.store.unregister_client(self.client.session.id());
    }
}

struct Reactor {
    poll: Poll,
    listener: TcpListener,
    clients: HashMap<Token, Connection>,
    ready: VecDeque<Token>,
    asynchronous: HashSet<Token>,
    next_token: usize,
    accept_ready: bool,
    config: Config,
    store: Arc<Store>,
    admitted: Arc<AtomicUsize>,
    stopping: Arc<AtomicBool>,
}

impl Drop for Reactor {
    fn drop(&mut self) {
        // Cloned administration handles can keep an open-file description alive.
        // Deregister before closing any registered socket, including on errors.
        for connection in self.clients.values() {
            let fd = connection.client.stream.as_raw_fd();
            let _ = self.poll.registry().deregister(&mut SourceFd(&fd));
        }
        self.clients.clear();
    }
}

impl Reactor {
    fn enqueue(&mut self, token: Token) {
        if let Some(connection) = self.clients.get_mut(&token)
            && !connection.queued
        {
            connection.queued = true;
            self.ready.push_back(token);
        }
    }

    fn remove(&mut self, token: Token) {
        if let Some(connection) = self.clients.remove(&token) {
            let fd = connection.client.stream.as_raw_fd();
            let _ = self.poll.registry().deregister(&mut SourceFd(&fd));
        }
        self.asynchronous.remove(&token);
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
            // Never recycle a token: queued events for a closed/reused fd must
            // not dispatch against a different client's session.
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
            self.clients.insert(
                token,
                Connection {
                    client: Client {
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
                    },
                    store: Arc::clone(&self.store),
                    queued: false,
                    writable: false,
                    read_closed: false,
                },
            );
            self.enqueue(token);
        }
        // Keep accept readiness after a bounded batch, even without another edge.
        Ok(())
    }

    fn service(&mut self, token: Token, chunk: &mut [u8]) -> io::Result<Option<bool>> {
        let Some(connection) = self.clients.get_mut(&token) else {
            return Ok(None);
        };
        connection.queued = false;
        let client = &mut connection.client;
        client.retry_io = false;
        let mut progress = false;
        // A peer FIN may arrive behind queued pipeline bytes while a command is
        // deferred. Confirm EOF through bounded reads, not the readiness hint.
        if connection.read_closed
            && (client.session.is_blocked() || client.session.is_paused())
            && !client.input.is_empty()
            && !client.closing
        {
            progress = client.receive(self.config.limits, chunk)?;
            if client.eof {
                return Ok(None);
            }
        }
        let retry_before_dispatch = client.retry_io;
        let Some(made_progress) = client.poll(&self.store, &self.config, chunk)? else {
            return Ok(None);
        };
        progress |= made_progress;
        if client.session.is_blocked()
            || client.session.is_paused()
            || client.session.is_subscribed()
            || client.session.is_monitoring()
        {
            self.asynchronous.insert(token);
        } else {
            self.asynchronous.remove(&token);
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
                .reregister(&mut SourceFd(&fd), token, interest)?;
            connection.writable = writable;
        }
        Ok(Some(progress || retry_before_dispatch || client.retry_io))
    }

    fn run(&mut self) -> io::Result<()> {
        let mut events = Events::with_capacity(1024);
        let mut chunk = [0; 16 * 1024];
        let mut maintenance = Instant::now();
        let mut asynchronous = maintenance;
        while !self.stopping.load(Ordering::Acquire) && !self.store.shutdown_requested() {
            let now = Instant::now();
            if now.duration_since(maintenance) >= MAINTENANCE_TICK {
                let _ = self.store.journal_sync();
                self.store.purge_expired(128);
                self.store.auto_rewrite_if_due();
                // The normal client state machine services pending input/output
                // before enforcing inactivity, even after scheduler delays.
                let expired: Vec<_> = self
                    .clients
                    .iter()
                    .filter(|(_, connection)| {
                        connection.client.activity.elapsed() >= self.config.idle_timeout
                    })
                    .map(|(&token, _)| token)
                    .collect();
                for token in expired {
                    self.enqueue(token);
                }
                maintenance = now;
            }
            if !self.asynchronous.is_empty() && now.duration_since(asynchronous) >= ASYNC_TICK {
                let tokens: Vec<_> = self.asynchronous.iter().copied().collect();
                for token in tokens {
                    self.enqueue(token);
                }
                asynchronous = now;
            }
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
            for event in &events {
                let token = event.token();
                if token == LISTENER {
                    self.accept_ready = true;
                } else {
                    if let Some(connection) = self.clients.get_mut(&token) {
                        connection.read_closed |= event.is_read_closed();
                    }
                    self.enqueue(token);
                }
            }
            if self.accept_ready {
                self.accept()?;
            }
            for _ in 0..RUN_QUANTUM {
                let Some(token) = self.ready.pop_front() else {
                    break;
                };
                match self.service(token, &mut chunk) {
                    Ok(Some(true)) => self.enqueue(token),
                    Ok(Some(false)) => {}
                    Ok(None) | Err(_) => self.remove(token),
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
    let poll = Poll::new()?;
    let fd = listener.as_raw_fd();
    poll.registry()
        .register(&mut SourceFd(&fd), LISTENER, Interest::READABLE)?;
    Reactor {
        poll,
        listener,
        clients: HashMap::new(),
        ready: VecDeque::new(),
        asynchronous: HashSet::new(),
        next_token: 1,
        accept_ready: false,
        config,
        store,
        admitted: Arc::new(AtomicUsize::new(0)),
        stopping,
    }
    .run()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Reactor {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let config = Config::default();
        let store = Arc::new(Store::new(config.max_data_bytes));
        store.set_max_clients(128);
        Reactor {
            poll: Poll::new().unwrap(),
            listener,
            config,
            store,
            clients: HashMap::new(),
            ready: VecDeque::new(),
            asynchronous: HashSet::new(),
            next_token: 1,
            accept_ready: true,
            admitted: Arc::new(AtomicUsize::new(0)),
            stopping: Arc::new(AtomicBool::new(false)),
        }
    }

    #[test]
    fn bounded_accept_retains_readiness_until_the_kernel_queue_is_drained() {
        let mut reactor = fixture();
        let peers: Vec<_> = (0..95)
            .map(|_| TcpStream::connect(reactor.listener.local_addr().unwrap()).unwrap())
            .collect();
        reactor.accept().unwrap();
        assert_eq!(reactor.clients.len(), ACCEPT_QUANTUM);
        assert!(reactor.accept_ready, "a second edge is not guaranteed");
        reactor.accept().unwrap();
        assert_eq!(reactor.clients.len(), peers.len());
        assert!(!reactor.accept_ready);
        assert_eq!(reactor.ready.len(), peers.len());
        let store = Arc::clone(&reactor.store);
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
        for _ in 0..100 {
            reactor.enqueue(Token(1));
        }
        assert_eq!(reactor.ready.len(), 1);
        reactor.remove(Token(1));
        let _second = TcpStream::connect(reactor.listener.local_addr().unwrap()).unwrap();
        reactor.accept().unwrap();
        assert!(reactor.clients.contains_key(&Token(2)));
        assert!(!reactor.clients.contains_key(&Token(1)));
        reactor.enqueue(Token(1));
        assert_eq!(
            reactor.ready.iter().copied().collect::<Vec<_>>(),
            vec![Token(1), Token(2)]
        );
        assert_eq!(reactor.service(Token(1), &mut [0; 1024]).unwrap(), None);
        assert!(reactor.clients.contains_key(&Token(2)));
    }
}
