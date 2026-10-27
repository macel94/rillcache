//! Atomic in-memory string commands with expiration and a key/value payload budget.

use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::protocol::{Limits, encode_bulk};
mod acl_log;
mod administration;
mod auth;
mod bitfield;
mod blocking;
mod clients;
mod collection_commands;
mod collection_config;
mod collections;
mod commands;
mod data;
mod discovery;
mod error_dialect;
pub(crate) use discovery::is_write_command;
mod eviction;
mod geo;
mod hll;
mod introspection;
mod journal;
mod keyspace;
mod latency;
mod multikey;
mod notify;
mod options;
mod pubsub;
mod scan;
mod sha256;
mod slowlog;
mod sort;
mod streams;
mod string_algorithms;
mod string_ranges;
mod transaction;
mod unsupported;
mod xxh3;
pub use acl_log::AclLogEntry;
pub use auth::{Access, AclError, Auth, AuthError, UserDetails};
pub use blocking::PopOutcome;
pub use blocking::reply_value as blocking_reply;
pub use clients::ClientSnapshot;
pub(crate) use collection_commands::{
    ListMoveOutcome, ListPopOutcome, ZSetPopOutcome, list_move_reply,
    list_pop_reply as blocking_lmpop_reply, parse_bzmpop_timeout, parse_list_mpop, parse_mpop,
    parse_side, zset_pop_reply as blocking_zpop_reply,
};
use collection_config::CollectionConfig;
use data::Data;
pub(crate) use data::StreamId;
pub use error_dialect::ErrorDialect;
pub use eviction::EvictionPolicy;
pub use journal::FsyncPolicy;
pub use notify::{Category as NotifyCategory, NotifyFlags};
use options::{Condition, ExpiryKind, SetOptions};
pub(crate) use streams::{
    XReadGroupOptions, XReadGroupOutcome, XReadGroupStart, XReadOutcome, parse_xread,
    parse_xreadgroup, xread_reply, xreadgroup_reply,
};
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

#[derive(Clone, Copy, Debug)]
pub struct Stats {
    pub total_connections_received: u64,
    pub total_commands_processed: u64,
    pub rejected_connections: u64,
    pub expired_keys: u64,
    pub expired_subkeys: u64,
    pub evicted_keys: u64,
    pub pubsub_channels: usize,
    pub pubsub_patterns: usize,
}

pub struct Store {
    data: RwLock<Data>,
    max_payload_bytes: usize,
    error_dialect: ErrorDialect,
    collection_config: RwLock<CollectionConfig>,
    clock: Arc<dyn Clock>,
    transaction_gate: RwLock<()>,
    auth: RwLock<Auth>,
    auth_required: AtomicBool,
    acl_restricted: AtomicBool,
    journal: Option<std::sync::Mutex<journal::Journal>>,
    journal_limits: Limits,
    pubsub: RwLock<pubsub::PubSub>,
    clients: RwLock<std::collections::BTreeMap<u64, clients::ClientRecord>>,
    notifications: std::sync::atomic::AtomicU32,
    acl_log: RwLock<std::collections::VecDeque<acl_log::AclLogEntry>>,
    connections_received: std::sync::atomic::AtomicU64,
    commands_processed: std::sync::atomic::AtomicU64,
    rejected_connections: std::sync::atomic::AtomicU64,
    slowlog: std::sync::RwLock<std::collections::VecDeque<slowlog::SlowlogEntry>>,
    slowlog_next_id: std::sync::atomic::AtomicU64,
    slowlog_threshold_us: std::sync::atomic::AtomicI64,
    slowlog_max_len: std::sync::atomic::AtomicUsize,
    client_pause: std::sync::atomic::AtomicU64,
    max_clients: std::sync::atomic::AtomicUsize,
    latency_monitor_threshold_ms: std::sync::atomic::AtomicU64,
    latency: std::sync::RwLock<latency::State>,
    auto_aof_rewrite_percentage: std::sync::atomic::AtomicU64,
    auto_aof_rewrite_min_size: std::sync::atomic::AtomicU64,
    aof_config: RwLock<journal::AofConfig>,
    error_stats: RwLock<std::collections::BTreeMap<Vec<u8>, u64>>,
    shutdown_requested: AtomicBool,
}
impl Store {
    pub fn new(max_payload_bytes: usize) -> Self {
        Self::with_clock(max_payload_bytes, Arc::new(SystemClock))
    }
    pub fn with_clock(max_payload_bytes: usize, clock: Arc<dyn Clock>) -> Self {
        Self::with_clock_and_auth(max_payload_bytes, clock, Auth::default())
    }
    /// Server construction with the configured default-user credential.
    pub fn with_auth(max_payload_bytes: usize, auth: Auth) -> Self {
        Self::with_clock_and_auth(max_payload_bytes, Arc::new(SystemClock), auth)
    }
    /// Server construction with an append-only journal. The file is replayed
    /// into a fresh store before the journal is attached for new writes.
    pub fn with_auth_and_journal(
        max_payload_bytes: usize,
        auth: Auth,
        path: &std::path::Path,
        policy: FsyncPolicy,
        limits: Limits,
    ) -> std::io::Result<Self> {
        let mut store = Self::with_auth(max_payload_bytes, auth);
        let mut journal = journal::Journal::open(path, policy, limits)?;
        journal.replay(&store)?;
        store.attach_journal(journal);
        let filename = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("appendonly.aof");
        store.set_aof_config(true, filename, policy);
        Ok(store)
    }

