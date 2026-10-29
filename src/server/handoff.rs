//! Small-budget busy-client I/O leases to an existing managed client thread.
//! Quiet connections return to their blocking owners: no new permanent poller.

use super::pool::Client;
use super::*;
use crate::session::Session;
use std::sync::atomic::AtomicU64;
use std::sync::{
    Mutex,
    mpsc::{self, Receiver, Sender},
};

/// Stable round-robin connection ownership; each group elects its own temporary
/// coordinator. Data/transaction synchronization remains shared across groups.
pub(super) struct Lanes {
    lanes: Vec<Arc<Sharing>>,
    next: usize,
}
impl Lanes {
    pub(super) fn new(
        config: &Config,
        stopping: &Arc<AtomicBool>,
        policy: SchedulingPolicy,
    ) -> Self {
        Self {
            lanes: (0..policy.client_handoff_lanes)
                .map(|_| {
                    let mut sharing = Sharing::new(
                        config.clone(),
                        Arc::clone(stopping),
                        policy.client_handoff_idle_us,
                    );
                    sharing.adaptive_poll = !policy.single_reply_yield;
                    Arc::new(sharing)
                })
                .collect(),
            next: 0,
        }
    }

    pub(super) fn assign(&mut self) -> Option<Arc<Sharing>> {
        let sharing = Arc::clone(self.lanes.get(self.next)?);
        self.next = (self.next + 1) % self.lanes.len();
        Some(sharing)
    }
}

pub(super) struct Sharing {
    leader: Mutex<Option<Sender<Loan>>>,
    leader_active: AtomicBool,
    recent_micros: AtomicU64,
    base: Instant,
    config: Config,
    stopping: Arc<AtomicBool>,
    idle: Duration,
    idle_micros: u64,
    max_epoch: Duration,
    adaptive_poll: bool,
}

/// Short processor-hint backoff after empty scans, not an OS yield or sleep.
/// Real progress resets the sequence; the existing epoch still ends idle work.
#[derive(Default)]
struct IdleBackoff {
    step: u32,
}
impl IdleBackoff {
    fn observe(&mut self, progress: bool) -> usize {
        if progress {
            self.step = 0;
            return 0;
        }
        let spins = 1 << self.step;
        self.step = (self.step + 1).min(6);
        spins
    }
}

struct EpochWindow {
    until: Instant,
    deadline: Instant,
    idle: Duration,
}
impl EpochWindow {
    fn new(now: Instant, idle: Duration, maximum: Duration) -> Self {
        Self {
            until: now + idle,
            deadline: now + maximum,
            idle,
        }
    }
    fn active(&mut self, now: Instant, progress: bool) -> bool {
        if progress {
            self.until = now + self.idle;
        }
        now < self.until && now < self.deadline
    }
}

// One outstanding loan per admitted client. Returning exactly once also bounds
// these channels; queue capacity does not depend on untrusted frame lengths.
struct Loan {
    client: Option<Client>,
    home: Sender<Option<Client>>,
    stopping: Arc<AtomicBool>,
    coordinator: bool,
}
impl Loan {
    fn quiet(&self, now: Instant, idle: Duration) -> bool {
        // The coordinator cannot block waiting for its own returned socket
        // while it still services others. At most that one quiet loan remains.
        !self.coordinator
            && self.client.as_ref().is_some_and(|client| {
                now.checked_duration_since(client.activity)
                    .is_some_and(|elapsed| elapsed >= idle)
            })
    }
}
impl Drop for Loan {
    fn drop(&mut self) {
        if self.stopping.load(Ordering::Acquire) {
            self.client.take();
        }
        let _ = self.home.send(self.client.take());
    }
}

/// Per-connection return channel and inactive lease shell. Retain the cloned
/// socket and placeholder session/receiver instead of rebuilding them per epoch.
pub(super) struct LeaseChannel {
    home: Sender<Option<Client>>,
    returned: Receiver<Option<Client>>,
    cached: Option<Client>,
}

