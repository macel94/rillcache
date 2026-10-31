#![allow(dead_code)] // multi-owner internals; diagnostics wiring is incremental
//! Multi-owner routing and whole-owner transaction reservations.
//!
//! Data stays on its owner thread. A command whose keys all belong to one owner
//! is one owned request/reply hop; it never carries `Data` or a data guard. A
//! command that spans owners holds whole-owner reservations on every participant
//! in increasing owner order, validates once, and executes against a scratch
//! executor that owns cloned entries for exactly the keys the command may touch.
//! Only keys the command actually wrote are transplanted back to their owners, so
//! same-value writes still advance `WATCH` versions and a rejected command
//! publishes no mutation.
//!
//! The socket adapters are synchronous today, so this module offers a blocking
//! bridge over the same bounded mailboxes the owner runtimes use. The router is
//! also usable from an async caller; no owner turn awaits or blocks.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::protocol::encode_bulk;

use super::data::{Data, Entry};
use super::mailbox::{
    ByteBudget, ByteCredit, MailboxError, OwnerSender, ReplyReceiver, ReplySender, RetainedBytes,
    reply_channel,
};
use super::owner_actor::{Control, DataWork, OwnerActor, OwnerCore, OwnerError, ReservationId};
use super::partition::{LayoutError, OwnerId, PartitionLayout};
use super::shard::{MAX_SHARDS, ShardRouter};
use super::{
    Action, DatasetStats, Execution, ServerServices, ShardExecutor, Stats, TransactionOutcome,
    WatchState, WatchToken,
};

/// Largest scratch key set retained for one multi-owner command. Independent of
/// the per-owner payload budget, so a hostile multi-key command cannot pin an
/// unbounded second copy while owners are fenced.
const MAX_TRANSACTION_BYTES: usize = 32 * 1024 * 1024;
/// Cursor field spacing so one decimal cursor encodes an owner and a local index.
const SCAN_OWNER_STRIDE: u64 = 1 << 40;

pub(crate) struct RouterSizing {
    pub data_messages: usize,
    pub data_bytes: usize,
    pub control_messages: usize,
    pub control_bytes: usize,
    pub request_bytes: usize,
    pub reply_bytes: usize,
    pub reply_per_request: usize,
}

impl RouterSizing {
    pub(crate) fn for_limits(max_clients: usize, max_frame_bytes: usize) -> Self {
        let concurrency = max_clients.clamp(16, 64);
        let request = max_frame_bytes
            .saturating_mul(4)
            .clamp(1 << 20, 256 * 1024 * 1024);
        let reply = max_frame_bytes
            .saturating_mul(16)
            .clamp(1 << 20, 512 * 1024 * 1024);
        Self {
            data_messages: concurrency,
            data_bytes: request,
            control_messages: (MAX_SHARDS * 4).max(64),
            control_bytes: 1 << 20,
            request_bytes: request,
            reply_bytes: reply,
            reply_per_request: max_frame_bytes.min(reply),
        }
    }
}

#[derive(Debug)]
pub(crate) enum RouterError {
    Layout(LayoutError),
    Mailbox(MailboxError),
    Owner(OwnerError),
    Stopped,
}
impl std::fmt::Display for RouterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Layout(error) => write!(f, "{error}"),
            Self::Mailbox(error) => write!(f, "{error}"),
            Self::Owner(error) => write!(f, "{error}"),
            Self::Stopped => f.write_str("multi-owner router is stopped"),
        }
    }
}
impl std::error::Error for RouterError {}
impl RetainedBytes for RouterError {
    fn retained_bytes(&self) -> Option<usize> {
        Some(0)
    }
}
impl RetainedBytes for Stats {
    fn retained_bytes(&self) -> Option<usize> {
        Some(0)
    }
}
impl RetainedBytes for DatasetStats {
    fn retained_bytes(&self) -> Option<usize> {
        Some(0)
    }
}

