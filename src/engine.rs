//! Atomic in-memory string commands with expiration and a key/value payload budget.

use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::protocol::{Limits, encode_bulk};

const COMMAND_COUNTER_SHARDS: usize = 64;

mod acl_log;
mod administration;
mod audit;
mod audit_syslog;
mod auth;
mod auth_guard;
mod bitfield;
mod blocking;
mod clients;
mod collection_commands;
mod collection_config;
mod collections;
mod commands;
mod data;
mod discovery;
mod dispatch;
mod error_dialect;
pub(crate) use discovery::is_write_command;
mod eviction;
mod geo;
mod hll;
mod introspection;
mod journal;
mod keyspace;
mod latency;
mod mailbox;
mod monitor;
mod multikey;
mod notify;
mod options;
mod owner_actor;
mod partition;
mod pubsub;
mod router;
mod scan;
mod sha256;
mod shard;
mod slowlog;
mod sort;
mod streams;
mod string_algorithms;
mod string_ranges;
mod transaction;
mod unsupported;
mod xxh3;
pub use acl_log::AclLogEntry;

/// Verify a per-line audit HMAC with a shared 16..=1024-byte key.
pub fn verify_audit_line(key: &[u8], line: &str) -> bool {
    audit::verify_line(key, line)
}

/// Parse a hex-encoded audit keyring used for key rotation. The first key is the
/// signing key; all keys are valid verification keys.
pub fn parse_audit_keyring(text: &str) -> Result<Vec<Vec<u8>>, String> {
    audit::parse_keyring(text)
}

/// Read a hex audit keyring with a 64 KiB text bound, including comments.
pub fn read_audit_keyring(path: &std::path::Path) -> std::io::Result<Vec<Vec<u8>>> {
    audit::read_keyring(path)
}
pub use auth::{Access, AclError, Auth, AuthError, UserDetails};
pub use blocking::PopOutcome;
pub use blocking::reply_value as blocking_reply;
pub use clients::{ClientSnapshot, UnblockMode};
pub(crate) use collection_commands::{
    ListMoveOutcome, ListPopOutcome, ZSetPopOutcome, list_move_reply,
    list_pop_reply as blocking_lmpop_reply, parse_bzmpop_timeout, parse_list_mpop, parse_mpop,
    parse_side, zset_pop_reply as blocking_zpop_reply, zset_pop_single_reply,
};
use collection_config::CollectionConfig;
use data::Data;
pub(crate) use data::StreamId;
pub use error_dialect::ErrorDialect;
pub use eviction::EvictionPolicy;
pub use journal::FsyncPolicy;
pub use notify::{Category as NotifyCategory, NotifyFlags};
mod bridge;
pub use bridge::Store;
pub(crate) use bridge::{BridgeInbox, Execution, Request, StoreHandle};
pub(crate) use router::{MultiOwner, RouterSizing};
pub(crate) use streams::{
    XReadGroupOptions, XReadGroupOutcome, XReadGroupStart, XReadOptions, XReadOutcome, parse_xread,
    parse_xreadgroup, xread_reply, xreadgroup_reply,
};
pub(crate) use transaction::WatchToken;
pub use transaction::{TransactionOutcome, WatchState};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Continue,
    Close,
}

/// Unix milliseconds, injectable for deterministic expiration tests.
/// Expiration intentionally follows wall time, as Redis does; socket deadlines do not.
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> i64;
}
struct SystemClock;
impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(i64::MAX as u128) as i64
    }
}

/// Per-command counters exposed through `INFO commandstats`. Each slot maps to
/// one registered command and is updated with relaxed atomics so recording does
/// not serialize independent worker threads.
#[derive(Default)]
pub(super) struct CommandStat {
    calls: std::sync::atomic::AtomicU64,
    usec: std::sync::atomic::AtomicU64,
    rejected_calls: std::sync::atomic::AtomicU64,
    failed_calls: std::sync::atomic::AtomicU64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CommandStatValues {
    pub name: &'static str,
    pub calls: u64,
    pub usec: u64,
    pub rejected_calls: u64,
    pub failed_calls: u64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub total_connections_received: u64,
    pub total_commands_processed: u64,
    pub rejected_connections: u64,
    pub expired_keys: u64,
    pub expired_subkeys: u64,
    pub evicted_keys: u64,
    pub pubsub_channels: usize,
    pub pubsub_patterns: usize,
    pub audit_log_write_failures: u64,
    pub audit_syslog_write_failures: u64,
}

/// Aggregated keyspace accounting used by INFO/MEMORY/STATS. Each owner computes
/// its own live keys, expiry deadlines, payload and lifecycle counters; the
/// multi-owner router sums them under one barrier instead of reading another
/// worker's `Data`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DatasetStats {
    pub(crate) live_keys: usize,
    pub(crate) expiring_keys: usize,
    pub(crate) subexpiry_keys: usize,
    pub(crate) ttl_sum: u64,
    pub(crate) ttl_count: u64,
    pub(crate) payload_bytes: usize,
    pub(crate) peak_payload_bytes: usize,
    pub(crate) expired_keys: u64,
    pub(crate) expired_subkeys: u64,
    pub(crate) evicted_keys: u64,
}
impl DatasetStats {
    pub(crate) fn merge(&mut self, other: Self) {
        self.live_keys = self.live_keys.saturating_add(other.live_keys);
        self.expiring_keys = self.expiring_keys.saturating_add(other.expiring_keys);
        self.subexpiry_keys = self.subexpiry_keys.saturating_add(other.subexpiry_keys);
        self.ttl_sum = self.ttl_sum.saturating_add(other.ttl_sum);
        self.ttl_count = self.ttl_count.saturating_add(other.ttl_count);
        self.payload_bytes = self.payload_bytes.saturating_add(other.payload_bytes);
        self.peak_payload_bytes = self.peak_payload_bytes.max(other.peak_payload_bytes);
        self.expired_keys = self.expired_keys.saturating_add(other.expired_keys);
        self.expired_subkeys = self.expired_subkeys.saturating_add(other.expired_subkeys);
        self.evicted_keys = self.evicted_keys.saturating_add(other.evicted_keys);
    }
    #[allow(dead_code)] // surfaced by a later INFO field
    pub(crate) fn avg_ttl(&self) -> u64 {
        self.ttl_sum.checked_div(self.ttl_count).unwrap_or(0)
    }
}

