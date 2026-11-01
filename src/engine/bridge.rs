//! Synchronous compatibility boundary. Requests contain operations, never Data.
//!
//! Local socket execution uses `ShardExecutor` directly. Only external callers
//! enter this bounded mailbox; the owner never waits for a caller to return data.

use super::*;
use std::net::{SocketAddr, TcpStream};
use std::sync::{OnceLock, mpsc};

#[cfg(test)]
mod tests;

pub(crate) type Request = Box<dyn FnOnce(&ShardExecutor) + Send>;

pub(crate) struct BridgeInbox(mpsc::Receiver<Request>);

impl BridgeInbox {
    pub(crate) fn drain<E: Execution>(&self, executor: &E) {
        for _ in 0..64 {
            let Ok(request) = self.0.try_recv() else {
                break;
            };
            executor.run_request(request);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BridgeError {
    Admission,
    Start,
    Stopped,
}
impl std::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Store owner unavailable: {self:?}")
    }
}
impl std::error::Error for BridgeError {}
pub(crate) trait BridgeFallback {
    fn owner_failed(error: BridgeError) -> Self;
}
macro_rules! fallback {
    ($ty:ty, $value:expr) => {
        impl BridgeFallback for $ty {
            fn owner_failed(_: BridgeError) -> Self {
                $value
            }
        }
    };
}
fallback!((), ());
fallback!(bool, false);
fallback!(usize, 0);
fallback!(Stats, Stats::default());
fallback!(PopOutcome, PopOutcome::JournalError);
fallback!(ListPopOutcome, ListPopOutcome::JournalError);
fallback!(ListMoveOutcome, ListMoveOutcome::JournalError);
fallback!(ZSetPopOutcome, ZSetPopOutcome::JournalError);
fallback!(XReadGroupOutcome, XReadGroupOutcome::JournalError);
fallback!(Vec<u8>, b"-ERR internal store error\r\n".to_vec());
fallback!(Vec<(Vec<u8>, super::data::Entry)>, Vec::new());
fallback!(
    (Action, Vec<u8>),
    (Action::Continue, b"-ERR internal store error\r\n".to_vec())
);
fallback!(
    (TransactionOutcome, Vec<u8>),
    (
        TransactionOutcome::InternalError,
        b"-ERR internal store error\r\n".to_vec()
    )
);
impl<T> BridgeFallback for Option<T> {
    fn owner_failed(_: BridgeError) -> Self {
        None
    }
}
impl<T> BridgeFallback for std::io::Result<T> {
    fn owner_failed(error: BridgeError) -> Self {
        Err(std::io::Error::other(error))
    }
}
impl BridgeFallback for Result<(u64, u64), &'static [u8]> {
    fn owner_failed(_: BridgeError) -> Self {
        Err(b"-ERR internal store error\r\n")
    }
}

struct OwnerFailure(Arc<ServerServices>);
impl Drop for OwnerFailure {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.0.owner_failed.store(true, Ordering::Release);
            self.0.request_shutdown();
        }
    }
}

