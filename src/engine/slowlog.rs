//! Bounded SLOWLOG command timing and storage.
//!
//! Slow commands are recorded only when `slowlog-log-slower-than` is
//! non-negative and `slowlog-max-len` is non-zero. Entries keep at most 32
//! arguments and 128 bytes per argument, mirroring Redis's truncation, so an
//! unbounded client request cannot become unbounded retained memory.

use super::Store;
use crate::protocol::encode_bulk;
use std::sync::atomic::Ordering;
use std::time::Duration;

pub(crate) const DEFAULT_THRESHOLD_US: i64 = 10_000;
pub(crate) const DEFAULT_MAX_LEN: usize = 128;
pub(crate) const MAX_MAX_LEN: usize = 1_000_000;
const MAX_ARGC: usize = 32;
const MAX_ARG_BYTES: usize = 128;

#[derive(Clone, Debug)]
pub(crate) struct SlowlogEntry {
    id: u64,
    timestamp: i64,
    duration_us: i64,
    argv: Vec<Vec<u8>>,
    client_addr: String,
    client_name: Vec<u8>,
}

impl Store {
    pub(crate) fn slowlog_threshold_us(&self) -> i64 {
        self.slowlog_threshold_us.load(Ordering::Relaxed)
    }

    pub(crate) fn slowlog_max_len(&self) -> usize {
        self.slowlog_max_len.load(Ordering::Relaxed)
    }

    pub(crate) fn slowlog_enabled(&self) -> bool {
        self.slowlog_threshold_us() >= 0 && self.slowlog_max_len() > 0
    }

    pub(crate) fn set_slowlog_threshold_us(&self, value: i64) {
        self.slowlog_threshold_us.store(value, Ordering::Relaxed);
    }

    pub(crate) fn set_slowlog_max_len(&self, value: usize) {
        let value = value.min(MAX_MAX_LEN);
        self.slowlog_max_len.store(value, Ordering::Relaxed);
        if let Ok(mut entries) = self.slowlog.write() {
            while entries.len() > value {
                entries.pop_back();
            }
        }
    }

    pub(crate) fn slowlog_len(&self) -> usize {
        self.slowlog
            .read()
            .map(|entries| entries.len())
            .unwrap_or(0)
    }

    pub(crate) fn slowlog_reset(&self) {
        if let Ok(mut entries) = self.slowlog.write() {
            entries.clear();
        }
    }

    /// Record one command if it reached the configured duration threshold.
    /// Called after the reply is produced; the store and socket locks are never
    /// held while the command runs.
    pub(crate) fn slowlog_push(
        &self,
        arguments: &[&[u8]],
        elapsed: Duration,
        metadata: impl FnOnce() -> (String, Vec<u8>),
    ) {
        let threshold = self.slowlog_threshold_us();
        let max_len = self.slowlog_max_len();
        if threshold < 0 || max_len == 0 {
            return;
        }
        let duration_us = i64::try_from(elapsed.as_micros()).unwrap_or(i64::MAX);
        if duration_us < threshold {
            return;
        }
        // Client lookup/name cloning is cold-path work: never do it for the
        // overwhelmingly common below-threshold command or a disabled logger.
        let (client_addr, client_name) = metadata();
        let entry = SlowlogEntry {
            id: self.slowlog_next_id.fetch_add(1, Ordering::Relaxed),
            timestamp: self.clock.now_ms().div_euclid(1000),
            duration_us,
            argv: capture_arguments(arguments),
            client_addr,
            client_name,
        };
        let Ok(mut entries) = self.slowlog.write() else {
            return;
        };
        entries.push_front(entry);
        while entries.len() > max_len {
            entries.pop_back();
        }
    }