#[doc(hidden)]
/// Snapshot of the enabled observation collectors, read with one load.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Observers {
    pub slowlog: bool,
    pub latency: bool,
    pub commandstats: bool,
}

pub struct ServerServices {
    #[cfg(test)]
    owner_events: std::sync::Mutex<Vec<(&'static str, std::thread::ThreadId)>>,
    #[cfg(test)]
    bridge_submissions: std::sync::atomic::AtomicUsize,
    router: RwLock<shard::ShardRouter>,
    eviction_policy: std::sync::atomic::AtomicU8,
    max_payload_bytes: std::sync::atomic::AtomicUsize,
    /// Global payload admitted by every owner, used as a local credit bound.
    payload_committed: std::sync::atomic::AtomicUsize,
    /// Set once a multi-owner router (not a single local owner) serves requests.
    multi_owner: std::sync::atomic::AtomicBool,
    dataset_override: std::sync::Mutex<Option<DatasetStats>>,
    error_dialect: RwLock<ErrorDialect>,
    collection_config: RwLock<CollectionConfig>,
    clock: Arc<dyn Clock>,
    auth: RwLock<Auth>,
    auth_required: AtomicBool,
    acl_restricted: AtomicBool,
    journal: Option<std::sync::Mutex<journal::Journal>>,
    journal_limits: Limits,
    pubsub: RwLock<pubsub::PubSub>,
    clients: RwLock<std::collections::BTreeMap<u64, clients::ClientRecord>>,
    unblock_requests: std::sync::Mutex<std::collections::BTreeMap<u64, clients::UnblockMode>>,
    monitors: RwLock<std::collections::BTreeMap<u64, std::sync::mpsc::SyncSender<Vec<u8>>>>,
    monitor_count: std::sync::atomic::AtomicUsize,
    notifications: std::sync::atomic::AtomicU32,
    notification_epoch: std::sync::atomic::AtomicU64,
    /// Bumped by every setting the owner caches (notifications, eviction,
    /// collect-events, payload limit, owner mode), so the per-command fast path
    /// can check one atomic instead of reloading each of them.
    config_epoch: std::sync::atomic::AtomicU64,
    acl_log: RwLock<std::collections::VecDeque<acl_log::AclLogEntry>>,
    audit_log: std::sync::Mutex<Option<audit::AuditLog>>,
    audit_log_write_failures: std::sync::atomic::AtomicU64,
    audit_hmac_key: RwLock<Option<Vec<u8>>>,
    audit_syslog: std::sync::Mutex<Option<audit_syslog::SyslogSink>>,
    audit_syslog_write_failures: std::sync::atomic::AtomicU64,
    connections_received: std::sync::atomic::AtomicU64,
    commands_processed: [std::sync::atomic::AtomicU64; COMMAND_COUNTER_SHARDS],
    rejected_connections: std::sync::atomic::AtomicU64,
    slowlog: std::sync::RwLock<std::collections::VecDeque<slowlog::SlowlogEntry>>,
    slowlog_next_id: std::sync::atomic::AtomicU64,
    slowlog_threshold_us: std::sync::atomic::AtomicI64,
    slowlog_max_len: std::sync::atomic::AtomicUsize,
    client_pause: std::sync::atomic::AtomicU64,
    max_clients: std::sync::atomic::AtomicUsize,
    auth_failure_limit: std::sync::atomic::AtomicUsize,
    auth_source_limit: std::sync::atomic::AtomicUsize,
    auth_guard: std::sync::Mutex<auth_guard::AuthGuard>,
    latency_monitor_threshold_ms: std::sync::atomic::AtomicU64,
    latency: std::sync::RwLock<latency::State>,
    auto_aof_rewrite_percentage: std::sync::atomic::AtomicU64,
    auto_aof_rewrite_min_size: std::sync::atomic::AtomicU64,
    aof_config: RwLock<journal::AofConfig>,
    error_stats: RwLock<std::collections::BTreeMap<Vec<u8>, u64>>,
    shutdown_requested: AtomicBool,
    owner_failed: AtomicBool,
    commandstats_enabled: AtomicBool,
    /// Bit set of enabled observation collectors, so the per-command epilogue
    /// makes one load instead of one per collector.
    observer_word: std::sync::atomic::AtomicU32,
    command_stats: Vec<CommandStat>,
}

/// Synchronous, thread-confined data execution. No data reference escapes this type.
/// The Rc marker makes the entire executor !Send and !Sync.
pub(crate) struct ShardExecutor {
    services: Arc<ServerServices>,
    /// Thread that constructed this executor; asserted on drop in tests so
    /// several owner threads can exist without sharing one event order.
    #[cfg(test)]
    owner_thread: std::thread::ThreadId,
    data: std::cell::RefCell<Data>,
    notification_epoch: std::cell::Cell<u64>,
    config_epoch: std::cell::Cell<u64>,
    /// Cached from the configuration epoch: whether any observer can consume
    /// per-command effects, so the hot path never reloads those atomics.
    effects_enabled: std::cell::Cell<bool>,
    /// Last payload total this owner contributed to the shared credit counter.
    published_payload: std::cell::Cell<usize>,
    _local: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl std::ops::Deref for ShardExecutor {
    type Target = ServerServices;
    fn deref(&self) -> &ServerServices {
        &self.services
    }
}

impl ServerServices {
    /// Publish one coherent cross-owner dataset aggregate for the duration of an
    /// informational command; `None` clears it again.
    pub(crate) fn publish_dataset(&self, stats: Option<DatasetStats>) {
        if let Ok(mut guard) = self.dataset_override.lock() {
            *guard = stats;
        }
    }

    /// Publish the fixed startup layout so diagnostics and routing agree.
    pub(crate) fn set_router(&self, router: shard::ShardRouter) {
        if let Ok(mut shared) = self.router.write() {
            *shared = router;
        }
    }

