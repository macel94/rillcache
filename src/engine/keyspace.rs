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
        let destination_bytes = data
            .entries
            .get(destination)
            .map_or(0, |entry| destination.len() + entry.payload_len());
        let Some(payload) = data
            .payload_bytes
            .checked_sub(destination_bytes)
            .and_then(|value| value.checked_add(destination.len()))
            .and_then(|value| value.checked_add(source_payload))
            .filter(|value| *value <= self.max_payload_bytes)
        else {
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
                    if let Some((matched, after)) = glob_class(pattern, p, text[s]) {
                        if matched {
                            p = after;
                            s += 1;
                            continue;
                        }
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
        if let Some(star) = retry_pattern {
            if retry_text < text.len() {
                retry_text += 1;
                s = retry_text;
                p = star;
                continue;
            }
        }
        return false;
    }
}

fn glob_class(pattern: &[u8], start: usize, byte: u8) -> Option<(bool, usize)> {
    let mut index = start + 1;
    let negate = index < pattern.len() && pattern[index] == b'^';
    if negate {
        index += 1;
    }
    let mut matched = false;
    let mut closed = false;
    while index < pattern.len() {
        if pattern[index] == b']' {
            index += 1;
            closed = true;
            break;
        }
        let first = if pattern[index] == b'\\' && index + 1 < pattern.len() {
            index += 1;
            pattern[index]
        } else {
            pattern[index]
        };
        if index + 2 < pattern.len() && pattern[index + 1] == b'-' {
            let last = pattern[index + 2];
            let (low, high) = if first <= last {
                (first, last)
            } else {
                (last, first)
            };
            matched |= (low..=high).contains(&byte);
            index += 3;
        } else {
            matched |= first == byte;
            index += 1;
        }
    }
    closed.then_some((matched != negate, index))
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
        let pattern = b"*?".repeat(50_000);
        assert_eq!(call(&store, &[b"KEYS", &pattern]), b"*0\r\n");
        assert_eq!(
            glob_match(
                b"a*a*a*a*a*a*a*a*a*b",
                b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            ),
            false
        );
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
    fn unknown_command_leaves_output_untouched() {
        let store = Store::new(10);
        let mut output = b"prefix".to_vec();
        assert!(!store.execute_keyspace(&[b"NOTKEYSPACE"], &mut output));
        assert_eq!(output, b"prefix");
    }
}