/// Owned reply for one owner round trip.
pub(crate) struct CommandReply {
    pub action: Action,
    pub bytes: Vec<u8>,
}
impl RetainedBytes for CommandReply {
    fn retained_bytes(&self) -> Option<usize> {
        Some(self.bytes.capacity())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct KeyChange {
    pub key: Vec<u8>,
    pub entry: Option<Entry>,
}
impl KeyChange {
    fn retained(&self) -> usize {
        self.key.len() + self.entry.as_ref().map_or(0, |entry| entry.payload_len)
    }
}
impl RetainedBytes for KeyChange {
    fn retained_bytes(&self) -> Option<usize> {
        Some(self.retained())
    }
}

pub(crate) struct Gathered {
    pub changes: Vec<KeyChange>,
    pub payload_bytes: usize,
}
impl RetainedBytes for Gathered {
    fn retained_bytes(&self) -> Option<usize> {
        Some(self.changes.iter().map(KeyChange::retained).sum())
    }
}

pub(crate) struct WatchProbe {
    pub epoch: u64,
    pub keys: Vec<(Vec<u8>, Option<u64>, bool)>,
}
impl RetainedBytes for WatchProbe {
    fn retained_bytes(&self) -> Option<usize> {
        Some(self.keys.iter().map(|(key, _, _)| key.len()).sum())
    }
}

struct OwnerLink {
    sender: OwnerSender<DataWork, Control>,
    thread: Mutex<Option<std::thread::JoinHandle<Result<(), OwnerError>>>>,
}

/// Shared multi-owner router. `Arc<MultiOwner>` is the socket-facing handle.
pub(crate) struct MultiOwner {
    services: Arc<ServerServices>,
    layout: PartitionLayout,
    owners: Vec<OwnerLink>,
    requests: ByteBudget,
    replies: ByteBudget,
    reply_per_request: usize,
    next_reservation: AtomicU64,
    random_owner: AtomicU64,
    stopped: AtomicBool,
}

impl std::ops::Deref for MultiOwner {
    type Target = ServerServices;
    fn deref(&self) -> &ServerServices {
        &self.services
    }
}

/// Which workers a command must involve.
enum Classification {
    /// Shared control-plane work with no keyspace access; one hop to owner 0.
    Shared,
    /// Bounded key access; the listed owners are exactly the participants.
    Owners(Vec<OwnerId>),
    /// Whole-keyspace data command; reserve every owner.
    Global,
}

fn is_global_data_command(name: &[u8]) -> bool {
    [
        b"DBSIZE".as_slice(),
        b"KEYS",
        b"RANDOMKEY",
        b"SCAN",
        b"FLUSHALL",
        b"FLUSHDB",
        b"INFO",
        b"MEMORY",
    ]
    .iter()
    .any(|candidate| name.eq_ignore_ascii_case(candidate))
}

/// Keys not described by the static registry because a subcommand selects them.
fn extra_keys<'a>(name: &[u8], args: &'a [&'a [u8]]) -> Option<Vec<&'a [u8]>> {
    if name.eq_ignore_ascii_case(b"MEMORY")
        && args.len() == 3
        && args[1].eq_ignore_ascii_case(b"USAGE")
    {
        return Some(vec![args[2]]);
    }
    None
}

impl MultiOwner {
    pub(crate) fn start(
        services: Arc<ServerServices>,
        owners: usize,
        sizing: RouterSizing,
    ) -> Result<Arc<Self>, RouterError> {
        let partitions = if owners == 1 { 1 } else { MAX_SHARDS };
        let router =
            ShardRouter::new(partitions).map_err(|_| RouterError::Layout(LayoutError::NoOwners))?;
        // One shared layout: diagnostics and routing can never disagree.
        services.set_router(router);
        // Owners publish their payload totals into the shared credit counter, so
        // the configured budget stays a process-wide limit rather than per owner.
        services
            .multi_owner
            .store(true, std::sync::atomic::Ordering::Release);
        let layout = PartitionLayout::from_router(owners, router);
        let requests = ByteBudget::new(sizing.request_bytes).map_err(RouterError::Mailbox)?;
        let replies = ByteBudget::new(sizing.reply_bytes).map_err(RouterError::Mailbox)?;
        let mut links = Vec::with_capacity(owners);
        for owner in layout.owners() {
            let (sender, inbox) = super::mailbox::owner_mailbox::<DataWork, Control>(
                sizing.data_messages,
                sizing.data_bytes,
                sizing.control_messages,
                sizing.control_bytes,
            )
            .map_err(RouterError::Mailbox)?;
            let services = Arc::clone(&services);
            let thread = std::thread::Builder::new()
                .name(format!("rillcache-owner-{}", owner.index()))
                .spawn(move || {
                    let core = OwnerCore::new(owner, layout, services)?;
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|_| OwnerError::Failed)?;
                    let local = tokio::task::LocalSet::new();
                    local.block_on(
                        &runtime,
                        OwnerActor::new(std::rc::Rc::new(core), inbox).run(),
                    )
                })
                .map_err(|_| RouterError::Stopped)?;
            links.push(OwnerLink {
                sender,
                thread: Mutex::new(Some(thread)),
            });
        }
        Ok(Arc::new(Self {
            services,
            layout,
            owners: links,
            requests,
            replies,
            reply_per_request: sizing.reply_per_request,
            next_reservation: AtomicU64::new(1),
            random_owner: AtomicU64::new(0),
            stopped: AtomicBool::new(false),
        }))
    }

    /// Seed replayed append-only state into its owning workers before any
    /// frontend starts. Seeding publishes no journal record: replay already did.
    pub(crate) fn seed(&self, entries: Vec<(Vec<u8>, Entry)>) -> Result<(), RouterError> {
        let now = self.services.clock.now_ms().max(0);
        for (key, entry) in entries {
            let owner = self.layout.owner_for(&key);
            self.owner_op(owner, move |executor| {
                let mut data = executor.data.borrow_mut();
                data.scatter_entry(&key, Some(entry), now);
            })?;
        }
        Ok(())
    }

    pub(crate) fn owner_status(&self) -> std::io::Result<()> {
        if self.services.owner_failed.load(Ordering::Acquire) {
            Err(std::io::Error::other("storage owner failed"))
        } else {
            Ok(())
        }
    }

    pub(crate) fn layout(&self) -> PartitionLayout {
        self.layout
    }
    pub(crate) fn owner_count(&self) -> usize {
        self.layout.owner_count()
    }
    pub(crate) fn partition_count(&self) -> usize {
        self.layout.partition_count()
    }

    /// Close admission, finish admitted work and join every owner thread.
    pub(crate) fn shutdown(&self) -> Result<(), RouterError> {
        if self.stopped.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        for link in &self.owners {
            link.sender.data.close();
        }
        let mut first: Result<(), RouterError> = Ok(());
        for link in &self.owners {
            let Ok(credit) = self.replies.reserve_blocking(0) else {
                break;
            };
            let (answer, reply) = reply_channel::<Result<(), OwnerError>>(credit);
            if let Ok(admission) = link.sender.control.reserve_blocking(0) {
                admission.send_with(|| Control::Stop { reply: answer });
                let _ = reply.blocking_recv();
            }
            if let Ok(mut guard) = link.thread.lock()
                && let Some(thread) = guard.take()
            {
                match thread.join() {
                    Ok(Ok(())) | Ok(Err(OwnerError::Stopped)) => {}
                    Ok(Err(error)) => {
                        if first.is_ok() {
                            first = Err(RouterError::Owner(error));
                        }
                    }
                    Err(_) => {
                        if first.is_ok() {
                            first = Err(RouterError::Owner(OwnerError::Failed));
                        }
                    }
                }
            }
        }
        self.requests.close();
        self.replies.close();
        first
    }

    fn fail(&self) -> RouterError {
        self.services.owner_failed.store(true, Ordering::Release);
        self.services.request_shutdown();
        RouterError::Stopped
    }

    fn link(&self, owner: OwnerId) -> Result<&OwnerLink, RouterError> {
        self.owners.get(owner.index()).ok_or_else(|| self.fail())
    }

    fn next_reservation(&self) -> Result<ReservationId, RouterError> {
        ReservationId::new(self.next_reservation.fetch_add(1, Ordering::Relaxed))
            .map_err(|_| self.fail())
    }

    fn request_credit(&self, bytes: usize) -> Result<ByteCredit, RouterError> {
        self.requests
            .reserve_blocking(bytes)
            .map_err(|error| match error {
                MailboxError::Closed | MailboxError::TooLarge => self.fail(),
                other => RouterError::Mailbox(other),
            })
    }

    fn reply_credit(&self) -> Result<ByteCredit, RouterError> {
        self.replies
            .reserve_blocking(self.reply_per_request)
            .map_err(|error| match error {
                MailboxError::Closed | MailboxError::TooLarge => self.fail(),
                other => RouterError::Mailbox(other),
            })
    }

    fn classify(&self, args: &[&[u8]]) -> Classification {
        let Some(name) = args.first() else {
            return Classification::Shared;
        };
        if let Some(keys) = extra_keys(name, args) {
            return self.classification_for_keys(keys);
        }
        let keys = super::discovery::key_arguments(name, args);
        if !keys.is_empty() {
            return self.classification_for_keys(keys);
        }
        if is_global_data_command(name) {
            Classification::Global
        } else {
            Classification::Shared
        }
    }

    fn classification_for_keys(&self, keys: Vec<&[u8]>) -> Classification {
        let mut owners: Vec<OwnerId> = keys
            .into_iter()
            .map(|key| self.layout.owner_for(key))
            .collect();
        owners.sort_unstable();
        owners.dedup();
        Classification::Owners(owners)
    }

    fn all_owners(&self) -> Vec<OwnerId> {
        self.layout.owners().collect()
    }

    /// A finite payload budget plus an eviction policy needs every owner's
    /// accounting to reclaim spare credits before rejecting a valid write.
    fn needs_global_budget(&self, args: &[&[u8]]) -> bool {
        self.services.max_payload_bytes.load(Ordering::Relaxed) > 0
            && args
                .first()
                .is_some_and(|name| super::discovery::is_write_command(name))
    }

    // -- owner round trips -------------------------------------------------

    fn forward(&self, owner: OwnerId, args: &[&[u8]]) -> Result<CommandReply, RouterError> {
        let bytes: usize = args.iter().map(|arg| arg.len()).sum();
        let input = self.request_credit(bytes)?;
        let reply = self.reply_credit()?;
        let (sender, receiver) = reply_channel::<Result<CommandReply, OwnerError>>(reply);
        let owned: Vec<Vec<u8>> = args.iter().map(|arg| arg.to_vec()).collect();
        self.link(owner)?
            .sender
            .data
            .reserve_blocking(bytes)
            .map_err(|error| match error {
                MailboxError::Closed => self.fail(),
                other => RouterError::Mailbox(other),
            })?
            .send_with(|| {
                let input = input;
                Box::new(move |executor: &ShardExecutor| {
                    let borrowed: Vec<&[u8]> = owned.iter().map(Vec::as_slice).collect();
                    let mut bytes = Vec::new();
                    let action = executor.execute(&borrowed, &mut bytes);
                    let _ = sender.send(Ok(CommandReply { action, bytes }));
                    drop(input);
                }) as DataWork
            });
        self.await_reply(receiver)
    }

    fn owner_op<T, F>(&self, owner: OwnerId, operation: F) -> Result<T, RouterError>
    where
        T: RetainedBytes + Send + 'static,
        F: FnOnce(&ShardExecutor) -> T + Send + 'static,
    {
        let reply = self.reply_credit()?;
        let (sender, receiver) = reply_channel::<Result<T, OwnerError>>(reply);
        self.link(owner)?
            .sender
            .data
            .reserve_blocking(0)
            .map_err(|error| match error {
                MailboxError::Closed => self.fail(),
                other => RouterError::Mailbox(other),
            })?
            .send_with(|| {
                Box::new(move |executor: &ShardExecutor| {
                    let _ = sender.send(Ok(operation(executor)));
                }) as DataWork
            });
        self.await_reply(receiver)
    }

    fn await_reply<T: RetainedBytes>(
        &self,
        receiver: ReplyReceiver<Result<T, OwnerError>>,
    ) -> Result<T, RouterError> {
        match receiver.blocking_recv() {
            Ok(value) => match value.into_inner() {
                Ok(inner) => Ok(inner),
                Err(error) => Err(RouterError::Owner(error)),
            },
            Err(_) => Err(self.fail()),
        }
    }

    // -- reservations ------------------------------------------------------

    fn lock(&self, id: ReservationId, requested: &[OwnerId]) -> Result<Vec<OwnerId>, RouterError> {
        let mut owners = requested.to_vec();
        owners.sort_unstable();
        owners.dedup();
        let mut granted: Vec<OwnerId> = Vec::with_capacity(owners.len());
        for owner in owners {
            let reply = self.reply_credit()?;
            let (sender, receiver) = reply_channel::<Result<(), OwnerError>>(reply);
            let sent = self
                .link(owner)
                .and_then(|link| {
                    link.sender
                        .control
                        .reserve_blocking(0)
                        .map_err(RouterError::Mailbox)
                })
                .map(|admission| {
                    admission.send_with(|| Control::Reserve { id, reply: sender });
                });
            if let Err(error) = sent {
                let _ = self.unlock_abort(id, &granted);
                return Err(error);
            }
            let granted_ok = match receiver.blocking_recv() {
                Ok(value) => match value.into_inner() {
                    Ok(()) => true,
                    Err(error) => {
                        let _ = self.unlock_abort(id, &granted);
                        return Err(RouterError::Owner(error));
                    }
                },
                Err(_) => {
                    let _ = self.unlock_abort(id, &granted);
                    return Err(self.fail());
                }
            };
            if granted_ok {
                granted.push(owner);
            }
        }
        Ok(granted)
    }

    fn unlock_finish(&self, id: ReservationId, owners: &[OwnerId]) -> Result<(), RouterError> {
        for owner in owners {
            self.control_ack(*owner, |reply| Control::Decide { id, reply })?;
        }
        for owner in owners {
            self.control_apply_noop(*owner, id)?;
        }
        for owner in owners {
            self.control_ack(*owner, |reply| Control::Release { id, reply })?;
        }
        Ok(())
    }

    fn unlock_abort(&self, id: ReservationId, owners: &[OwnerId]) -> Result<(), RouterError> {
        for owner in owners {
            self.control_ack(*owner, |reply| Control::Abort { id, reply })?;
        }
        Ok(())
    }

    fn control_ack(
        &self,
        owner: OwnerId,
        make: impl FnOnce(ReplySender<Result<(), OwnerError>>) -> Control,
    ) -> Result<(), RouterError> {
        let reply = self.reply_credit()?;
        let (sender, receiver) = reply_channel::<Result<(), OwnerError>>(reply);
        self.link(owner)?
            .sender
            .control
            .reserve_blocking(0)
            .map_err(|error| match error {
                MailboxError::Closed => self.fail(),
                other => RouterError::Mailbox(other),
            })?
            .send_with(|| make(sender));
        self.await_reply(receiver)?;
        Ok(())
    }

    fn control_apply_noop(&self, owner: OwnerId, id: ReservationId) -> Result<(), RouterError> {
        let reply = self.reply_credit()?;
        let (sender, receiver) = reply_channel::<Result<(), OwnerError>>(reply);
        self.link(owner)?
            .sender
            .control
            .reserve_blocking(0)
            .map_err(|error| match error {
                MailboxError::Closed => self.fail(),
                other => RouterError::Mailbox(other),
            })?
            .send_with(|| Control::Apply {
                id,
                now: 0,
                reply: sender,
                op: Box::new(|_, _| Ok(())),
            });
        match self.await_reply(receiver) {
            Ok(()) | Err(RouterError::Owner(OwnerError::AlreadyApplied)) => Ok(()),
            Err(error) => Err(error),
        }
    }

    /// Scoped read or write on a fenced owner. The closure runs on the owner
    /// thread with exclusive access; result bytes are delivered by the closure.
    fn scoped_op<T, F>(
        &self,
        id: ReservationId,
        owner: OwnerId,
        now: i64,
        operation: F,
    ) -> Result<T, RouterError>
    where
        T: RetainedBytes + Send + 'static,
        F: FnOnce(&ShardExecutor, i64) -> T + Send + 'static,
    {
        let ack = self.reply_credit()?;
        let (ack_sender, ack_receiver) = reply_channel::<Result<(), OwnerError>>(ack);
        let value = self.reply_credit()?;
        let (value_sender, value_receiver) = reply_channel::<Result<T, OwnerError>>(value);
        self.link(owner)?
            .sender
            .control
            .reserve_blocking(0)
            .map_err(|error| match error {
                MailboxError::Closed => self.fail(),
                other => RouterError::Mailbox(other),
            })?
            .send_with(|| Control::Read {
                id,
                now,
                reply: ack_sender,
                op: Box::new(move |executor, now| {
                    let _ = value_sender.send(Ok(operation(executor, now)));
                }),
            });
        self.await_reply(ack_receiver)?;
        self.await_reply(value_receiver)
    }

    // -- locked command execution -----------------------------------------

    fn gather(
        &self,
        id: ReservationId,
        owner: OwnerId,
        keys: Vec<Vec<u8>>,
        now: i64,
    ) -> Result<Gathered, RouterError> {
        self.scoped_op(id, owner, now, move |executor, now| {
            let mut data = executor.data.borrow_mut();
            let payload_bytes = data.payload_bytes;
            let mut changes = Vec::with_capacity(keys.len());
            for key in keys {
                data.expire_key(&key, now);
                let entry = data.gather_entry(&key);
                changes.push(KeyChange { key, entry });
            }
            Gathered {
                changes,
                payload_bytes,
            }
        })
    }

    fn scatter(
        &self,
        id: ReservationId,
        owner: OwnerId,
        changes: Vec<KeyChange>,
        now: i64,
    ) -> Result<(), RouterError> {
        if changes.is_empty() {
            return Ok(());
        }
        self.scoped_op(id, owner, now, move |executor, now| {
            let mut data = executor.data.borrow_mut();
            for change in changes {
                data.scatter_entry(&change.key, change.entry, now);
            }
        })
    }

    /// Execute `args` while the needed owners are already fenced.
    fn locked_execute(
        &self,
        id: ReservationId,
        now: i64,
        args: &[Vec<u8>],
        output: &mut Vec<u8>,
    ) -> Result<Action, RouterError> {
        let borrowed: Vec<&[u8]> = args.iter().map(Vec::as_slice).collect();
        match self.classify(&borrowed) {
            Classification::Shared => {
                let reply = self.locked_owner_command(id, now, OwnerId::new(0), args)?;
                output.extend_from_slice(&reply.bytes);
                Ok(reply.action)
            }
            Classification::Owners(owners) if owners.len() == 1 => {
                let reply = self.locked_owner_command(id, now, owners[0], args)?;
                output.extend_from_slice(&reply.bytes);
                Ok(reply.action)
            }
            Classification::Owners(owners) => self.locked_scratch(id, now, &owners, args, output),
            Classification::Global => self.locked_global(id, now, args, output),
        }
    }

    fn locked_owner_command(
        &self,
        id: ReservationId,
        now: i64,
        owner: OwnerId,
        args: &[Vec<u8>],
    ) -> Result<CommandReply, RouterError> {
        let args = args.to_vec();
        self.scoped_op(id, owner, now, move |executor, _| {
            let borrowed: Vec<&[u8]> = args.iter().map(Vec::as_slice).collect();
            let mut bytes = Vec::new();
            let action = executor.execute(&borrowed, &mut bytes);
            CommandReply { action, bytes }
        })
    }

    fn locked_scratch(
        &self,
        id: ReservationId,
        now: i64,
        owners: &[OwnerId],
        args: &[Vec<u8>],
        output: &mut Vec<u8>,
    ) -> Result<Action, RouterError> {
        let borrowed: Vec<&[u8]> = args.iter().map(Vec::as_slice).collect();
        let keys = super::discovery::key_arguments(borrowed[0], &borrowed);
        let mut by_owner: BTreeMap<usize, Vec<Vec<u8>>> = BTreeMap::new();
        for key in keys {
            by_owner
                .entry(self.layout.owner_for(key).index())
                .or_default()
                .push(key.to_vec());
        }
        let mut entries = Vec::new();
        let mut gathered_bytes = 0usize;
        for (index, owner_keys) in by_owner {
            let owner = owners
                .iter()
                .copied()
                .find(|owner| owner.index() == index)
                .ok_or_else(|| self.fail())?;
            let gathered = self.gather(id, owner, owner_keys, now)?;
            gathered_bytes = gathered_bytes.saturating_add(
                gathered
                    .changes
                    .iter()
                    .map(KeyChange::retained)
                    .sum::<usize>(),
            );
            for change in gathered.changes {
                if let Some(entry) = change.entry {
                    entries.push((change.key, entry));
                }
            }
        }
        if gathered_bytes > MAX_TRANSACTION_BYTES {
            output.clear();
            output.extend_from_slice(
                b"-ERR multi-owner transaction exceeds the scratch memory bound\r\n",
            );
            return Ok(Action::Continue);
        }
        let data = Data::from_entries(entries, self.services.eviction_policy());
        let scratch = ShardExecutor::with_data(Arc::clone(&self.services), data);
        let action = scratch.execute(&borrowed, output);
        self.scatter_scratch(id, now, owners, &scratch)?;
        Ok(action)
    }

    fn scatter_scratch(
        &self,
        id: ReservationId,
        now: i64,
        owners: &[OwnerId],
        scratch: &ShardExecutor,
    ) -> Result<(), RouterError> {
        let touched = scratch.data.borrow().touched_keys();
        if touched.is_empty() {
            return Ok(());
        }
        let mut changes: BTreeMap<usize, Vec<KeyChange>> = BTreeMap::new();
        {
            let data = scratch.data.borrow();
            for key in touched {
                let entry = data.gather_entry(&key);
                changes
                    .entry(self.layout.owner_for(&key).index())
                    .or_default()
                    .push(KeyChange { key, entry });
            }
        }
        for (index, changes) in changes {
            let owner = owners
                .iter()
                .copied()
                .find(|owner| owner.index() == index)
                .ok_or_else(|| self.fail())?;
            self.scatter(id, owner, changes, now)?;
        }
        Ok(())
    }

    fn locked_global(
        &self,
        id: ReservationId,
        now: i64,
        args: &[Vec<u8>],
        output: &mut Vec<u8>,
    ) -> Result<Action, RouterError> {
        let owners = self.all_owners();
        let name = args[0].as_slice();
        if name.eq_ignore_ascii_case(b"KEYS") {
            let mut keys = Vec::new();
            for owner in &owners {
                let reply = self.locked_owner_command(id, now, *owner, args)?;
                if let Some(mut part) = parse_bulk_array(&reply.bytes) {
                    keys.append(&mut part);
                }
            }
            write_bulk_array(&keys, output);
            return Ok(Action::Continue);
        }
        if name.eq_ignore_ascii_case(b"DBSIZE") {
            let mut total = 0usize;
            for owner in &owners {
                let reply = self.locked_owner_command(id, now, *owner, args)?;
                total = total.saturating_add(parse_integer_reply(&reply.bytes).unwrap_or(0));
            }
            output.extend_from_slice(format!(":{total}\r\n").as_bytes());
            return Ok(Action::Continue);
        }
        if name.eq_ignore_ascii_case(b"RANDOMKEY") {
            let start = self.random_owner.fetch_add(1, Ordering::Relaxed) as usize;
            for offset in 0..owners.len() {
                let owner = owners[(start + offset) % owners.len()];
                let reply = self.locked_owner_command(id, now, owner, args)?;
                if reply.bytes != b"$-1\r\n" {
                    output.extend_from_slice(&reply.bytes);
                    return Ok(Action::Continue);
                }
            }
            output.extend_from_slice(b"$-1\r\n");
            return Ok(Action::Continue);
        }
        if name.eq_ignore_ascii_case(b"SCAN") {
            return self.locked_scan(id, now, args, output);
        }
        if name.eq_ignore_ascii_case(b"FLUSHALL") || name.eq_ignore_ascii_case(b"FLUSHDB") {
            for (index, owner) in owners.iter().enumerate() {
                let reply = self.locked_owner_command(id, now, *owner, args)?;
                if index == 0 {
                    output.extend_from_slice(&reply.bytes);
                }
            }
            return Ok(Action::Continue);
        }
        let reply = self.locked_owner_command(id, now, OwnerId::new(0), args)?;
        output.extend_from_slice(&reply.bytes);
        Ok(reply.action)
    }

    fn locked_scan(
        &self,
        id: ReservationId,
        now: i64,
        args: &[Vec<u8>],
        output: &mut Vec<u8>,
    ) -> Result<Action, RouterError> {
        let Some(raw) = args.get(1) else {
            output.extend_from_slice(b"-ERR wrong number of arguments for 'scan' command\r\n");
            return Ok(Action::Continue);
        };
        let Some(cursor) = parse_u64(raw) else {
            output.extend_from_slice(b"-ERR invalid cursor\r\n");
            return Ok(Action::Continue);
        };
        let owners = self.all_owners();
        let mut owner_index = (cursor / SCAN_OWNER_STRIDE) as usize;
        let mut local = cursor % SCAN_OWNER_STRIDE;
        if owner_index >= owners.len() {
            owner_index = 0;
            local = 0;
        }
        let owner = owners[owner_index];
        let mut forwarded = args.to_vec();
        forwarded[1] = local.to_string().into_bytes();
        let reply = self.locked_owner_command(id, now, owner, &forwarded)?;
        let Some((next, keys)) = parse_scan_reply(&reply.bytes) else {
            output.extend_from_slice(&reply.bytes);
            return Ok(Action::Continue);
        };
        let next_cursor = if next != 0 {
            (owner_index as u64) * SCAN_OWNER_STRIDE + next
        } else if owner_index + 1 < owners.len() {
            ((owner_index + 1) as u64) * SCAN_OWNER_STRIDE
        } else {
            0
        };
        let cursor = next_cursor.to_string();
        output.extend_from_slice(b"*2\r\n");
        encode_bulk(Some(cursor.as_bytes()), output);
        write_bulk_array(&keys, output);
        Ok(Action::Continue)
    }

    // -- public execution boundary ----------------------------------------

    pub(crate) fn execute_blocking(&self, args: &[&[u8]], output: &mut Vec<u8>) -> Action {
        match self.classify(args) {
            Classification::Shared => match self.forward(OwnerId::new(0), args) {
                Ok(reply) => {
                    output.extend_from_slice(&reply.bytes);
                    reply.action
                }
                Err(error) => self.failed_output(error, output),
            },
            Classification::Owners(owners) if owners.len() == 1 => {
                match self.forward(owners[0], args) {
                    Ok(reply) => {
                        output.extend_from_slice(&reply.bytes);
                        reply.action
                    }
                    Err(error) => self.failed_output(error, output),
                }
            }
            Classification::Owners(owners) => {
                let owners = if owners.len() > 1 && self.needs_global_budget(args) {
                    self.all_owners()
                } else {
                    owners
                };
                self.atomic_blocking(&owners, args, output)
            }
            Classification::Global => self.atomic_blocking(&self.all_owners(), args, output),
        }
    }

    fn atomic_blocking(&self, owners: &[OwnerId], args: &[&[u8]], output: &mut Vec<u8>) -> Action {
        let id = match self.next_reservation() {
            Ok(id) => id,
            Err(error) => return self.failed_output(error, output),
        };
        let granted = match self.lock(id, owners) {
            Ok(granted) => granted,
            Err(error) => return self.failed_output(error, output),
        };
        let now = self.services.clock.now_ms().max(0);
        let owned: Vec<Vec<u8>> = args.iter().map(|arg| arg.to_vec()).collect();
        let result = self.locked_execute(id, now, &owned, output);
        let finish = self.unlock_finish(id, &granted);
        match (result, finish) {
            (Ok(action), Ok(())) => action,
            (Err(error), _) | (_, Err(error)) => self.failed_output(error, output),
        }
    }

    fn failed_output(&self, error: RouterError, output: &mut Vec<u8>) -> Action {
        let _ = self.fail();
        let _ = error;
        output.extend_from_slice(b"-ERR internal store error\r\n");
        Action::Close
    }

    // -- WATCH and EXEC ----------------------------------------------------

    pub(crate) fn watch_snapshot_blocking(&self, keys: &[&[u8]]) -> Option<WatchState> {
        let now = self.services.clock.now_ms().max(0);
        let mut by_owner: BTreeMap<usize, Vec<Vec<u8>>> = BTreeMap::new();
        for key in keys {
            by_owner
                .entry(self.layout.owner_for(key).index())
                .or_default()
                .push(key.to_vec());
        }
        let mut tokens = Vec::with_capacity(keys.len());
        for (owner_index, owner_keys) in by_owner {
            let probe = self
                .owner_op(OwnerId::new(owner_index), move |executor| {
                    let mut data = executor.data.borrow_mut();
                    let epoch = data.epoch();
                    let keys = owner_keys
                        .iter()
                        .map(|key| {
                            data.expire_key(key, now);
                            (
                                key.clone(),
                                data.version(key),
                                data.entries.get(key).is_some_and(|entry| entry.alive(now)),
                            )
                        })
                        .collect::<Vec<_>>();
                    WatchProbe { epoch, keys }
                })
                .ok()?;
            tokens.extend(
                probe
                    .keys
                    .into_iter()
                    .map(|(key, version, present)| WatchToken {
                        owner: owner_index,
                        key,
                        version,
                        present,
                        epoch: probe.epoch,
                    }),
            );
        }
        Some(WatchState::from_tokens(tokens))
    }

    fn probe_owner(
        &self,
        id: ReservationId,
        owner: OwnerId,
        keys: Vec<Vec<u8>>,
        now: i64,
    ) -> Result<WatchProbe, RouterError> {
        self.scoped_op(id, owner, now, move |executor, now| {
            let mut data = executor.data.borrow_mut();
            let epoch = data.epoch();
            let keys = keys
                .iter()
                .map(|key| {
                    data.expire_key(key, now);
                    (
                        key.clone(),
                        data.version(key),
                        data.entries.get(key).is_some_and(|entry| entry.alive(now)),
                    )
                })
                .collect::<Vec<_>>();
            WatchProbe { epoch, keys }
        })
    }

    fn participants_for_transaction(
        &self,
        commands: &[Vec<Vec<u8>>],
        watches: &WatchState,
    ) -> Vec<OwnerId> {
        let mut owners: Vec<OwnerId> = watches
            .tokens()
            .iter()
            .map(|token| OwnerId::new(token.owner))
            .collect();
        for command in commands {
            let borrowed: Vec<&[u8]> = command.iter().map(Vec::as_slice).collect();
            match self.classify(&borrowed) {
                Classification::Shared => owners.push(OwnerId::new(0)),
                Classification::Owners(participants) => owners.extend(participants),
                Classification::Global => return self.all_owners(),
            }
        }
        owners.sort_unstable();
        owners.dedup();
        owners
    }

    pub(crate) fn execute_transaction_blocking(
        &self,
        commands: &[Vec<Vec<u8>>],
        watches: &WatchState,
        output: &mut Vec<u8>,
    ) -> TransactionOutcome {
        let owners = self.participants_for_transaction(commands, watches);
        let id = match self.next_reservation() {
            Ok(id) => id,
            Err(error) => {
                self.failed_output(error, output);
                return TransactionOutcome::InternalError;
            }
        };
        let granted = match self.lock(id, &owners) {
            Ok(granted) => granted,
            Err(error) => {
                self.failed_output(error, output);
                return TransactionOutcome::InternalError;
            }
        };
        let now = self.services.clock.now_ms().max(0);
        if self.watches_changed(id, watches, now) {
            let _ = self.unlock_abort(id, &granted);
            output.extend_from_slice(b"*-1\r\n");
            return TransactionOutcome::Aborted;
        }
        let _ = std::io::Write::write_fmt(output, format_args!("*{}\r\n", commands.len()));
        let mut action = Action::Continue;
        for command in commands {
            match self.locked_execute(id, now, command, output) {
                Ok(Action::Close) => {
                    action = Action::Close;
                    break;
                }
                Ok(_) => {}
                Err(error) => {
                    let _ = self.unlock_abort(id, &granted);
                    self.failed_output(error, output);
                    return TransactionOutcome::InternalError;
                }
            }
        }
        match self.unlock_finish(id, &granted) {
            Ok(()) => TransactionOutcome::Completed(action),
            Err(error) => {
                self.failed_output(error, output);
                TransactionOutcome::InternalError
            }
        }
    }

    fn watches_changed(&self, id: ReservationId, watches: &WatchState, now: i64) -> bool {
        let mut by_owner: BTreeMap<usize, Vec<&WatchToken>> = BTreeMap::new();
        for token in watches.tokens() {
            by_owner.entry(token.owner).or_default().push(token);
        }
        for (owner_index, tokens) in by_owner {
            let keys: Vec<Vec<u8>> = tokens.iter().map(|token| token.key.clone()).collect();
            let owner = OwnerId::new(owner_index);
            let Ok(probe) = self.probe_owner(id, owner, keys, now) else {
                return true;
            };
            if tokens.iter().any(|token| token.epoch != probe.epoch) {
                return true;
            }
            for (key, version, present) in &probe.keys {
                if let Some(token) = tokens.iter().find(|token| &token.key == key)
                    && (token.version != *version || token.present != *present)
                {
                    return true;
                }
            }
        }
        false
    }

    // -- blocking commands -------------------------------------------------

    fn scratch_outcome<T, F>(&self, keys: &[&[u8]], operation: F) -> Result<T, RouterError>
    where
        T: RetainedBytes + Send + 'static,
        F: FnOnce(&ShardExecutor) -> T,
    {
        let mut owners: Vec<OwnerId> = keys.iter().map(|key| self.layout.owner_for(key)).collect();
        owners.sort_unstable();
        owners.dedup();
        let id = self.next_reservation()?;
        let granted = self.lock(id, &owners)?;
        let now = self.services.clock.now_ms().max(0);
        let result = (|| -> Result<T, RouterError> {
            let mut by_owner: BTreeMap<usize, Vec<Vec<u8>>> = BTreeMap::new();
            for key in keys {
                by_owner
                    .entry(self.layout.owner_for(key).index())
                    .or_default()
                    .push(key.to_vec());
            }
            let mut entries = Vec::new();
            for (index, owner_keys) in by_owner {
                let gathered = self.gather(id, OwnerId::new(index), owner_keys, now)?;
                for change in gathered.changes {
                    if let Some(entry) = change.entry {
                        entries.push((change.key, entry));
                    }
                }
            }
            let data = Data::from_entries(entries, self.services.eviction_policy());
            let scratch = ShardExecutor::with_data(Arc::clone(&self.services), data);
            let outcome = operation(&scratch);
            self.scatter_scratch(id, now, &owners, &scratch)?;
            Ok(outcome)
        })();
        let finish = self.unlock_finish(id, &granted);
        match (result, finish) {
            (Ok(outcome), Ok(())) => Ok(outcome),
            (Err(error), _) | (_, Err(error)) => Err(error),
        }
    }

    // -- maintenance -------------------------------------------------------

    pub(crate) fn purge_expired_blocking(&self, limit: usize) -> usize {
        let mut total = 0usize;
        for owner in self.all_owners() {
            if let Ok(count) = self.owner_op(owner, move |executor| executor.purge_expired(limit)) {
                total = total.saturating_add(count);
            }
        }
        total
    }

    pub(crate) fn stats_blocking(&self) -> Stats {
        let mut stats = match self.owner_op(OwnerId::new(0), |executor| executor.stats()) {
            Ok(stats) => stats,
            Err(_) => return Stats::default(),
        };
        for owner in self.all_owners().into_iter().skip(1) {
            if let Ok(part) = self.owner_op(owner, |executor| executor.stats()) {
                stats.expired_keys = stats.expired_keys.saturating_add(part.expired_keys);
                stats.expired_subkeys = stats.expired_subkeys.saturating_add(part.expired_subkeys);
                stats.evicted_keys = stats.evicted_keys.saturating_add(part.evicted_keys);
            }
        }
        stats
    }

    pub(crate) fn journal_sync_blocking(&self) -> std::io::Result<()> {
        match self.owner_op(OwnerId::new(0), |executor| executor.journal_sync()) {
            Ok(result) => result,
            Err(_) => Err(std::io::Error::other("journal owner unavailable")),
        }
    }

    pub(crate) fn auto_rewrite_blocking(&self) -> bool {
        // Rewriting builds one coherent snapshot from every owner under a
        // barrier; per-owner rewrites would publish a partial keyspace.
        let owners = self.all_owners();
        let Ok(id) = self.next_reservation() else {
            return false;
        };
        let Ok(granted) = self.lock(id, &owners) else {
            return false;
        };
        let now = self.services.clock.now_ms().max(0);
        let result = (|| -> Result<bool, RouterError> {
            let mut entries = Vec::new();
            let mut total = 0usize;
            for owner in &owners {
                let all = self.scoped_op(id, *owner, now, |executor, _| {
                    let data = executor.data.borrow();
                    let mut keys: Vec<Vec<u8>> = data.entries.keys().cloned().collect();
                    keys.sort_unstable();
                    let mut changes = Vec::with_capacity(keys.len());
                    for key in keys {
                        let entry = data.gather_entry(&key);
                        changes.push(KeyChange { key, entry });
                    }
                    Gathered {
                        changes,
                        payload_bytes: data.payload_bytes,
                    }
                })?;
                total = total.saturating_add(all.payload_bytes);
                for change in all.changes {
                    if let Some(entry) = change.entry {
                        entries.push((change.key, entry));
                    }
                }
            }
            if total > MAX_TRANSACTION_BYTES {
                return Ok(false);
            }
            let data = Data::from_entries(entries, self.services.eviction_policy());
            let scratch = ShardExecutor::with_data(Arc::clone(&self.services), data);
            Ok(scratch.auto_rewrite_if_due())
        })();
        let finish = self.unlock_finish(id, &granted);
        match (result, finish) {
            (Ok(rebuilt), Ok(())) => rebuilt,
            _ => false,
        }
    }
}

// -- RESP helpers ----------------------------------------------------------

fn parse_u64(raw: &[u8]) -> Option<u64> {
    if raw.is_empty() || !raw.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(raw).ok()?.parse().ok()
}

fn parse_integer_reply(reply: &[u8]) -> Option<usize> {
    let raw = reply.strip_prefix(b":")?;
    let (value, _) = split_line(raw)?;
    std::str::from_utf8(value).ok()?.parse().ok()
}

/// Parse a flat RESP2 array of bulk strings, e.g. the reply of `KEYS`.
fn parse_bulk_array(reply: &[u8]) -> Option<Vec<Vec<u8>>> {
    let rest = reply.strip_prefix(b"*")?;
    let (count, mut rest) = split_line(rest)?;
    let count: usize = std::str::from_utf8(count).ok()?.parse().ok()?;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        rest = rest.strip_prefix(b"$")?;
        let (length, tail) = split_line(rest)?;
        let length: usize = std::str::from_utf8(length).ok()?.parse().ok()?;
        if tail.len() < length + 2 || &tail[length..length + 2] != b"\r\n" {
            return None;
        }
        values.push(tail[..length].to_vec());
        rest = &tail[length + 2..];
    }
    Some(values)
}

/// Parse the `[cursor, [keys...]]` reply of `SCAN`.
fn parse_scan_reply(reply: &[u8]) -> Option<(u64, Vec<Vec<u8>>)> {
    let rest = reply.strip_prefix(b"*2\r\n")?;
    let rest = rest.strip_prefix(b"$")?;
    let (length, rest) = split_line(rest)?;
    let length: usize = std::str::from_utf8(length).ok()?.parse().ok()?;
    if rest.len() < length + 2 {
        return None;
    }
    let cursor = parse_u64(&rest[..length])?;
    let rest = &rest[length + 2..];
    let keys = parse_bulk_array(rest)?;
    Some((cursor, keys))
}

fn split_line(input: &[u8]) -> Option<(&[u8], &[u8])> {
    let end = input.windows(2).position(|window| window == b"\r\n")?;
    Some((&input[..end], &input[end + 2..]))
}

fn write_bulk_array(values: &[Vec<u8>], output: &mut Vec<u8>) {
    output.extend_from_slice(format!("*{}\r\n", values.len()).as_bytes());
    for value in values {
        encode_bulk(Some(value), output);
    }
}

// -- trait implementations ------------------------------------------------

impl Execution for MultiOwner {
    fn services_handle(&self) -> Arc<ServerServices> {
        Arc::clone(&self.services)
    }
    fn run_request(&self, request: super::bridge::Request) {
        let _ = self.owner_op(OwnerId::new(0), move |executor| {
            request(executor);
        });
    }
    fn journal_sync(&self) -> std::io::Result<()> {
        self.journal_sync_blocking()
    }
    fn purge_expired(&self, limit: usize) -> usize {
        self.purge_expired_blocking(limit)
    }
    fn auto_rewrite_if_due(&self) -> bool {
        self.auto_rewrite_blocking()
    }