impl LeaseChannel {
    pub(super) fn new() -> Self {
        let (home, returned) = mpsc::channel();
        Self {
            home,
            returned,
            cached: None,
        }
    }
}

/// Mutable per-connection channels used while a socket is lent.
pub(super) struct LeaseIO<'a> {
    pub(super) pubsub: &'a mut Receiver<Vec<u8>>,
    pub(super) channel: &'a mut LeaseChannel,
}

struct Leadership<'a> {
    sharing: &'a Sharing,
    receiver: Receiver<Loan>,
}
impl Drop for Leadership<'_> {
    fn drop(&mut self) {
        // Close admission to this epoch before dropping its receiver. A sender
        // cloned just before this point either succeeds into the old queue or
        // recovers its Loan on send failure. Both paths run Loan::drop.
        let mut leader = self
            .sharing
            .leader
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        leader.take();
        // Publish while holding the admission lock: an old epoch must not clear
        // the presence flag after a new leader has installed its sender.
        self.sharing.leader_active.store(false, Ordering::Release);
    }
}

impl Sharing {
    pub(super) fn new(config: Config, stopping: Arc<AtomicBool>, idle_us: u64) -> Self {
        Self {
            leader: Mutex::new(None),
            leader_active: AtomicBool::new(false),
            recent_micros: AtomicU64::new(0),
            base: Instant::now(),
            config,
            stopping,
            idle: Duration::from_micros(idle_us),
            idle_micros: idle_us,
            max_epoch: Duration::from_millis(CLIENT_HANDOFF_MAX_EPOCH_MS),
            adaptive_poll: false,
        }
    }

    /// Whether an epoch is already accepting borrowers. Kept as a separate
    /// cheap atomic check so callers and tests can inspect leader presence
    /// without the activity probe.
    pub(super) fn active(&self) -> bool {
        self.leader_active.load(Ordering::Relaxed)
    }

    /// v2 avoids lease allocation/polling for isolated sparse replies. Joining
    /// an existing epoch is allowed even if the previous local observation is old.
    pub(super) fn should_lend(&self) -> bool {
        // An installed epoch accepts every reply, so check presence first. This
        // skips the monotonic-clock read and atomic swap on the busiest path;
        // the recent-activity window only matters when deciding whether to
        // start a new epoch.
        if self.active() {
            return true;
        }
        let now = self.base.elapsed().as_micros() as u64;
        let previous = self.recent_micros.swap(now, Ordering::Relaxed);
        Self::recent_burst(previous, now, self.idle_micros)
    }

    fn recent_burst(previous: u64, now: u64, idle_us: u64) -> bool {
        previous != 0
            && now
                .checked_sub(previous)
                .is_some_and(|elapsed| elapsed < idle_us)
    }

