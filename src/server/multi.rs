//! Readiness reactor for opt-in multi-owner execution.
//!
//! The default single-owner reactor never leaves its thread. In multi-owner
//! mode a command may belong to another worker, so this loop submits an ordered
//! same-owner batch and polls the reply instead of blocking on a channel. A
//! connection has at most one active run; its reply slots are emitted strictly
//! in request order, including local commands that fall back to the synchronous
//! path.

use super::pool::Client;
use super::*;
use crate::engine::{
    Action, CommandReply, DeferredBatch, Execution, MultiOwner, OwnerId, ServerServices,
};
use mio::unix::SourceFd;
use mio::{Events, Interest, Poll, Token};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::os::fd::AsRawFd;

const LISTENER: Token = Token(0);
const ACCEPT_QUANTUM: usize = 64;
const RUN_QUANTUM: usize = 64;
const STOP_TICK: Duration = Duration::from_millis(10);
const ASYNC_TICK: Duration = Duration::from_millis(1);
const MAINTENANCE_TICK: Duration = Duration::from_millis(100);
const FAST_WAIT: Duration = Duration::from_micros(8);
/// Owner replies arrive on a oneshot, not a socket readiness edge. A short
/// timed poll is the wakeup; it is a poll, not a blocking receive, so other
/// connections keep being serviced between checks.
const DEFERRED_TICK: Duration = Duration::from_micros(50);
const INTERNAL_ERROR: &[u8] = b"-ERR internal store error\r\n";

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct ReadyKey {
    slot: usize,
    token: Token,
}

type CommandArguments = Arc<Vec<Vec<u8>>>;

struct Batch {
    owner: OwnerId,
    positions: Vec<usize>,
    commands: Option<Vec<CommandArguments>>,
    pending: Option<DeferredBatch>,
    bytes: usize,
}

impl Batch {
    fn new(owner: OwnerId) -> Self {
        Self {
            owner,
            positions: Vec::new(),
            commands: Some(Vec::new()),
            pending: None,
            bytes: 0,
        }
    }
    fn finished(&self) -> bool {
        self.commands.is_none() && self.pending.is_none()
    }
}

/// One connection's parsed deferred run. Slots are the original pipeline
/// positions, independent of which owner receives each command.
struct BatchRun {
    slots: Vec<Option<CommandReply>>,
    /// Original arguments and start time for the slowlog entry, shared with the
    /// owner request rather than copied.
    slow: Vec<Option<(Instant, CommandArguments)>>,
    next: usize,
    batches: Vec<Batch>,
    last_for_owner: BTreeMap<usize, usize>,
    tail: Vec<u8>,
    tail_close: bool,
}

impl BatchRun {
    fn new() -> Self {
        Self {
            slots: Vec::new(),
            slow: Vec::new(),
            next: 0,
            batches: Vec::new(),
            last_for_owner: BTreeMap::new(),
            tail: Vec::new(),
            tail_close: false,
        }
    }
    fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
    fn len(&self) -> usize {
        self.slots.len()
    }
    fn push(
        &mut self,
        command: crate::engine::DeferredCommand,
        started: Instant,
        max_bytes: usize,
    ) {
        let position = self.slots.len();
        self.slots.push(None);
        let arguments = command.arguments;
        let bytes: usize = arguments.iter().map(Vec::len).sum();
        let index = match self.last_for_owner.get(&command.owner).copied() {
            Some(index) if self.batches[index].bytes.saturating_add(bytes) <= max_bytes => index,
            _ => {
                let index = self.batches.len();
                self.batches.push(Batch::new(OwnerId::new(command.owner)));
                self.last_for_owner.insert(command.owner, index);
                index
            }
        };
        let batch = &mut self.batches[index];
        batch
            .commands
            .as_mut()
            .expect("a run is assembled before submission")
            .push(Arc::clone(&arguments));
        batch.positions.push(position);
        batch.bytes = batch.bytes.saturating_add(bytes);
        self.slow.push(Some((started, arguments)));
    }
    fn complete(&self) -> bool {
        self.next == self.slots.len() && self.batches.iter().all(Batch::finished)
    }
    fn fail(&mut self) {
        for slow in self.slow.iter_mut().skip(self.next) {
            *slow = None;
        }
        for slot in self.slots.iter_mut().skip(self.next) {
            if slot.is_none() {
                *slot = Some(CommandReply {
                    action: Action::Close,
                    bytes: INTERNAL_ERROR.to_vec(),
                });
            }
        }
        self.batches.clear();
    }
}

