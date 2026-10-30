//! Worker-local owner execution and whole-owner reservation state machine.
//!
//! No state transition awaits. The actor only awaits inbox readiness between
//! turns; a fence disables DATA reception, never CONTROL reception. Scoped
//! CONTROL work runs on the owner thread with the executor borrowed nowhere
//! else, so a closure may borrow `Data`, run a complete command turn, or read
//! versions without ever moving `Data` off this thread.
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use tokio::sync::Notify;

use super::partition::{OwnerId, PartitionLayout};
use super::data::Data;
use super::{Action, ServerServices, ShardExecutor};
use super::mailbox::{Incoming, OwnerInbox, ReplySender, RetainedBytes};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ReservationId(u64);
impl ReservationId {
    pub(crate) fn new(value: u64) -> Result<Self, OwnerError> {
        if value == 0 {
            Err(OwnerError::InvalidReservation)
        } else {
            Ok(Self(value))
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OwnerError {
    InvalidReservation,
    WrongReservation,
    Fenced,
    #[cfg(test)]
    NeedsCoordinator,
    WrongOwner,
    DecisionRecorded,
    NotDecided,
    NotApplied,
    AlreadyApplied,
    Stopped,
    Failed,
    ControlClosed,
}
impl std::fmt::Display for OwnerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "storage owner: {self:?}")
    }
}
impl std::error::Error for OwnerError {}
impl RetainedBytes for OwnerError {
    fn retained_bytes(&self) -> Option<usize> {
        Some(0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Granted,
    Decided,
    Applied,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Active,
    Reserved(ReservationId, Stage),
    Stopped,
    Failed,
}

pub(crate) struct OwnerCore {
    executor: ShardExecutor,
    #[cfg(test)]
    owner: OwnerId,
    #[cfg(test)]
    layout: PartitionLayout,
    phase: Cell<Phase>,
    released: Notify,
}

impl OwnerCore {
    pub(crate) fn new(
        owner: OwnerId,
        layout: PartitionLayout,
        services: Arc<ServerServices>,
    ) -> Result<Self, OwnerError> {
        if owner.index() >= layout.owner_count() {
            return Err(OwnerError::WrongOwner);
        }
        Ok(Self {
            executor: ShardExecutor::with_services(services),
            #[cfg(test)]
            owner,
            #[cfg(test)]
            layout,
            phase: Cell::new(Phase::Active),
            released: Notify::new(),
        })
    }

    #[cfg(test)]
    pub(crate) fn owner(&self) -> OwnerId {
        self.owner
    }
    pub(crate) fn data_enabled(&self) -> bool {
        self.phase.get() == Phase::Active
    }

    fn active(&self) -> Result<(), OwnerError> {
        match self.phase.get() {
            Phase::Active => Ok(()),
            Phase::Reserved(..) => Err(OwnerError::Fenced),
            Phase::Stopped => Err(OwnerError::Stopped),
            Phase::Failed => Err(OwnerError::Failed),
        }
    }

    pub(crate) fn turn<R>(&self, operation: impl FnOnce(&ShardExecutor) -> R) -> Result<R, OwnerError> {
        self.active()?;
        Ok(operation(&self.executor))
    }

    #[cfg(test)]
    fn owns_command(&self, args: &[&[u8]]) -> Result<(), OwnerError> {
        let Some(command) = args.first() else {
            return Err(OwnerError::NeedsCoordinator);
        };
        // These can reference keys through patterns/options beyond their source.
        // The global path retains the full implementation and validation.
        if [
            b"SORT".as_slice(),
            b"SORT_RO",
            b"GEORADIUS",
            b"GEORADIUSBYMEMBER",
        ]
        .iter()
        .any(|name| command.eq_ignore_ascii_case(name))
        {
            return Err(OwnerError::NeedsCoordinator);
        }
        let keys = super::discovery::key_arguments(command, args);
        if keys.is_empty() {
            return Err(OwnerError::NeedsCoordinator);
        }
        if keys.iter().any(|key| self.layout.owner_for(key) != self.owner) {
            return Err(OwnerError::WrongOwner);
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn try_execute(&self, args: &[&[u8]], output: &mut Vec<u8>) -> Result<Action, OwnerError> {
        self.owns_command(args)?;
        self.turn(|executor| executor.execute(args, output))
    }

    #[cfg(test)]
    pub(crate) async fn execute(&self, args: &[&[u8]], output: &mut Vec<u8>) -> Result<Action, OwnerError> {
        // No notification/channel/global counter on the uncontended path.
        match self.try_execute(args, output) {
            Err(OwnerError::Fenced) => {}
            result => return result,
        }
        loop {
            // Broadcast is observed from creation, even before first polling.
            let released = self.released.notified();
            match self.try_execute(args, output) {
                Err(OwnerError::Fenced) => released.await,
                result => return result,
            }
        }
    }

    fn reservation(&self, id: ReservationId) -> Result<Stage, OwnerError> {
        match self.phase.get() {
            Phase::Reserved(current, stage) if current == id => Ok(stage),
            Phase::Failed => Err(OwnerError::Failed),
            Phase::Stopped => Err(OwnerError::Stopped),
            _ => Err(OwnerError::WrongReservation),
        }
    }

    pub(crate) fn reserve(&self, id: ReservationId) -> Result<(), OwnerError> {
        self.active()?;
        self.phase.set(Phase::Reserved(id, Stage::Granted));
        Ok(())
    }

    pub(crate) fn decide(&self, id: ReservationId) -> Result<(), OwnerError> {
        match self.reservation(id)? {
            Stage::Granted => self.phase.set(Phase::Reserved(id, Stage::Decided)),
            Stage::Decided | Stage::Applied => {}
        }
        Ok(())
    }

    /// Read-only scoped work under a granted fence. The closure owns any result
    /// delivery; the owner only guarantees exclusivity for the duration.
    pub(crate) fn read_scoped(
        &self,
        id: ReservationId,
        now: i64,
        operation: impl FnOnce(&ShardExecutor, i64),
    ) -> Result<(), OwnerError> {
        self.reservation(id)?;
        operation(&self.executor, now);
        Ok(())
    }

    /// Borrow-confined read under a granted fence. Kept for direct owner tests;
    /// the router uses [`Self::read_scoped`] so the result can own its bytes.
    #[cfg(test)]
    pub(crate) fn read_reserved<R>(
        &self,
        id: ReservationId,
        read: impl FnOnce(&Data) -> R,
    ) -> Result<R, OwnerError> {
        self.reservation(id)?;
        let data = self.executor.data.try_borrow().map_err(|_| self.fail())?;
        Ok(read(&data))
    }

    /// Borrow-confined mutation used by direct owner tests and by the router's
    /// single-key fast paths while a fence is held.
    #[cfg(test)]
    pub(crate) fn apply<R>(
        &self,
        id: ReservationId,
        apply: impl FnOnce(&mut Data) -> Result<R, OwnerError>,
    ) -> Result<R, OwnerError> {
        match self.reservation(id)? {
            Stage::Granted => return Err(OwnerError::NotDecided),
            Stage::Applied => return Err(OwnerError::AlreadyApplied),
            Stage::Decided => {}
        }
        let mut data = self.executor.data.try_borrow_mut().map_err(|_| self.fail())?;
        let result = apply(&mut data);
        drop(data);
        match result {
            Ok(result) => {
                self.phase.set(Phase::Reserved(id, Stage::Applied));
                Ok(result)
            }
            Err(_) => Err(self.fail()),
        }
    }

    /// Mutating scoped work. The decision is published before any mutation, so a
    /// failure after a partial write is fail-stop, never continued service.
    pub(crate) fn apply_scoped(
        &self,
        id: ReservationId,
        now: i64,
        operation: impl FnOnce(&ShardExecutor, i64) -> Result<(), OwnerError>,
    ) -> Result<(), OwnerError> {
        match self.reservation(id)? {
            Stage::Granted => return Err(OwnerError::NotDecided),
            Stage::Applied => return Err(OwnerError::AlreadyApplied),
            Stage::Decided => {}
        }
        match operation(&self.executor, now) {
            Ok(()) => {
                self.phase.set(Phase::Reserved(id, Stage::Applied));
                Ok(())
            }
            Err(_) => Err(self.fail()),
        }
    }

    pub(crate) fn release(&self, id: ReservationId) -> Result<(), OwnerError> {
        if self.reservation(id)? != Stage::Applied {
            return Err(OwnerError::NotApplied);
        }
        self.phase.set(Phase::Active);
        self.released.notify_waiters();
        Ok(())
    }

    pub(crate) fn abort(&self, id: ReservationId) -> Result<(), OwnerError> {
        if self.reservation(id)? != Stage::Granted {
            return Err(OwnerError::DecisionRecorded);
        }
        self.phase.set(Phase::Active);
        self.released.notify_waiters();
        Ok(())
    }

    pub(crate) fn stop(&self) -> Result<(), OwnerError> {
        match self.phase.get() {
            Phase::Active | Phase::Reserved(_, Stage::Granted) => {}
            Phase::Reserved(..) => return Err(OwnerError::DecisionRecorded),
            Phase::Failed => return Err(OwnerError::Failed),
            Phase::Stopped => return Ok(()),
        }
        self.phase.set(Phase::Stopped);
        self.released.notify_waiters();
        Ok(())
    }

    fn fail(&self) -> OwnerError {
        self.phase.set(Phase::Failed);
        self.executor
            .owner_failed
            .store(true, std::sync::atomic::Ordering::Release);
        self.executor.request_shutdown();
        self.released.notify_waiters();
        OwnerError::Failed
    }

}

pub(crate) type DataWork = Box<dyn FnOnce(&ShardExecutor) + Send>;
pub(crate) type ScopedWork = Box<dyn FnOnce(&ShardExecutor, i64) + Send>;
pub(crate) type ScopedApply = Box<dyn FnOnce(&ShardExecutor, i64) -> Result<(), OwnerError> + Send>;
pub(crate) type Acknowledge = ReplySender<Result<(), OwnerError>>;

pub(crate) enum Control {
    Reserve { id: ReservationId, reply: Acknowledge },
    Decide { id: ReservationId, reply: Acknowledge },
    Read { id: ReservationId, now: i64, op: ScopedWork, reply: Acknowledge },
    Apply { id: ReservationId, now: i64, op: ScopedApply, reply: Acknowledge },
    Release { id: ReservationId, reply: Acknowledge },
    Abort { id: ReservationId, reply: Acknowledge },
    Stop { reply: Acknowledge },
}

pub(crate) struct OwnerActor {
    pub(crate) core: Rc<OwnerCore>,
    inbox: OwnerInbox<DataWork, Control>,
}

impl OwnerActor {
    pub(crate) fn new(core: Rc<OwnerCore>, inbox: OwnerInbox<DataWork, Control>) -> Self {
        Self { core, inbox }
    }

    pub(crate) async fn run(mut self) -> Result<(), OwnerError> {
        loop {
            let incoming = self.inbox.next(self.core.data_enabled()).await;
            match incoming {
                Some(Incoming::Data(work)) => {
                    let result = work.dispatch(|work| self.core.turn(work));
                    if result.is_err() {
                        return Err(self.core.fail());
                    }
                }
                Some(Incoming::Control(control)) => {
                    let stop = control.dispatch(|control| self.control(control))?;
                    if stop {
                        self.inbox.close_data();
                        return Ok(());
                    }
                }
                None => {
                    self.core.fail();
                    return Err(OwnerError::ControlClosed);
                }
            }
        }
    }

    fn control(&self, control: Control) -> Result<bool, OwnerError> {
        match control {
            Control::Reserve { id, reply } => {
                if reply.is_closed() {
                    return Ok(false);
                }
                let result = self.core.reserve(id);
                // A lost grant acknowledgement must not leave an undecided fence.
                if reply.send(result).is_err() && result.is_ok() {
                    self.core.abort(id)?;
                }
            }
            Control::Decide { id, reply } => {
                let result = self.core.decide(id);
                let _ = reply.send(result);
            }
            Control::Read { id, now, op, reply } => {
                let result = self.core.read_scoped(id, now, op);
                let _ = reply.send(result);
            }
            Control::Apply { id, now, op, reply } => {
                let result = self.core.apply_scoped(id, now, op);
                let _ = reply.send(result);
                if matches!(result, Err(OwnerError::Failed)) {
                    return Err(OwnerError::Failed);
                }
            }
            Control::Release { id, reply } => {
                let result = self.core.release(id);
                let _ = reply.send(result);
            }
            Control::Abort { id, reply } => {
                let result = self.core.abort(id);
                let _ = reply.send(result);
            }
            Control::Stop { reply } => {
                let result = self.core.stop();
                let _ = reply.send(result);
                if result.is_ok() {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests;