    pub(super) fn execute_slowlog(&self, arguments: &[&[u8]], output: &mut Vec<u8>) -> bool {
        let Some(command) = arguments.first() else {
            return false;
        };
        if !command.eq_ignore_ascii_case(b"SLOWLOG") {
            return false;
        }
        let Some(subcommand) = arguments.get(1) else {
            output.extend_from_slice(b"-ERR wrong number of arguments for 'slowlog' command\r\n");
            return true;
        };
        if subcommand.eq_ignore_ascii_case(b"HELP") {
            if arguments.len() != 2 {
                slowlog_syntax_error(subcommand, output);
                return true;
            }
            const HELP: &[&[u8]] = &[
                b"GET [<count>]",
                b"    Return top <count> entries from the slowlog (default: 10, -1 mean all).",
                b"    Entries are made of:",
                b"    id, timestamp, time in microseconds, arguments array, client IP and port,",
                b"    client name",
                b"LEN",
                b"    Return the length of the slowlog.",
                b"RESET",
                b"    Reset the slowlog.",
            ];
            write_array_len(HELP.len(), output);
            for line in HELP {
                encode_bulk(Some(line), output);
            }
        } else if subcommand.eq_ignore_ascii_case(b"LEN") {
            if arguments.len() != 2 {
                slowlog_syntax_error(subcommand, output);
            } else {
                super::integer(self.slowlog_len() as i64, output);
            }
        } else if subcommand.eq_ignore_ascii_case(b"RESET") {
            if arguments.len() != 2 {
                slowlog_syntax_error(subcommand, output);
            } else {
                self.slowlog_reset();
                output.extend_from_slice(b"+OK\r\n");
            }
        } else if subcommand.eq_ignore_ascii_case(b"GET") {
            if arguments.len() > 3 {
                slowlog_syntax_error(subcommand, output);
                return true;
            }
            let count = match arguments.get(2) {
                None => 10,
                Some(raw) => match parse_i64(raw) {
                    Some(value) if value >= -1 => value,
                    Some(_) => {
                        output.extend_from_slice(
                            b"-ERR count should be greater than or equal to -1\r\n",
                        );
                        return true;
                    }
                    None => {
                        output
                            .extend_from_slice(b"-ERR value is not an integer or out of range\r\n");
                        return true;
                    }
                },
            };
            let Ok(entries) = self.slowlog.read() else {
                super::internal_error(output);
                return true;
            };
            let limit = if count == -1 {
                entries.len()
            } else {
                usize::try_from(count)
                    .unwrap_or(usize::MAX)
                    .min(entries.len())
            };
            write_array_len(limit, output);
            for entry in entries.iter().take(limit) {
                write_entry(entry, output);
            }
        } else {
            slowlog_syntax_error(subcommand, output);
        }
        true
    }
}

fn capture_arguments(arguments: &[&[u8]]) -> Vec<Vec<u8>> {
    let take = arguments.len().min(MAX_ARGC);
    let redacted = redacted_arguments(arguments);
    let mut captured = Vec::with_capacity(take);
    for (index, argument) in arguments.iter().take(take).enumerate() {
        if redacted.get(index).copied().unwrap_or(false) {
            captured.push(b"(redacted)".to_vec());
        } else if arguments.len() > MAX_ARGC && index == MAX_ARGC - 1 {
            captured.push(
                format!("... ({} more arguments)", arguments.len() - MAX_ARGC + 1).into_bytes(),
            );
        } else if argument.len() > MAX_ARG_BYTES {
            let mut truncated = argument[..MAX_ARG_BYTES].to_vec();
            truncated.extend_from_slice(
                format!("... ({} more bytes)", argument.len() - MAX_ARG_BYTES).as_bytes(),
            );
            captured.push(truncated);
        } else {
            captured.push(argument.to_vec());
        }
    }
    captured
}

/// Redis marks credential-bearing arguments as sensitive so slowlog consumers
/// cannot recover passwords from diagnostic output.
fn redacted_arguments(arguments: &[&[u8]]) -> Vec<bool> {
    let mut redacted = vec![false; arguments.len()];
    let Some(command) = arguments.first() else {
        return redacted;
    };
    if command.eq_ignore_ascii_case(b"AUTH") {
        for item in redacted.iter_mut().skip(1) {
            *item = true;
        }
    } else if command.eq_ignore_ascii_case(b"HELLO") {
        let mut index = 1;
        while index < arguments.len() {
            if arguments[index].eq_ignore_ascii_case(b"AUTH") && index + 2 < arguments.len() {
                redacted[index + 1] = true;
                redacted[index + 2] = true;
                index += 3;
            } else {
                index += 1;
            }
        }
    } else if command.eq_ignore_ascii_case(b"CONFIG")
        && arguments
            .get(1)
            .is_some_and(|sub| sub.eq_ignore_ascii_case(b"SET"))
    {
        for (index, pair) in arguments[2..].chunks(2).enumerate() {
            if pair.len() == 2
                && (pair[0].eq_ignore_ascii_case(b"requirepass")
                    || pair[0].eq_ignore_ascii_case(b"masterauth"))
            {
                redacted[2 + index * 2 + 1] = true;
            }
        }
    } else if command.eq_ignore_ascii_case(b"ACL")
        && arguments.get(1).is_some_and(|sub| {
            sub.eq_ignore_ascii_case(b"SETUSER")
                || sub.eq_ignore_ascii_case(b"DELUSER")
                || sub.eq_ignore_ascii_case(b"GETUSER")
        })
    {
        for item in redacted.iter_mut().skip(2) {
            *item = true;
        }
    }
    redacted
}

fn parse_i64(raw: &[u8]) -> Option<i64> {
    std::str::from_utf8(raw).ok()?.parse().ok()
}

