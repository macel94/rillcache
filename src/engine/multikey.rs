//! Atomic multi-key string commands.

use std::collections::BTreeMap;

use super::options::OOM;
use super::{Store, internal_error};
use crate::protocol::encode_bulk;

/// Bound per-command reply buffering independently of duplicate-key amplification.
const MAX_MGET_REPLY_BYTES: usize = 2 * 1024 * 1024;

impl Store {
    fn msetex(&self, arguments: &[&[u8]], output: &mut Vec<u8>) {
        use super::options::{Condition, SetOptions, parse_integer};
        // Native command dispatch enforces MSETEX's minimum arity (4 arguments).
        if arguments.len() < 4 {
            arity("msetex", output);
            return;
        }
        let Some(count) =
            parse_integer(arguments[1]).filter(|count| *count > 0 && *count <= i32::MAX as i64)
        else {
            output.extend_from_slice(b"-ERR invalid numkeys value or out of range\r\n");
            return;
        };
        let Some(pair_end) = usize::try_from(count)
            .ok()
            .and_then(|n| n.checked_mul(2))
            .and_then(|n| n.checked_add(2))
        else {
            output.extend_from_slice(b"-ERR invalid numkeys value or out of range\r\n");
            return;
        };
        if arguments.len() < pair_end {
            output.extend_from_slice(b"-ERR syntax error\r\n");
            return;
        }
        let options = match SetOptions::parse(&arguments[pair_end..], false) {
            Ok(options) => options,
            Err(error) => {
                output.extend_from_slice(error);
                return;
            }
        };
        if options.get
            || matches!(
                options.condition,
                Condition::ValueEqual(_)
                    | Condition::ValueNotEqual(_)
                    | Condition::DigestEqual(_)
                    | Condition::DigestNotEqual(_)
            )
        {
            output.extend_from_slice(b"-ERR syntax error\r\n");
            return;
        }
        // MSETEX accepts one NX/XX condition and one expiry choice, but not GET.
        let mut final_values = BTreeMap::new();
        for pair in arguments[2..pair_end].as_chunks::<2>().0 {
            final_values.insert(pair[0], pair[1]);
        }
        let Ok(mut data) = self.data.write() else {
            internal_error(output);
            return;
        };
        let now = self.clock.now_ms().max(0);
        let expires = match options.deadline(now, "msetex") {
            Ok(expires) => expires,
            Err(error) => {
                output.extend_from_slice(&error);
                return;
            }
        };
        data.reap(now, usize::MAX);
        let conditional = match options.condition {
            Condition::Always => true,
            Condition::Missing => !final_values
                .keys()
                .any(|key| data.entries.contains_key(*key)),
            Condition::Present => final_values
                .keys()
                .all(|key| data.entries.contains_key(*key)),
            _ => false,
        };
        if !conditional {
            output.extend_from_slice(b":0\r\n");
            return;
        }
        if !options.keep && expires.is_some_and(|deadline| deadline < now) {
            for key in final_values.keys() {
                data.remove(key);
            }
            output.extend_from_slice(b":1\r\n");
            return;
        }
        let mut payload = data.payload_bytes;
        for key in final_values.keys() {
            if let Some(entry) = data.entries.get(*key) {
                let Some(without_old) = key
                    .len()
                    .checked_add(entry.value.len())
                    .and_then(|old_bytes| payload.checked_sub(old_bytes))
                else {
                    internal_error(output);
                    return;
                };
                payload = without_old;
            }
        }
        for (key, value) in &final_values {
            let Some(next) = payload
                .checked_add(key.len())
                .and_then(|v| v.checked_add(value.len()))
            else {
                output.extend_from_slice(OOM);
                return;
            };
            payload = next;
        }
        if payload > self.max_payload_bytes {
            output.extend_from_slice(OOM);
            return;
        }
        for (key, value) in final_values {
            let ttl = if options.keep {
                data.entries.get(key).and_then(|entry| entry.expires)
            } else {
                expires
            };
            if ttl.is_some_and(|deadline| deadline < now) {
                data.remove(key);
            } else {
                data.put(key, value, ttl, payload);
            }
        }
        output.extend_from_slice(b":1\r\n");
    }