    #[cfg(test)]
    pub(crate) fn owner_trace(&self) -> Vec<(&'static str, std::thread::ThreadId)> {
        self.owner_events.lock().unwrap().clone()
    }
    #[cfg(test)]
    pub(crate) fn bridge_count(&self) -> usize {
        self.bridge_submissions.load(Ordering::Relaxed)
    }
    fn with_clock_and_auth(max_payload_bytes: usize, clock: Arc<dyn Clock>, auth: Auth) -> Self {
        Self {
            #[cfg(test)]
            owner_events: std::sync::Mutex::new(Vec::new()),
            #[cfg(test)]
            bridge_submissions: std::sync::atomic::AtomicUsize::new(0),
            router: RwLock::new(shard::ShardRouter::new(1).expect("single-owner layout")),
            eviction_policy: std::sync::atomic::AtomicU8::new(0),
            max_payload_bytes: std::sync::atomic::AtomicUsize::new(max_payload_bytes),
            payload_committed: std::sync::atomic::AtomicUsize::new(0),
            multi_owner: std::sync::atomic::AtomicBool::new(false),
            dataset_override: std::sync::Mutex::new(None),
            error_dialect: RwLock::new(ErrorDialect::default()),
            collection_config: RwLock::new(CollectionConfig::default()),
            clock,
            auth_required: AtomicBool::new(auth.required()),
            acl_restricted: AtomicBool::new(auth.restricted()),
            auth: RwLock::new(auth),
            journal: None,
            journal_limits: Limits::default(),
            pubsub: RwLock::new(pubsub::PubSub::default()),
            clients: RwLock::new(std::collections::BTreeMap::new()),
            unblock_requests: std::sync::Mutex::new(std::collections::BTreeMap::new()),
            monitors: RwLock::new(std::collections::BTreeMap::new()),
            monitor_count: std::sync::atomic::AtomicUsize::new(0),
            notifications: std::sync::atomic::AtomicU32::new(0),
            notification_epoch: std::sync::atomic::AtomicU64::new(0),
            config_epoch: std::sync::atomic::AtomicU64::new(0),
            acl_log: RwLock::new(std::collections::VecDeque::new()),
            audit_log: std::sync::Mutex::new(None),
            audit_log_write_failures: std::sync::atomic::AtomicU64::new(0),
            audit_hmac_key: RwLock::new(None),
            audit_syslog: std::sync::Mutex::new(None),
            audit_syslog_write_failures: std::sync::atomic::AtomicU64::new(0),
            connections_received: std::sync::atomic::AtomicU64::new(0),
            commands_processed: std::array::from_fn(|_| std::sync::atomic::AtomicU64::new(0)),
            rejected_connections: std::sync::atomic::AtomicU64::new(0),
            slowlog: std::sync::RwLock::new(std::collections::VecDeque::new()),
            slowlog_next_id: std::sync::atomic::AtomicU64::new(0),
            slowlog_threshold_us: std::sync::atomic::AtomicI64::new(slowlog::DEFAULT_THRESHOLD_US),
            slowlog_max_len: std::sync::atomic::AtomicUsize::new(slowlog::DEFAULT_MAX_LEN),
            client_pause: std::sync::atomic::AtomicU64::new(0),
            max_clients: std::sync::atomic::AtomicUsize::new(128),
            auth_failure_limit: std::sync::atomic::AtomicUsize::new(10),
            auth_source_limit: std::sync::atomic::AtomicUsize::new(0),
            auth_guard: std::sync::Mutex::new(auth_guard::AuthGuard::new(0, 60_000)),
            latency_monitor_threshold_ms: std::sync::atomic::AtomicU64::new(
                latency::DEFAULT_THRESHOLD_MS,
            ),
            latency: std::sync::RwLock::new(latency::State::default()),
            auto_aof_rewrite_percentage: std::sync::atomic::AtomicU64::new(100),
            auto_aof_rewrite_min_size: std::sync::atomic::AtomicU64::new(64 * 1024 * 1024),
            aof_config: RwLock::new(journal::AofConfig::default()),
            error_stats: RwLock::new(std::collections::BTreeMap::new()),
            shutdown_requested: AtomicBool::new(false),
            owner_failed: AtomicBool::new(false),
            commandstats_enabled: AtomicBool::new(false),
            observer_word: std::sync::atomic::AtomicU32::new(0),
            command_stats: (0..discovery::command_count())
                .map(|_| CommandStat::default())
                .collect(),
        }
    }

    /// Bounded best-effort background reclamation; no sleep-dependent correctness.
    /// Every lookup still treats logically expired entries as absent immediately.
    pub(crate) fn request_shutdown(&self) {
        self.shutdown_requested.store(true, Ordering::Release);
    }

    pub(crate) fn shutdown_requested(&self) -> bool {
        self.shutdown_requested.load(Ordering::Acquire)
    }

    /// Command-table arity used for MULTI queue-time validation and discovery.
    pub fn command_arity(name: &[u8]) -> Option<i64> {
        discovery::command_arity(name)
    }

    /// Whether this store requires authentication for data/admin commands.
    /// The atomic fast path keeps the common benchmark path lock-free.
    pub fn authentication_required(&self) -> bool {
        self.auth_required.load(Ordering::Acquire)
    }

    pub fn authenticate(&self, username: Option<&[u8]>, password: &[u8]) -> Result<(), AuthError> {
        self.authenticate_user(username, password).map(|_| ())
    }

    pub fn authenticate_user(
        &self,
        username: Option<&[u8]>,
        password: &[u8],
    ) -> Result<Vec<u8>, AuthError> {
        self.auth
            .read()
            .map_err(|_| AuthError::Internal)?
            .verify_user(username, password)
    }

    /// True when any user/rule/key restriction exists, so command authorization
    /// cannot use the lock-free open-server path.
    pub fn acl_restricted(&self) -> bool {
        self.acl_restricted.load(Ordering::Acquire)
    }

    pub fn authorize(&self, user: Option<&[u8]>, arguments: &[&[u8]]) -> Access {
        self.auth
            .read()
            .map_or(Access::NoAuth, |auth| auth.authorize(user, arguments))
    }

    pub fn whoami(&self, user: Option<&[u8]>) -> Option<Vec<u8>> {
        self.auth.read().ok()?.whoami(user)
    }