    pub fn with_clock_and_auth(
        max_payload_bytes: usize,
        clock: Arc<dyn Clock>,
        auth: Auth,
    ) -> Self {
        Self {
            data: RwLock::new(Data::default()),
            max_payload_bytes,
            error_dialect: ErrorDialect::default(),
            collection_config: RwLock::new(CollectionConfig::default()),
            clock,
            transaction_gate: RwLock::new(()),
            auth_required: AtomicBool::new(auth.required()),
            acl_restricted: AtomicBool::new(auth.restricted()),
            auth: RwLock::new(auth),
            journal: None,
            journal_limits: Limits::default(),
            pubsub: RwLock::new(pubsub::PubSub::default()),
            clients: RwLock::new(std::collections::BTreeMap::new()),
            notifications: std::sync::atomic::AtomicU32::new(0),
            acl_log: RwLock::new(std::collections::VecDeque::new()),
            connections_received: std::sync::atomic::AtomicU64::new(0),
            commands_processed: std::sync::atomic::AtomicU64::new(0),
            rejected_connections: std::sync::atomic::AtomicU64::new(0),
            slowlog: std::sync::RwLock::new(std::collections::VecDeque::new()),
            slowlog_next_id: std::sync::atomic::AtomicU64::new(0),
            slowlog_threshold_us: std::sync::atomic::AtomicI64::new(slowlog::DEFAULT_THRESHOLD_US),
            slowlog_max_len: std::sync::atomic::AtomicUsize::new(slowlog::DEFAULT_MAX_LEN),
            client_pause: std::sync::atomic::AtomicU64::new(0),
            max_clients: std::sync::atomic::AtomicUsize::new(128),
            latency_monitor_threshold_ms: std::sync::atomic::AtomicU64::new(
                latency::DEFAULT_THRESHOLD_MS,
            ),
            latency: std::sync::RwLock::new(latency::State::default()),
            auto_aof_rewrite_percentage: std::sync::atomic::AtomicU64::new(100),
            auto_aof_rewrite_min_size: std::sync::atomic::AtomicU64::new(64 * 1024 * 1024),
            aof_config: RwLock::new(journal::AofConfig::default()),
            error_stats: RwLock::new(std::collections::BTreeMap::new()),
            shutdown_requested: AtomicBool::new(false),
        }
    }
    /// Select verified vendor-specific error wording before sharing the store.
    /// Currently only MSETEX's short counted key/value block differs.
    pub fn with_error_dialect(mut self, dialect: ErrorDialect) -> Self {
        self.error_dialect = dialect;
        self
    }

