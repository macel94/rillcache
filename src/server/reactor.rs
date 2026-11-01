//! Default Unix plain-TCP readiness loop. Mio uses epoll on Linux and kqueue
//! where supported. Edge-triggered readiness is retained in the runnable queue
//! until WouldBlock; bounded passes never rely on receiving a second edge.

use super::pool::Client;
use super::transmit::{Completed, Job, Worker};
use super::*;
use crate::engine::{BridgeInbox, Execution, ServerServices, StoreHandle};
use mio::unix::SourceFd;
use mio::{Events, Interest, Poll, Token};
use std::collections::{HashMap, HashSet, VecDeque};
use std::os::fd::AsRawFd;
use std::rc::Rc;
use std::sync::mpsc::{TryRecvError, TrySendError};

const LISTENER: Token = Token(0);
const TRANSMITTED: Token = Token(usize::MAX);
const ACCEPT_QUANTUM: usize = 64;
const RUN_QUANTUM: usize = super::transmit::MAX_BATCH;
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

struct Connection {
    token: Token,
    client: Client,
    store: Arc<ServerServices>,
    queued: bool,
    asynchronous: bool,
    writable: bool,
    read_closed: bool,
    writer_stream: Option<Arc<TcpStream>>,
    write_queued: bool,
    writing: bool,
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.store.pubsub_remove_client(self.client.session.id());
        self.store.unregister_client(self.client.session.id());
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
    writer: Option<Worker<ReadyKey>>,
    pending_output: Vec<ReadyKey>,
    config: Config,
    store: Rc<E>,
    inbox: Option<BridgeInbox>,
    admitted: Arc<AtomicUsize>,
    stopping: Arc<AtomicBool>,
}

