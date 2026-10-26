use super::data::Entry;
use super::*;
use crate::protocol::encode_bulk;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime as RandomSystemTime, UNIX_EPOCH as RANDOM_EPOCH};

static RANDOMKEY_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const MAX_GLOB_STEPS: usize = 4_000_000;

impl Store {
    /// Execute keyspace commands; return false without touching output if unknown.
    pub(super) fn execute_keyspace(&self, arguments: &[&[u8]], output: &mut Vec<u8>) -> bool {
        let Some(command) = arguments.first() else {
            return false;
        };
        let (name, mode) = if command.eq_ignore_ascii_case(b"TYPE") {
            ("type", 0)
        } else if command.eq_ignore_ascii_case(b"RENAME") {
            ("rename", 1)
        } else if command.eq_ignore_ascii_case(b"RENAMENX") {
            ("renamenx", 2)
        } else if command.eq_ignore_ascii_case(b"TOUCH") {
            ("touch", 3)
        } else if command.eq_ignore_ascii_case(b"DBSIZE") {
            ("dbsize", 4)
        } else if command.eq_ignore_ascii_case(b"COPY") {
            ("copy", 5)
        } else if command.eq_ignore_ascii_case(b"KEYS") {
            ("keys", 6)
        } else if command.eq_ignore_ascii_case(b"RANDOMKEY") {
            ("randomkey", 7)
        } else {
            return false;
        };
        match mode {
            0 => {
                if arguments.len() != 2 {
                    arity(name, output);
                } else {
                    self.key_type(arguments[1], output);
                }
            }
            1 | 2 => {
                if arguments.len() != 3 {
                    arity(name, output);
                } else {
                    self.rename(arguments[1], arguments[2], mode == 2, output);
                }
            }
            3 => {
                if arguments.len() < 2 {
                    arity(name, output);
                } else {
                    self.touch(&arguments[1..], output);
                }
            }
            4 => {
                if arguments.len() != 1 {
                    arity(name, output);
                } else {
                    self.dbsize(output);
                }
            }
            5 => {
                if arguments.len() < 3 {
                    arity(name, output);
                } else {
                    self.copy(arguments, output);
                }
            }
            6 => {
                if arguments.len() != 2 {
                    arity(name, output);
                } else {
                    self.keys(arguments[1], output);
                }
            }
            7 => {
                if arguments.len() != 1 {
                    arity(name, output);
                } else {
                    self.random_key(output);
                }
            }
            _ => unreachable!(),
        }
        true
    }

    fn key_type(&self, key: &[u8], output: &mut Vec<u8>) {
        let Ok(mut data) = self.data.write() else {
            internal_error(output);
            return;
        };
        let now = self.now(&data);
        data.expire_key(key, now);
        if let Some(entry) = data.entries.get(key) {
            let kind: &[u8] = match entry.collection.as_ref() {
                None if entry.stream.is_some() => b"stream",
                None => b"string",
                Some(collection) if collection.is_list() => b"list",
                Some(collection) if collection.is_set() => b"set",
                Some(collection) if collection.is_hash() => b"hash",
                Some(collection) if collection.is_zset() => b"zset",
                Some(_) => b"string",
            };
            output.push(b'+');
            output.extend_from_slice(kind);
            output.extend_from_slice(b"\r\n");
        } else {
            output.extend_from_slice(b"+none\r\n");
        }
    }