    /// Execute MGET, MSET and MSETNX. Unrecognized commands leave `output` untouched.
    pub(super) fn execute_multikey(&self, arguments: &[&[u8]], output: &mut Vec<u8>) -> bool {
        let Some(command) = arguments.first() else {
            return false;
        };
        if command.eq_ignore_ascii_case(b"MSETEX") {
            self.msetex(arguments, output);
            return true;
        }
        if command.eq_ignore_ascii_case(b"MGET") {
            if arguments.len() < 2 {
                arity("mget", output);
                return true;
            }
            let Ok(mut data) = self.data.write() else {
                internal_error(output);
                return true;
            };
            let now = self.now(&data);
            data.reap(now, usize::MAX);
            if arguments[1..].iter().any(|key| {
                data.entries
                    .get(*key)
                    .is_some_and(|entry| entry.stream.is_some())
            }) {
                output.extend_from_slice(
                    b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
                );
                return true;
            }
            let count = arguments.len() - 1;
            let mut reply_bytes = 3 + decimal_digits(count);
            for key in &arguments[1..] {
                let value_len = data.entries.get(*key).map(|entry| entry.value.len());
                let element_bytes = match value_len {
                    Some(length) => 5usize
                        .checked_add(decimal_digits(length))
                        .and_then(|size| size.checked_add(length)),
                    None => Some(5),
                };
                let Some(total) = element_bytes.and_then(|size| reply_bytes.checked_add(size))
                else {
                    output.extend_from_slice(b"-ERR MGET reply limit exceeded\r\n");
                    return true;
                };
                reply_bytes = total;
                if reply_bytes > MAX_MGET_REPLY_BYTES {
                    output.extend_from_slice(b"-ERR MGET reply limit exceeded\r\n");
                    return true;
                }
            }
            use std::io::Write;
            write!(output, "*{count}\r\n").expect("Vec write");
            for key in &arguments[1..] {
                encode_bulk(
                    data.entries.get(*key).map(|entry| entry.value.as_slice()),
                    output,
                );
            }
            return true;
        }

        let nx = if command.eq_ignore_ascii_case(b"MSET") {
            false
        } else if command.eq_ignore_ascii_case(b"MSETNX") {
            true
        } else {
            return false;
        };
        let name = if nx { "msetnx" } else { "mset" };
        if arguments.len() < 3 || arguments.len().is_multiple_of(2) {
            arity(name, output);
            return true;
        }

        // Deduplicate before observing the store: last occurrence supplies the final value.
        // BTreeMap also gives deterministic application order; no result depends on hashing.
        let mut final_values = BTreeMap::new();
        for pair in arguments[1..].as_chunks::<2>().0 {
            final_values.insert(pair[0], pair[1]);
        }

        let Ok(mut data) = self.data.write() else {
            internal_error(output);
            return true;
        };
        let now = self.now(&data);
        // Reap globally before testing existence/admission; expired targets are absent and
        // their payload must not produce a false OOM on a replacement set.
        data.reap(now, usize::MAX);

        if nx
            && final_values
                .keys()
                .any(|key| data.entries.contains_key(*key))
        {
            output.extend_from_slice(b":0\r\n");
            return true;
        }

        let mut payload = data.payload_bytes;
        for key in final_values.keys() {
            if let Some(entry) = data.entries.get(*key) {
                let Some(without_old) = key
                    .len()
                    .checked_add(entry.payload_len)
                    .and_then(|old_bytes| payload.checked_sub(old_bytes))
                else {
                    internal_error(output);
                    return true;
                };
                payload = without_old;
            }
        }
        for (key, value) in &final_values {
            let Some(next) = payload
                .checked_add(key.len())
                .and_then(|n| n.checked_add(value.len()))
            else {
                output.extend_from_slice(OOM);
                return true;
            };
            payload = next;
        }
        if payload > self.max_payload_bytes {
            output.extend_from_slice(OOM);
            return true;
        }

        for (key, value) in final_values {
            data.put(key, value, None, payload);
        }
        if nx {
            output.extend_from_slice(b":1\r\n");
        } else {
            output.extend_from_slice(b"+OK\r\n");
        }
        true
    }
}

fn decimal_digits(mut value: usize) -> usize {
    let mut digits = 1;
    while value >= 10 {
        value /= 10;
        digits += 1;
    }
    digits
}