    pub fn acl_user_names(&self) -> Option<Vec<Vec<u8>>> {
        self.auth.read().ok().map(|auth| auth.user_names())
    }

    pub fn acl_line(&self, name: &[u8]) -> Option<String> {
        self.auth.read().ok()?.acl_line(name)
    }

    pub fn acl_details(&self, name: &[u8]) -> Option<UserDetails> {
        self.auth.read().ok()?.details(name)
    }

    pub fn acl_setuser(&self, name: &[u8], rules: &[&[u8]]) -> Result<(), AclError> {
        let mut auth = self.auth.write().map_err(|_| AclError::Syntax)?;
        auth.setuser(name, rules)?;
        self.refresh_auth_flags(&auth);
        Ok(())
    }

    pub fn acl_deluser(&self, names: &[&[u8]]) -> Result<usize, AclError> {
        let mut auth = self.auth.write().map_err(|_| AclError::Syntax)?;
        if names
            .iter()
            .any(|name| name.eq_ignore_ascii_case(b"default"))
        {
            return Err(AclError::ProtectedDefault);
        }
        let mut removed = 0;
        for name in names {
            if auth.remove_user(name) {
                removed += 1;
            }
        }
        self.refresh_auth_flags(&auth);
        Ok(removed)
    }

    pub fn acl_save(&self) -> Result<(), AclError> {
        let auth = self.auth.write().map_err(|_| AclError::Syntax)?;
        let Some(path) = auth.aclfile() else {
            return Err(AclError::NoAclFile);
        };
        auth.save_file(path)
    }

    pub fn acl_load(&self) -> Result<(), AclError> {
        let mut auth = self.auth.write().map_err(|_| AclError::Syntax)?;
        let Some(path) = auth.aclfile().map(Path::to_path_buf) else {
            return Err(AclError::NoAclFile);
        };
        auth.load_file(&path)?;
        self.refresh_auth_flags(&auth);
        Ok(())
    }

    fn refresh_auth_flags(&self, auth: &Auth) {
        self.auth_required.store(auth.required(), Ordering::Release);
        self.acl_restricted
            .store(auth.restricted(), Ordering::Release);
    }