    fn copy(&self, arguments: &[&[u8]], output: &mut Vec<u8>) {
        let mut replace = false;
        let mut index = 3;
        while index < arguments.len() {
            if arguments[index].eq_ignore_ascii_case(b"REPLACE") {
                replace = true;
            } else if arguments[index].eq_ignore_ascii_case(b"DB") {
                index += 1;
                let Some(raw_db) = arguments.get(index) else {
                    output.extend_from_slice(options::SYNTAX);
                    return;
                };
                let Some(db) = options::parse_integer(raw_db) else {
                    output.extend_from_slice(options::NOT_INTEGER);
                    return;
                };
                if db != 0 {
                    output.extend_from_slice(b"-ERR DB index is out of range\r\n");
                    return;
                }
            } else {
                output.extend_from_slice(options::SYNTAX);
                return;
            }
            index += 1;
        }
        let source = arguments[1];
        let destination = arguments[2];
        if source == destination {
            output.extend_from_slice(b"-ERR source and destination objects are the same\r\n");
            return;
        }
        let Ok(mut data) = self.data.write() else {
            internal_error(output);
            return;
        };
        let now = self.now(&data);
        data.expire_key(source, now);
        let Some(source_payload) = data.entries.get(source).map(Entry::payload_len) else {
            output.extend_from_slice(b":0\r\n");
            return;
        };
        data.expire_key(destination, now);
        if data.entries.contains_key(destination) && !replace {
            output.extend_from_slice(b":0\r\n");
            return;
        }
        let previous = data.entries.get(destination).map(Entry::payload_len);
        let Some(payload) = data.reserve(
            destination,
            source_payload,
            previous,
            self.max_payload_bytes,
            now,
        ) else {
            output.extend_from_slice(options::OOM);
            return;
        };
        let source_entry = data
            .entries
            .get(source)
            .expect("source remains present under lock");
        let copy = Entry {
            value: source_entry.value.clone(),
            collection: source_entry.collection.clone(),
            stream: source_entry.stream.clone(),
            payload_len: source_entry.payload_len,
            expires: source_entry.expires,
        };
        data.remove(destination);
        data.put_entry(destination, copy, payload);
        output.extend_from_slice(b":1\r\n");
    }

    fn keys(&self, pattern: &[u8], output: &mut Vec<u8>) {
        let Ok(mut data) = self.data.write() else {
            internal_error(output);
            return;
        };
        let now = self.now(&data);
        data.reap(now, usize::MAX);
        let mut matched: Vec<&[u8]> = data
            .entries
            .keys()
            .filter(|key| glob_match(pattern, key))
            .map(Vec::as_slice)
            .collect();
        matched.sort_unstable();
        write!(output, "*{}\r\n", matched.len()).expect("Vec write");
        for key in matched {
            encode_bulk(Some(key), output);
        }
    }

