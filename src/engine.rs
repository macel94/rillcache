//! Atomic in-memory string commands with expiration and a key/value payload budget.

use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::protocol::encode_bulk;
mod administration;
mod auth;
mod collection_config;
mod collections;
mod commands;
mod data;
mod discovery;
mod keyspace;
mod multikey;
mod options;
mod sha256;
mod streams;
mod string_algorithms;
mod string_ranges;
mod transaction;
mod xxh3;
pub use auth::{Access, AclError, Auth, AuthError, UserDetails};
use collection_config::CollectionConfig;
use data::Data;
use options::{Condition, ExpiryKind, SetOptions};
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

pub struct Store {
    data: RwLock<Data>,
    max_payload_bytes: usize,
    collection_config: RwLock<CollectionConfig>,
    clock: Arc<dyn Clock>,
    transaction_gate: RwLock<()>,
    auth: RwLock<Auth>,
    auth_required: AtomicBool,
    acl_restricted: AtomicBool,
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
    pub fn with_clock_and_auth(
        max_payload_bytes: usize,
        clock: Arc<dyn Clock>,
        auth: Auth,
    ) -> Self {
        Self {
            data: RwLock::new(Data::default()),
            max_payload_bytes,
            collection_config: RwLock::new(CollectionConfig::default()),
            clock,
            transaction_gate: RwLock::new(()),
            auth_required: AtomicBool::new(auth.required()),
            acl_restricted: AtomicBool::new(auth.restricted()),
            auth: RwLock::new(auth),
        }
    }
    /// Bounded best-effort background reclamation; no sleep-dependent correctness.
    /// Every lookup still treats logically expired entries as absent immediately.
    pub fn purge_expired(&self, limit: usize) -> usize {
        let Ok(_gate) = self.transaction_gate.read() else {
            return 0;
        };
        let Ok(mut data) = self.data.try_write() else {
            return 0;
        };
        if data.expirations.is_empty() {
            return 0;
        }
        data.reap(self.clock.now_ms().max(0), limit)
    }

    /// Append exactly one reply. No store lock is held during socket I/O.
    pub fn execute(&self, arguments: &[&[u8]], output: &mut Vec<u8>) -> Action {
        let Ok(_gate) = self.transaction_gate.read() else {
            internal_error(output);
            return Action::Continue;
        };
        self.execute_locked(arguments, output)
    }

    /// Dispatch without the transaction gate; only `EXEC` may call this while
    /// holding the gate's write side.
    pub(super) fn execute_locked(&self, arguments: &[&[u8]], output: &mut Vec<u8>) -> Action {
        if self.execute_collection_config(arguments, output)
            || self.execute_admin(arguments, output)
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
        } else if command.eq_ignore_ascii_case(b"DEL") {
            if arguments.len() < 2 {
                arity("del", output);
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
            && !self.execute_string_algorithms(arguments, output)
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
#[path = "engine/expiry_tests.rs"]
mod expiry_tests;
#[cfg(test)]
#[path = "engine/base_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "engine/transaction_tests.rs"]
mod transaction_tests;