    /// Called only after actual socket progress, with already-compacted input
    /// and fully flushed output. The original handle does no I/O while lent.
    /// Returns true if this connection has closed, with all pending replies sent.
    pub(super) fn exchange(
        &self,
        stream: &mut TcpStream,
        input: &mut Vec<u8>,
        output: &mut Vec<u8>,
        session: &mut Session,
        io: &mut LeaseIO<'_>,
        store: &Store,
    ) -> io::Result<bool> {
        debug_assert!(output.is_empty());
        if self.stopping.load(Ordering::Acquire) {
            return Ok(true);
        }
        let mut client = match io.channel.cached.take() {
            Some(client) => client,
            None => Client {
                stream: stream.try_clone()?,
                session: Session::new(),
                input: Vec::new(),
                output: Vec::new(),
                sent: 0,
                closing: false,
                eof: false,
                activity: Instant::now(),
                _permit: None,
                shutdown_on_drop: false,
                pubsub: std::sync::mpsc::sync_channel(1).1,
            },
        };
        client.stream.set_nonblocking(true)?;
        client.activity = Instant::now();
        client.closing = false;
        client.eof = false;
        client.shutdown_on_drop = true;
        std::mem::swap(session, &mut client.session);
        std::mem::swap(input, &mut client.input);
        std::mem::swap(output, &mut client.output);
        // Move the actual receiver with the lease; the parked owner gets only
        // the disconnected placeholder, reused on the next exchange.
        std::mem::swap(io.pubsub, &mut client.pubsub);
        let loan = Loan {
            client: Some(client),
            home: io.channel.home.clone(),
            stopping: Arc::clone(&self.stopping),
            coordinator: false,
        };
        let mut leader = self
            .leader
            .lock()
            .map_err(|_| io::Error::other("client I/O coordinator poisoned"))?;
        if let Some(sender) = leader.as_ref() {
            let sender = sender.clone();
            drop(leader);
            // SendError owns the loan and returns it through its destructor.
            let _ = sender.send(loan);
        } else {
            let (sender, receiver) = mpsc::channel();
            *leader = Some(sender);
            self.leader_active.store(true, Ordering::Release);
            drop(leader);
            self.run(
                loan,
                Leadership {
                    sharing: self,
                    receiver,
                },
                store,
            );
        }
        let Some(mut client) = io
            .channel
            .returned
            .recv()
            .map_err(|_| io::Error::other("client I/O lease lost"))?
        else {
            return Ok(true);
        };
        // Options propagate across cloned handles. Restore before waking the
        // original blocking loop; no coordinator still owns this client.
        client.stream.set_nonblocking(false)?;
        if client.sent > 0 {
            client.output.drain(..client.sent);
        }
        std::mem::swap(session, &mut client.session);
        std::mem::swap(io.pubsub, &mut client.pubsub);
        std::mem::swap(input, &mut client.input);
        std::mem::swap(output, &mut client.output);
        // A partially written reply was compacted above; the inactive shell has
        // no pending output. Dropping this cache must not shut down the owner.
        client.sent = 0;
        let closing = client.closing;
        client.shutdown_on_drop = false;
        io.channel.cached = Some(client);
        if !output.is_empty() {
            stream.write_all(output)?;
            output.clear();
        }
        Ok(closing)
    }

    fn run(&self, mut first: Loan, leadership: Leadership<'_>, store: &Store) {
        first.coordinator = true;
        let mut clients = vec![first];
        let mut chunk = [0; 16 * 1024];
        let mut window = EpochWindow::new(Instant::now(), self.idle, self.max_epoch);
        let mut backoff = IdleBackoff::default();
        loop {
            if self.stopping.load(Ordering::Acquire) {
                for loan in &mut clients {
                    loan.client.take();
                }
                break;
            }
            let mut progress = false;
            while let Ok(loan) = leadership.receiver.try_recv() {
                clients.push(loan);
                progress = true;
            }
            // One clock sample per pass: per-client quiet checks and the epoch
            // window share it instead of reading the clock once per client.
            let now = Instant::now();
            let mut index = 0;
            while index < clients.len() {
                match clients[index].client.as_mut().expect("active loan").poll(
                    store,
                    &self.config,
                    &mut chunk,
                ) {
                    Ok(Some(made_progress)) => {
                        progress |= made_progress;
                        if !made_progress && clients[index].quiet(now, self.idle) {
                            clients.swap_remove(index); // Return this borrower, not a busy epoch.
                        } else {
                            index += 1;
                        }
                    }
                    _ => {
                        // EOF/error/timeout: shut down before waking the owner.
                        let mut loan = clients.swap_remove(index);
                        loan.client.take();
                    }
                }
            }
            if clients.is_empty() {
                break;
            }
            if !window.active(now, progress) {
                break;
            }
            let spins = if self.adaptive_poll {
                backoff.observe(progress)
            } else {
                usize::from(!progress) // Keep the one-CPU v2 policy unchanged.
            };
            for _ in 0..spins {
                std::hint::spin_loop();
            }
        }
        // Return queued late loans too, even on an unwinding leadership epoch.
        // No locks or socket I/O are held during any home-channel send.
        drop(leadership);
        drop(clients);
    }
}

#[cfg(test)]
#[path = "handoff_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "handoff_burst_tests.rs"]
mod burst_tests;

#[cfg(test)]
#[path = "handoff_ownership_tests.rs"]
mod ownership_tests;