    fn random_key(&self, output: &mut Vec<u8>) {
        let Ok(mut data) = self.data.write() else {
            internal_error(output);
            return;
        };
        let now = self.now(&data);
        data.reap(now, usize::MAX);
        if data.entries.is_empty() {
            encode_bulk(None, output);
            return;
        }
        let sequence = RANDOMKEY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let nanos = RandomSystemTime::now()
            .duration_since(RANDOM_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        let mut random = nanos ^ sequence.wrapping_mul(0x9e3779b97f4a7c15);
        random = (random ^ (random >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        random = (random ^ (random >> 27)).wrapping_mul(0x94d049bb133111eb);
        random ^= random >> 31;
        let selected = random as usize % data.entries.len();
        let key = data
            .entries
            .keys()
            .nth(selected)
            .expect("index below key count");
        encode_bulk(Some(key), output);
    }

    fn touch(&self, keys: &[&[u8]], output: &mut Vec<u8>) {
        let Ok(mut data) = self.data.write() else {
            internal_error(output);
            return;
        };
        let now = self.now(&data);
        for key in keys {
            data.expire_key(key, now);
        }
        integer(
            keys.iter()
                .filter(|key| data.entries.contains_key(**key))
                .count() as i64,
            output,
        );
    }

    fn dbsize(&self, output: &mut Vec<u8>) {
        let Ok(mut data) = self.data.write() else {
            internal_error(output);
            return;
        };
        let now = self.now(&data);
        data.reap(now, usize::MAX);
        integer(data.entries.len() as i64, output);
    }

    fn rename(
        &self,
        source: &[u8],
        destination: &[u8],
        only_if_absent: bool,
        output: &mut Vec<u8>,
    ) {
        let Ok(mut data) = self.data.write() else {
            internal_error(output);
            return;
        };
        let now = self.now(&data);
        data.expire_key(source, now);
        if source == destination {
            if data.entries.contains_key(source) {
                if only_if_absent {
                    integer(0, output);
                } else {
                    output.extend_from_slice(b"+OK\r\n");
                }
            } else {
                output.extend_from_slice(b"-ERR no such key\r\n");
            }
            return;
        }
        data.expire_key(destination, now);
        let Some(entry) = data.entries.get(source) else {
            output.extend_from_slice(b"-ERR no such key\r\n");
            return;
        };
        if only_if_absent && data.entries.contains_key(destination) {
            integer(0, output);
            return;
        }

        // Compute the final exact payload before removing either key. This makes
        // an over-budget longer destination an atomic failure.
        let source_bytes = source.len().checked_add(entry.payload_len());
        let destination_bytes = data
            .entries
            .get(destination)
            .and_then(|old| destination.len().checked_add(old.payload_len()))
            .unwrap_or(0);
        let Some(payload) = source_bytes
            .and_then(|bytes| data.payload_bytes.checked_sub(bytes))
            .and_then(|remaining| remaining.checked_sub(destination_bytes))
            .and_then(|remaining| remaining.checked_add(destination.len()))
            .and_then(|remaining| remaining.checked_add(entry.payload_len()))
            .filter(|bytes| *bytes <= self.max_payload_bytes)
        else {
            output.extend_from_slice(options::OOM);
            return;
        };

        let entry = data.remove(source).expect("source checked while locked");
        data.remove(destination);
        data.put_entry(destination, entry, payload);
        if only_if_absent {
            integer(1, output);
        } else {
            output.extend_from_slice(b"+OK\r\n");
        }
    }
}

/// Byte-oriented Redis-style glob matching. Wildcards are processed iteratively
/// with bounded backtracking; pathological recursion/CPU becomes no-match.
pub(super) fn glob_match(pattern: &[u8], text: &[u8]) -> bool {
    let mut star_groups = 0usize;
    let mut scan = 0usize;
    let mut in_class = false;
    while scan < pattern.len() {
        match pattern[scan] {
            b'\\' if scan + 1 < pattern.len() => scan += 2,
            b'[' => {
                in_class = true;
                scan += 1;
            }
            b']' => {
                in_class = false;
                scan += 1;
            }
            b'*' if !in_class => {
                star_groups += 1;
                while scan < pattern.len() && pattern[scan] == b'*' {
                    scan += 1;
                }
            }
            _ => scan += 1,
        }
    }
    if star_groups > 1000 {
        return false;
    }

    let (mut p, mut s) = (0usize, 0usize);
    let (mut retry_pattern, mut retry_text) = (None, 0usize);
    let mut steps = 0usize;
    loop {
        steps += 1;
        if steps > MAX_GLOB_STEPS {
            return false;
        }
        if s == text.len() {
            while p < pattern.len() && pattern[p] == b'*' {
                p += 1;
            }
            if p == pattern.len() {
                return true;
            }
        }
        if p == pattern.len() {
            if s == text.len() {
                return true;
            }
        } else if s < text.len() {
            match pattern[p] {
                b'*' => {
                    while p < pattern.len() && pattern[p] == b'*' {
                        p += 1;
                    }
                    if p == pattern.len() {
                        return true;
                    }
                    retry_pattern = Some(p);
                    retry_text = s;
                    continue;
                }
                b'?' => {
                    p += 1;
                    s += 1;
                    continue;
                }
                b'[' => {
                    if let Some((matched, after)) = glob_class(pattern, p, text[s], &mut steps)
                        && matched
                    {
                        p = after;
                        s += 1;
                        continue;
                    }
                }
                byte => {
                    let (literal, after) = if byte == b'\\' && p + 1 < pattern.len() {
                        (pattern[p + 1], p + 2)
                    } else {
                        (byte, p + 1)
                    };
                    if literal == text[s] {
                        p = after;
                        s += 1;
                        continue;
                    }
                }
            }
        }
        if let Some(star) = retry_pattern
            && retry_text < text.len()
        {
            retry_text += 1;
            s = retry_text;
            p = star;
            continue;
        }
        return false;
    }
}

fn glob_class(pattern: &[u8], start: usize, byte: u8, steps: &mut usize) -> Option<(bool, usize)> {
    let mut index = start + 1;
    let negate = index < pattern.len() && pattern[index] == b'^';
    if negate {
        index += 1;
    }
    let mut matched = false;
    while index < pattern.len() {
        *steps += 1;
        if *steps > MAX_GLOB_STEPS {
            return None;
        }
        if pattern[index] == b']' {
            index += 1;
            break;
        }
        if pattern[index] == b'\\' && index + 1 < pattern.len() {
            // Escaped class members are literals, not the start of a range.
            matched |= pattern[index + 1] == byte;
            index += 2;
        } else if index + 2 < pattern.len() && pattern[index + 1] == b'-' {
            let first = pattern[index];
            let last = pattern[index + 2];
            let (low, high) = if first <= last {
                (first, last)
            } else {
                (last, first)
            };
            matched |= (low..=high).contains(&byte);
            index += 3;
        } else {
            matched |= pattern[index] == byte;
            index += 1;
        }
    }
    // Redis permits unterminated classes: the available members still match.
    Some((matched != negate, index))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicI64, Ordering};

    struct TestClock(AtomicI64);
    impl Clock for TestClock {
        fn now_ms(&self) -> i64 {
            self.0.load(Ordering::SeqCst)
        }
    }

    fn call(store: &Store, args: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        assert_eq!(store.execute(args, &mut out), Action::Continue);
        out
    }

    #[test]
    fn commands_are_binary_safe_and_touch_counts_duplicates() {
        let store = Store::new(100);
        assert_eq!(call(&store, &[b"TYPE", b"\xff\0"]), b"+none\r\n");
        store.set(
            b"\xff\0",
            b"v\0",
            SetOptions::default(),
            "set",
            false,
            &mut Vec::new(),
        );
        assert_eq!(call(&store, &[b"TYPE", b"\xff\0"]), b"+string\r\n");
        assert_eq!(
            call(&store, &[b"TOUCH", b"\xff\0", b"\xff\0", b"missing"]),
            b":2\r\n"
        );
        assert_eq!(call(&store, &[b"DBSIZE"]), b":1\r\n");
    }

    #[test]
    fn rename_preserves_ttl_and_oom_is_atomic() {
        let clock = Arc::new(TestClock(AtomicI64::new(1000)));
        let store = Store::with_clock(7, clock.clone());
        store.set(
            b"a",
            b"123",
            SetOptions {
                expiry: Some((ExpiryKind::Px, b"100")),
                ..SetOptions::default()
            },
            "set",
            false,
            &mut Vec::new(),
        );
        store.set(
            b"x",
            b"v",
            SetOptions::default(),
            "set",
            false,
            &mut Vec::new(),
        );
        assert!(call(&store, &[b"RENAME", b"a", b"long"]).starts_with(b"-OOM"));
        assert_eq!(store.data.read().unwrap().payload_bytes, 6);
        assert_eq!(call(&store, &[b"RENAME", b"a", b"b"]), b"+OK\r\n");
        assert_eq!(
            store
                .data
                .read()
                .unwrap()
                .entries
                .get(b"b".as_slice())
                .unwrap()
                .expires,
            Some(1100)
        );
        clock.0.store(1101, Ordering::SeqCst);
        assert_eq!(call(&store, &[b"DBSIZE"]), b":1\r\n");
    }

    #[test]
    fn rename_variants_handle_missing_existing_and_self_keys() {
        let store = Store::new(100);
        assert_eq!(
            call(&store, &[b"RENAMENX", b"missing", b"dst"]),
            b"-ERR no such key\r\n"
        );
        store.set(
            b"src",
            b"s",
            SetOptions::default(),
            "set",
            false,
            &mut Vec::new(),
        );
        store.set(
            b"dst",
            b"d",
            SetOptions::default(),
            "set",
            false,
            &mut Vec::new(),
        );
        assert_eq!(call(&store, &[b"RENAMENX", b"src", b"dst"]), b":0\r\n");
        assert_eq!(call(&store, &[b"RENAME", b"src", b"src"]), b"+OK\r\n");
        assert_eq!(call(&store, &[b"RENAMENX", b"src", b"src"]), b":0\r\n");
        assert_eq!(call(&store, &[b"RENAME", b"src", b"dst"]), b"+OK\r\n");
        assert_eq!(store.data.read().unwrap().payload_bytes, 4);
    }

    #[test]
    fn copy_is_deep_type_aware_expiry_preserving_and_atomic() {
        let clock = Arc::new(TestClock(AtomicI64::new(1000)));
        let store = Store::with_clock(25, clock.clone());
        assert_eq!(call(&store, &[b"RPUSH", b"src", b"one", b"two"]), b":2\r\n");
        assert_eq!(call(&store, &[b"PEXPIRE", b"src", b"5000"]), b":1\r\n");
        assert_eq!(call(&store, &[b"COPY", b"src", b"dst"]), b":1\r\n");
        assert_eq!(call(&store, &[b"COPY", b"src", b"dst"]), b":0\r\n");
        assert_eq!(call(&store, &[b"TYPE", b"dst"]), b"+list\r\n");
        assert_eq!(call(&store, &[b"PTTL", b"dst"]), b":5000\r\n");
        assert_eq!(call(&store, &[b"LPOP", b"dst"]), b"$3\r\none\r\n");
        assert_eq!(
            call(&store, &[b"LRANGE", b"src", b"0", b"-1"]),
            b"*2\r\n$3\r\none\r\n$3\r\ntwo\r\n"
        );
        assert_eq!(
            call(&store, &[b"COPY", b"src", b"dst", b"REPLACE"]),
            b":1\r\n"
        );
        assert_eq!(
            call(&store, &[b"COPY", b"src", b"src"]),
            b"-ERR source and destination objects are the same\r\n"
        );
        assert_eq!(call(&store, &[b"COPY", b"absent", b"other"]), b":0\r\n");
        assert_eq!(
            call(&store, &[b"COPY", b"src", b"other", b"DB", b"notanumber"]),
            options::NOT_INTEGER
        );
        assert_eq!(
            call(&store, &[b"COPY", b"src", b"other", b"DB", b"2"]),
            b"-ERR DB index is out of range\r\n"
        );
        assert_eq!(
            call(&store, &[b"COPY", b"src", b"dst", b"BAD"]),
            options::SYNTAX
        );
        assert!(
            call(&store, &[b"COPY", b"src", b"very-long-destination-name"]).starts_with(b"-OOM")
        );
        assert_eq!(
            call(&store, &[b"LRANGE", b"src", b"0", b"-1"]),
            b"*2\r\n$3\r\none\r\n$3\r\ntwo\r\n"
        );
        clock.0.store(6001, Ordering::SeqCst);
        assert_eq!(call(&store, &[b"TYPE", b"dst"]), b"+none\r\n");

        let atomic = Store::new(20);
        assert_eq!(call(&atomic, &[b"RPUSH", b"s", b"123456789"]), b":1\r\n");
        assert_eq!(call(&atomic, &[b"SET", b"dst", b"x"]), b"+OK\r\n");
        let before = atomic.data.read().unwrap().payload_bytes;
        assert!(call(&atomic, &[b"COPY", b"s", b"dst", b"REPLACE"]).starts_with(b"-OOM"));
        assert_eq!(atomic.data.read().unwrap().payload_bytes, before);
        assert_eq!(call(&atomic, &[b"GET", b"dst"]), b"$1\r\nx\r\n");
        assert_eq!(call(&atomic, &[b"TYPE", b"s"]), b"+list\r\n");
    }

    #[test]
    fn copy_reclaims_unrelated_expired_payload_before_reporting_oom() {
        let clock = Arc::new(TestClock(AtomicI64::new(1000)));
        let store = Store::with_clock(12, clock.clone());
        assert_eq!(call(&store, &[b"RPUSH", b"s", b"1234"]), b":1\r\n");
        assert_eq!(
            call(&store, &[b"SET", b"x", b"123456", b"PX", b"1"]),
            b"+OK\r\n"
        );
        clock.0.store(1002, Ordering::SeqCst);
        assert_eq!(call(&store, &[b"COPY", b"s", b"d"]), b":1\r\n");
        assert_eq!(
            call(&store, &[b"LRANGE", b"d", b"0", b"-1"]),
            b"*1\r\n$4\r\n1234\r\n"
        );
        assert_eq!(store.data.read().unwrap().payload_bytes, 10);
        assert!(store.data.read().unwrap().expirations.is_empty());
    }

    #[test]
    fn copy_clones_set_hash_and_sorted_set_contents_independently() {
        let store = Store::new(1024);
        assert_eq!(call(&store, &[b"SADD", b"set", b"one"]), b":1\r\n");
        assert_eq!(call(&store, &[b"COPY", b"set", b"set-copy"]), b":1\r\n");
        assert_eq!(call(&store, &[b"SADD", b"set", b"two"]), b":1\r\n");
        assert_eq!(
            call(&store, &[b"SMEMBERS", b"set-copy"]),
            b"*1\r\n$3\r\none\r\n"
        );

        assert_eq!(
            call(&store, &[b"HSET", b"hash", b"field", b"before"]),
            b":1\r\n"
        );
        assert_eq!(call(&store, &[b"COPY", b"hash", b"hash-copy"]), b":1\r\n");
        assert_eq!(
            call(&store, &[b"HSET", b"hash", b"field", b"after"]),
            b":0\r\n"
        );
        assert_eq!(
            call(&store, &[b"HGET", b"hash-copy", b"field"]),
            b"$6\r\nbefore\r\n"
        );

        assert_eq!(
            call(&store, &[b"ZADD", b"zset", b"1", b"member"]),
            b":1\r\n"
        );
        assert_eq!(call(&store, &[b"COPY", b"zset", b"zset-copy"]), b":1\r\n");
        assert_eq!(
            call(&store, &[b"ZADD", b"zset", b"2", b"member"]),
            b":0\r\n"
        );
        assert_eq!(
            call(&store, &[b"ZSCORE", b"zset-copy", b"member"]),
            b"$1\r\n1\r\n"
        );
    }

    #[test]
    fn keys_globs_are_binary_safe_and_pathological_inputs_are_bounded() {
        let store = Store::new(128_000);
        for key in [b"alpha".as_slice(), b"alps", b"a?", b"a*", b"a\\"] {
            store.set(
                key,
                b"v",
                SetOptions::default(),
                "set",
                false,
                &mut Vec::new(),
            );
        }
        assert_eq!(
            call(&store, &[b"KEYS", b"al*"]),
            b"*2\r\n$5\r\nalpha\r\n$4\r\nalps\r\n"
        );
        assert_eq!(
            call(&store, &[b"KEYS", b"a[lp]pha"]),
            b"*1\r\n$5\r\nalpha\r\n"
        );
        assert_eq!(call(&store, &[b"KEYS", b"a\\?"]), b"*1\r\n$2\r\na?\r\n");
        let long_key = vec![b'a'; 50_000];
        store.set(
            &long_key,
            b"v",
            SetOptions::default(),
            "set",
            false,
            &mut Vec::new(),
        );
        let mut expected = b"*1\r\n$50000\r\n".to_vec();
        expected.extend_from_slice(&long_key);
        expected.extend_from_slice(b"\r\n");
        assert_eq!(call(&store, &[b"KEYS", &long_key]), expected);
        let pattern = b"*?".repeat(50_000);
        assert_eq!(call(&store, &[b"KEYS", &pattern]), b"*0\r\n");
        assert!(!glob_match(
            b"a*a*a*a*a*a*a*a*a*b",
            b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        ));
    }

    #[test]
    fn glob_character_classes_follow_escaping_and_share_the_work_budget() {
        for (pattern, text, expected) in [
            (b"[a".as_slice(), b"a".as_slice(), true),
            (b"[a", b"b", false),
            (b"[^a", b"b", true),
            (b"[\\a-z]", b"b", false),
            (b"[\\a-z]", b"-", true),
            (b"[z-a]", b"b", true),
            (b"[\\]]", b"]", true),
            (b"[\x80-\xff]", b"\xfe", true),
            (b"a?", b"a\0", true),
        ] {
            assert_eq!(glob_match(pattern, text), expected, "{pattern:?} {text:?}");
        }
        let mut pattern = b"*[".to_vec();
        pattern.extend(std::iter::repeat_n(b'a', 10_000));
        pattern.push(b']');
        assert!(!glob_match(&pattern, &vec![b'b'; 10_000]));
        let mut budget = MAX_GLOB_STEPS;
        assert!(glob_class(b"[a]", 0, b'a', &mut budget).is_none());
    }

    #[test]
    fn randomkey_handles_empty_and_selects_beyond_the_first_key() {
        let store = Store::new(100);
        assert_eq!(call(&store, &[b"RANDOMKEY"]), b"$-1\r\n");
        for key in [b"foo".as_slice(), b"bar"] {
            store.set(
                key,
                b"v",
                SetOptions::default(),
                "set",
                false,
                &mut Vec::new(),
            );
        }
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..100 {
            let reply = call(&store, &[b"RANDOMKEY"]);
            if reply == b"$3\r\nfoo\r\n" {
                seen.insert(b"foo".to_vec());
            }
            if reply == b"$3\r\nbar\r\n" {
                seen.insert(b"bar".to_vec());
            }
        }
        assert_eq!(seen.len(), 2);
    }

    #[test]
    fn typed_entries_share_admin_string_guards_accounting_and_expiry() {
        let clock = Arc::new(TestClock(AtomicI64::new(1000)));
        let store = Store::with_clock(10_000, clock.clone());
        for create in [
            &[b"RPUSH".as_slice(), b"l", b"value"][..],
            &[b"SADD".as_slice(), b"s", b"value"][..],
            &[b"HSET".as_slice(), b"h", b"field", b"value"][..],
            &[b"ZADD".as_slice(), b"z", b"1", b"value"][..],
            &[b"XADD".as_slice(), b"x", b"1", b"field", b"value"][..],
        ] {
            assert!(!call(&store, create).starts_with(b"-"));
        }
        let header = if usize::BITS == 64 { 16 } else { 12 };
        for (key, bytes) in [(b"l", 5), (b"s", 5), (b"h", 10), (b"z", 5), (b"x", 10)] {
            assert_eq!(
                call(&store, &[b"MEMORY", b"USAGE", key]),
                format!(":{}\r\n", header + 1 + bytes).as_bytes()
            );
            for args in [
                &[b"GET".as_slice(), key][..],
                &[b"GETDEL".as_slice(), key][..],
                &[b"DIGEST".as_slice(), key][..],
                &[b"DELIFEQ".as_slice(), key, b"value"][..],
                &[b"SET".as_slice(), key, b"v", b"IFEQ", b"value"][..],
                &[b"SET".as_slice(), key, b"v", b"IFNE", b"value"][..],
                &[b"SET".as_slice(), key, b"v", b"IFDEQ", b"0000000000000000"][..],
                &[b"SET".as_slice(), key, b"v", b"IFDNE", b"0000000000000000"][..],
                &[b"GETEX".as_slice(), key, b"PERSIST"][..],
                &[b"SET".as_slice(), key, b"replacement", b"GET"][..],
                &[b"APPEND".as_slice(), key, b"suffix"][..],
                &[b"STRLEN".as_slice(), key][..],
                &[b"GETRANGE".as_slice(), key, b"0", b"-1"][..],
                &[b"SETRANGE".as_slice(), key, b"0", b"v"][..],
                &[b"INCR".as_slice(), key][..],
                &[b"INCRBYFLOAT".as_slice(), key, b"1"][..],
                &[b"GETBIT".as_slice(), key, b"0"][..],
                &[b"SETBIT".as_slice(), key, b"0", b"1"][..],
                &[b"LCS".as_slice(), key, b"missing"][..],
                &[b"LCS".as_slice(), b"missing", key][..],
            ] {
                assert!(call(&store, args).starts_with(b"-WRONGTYPE"), "{args:?}");
            }
            assert_eq!(
                call(&store, &[b"DELEX", key, b"IFEQ", b"value"]),
                b"-ERR Key should be of string type if conditions are specified\r\n"
            );
            assert_eq!(call(&store, &[b"PEXPIRE", key, b"10"]), b":1\r\n");
        }
        assert_eq!(
            call(&store, &[b"MGET", b"l", b"s", b"h", b"z", b"x"]),
            b"*5\r\n$-1\r\n$-1\r\n$-1\r\n$-1\r\n$-1\r\n"
        );
        {
            let data = store.data.read().unwrap();
            assert_eq!(data.payload_bytes, 40);
            for entry in data.entries.values() {
                let actual = if let Some(collection) = &entry.collection {
                    collection.payload_len()
                } else {
                    entry.stream.as_ref().unwrap().payload_bytes().unwrap()
                };
                assert_eq!(entry.payload_len(), actual);
            }
        }
        clock.0.store(1011, Ordering::SeqCst);
        assert_eq!(call(&store, &[b"DBSIZE"]), b":0\r\n");
        assert_eq!(store.data.read().unwrap().payload_bytes, 0);
        assert!(store.data.read().unwrap().expirations.is_empty());
        call(&store, &[b"RPUSH", b"l", b"value"]);
        call(&store, &[b"XADD", b"x", b"1", b"field", b"value"]);
        assert_eq!(call(&store, &[b"FLUSHALL", b"ASYNC"]), b"+OK\r\n");
        assert_eq!(store.data.read().unwrap().payload_bytes, 0);
        assert_eq!(call(&store, &[b"CONFIG", b"RESETSTAT"]), b"+OK\r\n");
    }

    #[test]
    fn msetex_replaces_collections_with_exact_ttl_accounting_and_atomic_oom() {
        let clock = Arc::new(TestClock(AtomicI64::new(1000)));
        let store = Store::with_clock(16, clock);
        assert_eq!(call(&store, &[b"RPUSH", b"a", b"1234"]), b":1\r\n");
        assert_eq!(call(&store, &[b"HSET", b"b", b"f", b"123"]), b":1\r\n");
        assert_eq!(call(&store, &[b"PEXPIRE", b"a", b"50"]), b":1\r\n");
        assert_eq!(call(&store, &[b"PEXPIRE", b"b", b"100"]), b":1\r\n");
        assert_eq!(
            call(&store, &[b"MSETEX", b"1", b"a", b"new", b"NX"]),
            b":0\r\n"
        );
        assert!(
            call(
                &store,
                &[
                    b"MSETEX",
                    b"2",
                    b"a",
                    b"x",
                    b"b",
                    b"1234567890123456",
                    b"XX"
                ]
            )
            .starts_with(b"-OOM")
        );
        assert_eq!(store.data.read().unwrap().payload_bytes, 10);
        assert_eq!(call(&store, &[b"TYPE", b"a"]), b"+list\r\n");
        assert_eq!(call(&store, &[b"TYPE", b"b"]), b"+hash\r\n");
        assert_eq!(call(&store, &[b"PTTL", b"a"]), b":50\r\n");
        assert_eq!(
            call(
                &store,
                &[
                    b"MSETEX", b"2", b"a", b"one", b"b", b"two", b"XX", b"KEEPTTL"
                ]
            ),
            b":1\r\n"
        );
        assert_eq!(store.data.read().unwrap().payload_bytes, 8);
        assert_eq!(
            call(&store, &[b"MGET", b"a", b"b"]),
            b"*2\r\n$3\r\none\r\n$3\r\ntwo\r\n"
        );
        assert_eq!(call(&store, &[b"PTTL", b"a"]), b":50\r\n");
        assert_eq!(call(&store, &[b"PTTL", b"b"]), b":100\r\n");
        assert_eq!(call(&store, &[b"SADD", b"s", b"item"]), b":1\r\n");
        assert_eq!(call(&store, &[b"DELEX", b"s"]), b":1\r\n");
        assert_eq!(store.data.read().unwrap().payload_bytes, 8);
    }

    #[test]
    fn unknown_command_leaves_output_untouched() {
        let store = Store::new(10);
        let mut output = b"prefix".to_vec();
        assert!(!store.execute_keyspace(&[b"NOTKEYSPACE"], &mut output));
        assert_eq!(output, b"prefix");
    }
}