struct Owner {
    sender: Option<mpsc::SyncSender<Request>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Owner {
    fn drop(&mut self) {
        // Closing admission lets the owner finish the admitted operation, then
        // drop its local Data on that same thread. No detached worker survives.
        drop(self.sender.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Thread-safe external API. Data is constructed and remains on its executor.
pub struct Store {
    services: Arc<ServerServices>,
    owner: OnceLock<Result<Owner, BridgeError>>,
    // Serial admission bounds retained requests and replies to one external
    // operation; local network commands never acquire this mutex.
    admission: std::sync::Mutex<()>,
}

impl std::ops::Deref for Store {
    type Target = ServerServices;
    fn deref(&self) -> &ServerServices {
        &self.services
    }
}

impl Store {
    /// Shared control plane handle: auth, clients, pub/sub and the journal stay
    /// common while data execution moves to owners.
    pub(crate) fn services_handle(&self) -> Arc<ServerServices> {
        Arc::clone(&self.services)
    }

    /// Move replayed append-only state into the owners before they serve. The
    /// journal itself is untouched and still rewrites one coherent snapshot.
    pub(crate) fn take_initial_entries(&self) -> Vec<(Vec<u8>, super::data::Entry)> {
        self.call(|executor| {
            let data = executor.data.borrow();
            let mut keys: Vec<Vec<u8>> = data.entries.keys().cloned().collect();
            keys.sort_unstable();
            keys.into_iter()
                .filter_map(|key| data.gather_entry(&key).map(|entry| (key, entry)))
                .collect()
        })
    }

    #[cfg(test)]
    pub(crate) fn test_services(&self) -> Arc<ServerServices> {
        Arc::clone(&self.services)
    }
    pub fn new(max_payload_bytes: usize) -> Self {
        Self::with_clock(max_payload_bytes, Arc::new(SystemClock))
    }
    pub fn with_clock(max_payload_bytes: usize, clock: Arc<dyn Clock>) -> Self {
        Self::with_clock_and_auth(max_payload_bytes, clock, Auth::default())
    }
    pub fn with_auth(max_payload_bytes: usize, auth: Auth) -> Self {
        Self::with_clock_and_auth(max_payload_bytes, Arc::new(SystemClock), auth)
    }
    pub fn with_clock_and_auth(
        max_payload_bytes: usize,
        clock: Arc<dyn Clock>,
        auth: Auth,
    ) -> Self {
        Self {
            services: Arc::new(ServerServices::with_clock_and_auth(
                max_payload_bytes,
                clock,
                auth,
            )),
            owner: OnceLock::new(),
            admission: std::sync::Mutex::new(()),
        }
    }
    pub fn with_auth_and_journal(
        max_payload_bytes: usize,
        auth: Auth,
        path: &Path,
        policy: FsyncPolicy,
        limits: Limits,
    ) -> std::io::Result<Self> {
        let mut services =
            ServerServices::with_clock_and_auth(max_payload_bytes, Arc::new(SystemClock), auth);
        let journal = journal::Journal::open(path, policy, limits)?;
        services.journal_limits = limits;
        services.journal = Some(std::sync::Mutex::new(journal));
        let store = Self {
            services: Arc::new(services),
            owner: OnceLock::new(),
            admission: std::sync::Mutex::new(()),
        };
        // Replay is a complete owner operation and finishes before construction
        // returns. New requests cannot observe partially restored state.
        store.call(|executor| {
            executor
                .journal
                .as_ref()
                .expect("attached journal")
                .lock()
                .map_err(|_| std::io::Error::other("poisoned journal"))?
                .replay(executor)
        })?;
        let filename = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("appendonly.aof");
        store.set_aof_config(true, filename, policy);
        Ok(store)
    }
    pub fn with_error_dialect(self, dialect: ErrorDialect) -> Self {
        *self.error_dialect.write().expect("error dialect lock") = dialect;
        self
    }

    pub(crate) fn ensure_external_owner(&self) {
        self.call(|_| ());
    }

    pub(crate) fn owner_status(&self) -> std::io::Result<()> {
        if self.services.owner_failed.load(Ordering::Acquire) {
            Err(std::io::Error::other("storage owner failed"))
        } else {
            Ok(())
        }
    }

    /// Install the executor on the calling socket worker, before any bridge
    /// request has started an external owner. No Data is moved between threads.
    pub(crate) fn install_local(&self) -> Option<(ShardExecutor, BridgeInbox)> {
        if self.journal.is_some()
            || self.auth.read().ok()?.aclfile().is_some()
            || self.audit_log.lock().ok()?.is_some()
            || self.audit_syslog.lock().ok()?.is_some()
        {
            return None;
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        self.owner
            .set(Ok(Owner {
                sender: Some(sender),
                thread: None,
            }))
            .ok()?;
        Some((
            ShardExecutor::with_services(Arc::clone(&self.services)),
            BridgeInbox(receiver),
        ))
    }

    pub(crate) fn call<T: BridgeFallback + Send + 'static>(
        &self,
        operation: impl FnOnce(&ShardExecutor) -> T + Send + 'static,
    ) -> T {
        self.call_prepared(|| operation)
    }

    fn call_prepared<
        T: BridgeFallback + Send + 'static,
        F: FnOnce(&ShardExecutor) -> T + Send + 'static,
    >(
        &self,
        prepare: impl FnOnce() -> F,
    ) -> T {
        match self.try_submit(prepare) {
            Ok(result) => result,
            Err(error) => {
                // No further data service can safely continue after owner loss.
                self.services.owner_failed.store(true, Ordering::Release);
                self.request_shutdown();
                T::owner_failed(error)
            }
        }
    }

    fn try_submit<T: Send + 'static, F: FnOnce(&ShardExecutor) -> T + Send + 'static>(
        &self,
        prepare: impl FnOnce() -> F,
    ) -> Result<T, BridgeError> {
        let _admission = self.admission.lock().map_err(|_| BridgeError::Admission)?;
        // The copy/ownership boundary is strictly inside admission.
        let operation = prepare();
        let owner = self
            .owner
            .get_or_init(|| {
                let (sender, receiver) = mpsc::sync_channel::<Request>(1);
                let services = Arc::clone(&self.services);
                let thread = std::thread::Builder::new()
                    .name("rillcache-store-bridge".into())
                    .spawn(move || {
                        let _failure = OwnerFailure(Arc::clone(&services));
                        let executor = ShardExecutor::with_services(services);
                        while let Ok(request) = receiver.recv() {
                            request(&executor);
                        }
                    })
                    .map_err(|_| BridgeError::Start)?;
                Ok(Owner {
                    sender: Some(sender),
                    thread: Some(thread),
                })
            })
            .as_ref()
            .map_err(|error| *error)?;
        #[cfg(test)]
        self.services
            .bridge_submissions
            .fetch_add(1, Ordering::Relaxed);
        let (sender, reply) = mpsc::sync_channel(1);
        owner
            .sender
            .as_ref()
            .ok_or(BridgeError::Stopped)?
            .send(Box::new(move |executor| {
                let result = operation(executor);
                let _ = sender.send(result);
            }))
            .map_err(|_| BridgeError::Stopped)?;
        reply.recv().map_err(|_| BridgeError::Stopped)
    }

    pub fn execute(&self, arguments: &[&[u8]], output: &mut Vec<u8>) -> Action {
        let (action, reply) = self.call_prepared(|| {
            let arguments = OwnedArguments::copy(arguments);
            move |executor: &ShardExecutor| {
                let mut reply = Vec::new();
                let action = executor.execute(&arguments.borrowed(), &mut reply);
                (action, reply)
            }
        });
        output.extend_from_slice(&reply);
        action
    }
    pub fn execute_transaction(
        &self,
        commands: &[Vec<Vec<u8>>],
        watches: &WatchState,
        output: &mut Vec<u8>,
    ) -> TransactionOutcome {
        let (result, reply) = self.call_prepared(|| {
            let commands = commands.to_vec();
            let watches = watches.clone();
            move |executor: &ShardExecutor| {
                let mut reply = Vec::new();
                let result = executor.execute_transaction(&commands, &watches, &mut reply);
                (result, reply)
            }
        });
        output.extend_from_slice(&reply);
        result
    }
    pub fn watch_snapshot(&self, keys: &[&[u8]]) -> Option<WatchState> {
        self.call_prepared(|| {
            let keys = OwnedArguments::copy(keys);
            move |executor: &ShardExecutor| executor.watch_snapshot(&keys.borrowed())
        })
    }
    pub fn blocking_pop(&self, keys: &[&[u8]], left: bool) -> PopOutcome {
        self.call_prepared(|| {
            let keys = OwnedArguments::copy(keys);
            move |executor: &ShardExecutor| executor.blocking_pop(&keys.borrowed(), left)
        })
    }
    pub fn purge_expired(&self, limit: usize) -> usize {
        self.call(move |executor| executor.purge_expired(limit))
    }
    pub fn stats(&self) -> Stats {
        self.call(ShardExecutor::stats)
    }
    #[cfg(test)]
    pub(crate) fn journal_policy(&self) -> Option<FsyncPolicy> {
        self.call(ShardExecutor::journal_policy)
    }
    pub fn reset_stats(&self) -> bool {
        self.call(ShardExecutor::reset_stats)
    }
    pub fn set_payload_limit(&self, limit: usize) -> bool {
        self.call(move |executor| executor.set_payload_limit(limit))
    }
    pub fn journal_sync(&self) -> std::io::Result<()> {
        self.call(ShardExecutor::journal_sync)
    }
    pub fn journal_flush(&self) -> std::io::Result<()> {
        self.call(ShardExecutor::journal_flush)
    }
    pub fn auto_rewrite_if_due(&self) -> bool {
        self.call(ShardExecutor::auto_rewrite_if_due)
    }

    pub fn command_arity(name: &[u8]) -> Option<i64> {
        ServerServices::command_arity(name)
    }
    pub fn pubsub_queue_capacity() -> usize {
        ServerServices::pubsub_queue_capacity()
    }
    pub fn acl_categories() -> &'static [&'static str] {
        ServerServices::acl_categories()
    }
    pub fn acl_commands(category: &[u8]) -> Vec<String> {
        ServerServices::acl_commands(category)
    }
}

/// An ordinary single-owner command that can be handed to an owner worker
/// without blocking the socket thread. Only the multi-owner router produces
/// one; every other execution boundary reports `None` from [`Execution::defer`].
#[derive(Debug)]
pub(crate) struct DeferredCommand {
    pub(crate) owner: usize,
    /// Shared with the owner request so slowlog can keep the original arguments
    /// for a late completion without copying the payload.
    pub(crate) arguments: Arc<Vec<Vec<u8>>>,
}

/// The session's complete data boundary. All other session operations use the
/// identical shared services through Deref, including its observation epilogue.
pub(crate) trait Execution: std::ops::Deref<Target = ServerServices> {
    fn services_handle(&self) -> Arc<ServerServices>;
    fn run_request(&self, request: Request);

    /// Whether this command can be executed by a deferred owner batch without
    /// changing the reply's meaning. The default boundary answers `false`.
    fn can_defer(&self, _arguments: &[&[u8]]) -> bool {
        false
    }
    /// Copy a deferrable command so it can run on its owner after the socket
    /// thread has moved on. `None` keeps the synchronous `execute` path.
    fn defer(&self, _arguments: &[&[u8]]) -> Option<DeferredCommand> {
        None
    }

    fn execute(&self, arguments: &[&[u8]], output: &mut Vec<u8>) -> Action;
    fn watch_snapshot(&self, keys: &[&[u8]]) -> Option<WatchState>;
    fn execute_transaction(
        &self,
        commands: &[Vec<Vec<u8>>],
        watches: &WatchState,
        output: &mut Vec<u8>,
    ) -> TransactionOutcome;
    fn blocking_pop(&self, keys: &[&[u8]], left: bool) -> PopOutcome;
    fn blocking_lmpop(&self, keys: &[&[u8]], left: bool, count: usize) -> ListPopOutcome;
    fn blocking_list_move(
        &self,
        source: &[u8],
        destination: &[u8],
        from_left: bool,
        to_left: bool,
    ) -> ListMoveOutcome;
    fn blocking_zpop(&self, keys: &[&[u8]], max: bool, count: usize) -> ZSetPopOutcome;
    fn xread_start(
        &self,
        options: &streams::XReadOptions<'_>,
    ) -> Option<(Vec<StreamId>, XReadOutcome)>;
    fn xread_once(&self, keys: &[&[u8]], starts: &[StreamId], count: usize)
    -> Option<XReadOutcome>;
    fn blocking_xreadgroup(&self, options: &XReadGroupOptions<'_>) -> XReadGroupOutcome;
    fn waitaof_probe(&self, local: i64, replicas: i64) -> Result<(u64, u64), &'static [u8]>;
    fn journal_sync(&self) -> std::io::Result<()>;
    fn purge_expired(&self, limit: usize) -> usize;
    fn auto_rewrite_if_due(&self) -> bool;
    fn shutdown_command(&self, args: &[&[u8]], output: &mut Vec<u8>);
}

impl Execution for ShardExecutor {
    fn services_handle(&self) -> Arc<ServerServices> {
        self.services()
    }
    fn run_request(&self, request: Request) {
        request(self);
    }

    fn journal_sync(&self) -> std::io::Result<()> {
        ShardExecutor::journal_sync(self)
    }
    fn purge_expired(&self, limit: usize) -> usize {
        ShardExecutor::purge_expired(self, limit)
    }
    fn auto_rewrite_if_due(&self) -> bool {
        ShardExecutor::auto_rewrite_if_due(self)
    }
    fn execute(&self, a: &[&[u8]], out: &mut Vec<u8>) -> Action {
        ShardExecutor::execute(self, a, out)
    }
    fn watch_snapshot(&self, keys: &[&[u8]]) -> Option<WatchState> {
        ShardExecutor::watch_snapshot(self, keys)
    }
    fn execute_transaction(
        &self,
        commands: &[Vec<Vec<u8>>],
        watches: &WatchState,
        out: &mut Vec<u8>,
    ) -> TransactionOutcome {
        ShardExecutor::execute_transaction(self, commands, watches, out)
    }
    fn blocking_pop(&self, keys: &[&[u8]], left: bool) -> PopOutcome {
        ShardExecutor::blocking_pop(self, keys, left)
    }
    fn blocking_lmpop(&self, keys: &[&[u8]], left: bool, count: usize) -> ListPopOutcome {
        ShardExecutor::blocking_lmpop(self, keys, left, count)
    }
    fn blocking_list_move(
        &self,
        source: &[u8],
        destination: &[u8],
        from_left: bool,
        to_left: bool,
    ) -> ListMoveOutcome {
        ShardExecutor::blocking_list_move(self, source, destination, from_left, to_left)
    }
    fn blocking_zpop(&self, keys: &[&[u8]], max: bool, count: usize) -> ZSetPopOutcome {
        ShardExecutor::blocking_zpop(self, keys, max, count)
    }
    fn xread_start(
        &self,
        options: &streams::XReadOptions<'_>,
    ) -> Option<(Vec<StreamId>, XReadOutcome)> {
        ShardExecutor::xread_start(self, options)
    }
    fn xread_once(
        &self,
        keys: &[&[u8]],
        starts: &[StreamId],
        count: usize,
    ) -> Option<XReadOutcome> {
        ShardExecutor::xread_once(self, keys, starts, count)
    }
    fn blocking_xreadgroup(&self, options: &XReadGroupOptions<'_>) -> XReadGroupOutcome {
        ShardExecutor::blocking_xreadgroup(self, options)
    }
    fn waitaof_probe(&self, local: i64, replicas: i64) -> Result<(u64, u64), &'static [u8]> {
        ShardExecutor::waitaof_probe(self, local, replicas)
    }
    fn shutdown_command(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        ShardExecutor::shutdown_command(self, args, out)
    }
}

impl Execution for Store {
    fn services_handle(&self) -> Arc<ServerServices> {
        Arc::clone(&self.services)
    }
    fn run_request(&self, request: Request) {
        self.call(move |executor| request(executor));
    }

    fn journal_sync(&self) -> std::io::Result<()> {
        Store::journal_sync(self)
    }
    fn purge_expired(&self, limit: usize) -> usize {
        Store::purge_expired(self, limit)
    }
    fn auto_rewrite_if_due(&self) -> bool {
        Store::auto_rewrite_if_due(self)
    }
    fn execute(&self, a: &[&[u8]], out: &mut Vec<u8>) -> Action {
        Store::execute(self, a, out)
    }
    fn watch_snapshot(&self, keys: &[&[u8]]) -> Option<WatchState> {
        Store::watch_snapshot(self, keys)
    }
    fn execute_transaction(
        &self,
        commands: &[Vec<Vec<u8>>],
        watches: &WatchState,
        out: &mut Vec<u8>,
    ) -> TransactionOutcome {
        Store::execute_transaction(self, commands, watches, out)
    }
    fn blocking_pop(&self, keys: &[&[u8]], left: bool) -> PopOutcome {
        Store::blocking_pop(self, keys, left)
    }
    fn blocking_lmpop(&self, keys: &[&[u8]], left: bool, count: usize) -> ListPopOutcome {
        self.call_prepared(|| {
            let keys = OwnedArguments::copy(keys);
            move |e: &ShardExecutor| e.blocking_lmpop(&keys.borrowed(), left, count)
        })
    }
    fn blocking_list_move(
        &self,
        source: &[u8],
        destination: &[u8],
        from_left: bool,
        to_left: bool,
    ) -> ListMoveOutcome {
        self.call_prepared(|| {
            let args = OwnedArguments::copy(&[source, destination]);
            move |e: &ShardExecutor| {
                let args = args.borrowed();
                e.blocking_list_move(args[0], args[1], from_left, to_left)
            }
        })
    }
    fn blocking_zpop(&self, keys: &[&[u8]], max: bool, count: usize) -> ZSetPopOutcome {
        self.call_prepared(|| {
            let keys = OwnedArguments::copy(keys);
            move |e: &ShardExecutor| e.blocking_zpop(&keys.borrowed(), max, count)
        })
    }
    fn xread_start(
        &self,
        options: &streams::XReadOptions<'_>,
    ) -> Option<(Vec<StreamId>, XReadOutcome)> {
        self.call_prepared(|| {
            let keys = OwnedArguments::copy(&options.keys);
            let starts = options.starts.clone();
            let count = options.count;
            let block = options.block;
            move |e: &ShardExecutor| {
                e.xread_start(&streams::XReadOptions {
                    count,
                    block,
                    keys: keys.borrowed(),
                    starts,
                })
            }
        })
    }
    fn xread_once(
        &self,
        keys: &[&[u8]],
        starts: &[StreamId],
        count: usize,
    ) -> Option<XReadOutcome> {
        self.call_prepared(|| {
            let keys = OwnedArguments::copy(keys);
            let starts = starts.to_vec();
            move |e: &ShardExecutor| e.xread_once(&keys.borrowed(), &starts, count)
        })
    }
    fn blocking_xreadgroup(&self, options: &XReadGroupOptions<'_>) -> XReadGroupOutcome {
        self.call_prepared(|| {
            let keys = OwnedArguments::copy(&options.keys);
            let starts = options.starts.clone();
            let count = options.count;
            let block = options.block;
            let group = options.group.to_vec();
            let consumer = options.consumer.to_vec();
            let noack = options.noack;
            move |e: &ShardExecutor| {
                e.blocking_xreadgroup(&XReadGroupOptions {
                    count,
                    block,
                    noack,
                    group: &group,
                    consumer: &consumer,
                    keys: keys.borrowed(),
                    starts,
                })
            }
        })
    }
    fn waitaof_probe(&self, local: i64, replicas: i64) -> Result<(u64, u64), &'static [u8]> {
        self.call(move |e| e.waitaof_probe(local, replicas))
    }
    fn shutdown_command(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let reply = self.call_prepared(|| {
            let args = OwnedArguments::copy(args);
            move |e: &ShardExecutor| {
                let mut out = Vec::new();
                e.shutdown_command(&args.borrowed(), &mut out);
                out
            }
        });
        out.extend_from_slice(&reply);
    }
}
/// One copied argument frame and ranges, not one allocation per argument.
/// Preparation occurs only after admission. Direct Store callers retain their
/// own borrowed inputs; the bridge retains at most one input copy and reply.
/// The API historically has no frame-size limit. Network callers retain their
/// existing parser/transaction byte limits; no new API rejection is invented.
struct OwnedArguments {
    bytes: Vec<u8>,
    ranges: Vec<std::ops::Range<usize>>,
}
impl OwnedArguments {
    fn copy(arguments: &[&[u8]]) -> Self {
        let mut bytes = Vec::with_capacity(arguments.iter().map(|a| a.len()).sum());
        let mut ranges = Vec::with_capacity(arguments.len());
        for argument in arguments {
            let start = bytes.len();
            bytes.extend_from_slice(argument);
            ranges.push(start..bytes.len());
        }
        Self { bytes, ranges }
    }
    fn borrowed(&self) -> Vec<&[u8]> {
        self.ranges.iter().map(|r| &self.bytes[r.clone()]).collect()
    }
}

/// Compatibility execution for TLS, portable and multi-socket-worker modes.
pub(crate) struct StoreHandle(pub(crate) Arc<Store>);
impl std::ops::Deref for StoreHandle {
    type Target = ServerServices;
    fn deref(&self) -> &ServerServices {
        &self.0.services
    }
}
impl Execution for StoreHandle {
    fn services_handle(&self) -> Arc<ServerServices> {
        self.0.services_handle()
    }
    fn run_request(&self, request: Request) {
        self.0.run_request(request);
    }

    fn execute(&self, arguments: &[&[u8]], output: &mut Vec<u8>) -> Action {
        self.0.execute(arguments, output)
    }
    fn watch_snapshot(&self, keys: &[&[u8]]) -> Option<WatchState> {
        self.0.watch_snapshot(keys)
    }
    fn execute_transaction(
        &self,
        commands: &[Vec<Vec<u8>>],
        watches: &WatchState,
        output: &mut Vec<u8>,
    ) -> TransactionOutcome {
        self.0.execute_transaction(commands, watches, output)
    }
    fn blocking_pop(&self, keys: &[&[u8]], left: bool) -> PopOutcome {
        self.0.blocking_pop(keys, left)
    }
    fn blocking_lmpop(&self, keys: &[&[u8]], left: bool, count: usize) -> ListPopOutcome {
        self.0.blocking_lmpop(keys, left, count)
    }
    fn blocking_list_move(
        &self,
        source: &[u8],
        destination: &[u8],
        from_left: bool,
        to_left: bool,
    ) -> ListMoveOutcome {
        self.0
            .blocking_list_move(source, destination, from_left, to_left)
    }
    fn blocking_zpop(&self, keys: &[&[u8]], max: bool, count: usize) -> ZSetPopOutcome {
        self.0.blocking_zpop(keys, max, count)
    }
    fn xread_start(
        &self,
        options: &streams::XReadOptions<'_>,
    ) -> Option<(Vec<StreamId>, XReadOutcome)> {
        self.0.xread_start(options)
    }
    fn xread_once(
        &self,
        keys: &[&[u8]],
        starts: &[StreamId],
        count: usize,
    ) -> Option<XReadOutcome> {
        self.0.xread_once(keys, starts, count)
    }
    fn blocking_xreadgroup(&self, options: &XReadGroupOptions<'_>) -> XReadGroupOutcome {
        self.0.blocking_xreadgroup(options)
    }
    fn waitaof_probe(&self, local: i64, replicas: i64) -> Result<(u64, u64), &'static [u8]> {
        self.0.waitaof_probe(local, replicas)
    }
    fn journal_sync(&self) -> std::io::Result<()> {
        self.0.journal_sync()
    }
    fn purge_expired(&self, limit: usize) -> usize {
        self.0.purge_expired(limit)
    }
    fn auto_rewrite_if_due(&self) -> bool {
        self.0.auto_rewrite_if_due()
    }
    fn shutdown_command(&self, args: &[&[u8]], output: &mut Vec<u8>) {
        self.0.shutdown_command(args, output)
    }
}

// Explicit forwarders retain Store::method UFCS/source compatibility.
impl Store {
    pub fn authentication_required(&self) -> bool {
        self.services.authentication_required()
    }
    pub fn authenticate(&self, username: Option<&[u8]>, password: &[u8]) -> Result<(), AuthError> {
        self.services.authenticate(username, password)
    }
    pub fn authenticate_user(
        &self,
        username: Option<&[u8]>,
        password: &[u8],
    ) -> Result<Vec<u8>, AuthError> {
        self.services.authenticate_user(username, password)
    }
    pub fn acl_restricted(&self) -> bool {
        self.services.acl_restricted()
    }
    pub fn authorize(&self, user: Option<&[u8]>, arguments: &[&[u8]]) -> Access {
        self.services.authorize(user, arguments)
    }
    pub fn whoami(&self, user: Option<&[u8]>) -> Option<Vec<u8>> {
        self.services.whoami(user)
    }
    pub fn acl_user_names(&self) -> Option<Vec<Vec<u8>>> {
        self.services.acl_user_names()
    }
    pub fn acl_line(&self, name: &[u8]) -> Option<String> {
        self.services.acl_line(name)
    }
    pub fn acl_details(&self, name: &[u8]) -> Option<UserDetails> {
        self.services.acl_details(name)
    }
    pub fn acl_setuser(&self, name: &[u8], rules: &[&[u8]]) -> Result<(), AclError> {
        self.services.acl_setuser(name, rules)
    }
    pub fn acl_deluser(&self, names: &[&[u8]]) -> Result<usize, AclError> {
        self.services.acl_deluser(names)
    }
    pub fn acl_save(&self) -> Result<(), AclError> {
        self.services.acl_save()
    }
    pub fn acl_load(&self) -> Result<(), AclError> {
        self.services.acl_load()
    }
    pub fn record_connection(&self) {
        self.services.record_connection()
    }
    pub fn record_rejected_connection(&self) {
        self.services.record_rejected_connection()
    }
    pub fn record_command(&self, client_id: u64) {
        self.services.record_command(client_id)
    }
    pub fn commandstats_enabled(&self) -> bool {
        self.services.commandstats_enabled()
    }
    pub fn set_commandstats_enabled(&self, enabled: bool) {
        self.services.set_commandstats_enabled(enabled)
    }
    pub fn set_max_clients(&self, value: usize) {
        self.services.set_max_clients(value)
    }
    pub fn max_clients(&self) -> usize {
        self.services.max_clients()
    }
    pub fn auth_failure_limit(&self) -> usize {
        self.services.auth_failure_limit()
    }
    pub fn set_auth_failure_limit(&mut self, limit: usize) {
        self.services.set_auth_failure_limit(limit)
    }
    pub fn set_auth_source_limit(&mut self, limit: usize, window_ms: u64) {
        self.services.set_auth_source_limit(limit, window_ms)
    }
    pub fn auth_source_limit(&self) -> usize {
        self.services.auth_source_limit()
    }
    pub fn auth_source_locked(&self, source: std::net::IpAddr) -> bool {
        self.services.auth_source_locked(source)
    }
    pub fn note_auth_source_failure(&self, source: std::net::IpAddr) {
        self.services.note_auth_source_failure(source)
    }
    pub fn reset_auth_source(&self, source: std::net::IpAddr) {
        self.services.reset_auth_source(source)
    }
    pub fn set_eviction_policy(&self, policy: EvictionPolicy) -> bool {
        self.services.set_eviction_policy(policy)
    }
    pub fn shard_for(&self, key: &[u8]) -> usize {
        self.services.shard_for(key)
    }
    pub fn shard_count(&self) -> usize {
        self.services.shard_count()
    }
    pub fn eviction_policy(&self) -> EvictionPolicy {
        self.services.eviction_policy()
    }
    pub fn set_notify_flags(&self, flags: NotifyFlags) {
        self.services.set_notify_flags(flags)
    }
    pub fn notify_flags(&self) -> NotifyFlags {
        self.services.notify_flags()
    }
    pub fn notifications_enabled(&self) -> bool {
        self.services.notifications_enabled()
    }
    pub fn add_monitor(&self, id: u64, sender: std::sync::mpsc::SyncSender<Vec<u8>>) -> bool {
        self.services.add_monitor(id, sender)
    }
    pub fn remove_monitor(&self, id: u64) {
        self.services.remove_monitor(id)
    }
    pub fn monitor_command(&self, source: u64, arguments: &[&[u8]]) {
        self.services.monitor_command(source, arguments)
    }
    pub fn pubsub_subscribe(
        &self,
        pattern: bool,
        name: &[u8],
        id: u64,
        sender: std::sync::mpsc::SyncSender<Vec<u8>>,
        resp3: bool,
    ) -> bool {
        self.services
            .pubsub_subscribe(pattern, name, id, sender, resp3)
    }
    pub fn pubsub_unsubscribe(&self, pattern: bool, name: &[u8], id: u64) -> bool {
        self.services.pubsub_unsubscribe(pattern, name, id)
    }
    pub fn pubsub_subscribe_shard(
        &self,
        name: &[u8],
        id: u64,
        sender: std::sync::mpsc::SyncSender<Vec<u8>>,
        resp3: bool,
    ) -> bool {
        self.services
            .pubsub_subscribe_shard(name, id, sender, resp3)
    }
    pub fn pubsub_unsubscribe_shard(&self, name: &[u8], id: u64) -> bool {
        self.services.pubsub_unsubscribe_shard(name, id)
    }
    pub fn publish_shard(&self, channel: &[u8], message: &[u8]) -> usize {
        self.services.publish_shard(channel, message)
    }
    pub fn pubsub_shard_channels(&self, pattern: Option<&[u8]>) -> Vec<Vec<u8>> {
        self.services.pubsub_shard_channels(pattern)
    }
    pub fn pubsub_shard_numsub(&self, channels: &[&[u8]]) -> Vec<(Vec<u8>, usize)> {
        self.services.pubsub_shard_numsub(channels)
    }
    pub fn pubsub_remove_client(&self, id: u64) {
        self.services.pubsub_remove_client(id)
    }
    pub fn pubsub_set_resp3(&self, id: u64, resp3: bool) {
        self.services.pubsub_set_resp3(id, resp3)
    }
    pub fn publish(&self, channel: &[u8], message: &[u8]) -> usize {
        self.services.publish(channel, message)
    }
    pub fn pubsub_channels(&self, pattern: Option<&[u8]>) -> Vec<Vec<u8>> {
        self.services.pubsub_channels(pattern)
    }
    pub fn pubsub_numsub(&self, channels: &[&[u8]]) -> Vec<(Vec<u8>, usize)> {
        self.services.pubsub_numsub(channels)
    }
    pub fn pubsub_numpat(&self) -> usize {
        self.services.pubsub_numpat()
    }
    pub fn pubsub_is_empty(&self) -> bool {
        self.services.pubsub_is_empty()
    }
    pub fn requirepass_configured(&self) -> bool {
        self.services.requirepass_configured()
    }
    pub fn set_requirepass(&self, password: Option<&[u8]>) -> bool {
        self.services.set_requirepass(password)
    }
    pub fn record_acl_denial(&self, reason: &str, context: &str, object: &[u8], username: &[u8]) {
        self.services
            .record_acl_denial(reason, context, object, username)
    }
    pub fn set_audit_hmac_key(&mut self, key: Vec<u8>) {
        self.services.set_audit_hmac_key(key)
    }
    pub fn attach_audit_syslog(
        &mut self,
        destination: std::net::SocketAddr,
    ) -> std::io::Result<()> {
        self.services.attach_audit_syslog(destination)
    }
    pub fn attach_audit_log(
        &mut self,
        path: &std::path::Path,
        max_bytes: u64,
    ) -> std::io::Result<()> {
        self.services.attach_audit_log(path, max_bytes)
    }
    pub fn acl_log(&self, count: usize) -> Vec<AclLogEntry> {
        self.services.acl_log(count)
    }
    pub fn reset_acl_log(&self) -> bool {
        self.services.reset_acl_log()
    }
    pub fn acl_dryrun(&self, username: &[u8], arguments: &[&[u8]]) -> Option<super::Access> {
        self.services.acl_dryrun(username, arguments)
    }
    pub fn generate_password(&self, bits: usize) -> Option<String> {
        self.services.generate_password(bits)
    }
    pub fn register_client(
        &self,
        id: u64,
        addr: SocketAddr,
        laddr: SocketAddr,
        control: TcpStream,
    ) {
        self.services.register_client(id, addr, laddr, control)
    }
    pub fn unregister_client(&self, id: u64) {
        self.services.unregister_client(id)
    }
    pub fn request_unblock(&self, id: u64, mode: UnblockMode) -> bool {
        self.services.request_unblock(id, mode)
    }
    pub fn take_unblock(&self, id: u64) -> Option<UnblockMode> {
        self.services.take_unblock(id)
    }
    pub fn update_client_metadata(
        &self,
        id: u64,
        name: Option<&[u8]>,
        library_name: Option<&[u8]>,
        library_version: Option<&[u8]>,
        resp3: bool,
        user: Option<&[u8]>,
    ) {
        self.services
            .update_client_metadata(id, name, library_name, library_version, resp3, user)
    }
    pub fn connected_clients(&self) -> usize {
        self.services.connected_clients()
    }
    pub fn client_snapshots(&self, type_filter: Option<bool>) -> Vec<ClientSnapshot> {
        self.services.client_snapshots(type_filter)
    }
    pub fn set_client_blocked(&self, id: u64, blocked: bool) {
        self.services.set_client_blocked(id, blocked)
    }
    pub fn blocked_clients(&self) -> usize {
        self.services.blocked_clients()
    }
    pub fn client_snapshot(&self, id: u64) -> Option<ClientSnapshot> {
        self.services.client_snapshot(id)
    }
    pub fn client_kill_ids(&self, ids: &[u64], skip: u64) -> usize {
        self.services.client_kill_ids(ids, skip)
    }
}
