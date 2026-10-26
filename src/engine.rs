//! Atomic in-memory string commands with expiration and a key/value payload budget.

use std::io::Write;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::protocol::encode_bulk;
mod commands;
mod data;
mod keyspace;
mod multikey;
mod options;
use data::Data;
use options::{Condition, ExpiryKind, SetOptions};

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
    clock: Arc<dyn Clock>,
}
impl Store {
    pub fn new(max_payload_bytes: usize) -> Self {
        Self::with_clock(max_payload_bytes, Arc::new(SystemClock))
    }
    pub fn with_clock(max_payload_bytes: usize, clock: Arc<dyn Clock>) -> Self {
        Self {
            data: RwLock::new(Data::default()),
            max_payload_bytes,
            clock,
        }
    }
    /// Bounded best-effort background reclamation; no sleep-dependent correctness.
    /// Every lookup still treats logically expired entries as absent immediately.
    pub fn purge_expired(&self, limit: usize) -> usize {
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
        if self.execute_multikey(arguments, output) {
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
        } else if !self.execute_keyspace(arguments, output) {
            output.extend_from_slice(b"-ERR unknown command\r\n");
        }
        Action::Continue
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

#[cfg(test)]
#[path = "engine/expiry_tests.rs"]
mod expiry_tests;
#[cfg(test)]
#[path = "engine/base_tests.rs"]
mod tests;