    pub fn record_connection(&self) {
        self.connections_received.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_rejected_connection(&self) {
        self.rejected_connections.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_command(&self, client_id: u64) {
        let shard = (client_id as usize) % COMMAND_COUNTER_SHARDS;
        self.commands_processed[shard].fetch_add(1, Ordering::Relaxed);
    }

    /// Record one network command dispatch. `reply` is only the bytes produced
    /// by this call, so a leading `-` identifies a failed call and NOAUTH/NOPERM
    /// identify authentication/authorization rejections.
    pub(crate) fn record_command_stats(
        &self,
        command: Option<&[u8]>,
        elapsed: Duration,
        reply: &[u8],
    ) {
        if !self.commandstats_enabled.load(Ordering::Relaxed) {
            return;
        }
        let Some(command) = command else {
            return;
        };
        let Some(stat) =
            discovery::command_index(command).and_then(|index| self.command_stats.get(index))
        else {
            return;
        };
        stat.calls.fetch_add(1, Ordering::Relaxed);
        stat.usec.fetch_add(
            u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
        if reply.starts_with(b"-") {
            stat.failed_calls.fetch_add(1, Ordering::Relaxed);
        }
        if reply.starts_with(b"-NOAUTH") || reply.starts_with(b"-NOPERM") {
            stat.rejected_calls.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn commandstats_enabled(&self) -> bool {
        self.observer_word.load(Ordering::Relaxed) & Self::OBSERVER_COMMANDSTATS != 0
    }

    const OBSERVER_SLOWLOG: u32 = 1;
    const OBSERVER_LATENCY: u32 = 2;
    const OBSERVER_COMMANDSTATS: u32 = 4;

    /// All observation collectors in one load.
    pub(crate) fn observers(&self) -> Observers {
        let word = self.observer_word.load(Ordering::Relaxed);
        Observers {
            slowlog: word & Self::OBSERVER_SLOWLOG != 0,
            latency: word & Self::OBSERVER_LATENCY != 0,
            commandstats: word & Self::OBSERVER_COMMANDSTATS != 0,
        }
    }

    /// Recompute the collector bit set from the individual settings.
    pub(crate) fn refresh_observers(&self) {
        let mut word = 0;
        if self.slowlog_threshold_us.load(Ordering::Relaxed) >= 0
            && self.slowlog_max_len.load(Ordering::Relaxed) > 0
        {
            word |= Self::OBSERVER_SLOWLOG;
        }
        if self.latency_monitor_threshold_ms.load(Ordering::Relaxed) > 0 {
            word |= Self::OBSERVER_LATENCY;
        }
        if self.commandstats_enabled.load(Ordering::Relaxed) {
            word |= Self::OBSERVER_COMMANDSTATS;
        }
        self.observer_word.store(word, Ordering::Relaxed);
    }

    pub fn set_commandstats_enabled(&self, enabled: bool) {
        let previous = self.commandstats_enabled.swap(enabled, Ordering::Relaxed);
        if enabled && !previous {
            self.reset_command_statistics();
        }
        self.refresh_observers();
    }

    fn reset_command_statistics(&self) {
        for stat in &self.command_stats {
            stat.calls.store(0, Ordering::Relaxed);
            stat.usec.store(0, Ordering::Relaxed);
            stat.rejected_calls.store(0, Ordering::Relaxed);
            stat.failed_calls.store(0, Ordering::Relaxed);
        }
    }

    /// Snapshot only commands with at least one recorded call, in registry order.
    pub(super) fn command_statistics(&self) -> Vec<CommandStatValues> {
        self.command_stats
            .iter()
            .enumerate()
            .filter_map(|(index, stat)| {
                let calls = stat.calls.load(Ordering::Relaxed);
                (calls > 0).then(|| CommandStatValues {
                    name: discovery::command_name_at(index).unwrap_or("unknown"),
                    calls,
                    usec: stat.usec.load(Ordering::Relaxed),
                    rejected_calls: stat.rejected_calls.load(Ordering::Relaxed),
                    failed_calls: stat.failed_calls.load(Ordering::Relaxed),
                })
            })
            .collect()
    }

    // INFO already captured these counters. Reuse them without extending
    // its data borrow across control-service snapshotting.
    fn stats_from_counters(
        &self,
        (expired_keys, expired_subkeys, evicted_keys): (u64, u64, u64),
    ) -> Stats {
        let (pubsub_channels, pubsub_patterns) = self.pubsub.read().map_or((0, 0), |pubsub| {
            (pubsub.channel_names(None).len(), pubsub.numpat())
        });
        Stats {
            total_connections_received: self.connections_received.load(Ordering::Relaxed),
            total_commands_processed: self
                .commands_processed
                .iter()
                .map(|counter| counter.load(Ordering::Relaxed))
                .sum(),
            rejected_connections: self.rejected_connections.load(Ordering::Relaxed),
            expired_keys,
            expired_subkeys,
            evicted_keys,
            pubsub_channels,
            pubsub_patterns,
            audit_log_write_failures: self.audit_log_write_failures.load(Ordering::Relaxed),
            audit_syslog_write_failures: self.audit_syslog_write_failures.load(Ordering::Relaxed),
        }
    }

    pub(crate) fn record_error(&self, reply: &[u8]) {
        let Some(body) = reply.strip_prefix(b"-") else {
            return;
        };
        let end = body
            .iter()
            .position(|byte| matches!(*byte, b' ' | b'\r' | b'\n'))
            .unwrap_or(body.len())
            .min(32);
        if end == 0 {
            return;
        }
        let code = body[..end].to_vec();
        if let Ok(mut stats) = self.error_stats.write() {
            if let Some(count) = stats.get_mut(&code) {
                *count = count.saturating_add(1);
            } else if stats.len() < 64 {
                stats.insert(code, 1);
            }
        }
    }

    pub(crate) fn error_stats(&self) -> Vec<(Vec<u8>, u64)> {
        self.error_stats
            .read()
            .map(|stats| {
                stats
                    .iter()
                    .map(|(code, count)| (code.clone(), *count))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn set_max_clients(&self, value: usize) {
        self.max_clients.store(value, Ordering::Relaxed);
    }

    pub fn max_clients(&self) -> usize {
        self.max_clients.load(Ordering::Relaxed)
    }

    pub fn auth_failure_limit(&self) -> usize {
        self.auth_failure_limit.load(Ordering::Acquire)
    }

    pub fn set_auth_failure_limit(&self, limit: usize) {
        self.auth_failure_limit.store(limit, Ordering::Release);
    }

    pub fn set_auth_source_limit(&self, limit: usize, window_ms: u64) {
        self.auth_source_limit.store(limit, Ordering::Release);
        if let Ok(mut guard) = self.auth_guard.lock() {
            guard.configure(limit, window_ms);
        }
    }

    pub fn auth_source_limit(&self) -> usize {
        self.auth_source_limit.load(Ordering::Acquire)
    }

    pub fn auth_source_locked(&self, source: std::net::IpAddr) -> bool {
        self.auth_guard
            .lock()
            .is_ok_and(|mut guard| guard.locked(source))
    }

    pub fn note_auth_source_failure(&self, source: std::net::IpAddr) {
        if let Ok(mut guard) = self.auth_guard.lock() {
            guard.note_failure(source);
        }
    }

    pub fn reset_auth_source(&self, source: std::net::IpAddr) {
        if let Ok(mut guard) = self.auth_guard.lock() {
            guard.reset(source);
        }
    }
    /// Invalidate the per-owner configuration cache.
    pub(crate) fn bump_config(&self) {
        self.config_epoch.fetch_add(1, Ordering::AcqRel);
    }

    pub fn set_eviction_policy(&self, policy: EvictionPolicy) -> bool {
        self.eviction_policy.store(policy as u8, Ordering::Release);
        self.bump_config();
        true
    }

    pub(crate) fn auto_aof_rewrite_percentage(&self) -> u64 {
        self.auto_aof_rewrite_percentage
            .load(std::sync::atomic::Ordering::Acquire)
    }

    pub(crate) fn set_auto_aof_rewrite_percentage(&self, value: u64) {
        self.auto_aof_rewrite_percentage
            .store(value, std::sync::atomic::Ordering::Release);
    }

    pub(crate) fn auto_aof_rewrite_min_size(&self) -> u64 {
        self.auto_aof_rewrite_min_size
            .load(std::sync::atomic::Ordering::Acquire)
    }

    pub(crate) fn set_auto_aof_rewrite_min_size(&self, value: u64) {
        self.auto_aof_rewrite_min_size
            .store(value, std::sync::atomic::Ordering::Release);
    }

    pub(crate) fn aof_config(&self) -> Option<journal::AofConfig> {
        self.aof_config.read().ok().map(|config| config.clone())
    }

    pub(crate) fn set_aof_config(
        &self,
        appendonly: bool,
        appendfilename: &str,
        policy: FsyncPolicy,
    ) -> bool {
        // Attaching or detaching the journal changes what an owner collects.
        self.bump_config();
        self.aof_config.write().is_ok_and(|mut config| {
            config.appendonly = appendonly;
            config.appendfilename = appendfilename.to_string();
            config.policy = policy;
            true
        })
    }

    /// Current payload-budget limit in key and value bytes.
    #[allow(dead_code)] // test-only today
    pub(super) fn payload_limit(&self) -> usize {
        self.max_payload_bytes.load(Ordering::Relaxed)
    }
    pub fn shard_for(&self, key: &[u8]) -> usize {
        self.router.read().map_or(0, |router| router.shard_for(key))
    }
    pub fn shard_count(&self) -> usize {
        self.router.read().map_or(1, |router| router.count())
    }
    pub fn eviction_policy(&self) -> EvictionPolicy {
        match self.eviction_policy.load(Ordering::Acquire) {
            1 => EvictionPolicy::AllKeysRandom,
            2 => EvictionPolicy::AllKeysLru,
            3 => EvictionPolicy::VolatileRandom,
            4 => EvictionPolicy::VolatileLru,
            5 => EvictionPolicy::VolatileTtl,
            _ => EvictionPolicy::NoEviction,
        }
    }
    pub fn set_notify_flags(&self, flags: NotifyFlags) {
        self.notifications.store(flags.bits(), Ordering::Release);
        self.notification_epoch.fetch_add(1, Ordering::AcqRel);
        self.bump_config();
    }

    pub fn notify_flags(&self) -> NotifyFlags {
        NotifyFlags::from_bits(self.notifications.load(Ordering::Acquire))
    }

    pub fn notifications_enabled(&self) -> bool {
        self.notifications.load(Ordering::Acquire) != 0
    }

    /// Register a MONITOR stream. Re-registering the same connection replaces
    /// its sender; the stream is bounded and uses the pubsub queue capacity.
    pub fn add_monitor(&self, id: u64, sender: std::sync::mpsc::SyncSender<Vec<u8>>) -> bool {
        let Ok(mut monitors) = self.monitors.write() else {
            return false;
        };
        monitors.insert(id, sender);
        self.monitor_count.store(monitors.len(), Ordering::Relaxed);
        true
    }

    pub fn remove_monitor(&self, id: u64) {
        let Ok(mut monitors) = self.monitors.write() else {
            return;
        };
        if monitors.remove(&id).is_some() {
            self.monitor_count.store(monitors.len(), Ordering::Relaxed);
        }
    }

    /// Stream one received command to every monitor except its source. A full
    /// monitor queue drops this line rather than blocking the command path.
    pub fn monitor_command(&self, source: u64, arguments: &[&[u8]]) {
        if self.monitor_count.load(Ordering::Relaxed) == 0 {
            return;
        }
        let address = self.client_address(source).unwrap_or_else(|| "?:0".into());
        let line = monitor::line(&address, arguments);
        let Ok(monitors) = self.monitors.read() else {
            return;
        };
        for (id, sender) in monitors.iter() {
            if *id == source {
                continue;
            }
            let _ = sender.try_send(line.clone());
        }
    }

    pub fn pubsub_queue_capacity() -> usize {
        pubsub::PubSub::queue_capacity()
    }

    pub fn pubsub_subscribe(
        &self,
        pattern: bool,
        name: &[u8],
        id: u64,
        sender: std::sync::mpsc::SyncSender<Vec<u8>>,
        resp3: bool,
    ) -> bool {
        self.pubsub.write().is_ok_and(|mut pubsub| {
            pubsub.subscribe(pattern, name, id, sender, resp3);
            true
        })
    }

    pub fn pubsub_unsubscribe(&self, pattern: bool, name: &[u8], id: u64) -> bool {
        self.pubsub
            .write()
            .is_ok_and(|mut pubsub| pubsub.unsubscribe(pattern, name, id))
    }
    pub fn pubsub_subscribe_shard(
        &self,
        name: &[u8],
        id: u64,
        sender: std::sync::mpsc::SyncSender<Vec<u8>>,
        resp3: bool,
    ) -> bool {
        self.pubsub.write().is_ok_and(|mut pubsub| {
            pubsub.subscribe_shard(name, id, sender, resp3);
            true
        })
    }

    pub fn pubsub_unsubscribe_shard(&self, name: &[u8], id: u64) -> bool {
        self.pubsub
            .write()
            .is_ok_and(|mut pubsub| pubsub.unsubscribe_shard(name, id))
    }

    pub fn publish_shard(&self, channel: &[u8], message: &[u8]) -> usize {
        self.pubsub
            .write()
            .map_or(0, |mut pubsub| pubsub.publish_shard(channel, message))
    }

    pub fn pubsub_shard_channels(&self, pattern: Option<&[u8]>) -> Vec<Vec<u8>> {
        self.pubsub
            .read()
            .map_or_else(|_| Vec::new(), |pubsub| pubsub.shard_channel_names(pattern))
    }

    pub fn pubsub_shard_numsub(&self, channels: &[&[u8]]) -> Vec<(Vec<u8>, usize)> {
        self.pubsub
            .read()
            .map_or_else(|_| Vec::new(), |pubsub| pubsub.shard_numsub(channels))
    }

    pub fn pubsub_remove_client(&self, id: u64) {
        if let Ok(mut pubsub) = self.pubsub.write() {
            pubsub.remove_client(id);
        }
    }

    pub fn pubsub_set_resp3(&self, id: u64, resp3: bool) {
        if let Ok(mut pubsub) = self.pubsub.write() {
            pubsub.set_resp3(id, resp3);
        }
    }

    pub fn publish(&self, channel: &[u8], message: &[u8]) -> usize {
        self.pubsub
            .write()
            .map_or(0, |mut pubsub| pubsub.publish(channel, message))
    }

    pub fn pubsub_channels(&self, pattern: Option<&[u8]>) -> Vec<Vec<u8>> {
        self.pubsub
            .read()
            .map_or_else(|_| Vec::new(), |pubsub| pubsub.channel_names(pattern))
    }

    pub fn pubsub_numsub(&self, channels: &[&[u8]]) -> Vec<(Vec<u8>, usize)> {
        self.pubsub
            .read()
            .map_or_else(|_| Vec::new(), |pubsub| pubsub.numsub(channels))
    }

    pub fn pubsub_numpat(&self) -> usize {
        self.pubsub.read().map_or(0, |pubsub| pubsub.numpat())
    }

    pub fn pubsub_is_empty(&self) -> bool {
        self.pubsub.read().is_ok_and(|pubsub| pubsub.is_empty())
    }

    pub fn acl_categories() -> &'static [&'static str] {
        discovery::CATEGORIES
    }

    pub fn acl_commands(category: &[u8]) -> Vec<String> {
        discovery::commands_in_category(category)
    }

    pub fn requirepass_configured(&self) -> bool {
        self.auth_required.load(Ordering::Acquire)
    }

    /// Atomically replace the default credential; only CONFIG SET uses this.
    pub fn set_requirepass(&self, password: Option<&[u8]>) -> bool {
        let Ok(mut auth) = self.auth.write() else {
            return false;
        };
        auth.set_password(password);
        self.refresh_auth_flags(&auth);
        true
    }
}

impl ShardExecutor {
    #[cfg(test)]
    pub(crate) fn new(limit: usize) -> Self {
        Self::with_clock(limit, Arc::new(SystemClock))
    }
    #[cfg(test)]
    pub(crate) fn with_clock(limit: usize, clock: Arc<dyn Clock>) -> Self {
        Self::with_clock_and_auth(limit, clock, Auth::default())
    }
    #[cfg(test)]
    pub(crate) fn with_auth(limit: usize, auth: Auth) -> Self {
        Self::with_clock_and_auth(limit, Arc::new(SystemClock), auth)
    }
    #[cfg(test)]
    pub(crate) fn with_clock_and_auth(limit: usize, clock: Arc<dyn Clock>, auth: Auth) -> Self {
        Self::with_services(Arc::new(ServerServices::with_clock_and_auth(
            limit, clock, auth,
        )))
    }
    pub(crate) fn services(&self) -> Arc<ServerServices> {
        Arc::clone(&self.services)
    }
    /// Scratch executor over a private, already-cloned key set. Used by the
    /// multi-owner router to run one command on a fenced snapshot.
    pub(crate) fn with_data(services: Arc<ServerServices>, data: Data) -> Self {
        #[cfg(test)]
        let owner_thread = std::thread::current().id();
        Self {
            notification_epoch: std::cell::Cell::new(
                services.notification_epoch.load(Ordering::Acquire),
            ),
            config_epoch: std::cell::Cell::new(u64::MAX),
            effects_enabled: std::cell::Cell::new(false),
            services,
            #[cfg(test)]
            owner_thread,
            data: std::cell::RefCell::new(data),
            published_payload: std::cell::Cell::new(0),
            _local: std::marker::PhantomData,
        }
    }

    pub(crate) fn with_services(services: Arc<ServerServices>) -> Self {
        #[cfg(test)]
        let owner_thread = std::thread::current().id();
        #[cfg(test)]
        services
            .owner_events
            .lock()
            .unwrap()
            .push(("construct", owner_thread));
        let mut data = Data::default();
        data.eviction = services.eviction_policy();
        data.set_collect_events(services.notifications_enabled());
        Self {
            notification_epoch: std::cell::Cell::new(
                services.notification_epoch.load(Ordering::Acquire),
            ),
            config_epoch: std::cell::Cell::new(u64::MAX),
            effects_enabled: std::cell::Cell::new(false),
            services,
            #[cfg(test)]
            owner_thread,
            data: std::cell::RefCell::new(data),
            published_payload: std::cell::Cell::new(0),
            _local: std::marker::PhantomData,
        }
    }

    fn refresh_data_config(&self) {
        let mut data = self.data.borrow_mut();
        let epoch = self.services.config_epoch.load(Ordering::Acquire);
        self.config_epoch.set(epoch);
        let epoch = self.services.notification_epoch.load(Ordering::Acquire);
        if self.notification_epoch.replace(epoch) != epoch {
            data.clear_lifecycle();
        }
        data.eviction = self.eviction_policy();
        let collect = self.journal.is_some() || self.notifications_enabled();
        self.effects_enabled.set(collect);
        data.set_collect_events(collect);
        // A single local owner enforces the whole budget itself. A multi-owner
        // worker admits against shared credit instead; a reduction then blocks
        // growth and is reclaimed by the router's global budget path.
        if !self.services.multi_owner.load(Ordering::Acquire) {
            let limit = self.max_payload_bytes.load(Ordering::Relaxed);
            if limit > 0 && data.payload_bytes > limit {
                data.enforce_budget(limit);
            }
        }
    }

    /// Publish this owner's payload contribution into the shared credit total.
    /// Called after every owner turn and scoped control operation.
    pub(crate) fn sync_payload_credit(&self) {
        if !self.services.multi_owner.load(Ordering::Acquire) {
            return;
        }
        let mine = self
            .data
            .try_borrow()
            .map(|data| data.payload_bytes)
            .unwrap_or(0);
        let previous = self.published_payload.replace(mine);
        if mine > previous {
            self.services
                .payload_committed
                .fetch_add(mine - previous, Ordering::AcqRel);
        } else if previous > mine {
            self.services
                .payload_committed
                .fetch_sub(previous - mine, Ordering::AcqRel);
        }
    }

    /// Local admission maximum: this owner's current payload plus the free part
    /// of the shared budget. One owner can never spend another owner's space.
    #[allow(dead_code)] // test-only today
    pub(crate) fn payload_limit(&self) -> usize {
        let limit = self.max_payload_bytes.load(Ordering::Relaxed);
        if limit == 0 || !self.services.multi_owner.load(Ordering::Acquire) {
            return limit;
        }
        let mine = self
            .data
            .try_borrow()
            .map(|data| data.payload_bytes)
            .unwrap_or(0);
        let committed = self.services.payload_committed.load(Ordering::Acquire);
        mine.saturating_add(limit.saturating_sub(committed.min(limit)))
    }

    pub fn purge_expired(&self, limit: usize) -> usize {
        self.refresh_data_config();
        let started = self.latency_enabled().then(std::time::Instant::now);

        let (count, expired) = {
            let Ok(mut data) = self.data.try_borrow_mut() else {
                return 0;
            };
            let count = data.reap(self.clock.now_ms().max(0), limit);
            let (expired, _) = data.drain_lifecycle();
            (count, expired)
        };
        for key in expired {
            self.notify_event("expired", &key, NotifyCategory::Expired);
        }
        if let Some(started) = started {
            self.record_expire_latency(started.elapsed());
        }
        count
    }

    /// Append exactly one reply in a synchronous owner turn. No data borrow
    /// survives socket I/O. The compatibility owner preserves global journal
    /// ordering through mutation, capture and append before its next turn.
    pub fn execute(&self, arguments: &[&[u8]], output: &mut Vec<u8>) -> Action {
        let epoch = self.services.config_epoch.load(Ordering::Acquire);
        if self.config_epoch.replace(epoch) != epoch {
            self.refresh_data_config();
        }
        if self.effects_enabled.get() {
            let marks = self.journal_marks();
            let start = output.len();
            let action = self.execute_command(arguments, output);
            self.commit_journal(marks, arguments, output, start);
            self.publish_lifecycle_events();
            return action;
        }

        let action = self.execute_command(arguments, output);
        self.publish_lifecycle_events();
        action
    }

    fn publish_lifecycle_events(&self) {
        if !self.effects_enabled.get() {
            return;
        }
        let flags = self.notify_flags();
        // No lifecycle classes means there is nothing to deliver. Avoid taking
        // a mutable data borrow on every command just to discard queued keys.
        if !flags.expired && !flags.evicted {
            return;
        }
        let (expired, evicted) = {
            let Ok(mut data) = self.data.try_borrow_mut() else {
                return;
            };
            data.drain_lifecycle()
        };
        if flags.expired {
            for key in expired {
                self.notify_event("expired", &key, NotifyCategory::Expired);
            }
        }
        if flags.evicted {
            for key in evicted {
                self.notify_event("evicted", &key, NotifyCategory::Evicted);
            }
        }
    }

    /// Dispatch inside an admitted owner turn. EXEC and journal replay use
    /// the same synchronous implementation without routing constituent commands.
    pub(super) fn execute_command(&self, arguments: &[&[u8]], output: &mut Vec<u8>) -> Action {
        // PING is the hottest command and stays ahead of all family routing.
        if let Some(command) = arguments.first() {
            if command.eq_ignore_ascii_case(b"PING") {
                match arguments.len() {
                    1 => output.extend_from_slice(b"+PONG\r\n"),
                    2 => encode_bulk(Some(arguments[1]), output),
                    _ => arity("ping", output),
                }
                return Action::Continue;
            }
            // GET/SET/ECHO are the measured hot path. Keep the single string
            // implementation, but call its small inlined entry directly so the
            // one-CPU pipeline-1 path does not pay for a family router call.
            if let Some(action) = dispatch::hot_string(self, command, arguments, output) {
                return action;
            }
        }
        dispatch::route(self, arguments, output)
    }

    pub fn stats(&self) -> Stats {
        let dataset = self.dataset_stats();
        self.stats_from_counters((
            dataset.expired_keys,
            dataset.expired_subkeys,
            dataset.evicted_keys,
        ))
    }

    /// Owner-local dataset accounting, ignoring any published aggregate.
    pub(crate) fn local_dataset_stats(&self) -> DatasetStats {
        let Ok(data) = self.data.try_borrow() else {
            return DatasetStats::default();
        };
        let now = self.clock.now_ms().max(0);
        let mut stats = DatasetStats {
            payload_bytes: data.payload_bytes,
            peak_payload_bytes: data.peak_payload_bytes(),
            expired_keys: data.expired_keys,
            expired_subkeys: data.expired_subkeys,
            evicted_keys: data.evicted_keys,
            ..DatasetStats::default()
        };
        for entry in data.entries.values().filter(|entry| entry.alive(now)) {
            stats.live_keys += 1;
            stats.subexpiry_keys += usize::from(!entry.field_ttl.is_empty());
            if let Some(deadline) = entry.expires {
                stats.expiring_keys += 1;
                stats.ttl_count += 1;
                stats.ttl_sum = stats.ttl_sum.saturating_add(
                    u64::try_from(deadline.saturating_sub(now).max(0)).unwrap_or(0),
                );
            }
        }
        stats
    }

    /// Accounting visible to INFO/MEMORY/STATS. A multi-owner router publishes a
    /// coherent aggregate for the duration of one informational command.
    pub(crate) fn dataset_stats(&self) -> DatasetStats {
        if let Ok(guard) = self.dataset_override.lock()
            && let Some(stats) = *guard
        {
            return stats;
        }
        self.local_dataset_stats()
    }

    pub fn reset_stats(&self) -> bool {
        if let Ok(mut stats) = self.error_stats.write() {
            stats.clear();
        }
        self.connections_received.store(0, Ordering::Relaxed);
        for counter in &self.commands_processed {
            counter.store(0, Ordering::Relaxed);
        }
        self.rejected_connections.store(0, Ordering::Relaxed);
        self.reset_command_statistics();
        self.data.try_borrow_mut().is_ok_and(|mut data| {
            data.expired_keys = 0;
            data.expired_subkeys = 0;
            data.evicted_keys = 0;
            true
        })
    }

    pub(crate) fn set_aof_policy(&self, policy: FsyncPolicy) -> std::io::Result<()> {
        if let Some(journal) = self.journal.as_ref() {
            let mut journal = journal
                .lock()
                .map_err(|_| std::io::Error::other("poisoned journal lock"))?;
            journal.set_policy(policy)?;
        }
        let mut config = self
            .aof_config
            .write()
            .map_err(|_| std::io::Error::other("poisoned AOF configuration lock"))?;
        config.policy = policy;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn journal_policy(&self) -> Option<FsyncPolicy> {
        self.journal
            .as_ref()
            .and_then(|journal| journal.lock().ok())
            .map(|journal| journal.policy())
    }

    /// Change the payload budget and evict down to it using the active policy.
    /// A noeviction or volatile policy may leave the store above the new budget;
    /// subsequent admissions then fail rather than deleting ineligible keys.
    pub fn set_payload_limit(&self, limit: usize) -> bool {
        self.refresh_data_config();
        self.max_payload_bytes.store(limit, Ordering::Relaxed);
        self.bump_config();
        self.data.try_borrow_mut().is_ok_and(|mut data| {
            data.enforce_budget(limit);
            true
        })
    }
}

fn arity(command: &str, output: &mut Vec<u8>) {
    write!(
        output,
        "-ERR wrong number of arguments for '{command}' command\r\n"
    )
    .expect("Vec write");
}
fn integer(value: i64, output: &mut Vec<u8>) {
    write!(output, ":{value}\r\n").expect("Vec write");
}
fn internal_error(output: &mut Vec<u8>) {
    output.extend_from_slice(b"-ERR internal store error\r\n");
}
fn wrong_type(output: &mut Vec<u8>) {
    output.extend_from_slice(
        b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
    );
}

#[cfg(test)]
#[path = "engine/conditional_tests.rs"]
mod conditional_tests;
#[cfg(test)]
#[path = "engine/eviction_tests.rs"]
mod eviction_tests;
#[cfg(test)]
#[path = "engine/expiry_tests.rs"]
mod expiry_tests;
#[cfg(test)]
#[path = "engine/base_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "engine/transaction_tests.rs"]
mod transaction_tests;