fn arity(command: &str, output: &mut Vec<u8>) {
    use std::io::Write;
    write!(
        output,
        "-ERR wrong number of arguments for '{command}' command\r\n"
    )
    .expect("Vec write");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};
    use std::thread;

    struct TestClock(AtomicI64);
    impl super::super::Clock for TestClock {
        fn now_ms(&self) -> i64 {
            self.0.load(Ordering::SeqCst)
        }
    }

    fn clock_store(budget: usize) -> (Store, Arc<TestClock>) {
        let clock = Arc::new(TestClock(AtomicI64::new(100)));
        (Store::with_clock(budget, clock.clone()), clock)
    }

    fn run(store: &Store, arguments: &[&[u8]]) -> Vec<u8> {
        let mut output = Vec::new();
        if !store.execute_multikey(arguments, &mut output) {
            store.execute(arguments, &mut output);
        }
        output
    }

    #[test]
    fn command_dispatch_arity_and_ordered_binary_mget() {
        let (store, _) = clock_store(128);
        let mut untouched = b"prefix".to_vec();
        assert!(!store.execute_multikey(&[b"NOPE"], &mut untouched));
        assert_eq!(untouched, b"prefix");
        assert_eq!(
            run(&store, &[b"mget"]),
            b"-ERR wrong number of arguments for 'mget' command\r\n"
        );
        assert_eq!(
            run(&store, &[b"MSET", b"only-key"]),
            b"-ERR wrong number of arguments for 'mset' command\r\n"
        );
        assert_eq!(
            run(&store, &[b"MSET", b"a", b"1", b"b"]),
            b"-ERR wrong number of arguments for 'mset' command\r\n"
        );
        assert_eq!(
            run(&store, &[b"MSET", b"k\0\xff", b"", b"other", b"\xff\0"]),
            b"+OK\r\n"
        );
        assert_eq!(
            run(
                &store,
                &[b"MGET", b"other", b"missing", b"k\0\xff", b"other"]
            ),
            b"*4\r\n$2\r\n\xff\0\r\n$-1\r\n$0\r\n\r\n$2\r\n\xff\0\r\n"
        );
    }

    #[test]
    fn duplicate_final_values_clear_ttl_and_accounting_is_exact() {
        let (store, _) = clock_store(12);
        assert_eq!(
            run(&store, &[b"SET", b"a", b"old", b"PX", b"1000"]),
            b"+OK\r\n"
        );
        assert_eq!(
            run(&store, &[b"MSET", b"a", b"x", b"a", b"123456789"]),
            b"+OK\r\n"
        );
        assert_eq!(store.data.read().unwrap().payload_bytes, 10);
        assert_eq!(run(&store, &[b"PTTL", b"a"]), b":-1\r\n");
        assert_eq!(run(&store, &[b"MGET", b"a"]), b"*1\r\n$9\r\n123456789\r\n");
        assert_eq!(
            run(&store, &[b"MSET", b"b", b"123"]),
            b"-OOM configured payload limit exceeded\r\n"
        );
        assert_eq!(run(&store, &[b"GET", b"b"]), b"$-1\r\n");
        assert_eq!(store.data.read().unwrap().payload_bytes, 10);
    }

    #[test]
    fn nx_is_atomic_duplicate_absent_is_single_key_and_expired_target_is_absent() {
        let (store, clock) = clock_store(16);
        assert_eq!(
            run(&store, &[b"MSETNX", b"dup", b"x", b"dup", b"last"]),
            b":1\r\n"
        );
        assert_eq!(
            run(&store, &[b"MGET", b"dup", b"dup"]),
            b"*2\r\n$4\r\nlast\r\n$4\r\nlast\r\n"
        );
        assert_eq!(
            run(&store, &[b"MSETNX", b"dup", b"n", b"new", b"y"]),
            b":0\r\n"
        );
        assert_eq!(run(&store, &[b"GET", b"new"]), b"$-1\r\n");
        assert_eq!(
            run(&store, &[b"MSETNX", b"over-budget", b"123456789"]),
            b"-OOM configured payload limit exceeded\r\n"
        );
        assert_eq!(run(&store, &[b"GET", b"over-budget"]), b"$-1\r\n");
        assert_eq!(
            run(&store, &[b"SET", b"stale", b"1234", b"PX", b"1"]),
            b"+OK\r\n"
        );
        clock.0.store(102, Ordering::SeqCst);
        assert_eq!(run(&store, &[b"MGET", b"stale"]), b"*1\r\n$-1\r\n");
        assert_eq!(run(&store, &[b"MSETNX", b"stale", b"z"]), b":1\r\n");
        assert_eq!(store.data.read().unwrap().payload_bytes, 13);
    }

    #[test]
    fn expired_payload_reclaimed_before_oom_and_failed_multi_write_is_unchanged() {
        let (store, clock) = clock_store(8);
        assert_eq!(
            run(&store, &[b"SET", b"old", b"123", b"PX", b"1"]),
            b"+OK\r\n"
        );
        clock.0.store(102, Ordering::SeqCst);
        assert_eq!(
            run(&store, &[b"MSET", b"a", b"123", b"b", b"456"]),
            b"+OK\r\n"
        );
        assert_eq!(store.data.read().unwrap().payload_bytes, 8);
        assert_eq!(run(&store, &[b"PEXPIRE", b"a", b"50"]), b":1\r\n");
        assert_eq!(
            run(&store, &[b"MSET", b"a", b"x", b"c", b"12345"]),
            b"-OOM configured payload limit exceeded\r\n"
        );
        assert_eq!(run(&store, &[b"PTTL", b"a"]), b":50\r\n");
        assert_eq!(
            run(&store, &[b"MGET", b"a", b"b", b"c"]),
            b"*3\r\n$3\r\n123\r\n$3\r\n456\r\n$-1\r\n"
        );
        assert_eq!(store.data.read().unwrap().payload_bytes, 8);
    }

    #[test]
    fn msetex_validates_options_and_applies_ttl_condition_and_atomic_budget() {
        let (store, clock) = clock_store(32);
        assert_eq!(
            run(
                &store,
                &[b"MSETEX", b"2", b"a", b"one", b"b", b"two", b"PX", b"50"]
            ),
            b":1\r\n"
        );
        assert_eq!(run(&store, &[b"PTTL", b"a"]), b":50\r\n");
        assert_eq!(
            run(&store, &[b"MSETEX", b"2", b"a", b"x", b"a", b"last", b"NX"]),
            b":0\r\n"
        );
        assert_eq!(
            run(
                &store,
                &[
                    b"MSETEX", b"2", b"a", b"x", b"b", b"y", b"XX", b"KEEPTTL", b"PX", b"20"
                ]
            ),
            b"-ERR syntax error\r\n"
        );
        assert_eq!(
            run(
                &store,
                &[
                    b"MSETEX", b"2", b"a", b"x", b"a", b"last", b"XX", b"KEEPTTL"
                ]
            ),
            b":1\r\n"
        );
        assert_eq!(run(&store, &[b"PTTL", b"a"]), b":50\r\n");
        assert_eq!(run(&store, &[b"GET", b"a"]), b"$4\r\nlast\r\n");
        assert_eq!(run(&store, &[b"GET", b"b"]), b"$3\r\ntwo\r\n");
        assert_eq!(
            run(
                &store,
                &[b"MSETEX", b"1", b"c", b"123456789012345678901234567890"]
            ),
            b"-OOM configured payload limit exceeded\r\n"
        );
        assert_eq!(run(&store, &[b"GET", b"c"]), b"$-1\r\n");
        assert_eq!(run(&store, &[b"GET", b"a"]), b"$4\r\nlast\r\n");
        assert_eq!(run(&store, &[b"PTTL", b"a"]), b":50\r\n");
        assert_eq!(
            run(&store, &[b"MSETEX", b"0", b"a", b"v"]),
            b"-ERR invalid numkeys value or out of range\r\n"
        );
        assert_eq!(
            run(&store, &[b"MSETEX", b"1", b"only-key"]),
            b"-ERR wrong number of arguments for 'msetex' command\r\n"
        );
        assert_eq!(
            run(&store, &[b"MSETEX", b"2", b"only-key", b"value"]),
            b"-ERR syntax error\r\n"
        );
        assert_eq!(
            run(
                &store,
                &[
                    b"MSETEX",
                    b"1",
                    b"past",
                    b"123456789012345678901234567890",
                    b"PXAT",
                    b"99",
                ]
            ),
            b":1\r\n"
        );
        assert_eq!(run(&store, &[b"GET", b"past"]), b"$-1\r\n");
        assert_eq!(
            run(
                &store,
                &[b"MSETEX", b"1", b"expired", b"v", b"PXAT", b"100"]
            ),
            b":1\r\n"
        );
        assert_eq!(run(&store, &[b"GET", b"expired"]), b"$1\r\nv\r\n");
        clock.0.store(101, Ordering::SeqCst);
        assert_eq!(run(&store, &[b"GET", b"expired"]), b"$-1\r\n");
        assert_eq!(store.data.read().unwrap().payload_bytes, 7);
    }

    #[test]
    fn simultaneous_msetnx_has_one_atomic_winner() {
        let (store, _) = clock_store(1024);
        let store = Arc::new(store);
        let barrier = Arc::new(Barrier::new(8));
        let winners = Arc::new(AtomicUsize::new(0));
        thread::scope(|scope| {
            for id in 0..8 {
                let store = store.clone();
                let barrier = barrier.clone();
                let winners = winners.clone();
                scope.spawn(move || {
                    let value = [id as u8];
                    let mut output = Vec::new();
                    barrier.wait();
                    store.execute_multikey(
                        &[b"MSETNX", b"left", &value, b"right", &value],
                        &mut output,
                    );
                    if output == b":1\r\n" {
                        winners.fetch_add(1, Ordering::SeqCst);
                    }
                });
            }
        });
        assert_eq!(winners.load(Ordering::SeqCst), 1);
        let left = run(&store, &[b"GET", b"left"]);
        let right = run(&store, &[b"GET", b"right"]);
        assert_eq!(left, right);
        assert!(store.data.read().unwrap().payload_bytes <= 1024);
    }
}