fn write_entry(entry: &SlowlogEntry, output: &mut Vec<u8>) {
    write_array_len(6, output);
    super::integer(i64::try_from(entry.id).unwrap_or(i64::MAX), output);
    super::integer(entry.timestamp, output);
    super::integer(entry.duration_us, output);
    write_array_len(entry.argv.len(), output);
    for argument in &entry.argv {
        encode_bulk(Some(argument), output);
    }
    encode_bulk(Some(entry.client_addr.as_bytes()), output);
    encode_bulk(Some(&entry.client_name), output);
}

fn write_array_len(length: usize, output: &mut Vec<u8>) {
    use std::io::Write;
    let _ = write!(output, "*{length}\r\n");
}

fn slowlog_syntax_error(subcommand: &[u8], output: &mut Vec<u8>) {
    output.extend_from_slice(b"-ERR unknown subcommand or wrong number of arguments for 'slowlog|");
    output.extend_from_slice(subcommand);
    output.extend_from_slice(b"'. Try SLOWLOG HELP.\r\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_are_truncated_to_bounded_shapes() {
        let long = vec![b'x'; 200];
        let mut arguments: Vec<&[u8]> = vec![b"CMD"];
        for _ in 0..40 {
            arguments.push(&long);
        }
        let captured = capture_arguments(&arguments);
        assert_eq!(captured.len(), MAX_ARGC);
        assert_eq!(captured[0], b"CMD");
        assert_eq!(captured[MAX_ARGC - 1], b"... (10 more arguments)");
        let expected = {
            let mut value = vec![b'x'; MAX_ARG_BYTES];
            value.extend_from_slice(b"... (72 more bytes)");
            value
        };
        assert_eq!(captured[1], expected);
    }

    #[test]
    fn credential_arguments_are_redacted() {
        let auth = redacted_arguments(&[b"AUTH", b"user", b"secret"]);
        assert_eq!(auth, vec![false, true, true]);
        let hello = redacted_arguments(&[
            b"HELLO", b"3", b"AUTH", b"user", b"secret", b"SETNAME", b"name",
        ]);
        assert_eq!(hello, vec![false, false, false, true, true, false, false]);
        let config = redacted_arguments(&[
            b"CONFIG",
            b"SET",
            b"slowlog-max-len",
            b"10",
            b"requirepass",
            b"secret",
        ]);
        assert_eq!(config, vec![false, false, false, false, false, true]);
        let acl = redacted_arguments(&[b"ACL", b"SETUSER", b"alice", b">secret"]);
        assert_eq!(acl, vec![false, false, true, true]);
        let visible = redacted_arguments(&[b"SET", b"key", b"value"]);
        assert_eq!(visible, vec![false, false, false]);
    }

    #[test]
    fn rejected_samples_never_capture_client_metadata() {
        let store = Store::new(1024);
        let captured = std::cell::Cell::new(0);
        let metadata = || {
            captured.set(captured.get() + 1);
            ("127.0.0.1:1234".into(), b"client".to_vec())
        };
        store.set_slowlog_threshold_us(100);
        store.slowlog_push(&[b"PING"], Duration::from_micros(99), metadata);
        assert_eq!(captured.get(), 0);
        store.slowlog_push(&[b"PING"], Duration::from_micros(100), metadata);
        assert_eq!(captured.get(), 1);
        let entries = store.slowlog.read().unwrap();
        assert_eq!(entries.front().unwrap().client_addr, "127.0.0.1:1234");
        assert_eq!(entries.front().unwrap().client_name, b"client");
        drop(entries);
        store.set_slowlog_threshold_us(-1);
        store.slowlog_push(&[b"PING"], Duration::from_secs(1), metadata);
        store.set_slowlog_threshold_us(0);
        store.set_slowlog_max_len(0);
        store.slowlog_push(&[b"PING"], Duration::from_secs(1), metadata);
        assert_eq!(captured.get(), 1);
    }

    #[test]
    fn threshold_and_max_len_bound_the_retained_entries() {
        let store = Store::new(1024);
        store.set_slowlog_threshold_us(0);
        store.set_slowlog_max_len(2);
        for index in 0..4 {
            let command = format!("CMD{index}");
            store.slowlog_push(&[command.as_bytes()], Duration::from_micros(10), || {
                ("127.0.0.1:1".into(), Vec::new())
            });
        }
        assert_eq!(store.slowlog_len(), 2);
        store.set_slowlog_threshold_us(-1);
        store.slowlog_push(&[b"CMD"], Duration::from_millis(1), || {
            ("127.0.0.1:1".into(), Vec::new())
        });
        assert_eq!(store.slowlog_len(), 2);
        store.set_slowlog_max_len(0);
        assert_eq!(store.slowlog_len(), 0);
    }
}