impl<E: Execution> Drop for Reactor<E> {
    fn drop(&mut self) {
        // Cloned administration handles can keep an open-file description alive.
        // Deregister before closing any registered socket, including on errors.
        for connection in self.clients.iter().flatten() {
            let fd = connection.client.stream.as_raw_fd();
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
        let fd = connection.client.stream.as_raw_fd();
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
            let writer_stream = self
                .writer
                .as_ref()
                .and_then(|_| stream.try_clone().ok())
                .map(Arc::new);
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
                store: self.store.services_handle(),
                queued: false,
                asynchronous: false,
                writable: false,
                read_closed: false,
                writer_stream,
                write_queued: false,
                writing: false,
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

    fn service(&mut self, key: ReadyKey, chunk: &mut [u8]) -> io::Result<Option<bool>> {
        let Some(connection) = self
            .clients
            .get_mut(key.slot)
            .and_then(Option::as_mut)
            .filter(|connection| connection.token == key.token)
        else {
            return Ok(None);
        };
        connection.queued = false;
        if connection.writing {
            return Ok(Some(false));
        }
        let buffered = connection.writer_stream.is_some();
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
        let result = if buffered {
            client.poll_buffered(self.store.as_ref(), &self.config, chunk)?
        } else {
            client.poll(self.store.as_ref(), &self.config, chunk)?
        };
        let Some(made_progress) = result else {
            return Ok(None);
        };
        progress |= made_progress;
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
        if buffered && !client.output.is_empty() && !connection.write_queued {
            connection.write_queued = true;
            self.pending_output.push(key);
        }
        let writable = !client.output.is_empty();
        if !connection.write_queued && writable != connection.writable {
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
        let runnable = progress || retry_before_dispatch || client.retry_io;
        if runnable && !connection.write_queued {
            // Own the continuation here: no second token lookup/queue admission.
            connection.queued = true;
            self.ready.push_back(key);
        }
        Ok(Some(runnable))
    }

    fn refresh_interest(&mut self, key: ReadyKey) -> io::Result<()> {
        let Some(connection) = self
            .clients
            .get_mut(key.slot)
            .and_then(Option::as_mut)
            .filter(|entry| entry.token == key.token)
        else {
            return Ok(());
        };
        let writable = !connection.client.output.is_empty();
        if writable != connection.writable {
            let interest = if writable {
                Interest::READABLE.add(Interest::WRITABLE)
            } else {
                Interest::READABLE
            };
            let fd = connection.client.stream.as_raw_fd();
            self.poll
                .registry()
                .reregister(&mut SourceFd(&fd), key.token, interest)?;
            connection.writable = writable;
        }
        Ok(())
    }

    fn inline_output(&mut self, key: ReadyKey) -> io::Result<()> {
        let Some(connection) = self
            .clients
            .get_mut(key.slot)
            .and_then(Option::as_mut)
            .filter(|entry| entry.token == key.token)
        else {
            return Ok(());
        };
        connection.write_queued = false;
        let result = connection.client.flush();
        match result {
            Ok(progress) => {
                let retry = connection.client.retry_io;
                self.refresh_interest(key)?;
                if progress || retry {
                    self.enqueue(key);
                }
            }
            Err(_) => self.remove(key),
        }
        Ok(())
    }

    fn finish_write(&mut self, done: Completed<ReadyKey>) -> io::Result<()> {
        let key = done.job.key;
        let Some(connection) = self
            .clients
            .get_mut(key.slot)
            .and_then(Option::as_mut)
            .filter(|entry| entry.token == key.token && entry.writing)
        else {
            return Ok(());
        };
        connection.writing = false;
        let client = &mut connection.client;
        debug_assert!(client.output.is_empty());
        client.output = done.job.output;
        client.sent = done.job.sent;
        let retry = match done.result {
            Ok(0) => {
                self.remove(key);
                return Ok(());
            }
            Ok(size) => {
                client.sent += size;
                client.activity = done.at;
                if client.sent == client.output.len() {
                    client.output.clear();
                    client.sent = 0;
                }
                true
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => false,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => true,
            Err(_) => {
                self.remove(key);
                return Ok(());
            }
        };
        self.refresh_interest(key)?;
        if retry {
            self.enqueue(key);
        }
        Ok(())
    }

    fn drain_writes(&mut self) -> io::Result<()> {
        loop {
            let Some(writer) = &self.writer else {
                return Ok(());
            };
            match writer.completed() {
                Ok(done) => self.finish_write(done)?,
                Err(TryRecvError::Empty) => return Ok(()),
                Err(TryRecvError::Disconnected) => {
                    return Err(io::Error::other("transmit worker exited"));
                }
            }
        }
    }

    fn inline_jobs(&mut self, jobs: Vec<Job<ReadyKey>>) -> io::Result<()> {
        for job in jobs {
            let key = job.key;
            if let Some(connection) = self
                .clients
                .get_mut(key.slot)
                .and_then(Option::as_mut)
                .filter(|entry| entry.token == key.token)
            {
                connection.writing = false;
                connection.client.output = job.output;
                connection.client.sent = job.sent;
                self.inline_output(key)?;
            }
        }
        Ok(())
    }

    fn flush_outputs(&mut self) -> io::Result<()> {
        let mut keys = std::mem::take(&mut self.pending_output);
        if keys.len() < 2 {
            for key in keys.drain(..) {
                self.inline_output(key)?;
            }
            self.pending_output = keys;
            return Ok(());
        }
        let mut jobs = Vec::with_capacity(keys.len());
        for key in keys.drain(..) {
            let Some(connection) = self
                .clients
                .get_mut(key.slot)
                .and_then(Option::as_mut)
                .filter(|entry| entry.token == key.token && entry.write_queued && !entry.writing)
            else {
                continue;
            };
            connection.write_queued = false;
            if connection.client.output.is_empty() {
                continue;
            }
            connection.writing = true;
            jobs.push(Job {
                key,
                stream: Arc::clone(connection.writer_stream.as_ref().expect("buffered socket")),
                output: std::mem::take(&mut connection.client.output),
                sent: std::mem::take(&mut connection.client.sent),
            });
        }
        self.pending_output = keys;
        if jobs.len() < 2 {
            return self.inline_jobs(jobs);
        }
        match self
            .writer
            .as_ref()
            .expect("buffered outputs require worker")
            .submit(jobs)
        {
            Ok(()) => {}
            Err(TrySendError::Full(jobs)) => self.inline_jobs(jobs)?,
            Err(TrySendError::Disconnected(_)) => {
                return Err(io::Error::other("transmit worker unavailable"));
            }
        }
        Ok(())
    }

    /// Turn readiness events into queued work. Tokens are validated against the
    /// live connection, so a recycled slot never services a stale event.
    fn ingest(&mut self, events: &Events) {
        for event in events {
            let token = event.token();
            if token == LISTENER {
                self.accept_ready = true;
            } else if token == TRANSMITTED {
                // Completion ownership is drained before servicing sockets.
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
            self.drain_writes()?;
            if let Some(inbox) = &self.inbox {
                inbox.drain(self.store.as_ref());
            }
            let now = Instant::now();
            if now.duration_since(maintenance) >= MAINTENANCE_TICK {
                let _ = self.store.journal_sync();
                self.store.purge_expired(128);
                self.store.auto_rewrite_if_due();
                let expired: Vec<_> = self
                    .clients
                    .iter()
                    .enumerate()
                    .filter_map(|(slot, entry)| {
                        entry
                            .as_ref()
                            .filter(|connection| {
                                connection.client.activity.elapsed() >= self.config.idle_timeout
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
            self.drain_writes()?;
            if self.accept_ready {
                self.accept()?;
            }
            let mut serviced = 0usize;
            for _ in 0..RUN_QUANTUM {
                let Some(key) = self.ready.pop_front() else {
                    break;
                };
                match self.service(key, &mut chunk) {
                    Ok(Some(_)) => serviced += 1,
                    Ok(None) | Err(_) => self.remove(key),
                }
            }
            self.flush_outputs()?;
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
    write_threads: usize,
) -> io::Result<()> {
    if let Some((executor, inbox)) = store.install_local() {
        serve_with(
            listener,
            config,
            Rc::new(executor),
            Some(inbox),
            stopping,
            write_threads,
        )
    } else {
        serve_with(
            listener,
            config,
            Rc::new(StoreHandle(store)),
            None,
            stopping,
            write_threads,
        )
    }
}

fn serve_with<E: Execution>(
    listener: TcpListener,
    config: Config,
    store: Rc<E>,
    inbox: Option<BridgeInbox>,
    stopping: Arc<AtomicBool>,
    write_threads: usize,
) -> io::Result<()> {
    let poll = Poll::new()?;
    let fd = listener.as_raw_fd();
    poll.registry()
        .register(&mut SourceFd(&fd), LISTENER, Interest::READABLE)?;
    let writer = if write_threads > 0 {
        Some(Worker::new(poll.registry(), TRANSMITTED)?)
    } else {
        None
    };
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
        writer,
        pending_output: Vec::new(),
        config,
        store,
        inbox,
        admitted: Arc::new(AtomicUsize::new(0)),
        stopping,
    };
    let result = reactor.run();
    // Close every original socket before joining a worker that may still hold
    // duplicate write handles. No command/data state is sent to that worker.
    for connection in reactor.clients.iter().flatten() {
        let fd = connection.client.stream.as_raw_fd();
        let _ = reactor.poll.registry().deregister(&mut SourceFd(&fd));
    }
    reactor.clients.clear();
    let joined = reactor.writer.as_mut().map_or(Ok(()), Worker::shutdown);
    result.and(joined)
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
            writer: None,
            pending_output: Vec::new(),
            admitted: Arc::new(AtomicUsize::new(0)),
            stopping: Arc::new(AtomicBool::new(false)),
        }
    }

    #[test]
    fn batched_writes_preserve_reply_order_and_gate_later_command_side_effects() {
        let mut reactor = fixture();
        reactor.writer = Some(Worker::new(reactor.poll.registry(), TRANSMITTED).unwrap());
        let mut peers: Vec<_> = (0..2)
            .map(|_| {
                let mut peer = TcpStream::connect(reactor.listener.local_addr().unwrap()).unwrap();
                peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                peer.write_all(b"*1\r\n$4\r\nPING\r\n").unwrap();
                peer
            })
            .collect();
        reactor.accept().unwrap();
        let keys: Vec<_> = reactor.ready.drain(..).collect();
        for &key in &keys {
            assert_eq!(reactor.service(key, &mut [0; 1024]).unwrap(), Some(true));
        }
        assert_eq!(reactor.pending_output.len(), 2);
        reactor.flush_outputs().unwrap();
        assert!(
            keys.iter()
                .all(|key| reactor.clients[key.slot].as_ref().unwrap().writing)
        );
        reactor.clients[keys[0].slot]
            .as_mut()
            .unwrap()
            .client
            .input
            .extend_from_slice(b"INCR after_write\r\n");
        assert_eq!(
            reactor.service(keys[0], &mut [0; 1024]).unwrap(),
            Some(false)
        );
        let mut value = Vec::new();
        reactor.store.execute(&[b"GET", b"after_write"], &mut value);
        assert_eq!(
            value, b"$-1\r\n",
            "a later command must not run while its output is in flight"
        );
        let deadline = Instant::now() + Duration::from_secs(3);
        while keys
            .iter()
            .any(|key| reactor.clients[key.slot].as_ref().unwrap().writing)
        {
            assert!(
                Instant::now() < deadline,
                "transmit completion did not wake reactor"
            );
            let mut events = Events::with_capacity(8);
            reactor
                .poll
                .poll(&mut events, Some(Duration::from_millis(10)))
                .unwrap();
            reactor.ingest(&events);
            reactor.drain_writes().unwrap();
        }
        for peer in &mut peers {
            let mut reply = [0; 7];
            peer.read_exact(&mut reply).unwrap();
            assert_eq!(&reply, b"+PONG\r\n");
        }
        reactor.service(keys[0], &mut [0; 1024]).unwrap();
        reactor.flush_outputs().unwrap();
        let mut reply = [0; 4];
        peers[0].read_exact(&mut reply).unwrap();
        assert_eq!(&reply, b":1\r\n");
    }

    #[test]
    fn a_lone_reply_stays_inline_and_wouldblock_keeps_the_unsent_buffer() {
        let mut reactor = fixture();
        reactor.writer = Some(Worker::new(reactor.poll.registry(), TRANSMITTED).unwrap());
        let mut peer = TcpStream::connect(reactor.listener.local_addr().unwrap()).unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        peer.write_all(b"PING\r\n").unwrap();
        reactor.accept().unwrap();
        let key = reactor.ready.pop_front().unwrap();
        reactor.service(key, &mut [0; 1024]).unwrap();
        reactor.flush_outputs().unwrap();
        assert!(!reactor.clients[key.slot].as_ref().unwrap().writing);
        let mut output = [0; 7];
        peer.read_exact(&mut output).unwrap();
        assert_eq!(&output, b"+PONG\r\n");
        reactor.ready.clear();
        let connection = reactor.clients[key.slot].as_mut().unwrap();
        connection.queued = false;
        connection.writing = true;
        let stream = Arc::clone(connection.writer_stream.as_ref().unwrap());
        reactor
            .finish_write(Completed {
                job: Job {
                    key,
                    stream,
                    output: b"unsent".to_vec(),
                    sent: 2,
                },
                result: Err(io::ErrorKind::WouldBlock.into()),
                at: Instant::now(),
            })
            .unwrap();
        let connection = reactor.clients[key.slot].as_ref().unwrap();
        assert_eq!(connection.client.sent, 2);
        assert_eq!(connection.client.output, b"unsent");
        assert!(connection.writable);
        assert!(
            !connection.queued,
            "WouldBlock must wait for writable readiness, not spin"
        );
    }

    #[test]
    fn partial_completion_retains_backpressure_and_stale_completion_cannot_touch_reused_slot() {
        let mut reactor = fixture();
        reactor.writer = Some(Worker::new(reactor.poll.registry(), TRANSMITTED).unwrap());
        let _peer = TcpStream::connect(reactor.listener.local_addr().unwrap()).unwrap();
        reactor.accept().unwrap();
        let key = reactor.ready.pop_front().unwrap();
        let connection = reactor.clients[key.slot].as_mut().unwrap();
        connection.queued = false;
        connection.writing = true;
        let stream = Arc::clone(connection.writer_stream.as_ref().unwrap());
        reactor
            .finish_write(Completed {
                job: Job {
                    key,
                    stream: Arc::clone(&stream),
                    output: b"abcdef".to_vec(),
                    sent: 1,
                },
                result: Ok(2),
                at: Instant::now(),
            })
            .unwrap();
        let connection = reactor.clients[key.slot].as_ref().unwrap();
        assert_eq!(connection.client.sent, 3);
        assert_eq!(connection.client.output, b"abcdef");
        assert!(connection.writable);
        assert!(connection.queued);
        reactor.remove(key);
        let _new_peer = TcpStream::connect(reactor.listener.local_addr().unwrap()).unwrap();
        reactor.accept().unwrap();
        let token = reactor.clients[key.slot].as_ref().unwrap().token;
        assert_ne!(token, key.token);
        reactor
            .finish_write(Completed {
                job: Job {
                    key,
                    stream,
                    output: b"stale".to_vec(),
                    sent: 0,
                },
                result: Err(io::ErrorKind::BrokenPipe.into()),
                at: Instant::now(),
            })
            .unwrap();
        let replacement = reactor.clients[key.slot].as_ref().unwrap();
        assert_eq!(replacement.token, token);
        assert!(replacement.client.output.is_empty());
    }

    #[test]
    fn write_worker_keeps_healthy_clients_and_joined_stop_progressing_under_backpressure() {
        let mut server = Server::bind(Config {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            ..Config::default()
        })
        .unwrap();
        // Exercise the worker even on a one-CPU test host; production uses only
        // the immutable effective-capacity decision, never a benchmark flag.
        server.policy.mio_write_threads = 1;
        let handle = server.spawn().unwrap();
        let mut seed = TcpStream::connect(handle.local_addr()).unwrap();
        seed.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        seed.set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let value = vec![b'x'; 64 * 1024];
        let mut request = Vec::new();
        crate::protocol::encode_request(&[b"SET", b"large", &value], &mut request);
        seed.write_all(&request).unwrap();
        let mut ok = [0; 5];
        seed.read_exact(&mut ok).unwrap();
        assert_eq!(&ok, b"+OK\r\n");
        request.clear();
        crate::protocol::encode_request(&[b"PING"], &mut request);
        for _ in 0..256 {
            crate::protocol::encode_request(&[b"GET", b"large"], &mut request);
        }
        let mut slow: Vec<_> = (0..2)
            .map(|_| {
                let mut peer = TcpStream::connect(handle.local_addr()).unwrap();
                peer.set_write_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                peer.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                peer.write_all(&request).unwrap();
                let mut pong = [0; 7];
                peer.read_exact(&mut pong).unwrap();
                assert_eq!(&pong, b"+PONG\r\n");
                peer
            })
            .collect();
        seed.write_all(b"PING\r\n").unwrap();
        let mut pong = [0; 7];
        seed.read_exact(&mut pong).unwrap();
        assert_eq!(&pong, b"+PONG\r\n");
        let began = Instant::now();
        handle.shutdown().unwrap();
        assert!(began.elapsed() < Duration::from_secs(3));
        // Keep unread peers alive until shutdown has actually joined the writer.
        slow.clear();
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
        assert_eq!(reactor.service(old, &mut [0; 1024]).unwrap(), None);
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
            .client
            .input
            .extend_from_slice(b"PING\r\n");
        assert_eq!(reactor.service(key, &mut [0; 1024]).unwrap(), Some(true));
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