    /// Bounded best-effort background reclamation; no sleep-dependent correctness.
    /// Every lookup still treats logically expired entries as absent immediately.
    pub(crate) fn request_shutdown(&self) {
        self.shutdown_requested.store(true, Ordering::Release);
    }

    pub(crate) fn shutdown_requested(&self) -> bool {
        self.shutdown_requested.load(Ordering::Acquire)
    }

    pub fn purge_expired(&self, limit: usize) -> usize {
        let started = self.latency_enabled().then(std::time::Instant::now);
        let Ok(_gate) = self.transaction_gate.read() else {
            return 0;
        };
        let (count, expired) = {
            let Ok(mut data) = self.data.try_write() else {
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

    /// Append exactly one reply. No store lock is held during socket I/O.
    /// With an append-only journal attached, commands take the exclusive
    /// transaction gate so mutation, capture and append keep one global order.
    pub fn execute(&self, arguments: &[&[u8]], output: &mut Vec<u8>) -> Action {
        if self.journal.is_some() || self.notifications_enabled() {
            let Ok(_gate) = self.transaction_gate.write() else {
                internal_error(output);
                return Action::Continue;
            };
            let marks = self.journal_marks();
            let start = output.len();
            let action = self.execute_locked(arguments, output);
            self.commit_journal(marks, arguments, output, start);
            self.publish_lifecycle_events();
            return action;
        }
        let Ok(_gate) = self.transaction_gate.read() else {
            internal_error(output);
            return Action::Continue;
        };
        let action = self.execute_locked(arguments, output);
        self.publish_lifecycle_events();
        action
    }

    fn publish_lifecycle_events(&self) {
        let flags = self.notify_flags();
        // No lifecycle classes means there is nothing to deliver. Avoid taking
        // the store write lock on every command just to discard queued keys.
        if !flags.expired && !flags.evicted {
            return;
        }
        let (expired, evicted) = {
            let Ok(mut data) = self.data.write() else {
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

    /// Dispatch without the transaction gate; only `EXEC` may call this while
    /// holding the gate's write side.
    pub(super) fn execute_locked(&self, arguments: &[&[u8]], output: &mut Vec<u8>) -> Action {
        if self.execute_collection_config(arguments, output)
            || self.execute_admin(arguments, output)
            || self.execute_scan(arguments, output)
            || self.execute_discovery(arguments, output)
            || self.execute_multikey(arguments, output)
            || self.execute_streams(arguments, output)
        {
            return Action::Continue;
        }
        let Some(command) = arguments.first() else {
            output.extend_from_slice(b"-ERR empty command\r\n");
            return Action::Continue;
        };
        if command.eq_ignore_ascii_case(b"PING") {
            match arguments.len() {
                1 => output.extend_from_slice(b"+PONG\r\n"),
                2 => encode_bulk(Some(arguments[1]), output),
                _ => arity("ping", output),
            }
        } else if command.eq_ignore_ascii_case(b"GET") {
            if arguments.len() != 2 {
                arity("get", output);
            } else {
                self.get(arguments[1], output);
            }
        } else if command.eq_ignore_ascii_case(b"SET") {
            if arguments.len() < 3 {
                arity("set", output);
            } else {
                match SetOptions::parse(&arguments[3..], false) {
                    Ok(options) => {
                        self.set(arguments[1], arguments[2], options, "set", false, output)
                    }
                    Err(error) => output.extend_from_slice(error),
                }
            }
        } else if command.eq_ignore_ascii_case(b"ECHO") {
            if arguments.len() == 2 {
                encode_bulk(Some(arguments[1]), output);
            } else {
                arity("echo", output);
            }
        } else if command.eq_ignore_ascii_case(b"QUIT") {
            if arguments.len() == 1 {
                output.extend_from_slice(b"+OK\r\n");
                return Action::Close;
            }
            arity("quit", output);
        } else if command.eq_ignore_ascii_case(b"DELEX") {
            self.delex(arguments, false, output);
        } else if command.eq_ignore_ascii_case(b"DELIFEQ") {
            self.delex(arguments, true, output);
        } else if command.eq_ignore_ascii_case(b"DIGEST") {
            if arguments.len() == 2 {
                self.digest(arguments[1], output);
            } else {
                arity("digest", output);
            }
        } else if command.eq_ignore_ascii_case(b"DEL") || command.eq_ignore_ascii_case(b"UNLINK") {
            if arguments.len() < 2 {
                arity(
                    if command.eq_ignore_ascii_case(b"UNLINK") {
                        "unlink"
                    } else {
                        "del"
                    },
                    output,
                );
            } else if let Ok(mut data) = self.data.write() {
                let now = self.now(&data);
                let mut count = 0;
                for key in &arguments[1..] {
                    data.expire_key(key, now);
                    count += i64::from(data.remove(key).is_some());
                }
                integer(count, output);
            } else {
                internal_error(output);
            }
        } else if command.eq_ignore_ascii_case(b"EXISTS") {
            if arguments.len() < 2 {
                arity("exists", output);
            } else {
                self.exists(&arguments[1..], output);
            }
        } else if command.eq_ignore_ascii_case(b"GETEX") {
            if arguments.len() < 2 {
                arity("getex", output);
            } else {
                match SetOptions::parse(&arguments[2..], true) {
                    Ok(options) => self.getex(arguments[1], options, output),
                    Err(error) => output.extend_from_slice(error),
                }
            }
        } else if command.eq_ignore_ascii_case(b"GETDEL")
            || command.eq_ignore_ascii_case(b"PERSIST")
        {
            let name = if command.eq_ignore_ascii_case(b"GETDEL") {
                "getdel"
            } else {
                "persist"
            };
            if arguments.len() != 2 {
                arity(name, output);
            } else if name == "getdel" {
                self.getdel(arguments[1], output);
            } else {
                self.persist(arguments[1], output);
            }
        } else if let Some((_, name, kind)) = [
            (b"SETEX".as_slice(), "setex", ExpiryKind::Ex),
            (b"PSETEX".as_slice(), "psetex", ExpiryKind::Px),
        ]
        .into_iter()
        .find(|(token, _, _)| command.eq_ignore_ascii_case(token))
        {
            if arguments.len() != 4 {
                arity(name, output);
            } else {
                self.set(
                    arguments[1],
                    arguments[3],
                    SetOptions {
                        expiry: Some((kind, arguments[2])),
                        ..SetOptions::default()
                    },
                    name,
                    false,
                    output,
                );
            }
        } else if command.eq_ignore_ascii_case(b"SETNX") || command.eq_ignore_ascii_case(b"GETSET")
        {
            let nx = command.eq_ignore_ascii_case(b"SETNX");
            let name = if nx { "setnx" } else { "getset" };
            if arguments.len() != 3 {
                arity(name, output);
            } else {
                self.set(
                    arguments[1],
                    arguments[2],
                    SetOptions {
                        condition: if nx {
                            Condition::Missing
                        } else {
                            Condition::Always
                        },
                        get: !nx,
                        ..SetOptions::default()
                    },
                    name,
                    nx,
                    output,
                );
            }
        } else if let Some((_, name, ms, absolute)) = [
            (b"TTL".as_slice(), "ttl", false, false),
            (b"PTTL".as_slice(), "pttl", true, false),
            (b"EXPIRETIME".as_slice(), "expiretime", false, true),
            (b"PEXPIRETIME".as_slice(), "pexpiretime", true, true),
        ]
        .into_iter()
        .find(|(token, _, _, _)| command.eq_ignore_ascii_case(token))
        {
            if arguments.len() != 2 {
                arity(name, output);
            } else {
                self.ttl(arguments[1], ms, absolute, output);
            }
        } else if let Some((_, name, seconds, absolute)) = [
            (b"EXPIRE".as_slice(), "expire", true, false),
            (b"PEXPIRE".as_slice(), "pexpire", false, false),
            (b"EXPIREAT".as_slice(), "expireat", true, true),
            (b"PEXPIREAT".as_slice(), "pexpireat", false, true),
        ]
        .into_iter()
        .find(|(token, _, _, _)| command.eq_ignore_ascii_case(token))
        {
            self.expire(arguments, name, seconds, absolute, output);
        } else if let Some((_, name)) = [
            (b"INCR".as_slice(), "incr"),
            (b"DECR".as_slice(), "decr"),
            (b"INCRBY".as_slice(), "incrby"),
            (b"DECRBY".as_slice(), "decrby"),
        ]
        .into_iter()
        .find(|(token, _)| command.eq_ignore_ascii_case(token))
        {
            self.increment(arguments, name, output);
        } else if !self.execute_keyspace(arguments, output)
            && !self.execute_string_ranges(arguments, output)
            && !self.execute_bitfield(arguments, output)
            && !self.execute_geo(arguments, output)
            && !self.execute_sort(arguments, output)
            && !self.execute_string_algorithms(arguments, output)
            && !self.execute_hll(arguments, output)
            && !self.execute_collection_commands(arguments, output)
            && !self.execute_unsupported(arguments, output)
            && !self.execute_collections(arguments, output)
        {
            output.extend_from_slice(b"-ERR unknown command\r\n");
        }
        Action::Continue
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

    pub fn record_command(&self) {
        self.commands_processed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn stats(&self) -> Stats {
        let (expired_keys, expired_subkeys, evicted_keys, pubsub_channels, pubsub_patterns) = {
            let (expired, subkeys, evicted) = self.data.read().map_or((0, 0, 0), |data| {
                (data.expired_keys, data.expired_subkeys, data.evicted_keys)
            });
            let (channels, patterns) = self.pubsub.read().map_or((0, 0), |pubsub| {
                (pubsub.channel_names(None).len(), pubsub.numpat())
            });
            (expired, subkeys, evicted, channels, patterns)
        };
        Stats {
            total_connections_received: self.connections_received.load(Ordering::Relaxed),
            total_commands_processed: self.commands_processed.load(Ordering::Relaxed),
            rejected_connections: self.rejected_connections.load(Ordering::Relaxed),
            expired_keys,
            expired_subkeys,
            evicted_keys,
            pubsub_channels,
            pubsub_patterns,
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

    pub fn reset_stats(&self) -> bool {
        if let Ok(mut stats) = self.error_stats.write() {
            stats.clear();
        }
        self.connections_received.store(0, Ordering::Relaxed);
        self.commands_processed.store(0, Ordering::Relaxed);
        self.rejected_connections.store(0, Ordering::Relaxed);
        self.data.write().is_ok_and(|mut data| {
            data.expired_keys = 0;
            data.expired_subkeys = 0;
            data.evicted_keys = 0;
            true
        })
    }

    pub fn set_max_clients(&self, value: usize) {
        self.max_clients.store(value, Ordering::Relaxed);
    }

    pub fn max_clients(&self) -> usize {
        self.max_clients.load(Ordering::Relaxed)
    }

    pub fn set_eviction_policy(&self, policy: EvictionPolicy) -> bool {
        self.data.write().is_ok_and(|mut data| {
            data.eviction = policy;
            true
        })
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
        self.aof_config.write().is_ok_and(|mut config| {
            config.appendonly = appendonly;
            config.appendfilename = appendfilename.to_string();
            config.policy = policy;
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

    pub fn eviction_policy(&self) -> EvictionPolicy {
        self.data
            .read()
            .map_or(EvictionPolicy::NoEviction, |data| data.eviction)
    }

    pub fn set_notify_flags(&self, flags: NotifyFlags) {
        // Events queued while a class was disabled are stale for delivery.
        if let Ok(mut data) = self.data.write() {
            data.clear_lifecycle();
        }
        self.notifications.store(flags.bits(), Ordering::Release);
    }

    pub fn notify_flags(&self) -> NotifyFlags {
        NotifyFlags::from_bits(self.notifications.load(Ordering::Acquire))
    }

    pub fn notifications_enabled(&self) -> bool {
        self.notifications.load(Ordering::Acquire) != 0
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

    pub fn acl_commands(category: &[u8]) -> Vec<&'static str> {
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
