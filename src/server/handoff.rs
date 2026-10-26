//! Single-CPU busy-client I/O leases to an existing managed client thread.
//! Quiet connections return to their blocking owners: no new permanent poller.

use super::pool::Client;
use super::*;
use crate::session::Session;
use std::sync::{
    Mutex,
    mpsc::{self, Receiver, Sender},
};

pub(super) struct Sharing {
    leader: Mutex<Option<Sender<Loan>>>,
    recent: Mutex<Burst>,
    config: Config,
    stopping: Arc<AtomicBool>,
    idle: Duration,
    max_epoch: Duration,
}

#[derive(Default)]
struct Burst {
    previous: Option<Instant>,
}
impl Burst {
    fn observe(&mut self, now: Instant, window: Duration) -> bool {
        self.previous
            .replace(now)
            .and_then(|then| now.checked_duration_since(then))
            .is_some_and(|elapsed| elapsed < window)
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

struct Leadership<'a> {
    sharing: &'a Sharing,
    receiver: Receiver<Loan>,
}
impl Drop for Leadership<'_> {
    fn drop(&mut self) {
        // Close admission to this epoch before dropping its receiver. A sender
        // cloned just before this point either succeeds into the old queue or
        // recovers its Loan on send failure. Both paths run Loan::drop.
        self.sharing
            .leader
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
    }
}

impl Sharing {
    pub(super) fn new(config: Config, stopping: Arc<AtomicBool>, idle_us: u64) -> Self {
        Self {
            leader: Mutex::new(None),
            recent: Mutex::new(Burst::default()),
            config,
            stopping,
            idle: Duration::from_micros(idle_us),
            max_epoch: Duration::from_millis(CLIENT_HANDOFF_MAX_EPOCH_MS),
        }
    }

    /// v2 avoids lease allocation/polling for isolated sparse replies. Joining
    /// an existing epoch is allowed even if the previous local observation is old.
    pub(super) fn should_lend(&self) -> bool {
        let recent = self
            .recent
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .observe(Instant::now(), self.idle);
        recent
            || self
                .leader
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_some()
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
        store: &Store,
    ) -> io::Result<bool> {
        debug_assert!(output.is_empty());
        if self.stopping.load(Ordering::Acquire) {
            return Ok(true);
        }
        let activity = Instant::now();
        let socket = stream.try_clone()?;
        socket.set_nonblocking(true)?;
        let client = Client {
            stream: socket,
            session: std::mem::take(session),
            input: std::mem::take(input),
            output: std::mem::take(output),
            sent: 0,
            closing: false,
            eof: false,
            activity,
            _permit: None,
            shutdown_on_drop: true,
        };
        let (home, returned) = mpsc::channel();
        let loan = Loan {
            client: Some(client),
            home,
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
        let Some(mut client) = returned
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
        *session = std::mem::take(&mut client.session);
        *input = std::mem::take(&mut client.input);
        *output = std::mem::take(&mut client.output);
        let closing = client.closing;
        client.shutdown_on_drop = false; // Drop the lease handle, not the socket.
        drop(client);
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
            let mut index = 0;
            while index < clients.len() {
                match clients[index].client.as_mut().expect("active loan").poll(
                    store,
                    &self.config,
                    &mut chunk,
                ) {
                    Ok(Some(made_progress)) => {
                        progress |= made_progress;
                        if !made_progress && clients[index].quiet(Instant::now(), self.idle) {
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
            if !window.active(Instant::now(), progress) {
                break;
            } else if !progress {
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