struct Connection {
    token: Token,
    client: Client,
    services: Arc<ServerServices>,
    run: Option<BatchRun>,
    queued: bool,
    asynchronous: bool,
    writable: bool,
    read_closed: bool,
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.services.pubsub_remove_client(self.client.session.id());
        self.services.unregister_client(self.client.session.id());
    }
}

struct Reactor {
    poll: Poll,
    listener: TcpListener,
    tokens: HashMap<Token, usize>,
    clients: Vec<Option<Connection>>,
    free_slots: Vec<usize>,
    ready: VecDeque<ReadyKey>,
    asynchronous: HashSet<ReadyKey>,
    deferred: HashSet<ReadyKey>,
    next_token: usize,
    accept_ready: bool,
    config: Config,
    store: Arc<MultiOwner>,
    admitted: Arc<AtomicUsize>,
    stopping: Arc<AtomicBool>,
}

impl Drop for Reactor {
    fn drop(&mut self) {
        for connection in self.clients.iter().flatten() {
            let fd = connection.client.stream.as_raw_fd();
            let _ = self.poll.registry().deregister(&mut SourceFd(&fd));
        }
        self.clients.clear();
    }
}

impl Reactor {
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
            return;
        }
        let connection = slot.take().expect("validated occupied connection slot");
        self.tokens.remove(&key.token);
        self.asynchronous.remove(&key);
        self.deferred.remove(&key);
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
                services: self.store.services_handle(),
                run: None,
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
        Ok(())
    }

    /// Build ordered owner batches from the commands already buffered for this
    /// connection. Only complete frames are consumed; a partial frame stays.
    fn poll_deferred(
        client: &mut Client,
        run: &mut Option<BatchRun>,
        router: &MultiOwner,
        config: &Config,
        chunk: &mut [u8],
    ) -> io::Result<Option<bool>> {
        client.retry_io = false;
        let mut progress = client.flush()?;
        if client.closing {
            return Ok(if client.output.is_empty() {
                None
            } else {
                client.open(progress, config.idle_timeout)
            });
        }
        if client.session.is_subscribed()
            || client.session.is_monitoring()
            || client.session.is_blocked()
            || client.session.is_paused()
        {
            return client.poll(router, config, chunk);
        }
        if !client.output.is_empty() {
            return Ok(client.open(progress, config.idle_timeout));
        }
        let read_first = client.input.is_empty() && !client.eof;
        if read_first {
            progress |= client.receive(config.limits, chunk)?;
            if client.input.is_empty() || client.closing {
                return Ok(client.open(progress, config.idle_timeout));
            }
        }
        let (max_commands, max_bytes) = router.defer_limits();
        let mut consumed = 0usize;
        let mut need_input = false;
        let mut scratch = Vec::new();
        let mut dispatched = 0usize;
        while dispatched < RUN_QUANTUM
            && run.as_ref().is_none_or(|batch| batch.len() < max_commands)
        {
            if consumed == client.input.len() {
                need_input = true;
                break;
            }
            if client.input[consumed] != b'*' {
                if run.is_some() {
                    break;
                }
                let started = Instant::now();
                let outcome = dispatch_request(&client.input[consumed..], config.limits, |args| {
                    if args.is_empty() {
                        return crate::session::DeferredOutcome::Complete(Action::Continue);
                    }
                    client
                        .session
                        .execute_deferred_on(router, args, &mut client.output)
                });
                match outcome {
                    Ok(Some((outcome, used))) => {
                        consumed += used;
                        dispatched += 1;
                        progress = true;
                        match outcome {
                            crate::session::DeferredOutcome::Deferred { command, action } => {
                                let batch = run.get_or_insert_with(BatchRun::new);
                                batch.push(command, started, max_bytes);
                                if action == Action::Close {
                                    batch.tail_close = true;
                                }
                            }
                            crate::session::DeferredOutcome::Complete(action) => {
                                if action == Action::Close {
                                    client.closing = true;
                                    break;
                                }
                            }
                        }
                        continue;
                    }
                    Ok(None) => {
                        need_input = true;
                        break;
                    }
                    Err(error) => {
                        write!(client.output, "-ERR Protocol error: {error}\r\n")?;
                        client.closing = true;
                        progress = true;
                        break;
                    }
                }
            }
            let request =
                match crate::protocol::parse_request(&client.input[consumed..], config.limits) {
                    Ok(Some(request)) => request,
                    Ok(None) => {
                        need_input = true;
                        break;
                    }
                    Err(error) => {
                        write!(client.output, "-ERR Protocol error: {error}\r\n")?;
                        client.closing = true;
                        progress = true;
                        break;
                    }
                };
            let args = request.arguments;
            if run.is_some() && !client.session.may_defer(router, &args) {
                break;
            }
            let target = if run.is_some() {
                &mut scratch
            } else {
                &mut client.output
            };
            let started = Instant::now();
            let outcome = client.session.execute_deferred_on(router, &args, target);
            consumed += request.consumed;
            dispatched += 1;
            progress = true;
            match outcome {
                crate::session::DeferredOutcome::Deferred { command, action } => {
                    let batch = run.get_or_insert_with(BatchRun::new);
                    batch.push(command, started, max_bytes);
                    if action == Action::Close {
                        batch.tail_close = true;
                    }
                }
                crate::session::DeferredOutcome::Complete(action) => {
                    if let Some(batch) = run.as_mut() {
                        batch.tail.extend_from_slice(&scratch);
                        scratch.clear();
                        if action == Action::Close {
                            batch.tail_close = true;
                            break;
                        }
                        break;
                    }
                    if action == Action::Close {
                        client.closing = true;
                        break;
                    }
                }
            }
        }
        if consumed == client.input.len() {
            client.input.clear();
        } else if consumed > 0 {
            client.input.drain(..consumed);
        }
        if let Some(batch) = run.as_mut()
            && batch.is_empty()
        {
            *run = None;
        }
        progress |= client.flush()?;
        if client.closing || !client.output.is_empty() || !need_input {
            return Ok(client.open(progress, config.idle_timeout));
        }
        if client.eof {
            if client.input.is_empty() {
                return Ok(None);
            }
            client
                .output
                .extend_from_slice(b"-ERR Protocol error: unexpected end of stream\r\n");
            client.closing = true;
            return Ok(Some(true));
        }
        if !read_first {
            progress |= client.receive(config.limits, chunk)?;
        }
        Ok(client.open(progress, config.idle_timeout))
    }

    fn submit_run(
        router: &MultiOwner,
        run: &mut BatchRun,
    ) -> Result<bool, crate::engine::RouterError> {
        for batch in &mut run.batches {
            if batch.pending.is_some() {
                continue;
            }
            let Some(commands) = batch.commands.take() else {
                continue;
            };
            match router.try_submit_batch(batch.owner, batch.positions.clone(), commands)? {
                crate::engine::DeferredSubmission::Submitted(pending) => {
                    batch.pending = Some(pending);
                }
                crate::engine::DeferredSubmission::Full(commands) => {
                    // Admission is momentarily full; keep the commands and retry
                    // on a later pass without consuming more input.
                    batch.commands = Some(commands);
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    fn emit_run(router: &MultiOwner, client: &mut Client, run: &mut BatchRun) -> bool {
        let mut progress = false;
        while run.next < run.slots.len() {
            let Some(reply) = run.slots[run.next].take() else {
                break;
            };
            let slow = run.slow.get_mut(run.next).and_then(Option::take);
            run.next += 1;
            let start = client
                .session
                .append_deferred_reply(&mut client.output, &reply.bytes);
            if client.output[start..].starts_with(b"-") {
                router.record_error(&client.output[start..]);
            }
            if let Some((started, arguments)) = slow {
                let borrowed: Vec<&[u8]> = arguments.iter().map(Vec::as_slice).collect();
                client
                    .session
                    .record_deferred_slowlog(router, &borrowed, started);
            }
            if reply.action == Action::Close {
                client.closing = true;
            }
            progress = true;
        }
        if run.complete() {
            if !run.tail.is_empty() {
                client.output.extend_from_slice(&run.tail);
                run.tail.clear();
            }
            if run.tail_close {
                client.closing = true;
            }
        }
        progress
    }

    fn drive_run(
        router: &MultiOwner,
        client: &mut Client,
        run: &mut BatchRun,
    ) -> Result<bool, crate::engine::RouterError> {
        let mut progress = false;
        // A later batch may be waiting for admission while an earlier one is
        // already running. Always poll every pending batch; otherwise its reply
        // credit could stay charged forever waiting for the full lane to clear.
        Self::submit_run(router, run)?;
        for batch in &mut run.batches {
            let Some(pending) = batch.pending.as_mut() else {
                continue;
            };
            if let Some(replies) = router.try_finish_batch(pending)? {
                if replies.len() != batch.positions.len() {
                    return Err(crate::engine::RouterError::Stopped);
                }
                for (position, reply) in batch.positions.iter().copied().zip(replies) {
                    run.slots[position] = Some(reply);
                }
                batch.pending = None;
                progress = true;
            }
        }
        progress |= Self::emit_run(router, client, run);
        Ok(progress)
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
        connection.client.retry_io = false;
        let mut progress = false;
        if connection.run.is_some() {
            let (client, run) = (&mut connection.client, connection.run.as_mut().unwrap());
            match Self::drive_run(self.store.as_ref(), client, run) {
                Ok(made) => progress |= made,
                Err(_) => {
                    let _ = self.store.poison();
                    run.fail();
                    progress |= Self::emit_run(self.store.as_ref(), client, run);
                }
            }
            if run.complete() {
                connection.run = None;
                // The run was built from bytes already read from the socket.
                // Edge-triggered readiness will not fire again for that buffer,
                // so own the next parse pass explicitly.
                connection.queued = true;
                self.ready.push_back(key);
            }
            progress |= connection.client.flush()?;
        } else {
            // A peer FIN may arrive behind queued pipeline bytes while a command
            // is deferred. Confirm EOF through bounded reads, not a hint.
            if connection.read_closed
                && (connection.client.session.is_blocked() || connection.client.session.is_paused())
                && !connection.client.input.is_empty()
                && !connection.client.closing
            {
                progress |= connection.client.receive(self.config.limits, chunk)?;
                if connection.client.eof {
                    return Ok(None);
                }
            }
            let retry_before_dispatch = connection.client.retry_io;
            let Some(made_progress) = Self::poll_deferred(
                &mut connection.client,
                &mut connection.run,
                self.store.as_ref(),
                &self.config,
                chunk,
            )?
            else {
                return Ok(None);
            };
            progress |= made_progress;
            if connection.run.is_some() {
                let run = connection.run.as_mut().unwrap();
                match Self::drive_run(self.store.as_ref(), &mut connection.client, run) {
                    Ok(made) => progress |= made,
                    Err(_) => {
                        let _ = self.store.poison();
                        run.fail();
                        progress |=
                            Self::emit_run(self.store.as_ref(), &mut connection.client, run);
                    }
                }
                if run.complete() {
                    connection.run = None;
                }
            }
            progress |= connection.client.flush()?;
            if connection.run.is_some() {
                self.deferred.insert(key);
            } else {
                self.deferred.remove(&key);
            }
            let needs_async = connection.client.session.is_blocked()
                || connection.client.session.is_paused()
                || connection.client.session.is_subscribed()
                || connection.client.session.is_monitoring();
            if needs_async != connection.asynchronous {
                if needs_async {
                    self.asynchronous.insert(key);
                } else {
                    self.asynchronous.remove(&key);
                }
                connection.asynchronous = needs_async;
            }
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
            let runnable = progress || retry_before_dispatch || connection.client.retry_io;
            if runnable {
                connection.queued = true;
                self.ready.push_back(key);
            }
            return Ok(Some(runnable));
        }
        if connection.run.is_some() {
            self.deferred.insert(key);
        } else {
            self.deferred.remove(&key);
        }
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
        Ok(Some(progress))
    }

    fn ingest(&mut self, events: &Events) {
        for event in events {
            let token = event.token();
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
        let mut deferred = maintenance;
        while !self.stopping.load(Ordering::Acquire) && !self.store.shutdown_requested() {
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
            if !self.deferred.is_empty() && now.duration_since(deferred) >= DEFERRED_TICK {
                let keys: Vec<_> = self.deferred.iter().copied().collect();
                for key in keys {
                    self.enqueue(key);
                }
                deferred = now;
            }
            let timeout = if self.accept_ready || !self.ready.is_empty() {
                Duration::ZERO
            } else if !self.deferred.is_empty() {
                DEFERRED_TICK
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
                    Ok(Some(_)) => serviced += 1,
                    Ok(None) | Err(_) => self.remove(key),
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
    store: Arc<MultiOwner>,
    stopping: Arc<AtomicBool>,
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
        deferred: HashSet::new(),
        next_token: 1,
        accept_ready: false,
        config,
        store,
        admitted: Arc::new(AtomicUsize::new(0)),
        stopping,
    }
    .run()
}