    fn execute(&self, arguments: &[&[u8]], output: &mut Vec<u8>) -> Action {
        // Informational commands see one coherent cross-owner aggregate; the
        // override is cleared immediately so no stale total can be observed.
        let informational = arguments.first().is_some_and(|command| {
            [b"INFO".as_slice(), b"MEMORY", b"STATS", b"DBSIZE"]
                .iter()
                .any(|name| command.eq_ignore_ascii_case(name))
        });
        if informational {
            let mut total = DatasetStats::default();
            for owner in self.layout.owners() {
                if let Ok(stats) = self.owner_op(owner, |executor| executor.local_dataset_stats()) {
                    total.merge(stats);
                }
            }
            self.services.publish_dataset(Some(total));
        }
        let action = self.execute_blocking(arguments, output);
        if informational {
            self.services.publish_dataset(None);
        }
        action
    }
    fn watch_snapshot(&self, keys: &[&[u8]]) -> Option<WatchState> {
        self.watch_snapshot_blocking(keys)
    }
    fn execute_transaction(
        &self,
        commands: &[Vec<Vec<u8>>],
        watches: &WatchState,
        output: &mut Vec<u8>,
    ) -> TransactionOutcome {
        self.execute_transaction_blocking(commands, watches, output)
    }
    fn blocking_pop(&self, keys: &[&[u8]], left: bool) -> super::PopOutcome {
        match self.scratch_outcome(keys, |scratch| scratch.blocking_pop(keys, left)) {
            Ok(outcome) => outcome,
            Err(_) => super::PopOutcome::JournalError,
        }
    }
    fn blocking_lmpop(&self, keys: &[&[u8]], left: bool, count: usize) -> super::ListPopOutcome {
        match self.scratch_outcome(keys, |scratch| scratch.blocking_lmpop(keys, left, count)) {
            Ok(outcome) => outcome,
            Err(_) => super::ListPopOutcome::JournalError,
        }
    }
    fn blocking_list_move(
        &self,
        source: &[u8],
        destination: &[u8],
        from_left: bool,
        to_left: bool,
    ) -> super::ListMoveOutcome {
        let keys = [source, destination];
        match self.scratch_outcome(keys.as_slice(), |scratch| {
            scratch.blocking_list_move(source, destination, from_left, to_left)
        }) {
            Ok(outcome) => outcome,
            Err(_) => super::ListMoveOutcome::JournalError,
        }
    }
    fn blocking_zpop(&self, keys: &[&[u8]], max: bool, count: usize) -> super::ZSetPopOutcome {
        match self.scratch_outcome(keys, |scratch| scratch.blocking_zpop(keys, max, count)) {
            Ok(outcome) => outcome,
            Err(_) => super::ZSetPopOutcome::JournalError,
        }
    }
    fn xread_start(
        &self,
        options: &super::streams::XReadOptions<'_>,
    ) -> Option<(Vec<super::StreamId>, super::XReadOutcome)> {
        self.scratch_outcome(&options.keys, |scratch| scratch.xread_start(options))
            .ok()
            .flatten()
    }
    fn xread_once(
        &self,
        keys: &[&[u8]],
        starts: &[super::StreamId],
        count: usize,
    ) -> Option<super::XReadOutcome> {
        self.scratch_outcome(keys, |scratch| scratch.xread_once(keys, starts, count))
            .ok()
            .flatten()
    }
    fn blocking_xreadgroup(
        &self,
        options: &super::XReadGroupOptions<'_>,
    ) -> super::XReadGroupOutcome {
        match self.scratch_outcome(&options.keys, |scratch| {
            scratch.blocking_xreadgroup(options)
        }) {
            Ok(outcome) => outcome,
            Err(_) => super::XReadGroupOutcome::JournalError,
        }
    }
    fn waitaof_probe(&self, local: i64, replicas: i64) -> Result<(u64, u64), &'static [u8]> {
        match self.owner_op(OwnerId::new(0), move |executor| {
            executor.waitaof_probe(local, replicas)
        }) {
            Ok(result) => result,
            Err(_) => Err(b"owner unavailable"),
        }
    }
    fn shutdown_command(&self, args: &[&[u8]], output: &mut Vec<u8>) {
        if let Ok(reply) = self.forward(OwnerId::new(0), args) {
            output.extend_from_slice(&reply.bytes);
        }
    }
}

impl Drop for MultiOwner {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

#[cfg(test)]
mod tests;
