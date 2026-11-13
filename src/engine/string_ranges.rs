use super::data::Entry;
use super::options::{NOT_INTEGER, OOM, parse_integer};
use super::{ShardExecutor, arity, integer, internal_error, wrong_type};
use crate::protocol::encode_bulk;

const MAX_STRING_BYTES: usize = 512 * 1024 * 1024;
const RANGE_LIMIT: &[u8] = b"-ERR string exceeds maximum allowed size (proto-max-bulk-len)\r\n";
const OFFSET_RANGE: &[u8] = b"-ERR offset is out of range\r\n";

impl ShardExecutor {
    /// Handles byte-string length, append, indexed ranges and replacement.
    pub(super) fn execute_string_ranges(&self, arguments: &[&[u8]], output: &mut Vec<u8>) -> bool {
        let Some(command) = arguments.first() else {
            return false;
        };
        if command.eq_ignore_ascii_case(b"APPEND") {
            if arguments.len() != 3 {
                arity("append", output);
            } else {
                self.append(arguments[1], arguments[2], output);
            }
        } else if command.eq_ignore_ascii_case(b"STRLEN") {
            if arguments.len() != 2 {
                arity("strlen", output);
            } else {
                self.strlen(arguments[1], output);
            }
        } else if command.eq_ignore_ascii_case(b"GETRANGE")
            || command.eq_ignore_ascii_case(b"SUBSTR")
        {
            if arguments.len() != 4 {
                arity(
                    if command.eq_ignore_ascii_case(b"SUBSTR") {
                        "substr"
                    } else {
                        "getrange"
                    },
                    output,
                );
            } else {
                self.getrange(arguments[1], arguments[2], arguments[3], output);
            }
        } else if command.eq_ignore_ascii_case(b"SETRANGE") {
            if arguments.len() != 4 {
                arity("setrange", output);
            } else {
                self.setrange(arguments[1], arguments[2], arguments[3], output);
            }
        } else {
            return false;
        }
        true
    }

    fn append(&self, key: &[u8], suffix: &[u8], output: &mut Vec<u8>) {
        let Ok(mut data) = self.data.try_borrow_mut() else {
            internal_error(output);
            return;
        };
        let now = self.now(&data);
        data.expire_key(key, now);
        if data
            .entries
            .get(key)
            .is_some_and(|entry| !entry.is_string())
        {
            wrong_type(output);
            return;
        }
        let entry = data.entries.get(key);
        let previous = entry.map(Entry::payload_len);
        let Some(length) = previous
            .unwrap_or(0)
            .checked_add(suffix.len())
            .filter(|&n| n <= MAX_STRING_BYTES)
        else {
            output.extend_from_slice(RANGE_LIMIT);
            return;
        };
        let Some(payload) = data.reserve(key, length, previous, self.payload_limit(), now) else {
            output.extend_from_slice(OOM);
            return;
        };
        if data
            .mutate_string(key, length, payload, self.payload_limit(), |value| {
                value.extend_from_slice(suffix);
            })
            .is_err()
        {
            output.extend_from_slice(OOM);
            return;
        }
        integer(length as i64, output);
    }

    fn strlen(&self, key: &[u8], output: &mut Vec<u8>) {
        let Ok(mut data) = self.data.try_borrow_mut() else {
            internal_error(output);
            return;
        };
        let now = self.now(&data);
        data.expire_key(key, now);
        let Some(entry) = data.entries.get(key) else {
            integer(0, output);
            return;
        };
        if !entry.is_string() {
            wrong_type(output);
            return;
        }
        integer(entry.value.len() as i64, output);
    }

    fn getrange(&self, key: &[u8], start: &[u8], end: &[u8], output: &mut Vec<u8>) {
        let (Some(start), Some(end)) = (parse_integer(start), parse_integer(end)) else {
            output.extend_from_slice(NOT_INTEGER);
            return;
        };
        let Ok(mut data) = self.data.try_borrow_mut() else {
            internal_error(output);
            return;
        };
        let now = self.now(&data);
        data.expire_key(key, now);
        if data
            .entries
            .get(key)
            .is_some_and(|entry| !entry.is_string())
        {
            wrong_type(output);
            return;
        }
        let value = data
            .entries
            .get(key)
            .map_or(&[][..], |entry| entry.value.as_slice());
        let len = value.len() as i128;
        if start < 0 && end < 0 && start > end {
            encode_bulk(Some(&[]), output);
            return;
        }
        let first = if start < 0 {
            (len + start as i128).max(0)
        } else {
            start as i128
        };
        let last = if end < 0 {
            (len + end as i128).max(0)
        } else {
            end as i128
        };
        if first >= len || first > last || len == 0 {
            encode_bulk(Some(&[]), output);
            return;
        }
        let last = last.min(len - 1);
        encode_bulk(Some(&value[first as usize..=last as usize]), output);
    }

    fn setrange(&self, key: &[u8], raw_offset: &[u8], replacement: &[u8], output: &mut Vec<u8>) {
        let Some(offset) = parse_integer(raw_offset) else {
            output.extend_from_slice(NOT_INTEGER);
            return;
        };
        let Ok(offset) = usize::try_from(offset) else {
            output.extend_from_slice(OFFSET_RANGE);
            return;
        };
        let Ok(mut data) = self.data.try_borrow_mut() else {
            internal_error(output);
            return;
        };
        let now = self.now(&data);
        data.expire_key(key, now);
        if data
            .entries
            .get(key)
            .is_some_and(|entry| !entry.is_string())
        {
            wrong_type(output);
            return;
        }
        let entry = data.entries.get(key);
        let previous = entry.map(Entry::payload_len);
        let old_len = previous.unwrap_or(0);
        if replacement.is_empty() {
            integer(old_len as i64, output);
            return;
        }
        let Some(end) = offset
            .checked_add(replacement.len())
            .filter(|&n| n <= MAX_STRING_BYTES)
        else {
            output.extend_from_slice(RANGE_LIMIT);
            return;
        };
        let length = old_len.max(end);
        let Some(payload) = data.reserve(key, length, previous, self.payload_limit(), now) else {
            output.extend_from_slice(OOM);
            return;
        };
        if data
            .mutate_string(key, length, payload, self.payload_limit(), |value| {
                value.resize(length, 0);
                value[offset..end].copy_from_slice(replacement);
            })
            .is_err()
        {
            output.extend_from_slice(OOM);
            return;
        }
        integer(length as i64, output);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Clock;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicI64, Ordering};
    struct TestClock(AtomicI64);
    impl Clock for TestClock {
        fn now_ms(&self) -> i64 {
            self.0.load(Ordering::SeqCst)
        }
    }
    #[test]
    fn in_place_mutation_reuses_capacity_and_updates_metadata_once() {
        let store = ShardExecutor::with_clock(4096, Arc::new(TestClock(AtomicI64::new(1000))));
        let mut out = Vec::new();
        store.execute(&[b"SET", b"k", &[b'a'; 32], b"PX", b"100"], &mut out);
        assert_eq!(out, b"+OK\r\n");
        let mut grows = 0;
        for length in (40..=1632).step_by(8) {
            let (old_capacity, old_version, old_access) = {
                let data = store.data.borrow();
                let entry = &data.entries[b"k".as_slice()];
                (
                    entry.value.capacity(),
                    data.version(b"k").unwrap(),
                    entry.last_access,
                )
            };
            out.clear();
            store.execute_string_ranges(&[b"APPEND", b"k", b"\0\xffabcdef"], &mut out);
            assert_eq!(out, format!(":{length}\r\n").as_bytes());
            let data = store.data.borrow();
            let entry = &data.entries[b"k".as_slice()];
            grows += usize::from(entry.value.capacity() != old_capacity);
            assert_eq!(entry.payload_len, length);
            assert_eq!(data.payload_bytes, 1 + length);
            assert_eq!(data.version(b"k"), Some(old_version + 1));
            assert!(entry.last_access > old_access);
            assert_eq!(entry.expires, Some(1100));
            assert!(data.expirations.contains(&(1100, b"k".to_vec())));
        }
        assert!(grows <= 6, "growth must be amortized, not per append");
        let (pointer, capacity, version) = {
            let data = store.data.borrow();
            let value = &data.entries[b"k".as_slice()].value;
            (
                value.as_ptr(),
                value.capacity(),
                data.version(b"k").unwrap(),
            )
        };
        out.clear();
        store.execute_string_ranges(&[b"SETRANGE", b"k", b"10", b"\xff\0"], &mut out);
        assert_eq!(out, b":1632\r\n");
        let data = store.data.borrow();
        let entry = &data.entries[b"k".as_slice()];
        assert_eq!(entry.value.as_ptr(), pointer);
        assert_eq!(entry.value.capacity(), capacity);
        assert_eq!(&entry.value[10..12], b"\xff\0");
        assert_eq!(data.payload_bytes, 1633);
        assert_eq!(data.version(b"k"), Some(version + 1));
    }

    #[test]
    fn growth_spare_capacity_is_bounded_and_rejections_do_not_touch_the_target() {
        let store = ShardExecutor::with_clock(97, Arc::new(TestClock(AtomicI64::new(1000))));
        let mut out = Vec::new();
        store.execute(&[b"SET", b"k", &[b'a'; 32]], &mut out);
        out.clear();
        store.execute_string_ranges(&[b"APPEND", b"k", &[b'b'; 32]], &mut out);
        assert_eq!(out, b":64\r\n");
        out.clear();
        store.execute_string_ranges(&[b"APPEND", b"k", &[b'c'; 8]], &mut out);
        assert_eq!(out, b":72\r\n");
        let (before, version) = {
            let data = store.data.borrow();
            let entry = &data.entries[b"k".as_slice()];
            assert!(entry.value.capacity() <= 96);
            (entry.clone(), data.version(b"k"))
        };
        for (args, reply) in [
            (vec![b"APPEND".as_slice(), b"k", &[b'x'; 25]], OOM),
            (vec![b"SETRANGE".as_slice(), b"k", b"90", b"1234567"], OOM),
            (
                vec![b"SETRANGE".as_slice(), b"k", b"536870912", b"x"],
                RANGE_LIMIT,
            ),
        ] {
            out.clear();
            store.execute_string_ranges(&args, &mut out);
            assert_eq!(out, reply);
            let data = store.data.borrow();
            assert_eq!(&data.entries[b"k".as_slice()], &before);
            assert_eq!(data.version(b"k"), version);
            assert_eq!(data.payload_bytes, 73);
        }
    }

    #[test]
    fn growth_eviction_preserves_target_and_reports_journal_and_rewrite_changes() {
        let store = ShardExecutor::with_clock(97, Arc::new(TestClock(AtomicI64::new(1000))));
        assert!(store.set_eviction_policy(super::super::EvictionPolicy::AllKeysLru));
        let mut out = Vec::new();
        store.execute(&[b"SET", b"k", &[b'a'; 64], b"PX", b"500"], &mut out);
        assert_eq!(out, b"+OK\r\n");
        out.clear();
        store.execute(&[b"SET", b"victim", &[b'b'; 24]], &mut out);
        assert_eq!(out, b"+OK\r\n");
        let sequence = {
            let mut data = store.data.borrow_mut();
            data.set_collect_events(true);
            data.begin_rewrite_tracking();
            data.journal_seq()
        };
        out.clear();
        store.execute_string_ranges(&[b"APPEND", b"k", &[b'c'; 16]], &mut out);
        assert_eq!(out, b":80\r\n");
        let mut data = store.data.borrow_mut();
        assert_eq!(data.entries.len(), 1);
        let entry = &data.entries[b"k".as_slice()];
        assert_eq!(&entry.value[..64], &[b'a'; 64]);
        assert_eq!(&entry.value[64..], &[b'c'; 16]);
        assert_eq!(entry.expires, Some(1500));
        assert_eq!(entry.payload_len, 80);
        assert_eq!(data.payload_bytes, 81);
        let keys = vec![b"k".to_vec(), b"victim".to_vec()];
        assert_eq!(data.changes_since(sequence), (keys.clone(), false));
        assert_eq!(data.take_rewrite_changes(), (keys, false));
    }

    #[test]
    fn fallible_capacity_failure_preserves_existing_and_missing_keys() {
        let mut data = super::super::data::Data::default();
        data.set_collect_events(true);
        data.put(b"k", b"original", Some(123), 9, 0);
        let before = data.entries[b"k".as_slice()].clone();
        let version = data.version(b"k");
        let sequence = data.journal_seq();
        data.begin_rewrite_tracking();
        for key in [b"k".as_slice(), b"absent"] {
            assert!(
                data.mutate_string(key, usize::MAX, usize::MAX, usize::MAX, |_| panic!(
                    "allocation must succeed before mutation"
                ))
                .is_err()
            );
            assert_eq!(&data.entries[b"k".as_slice()], &before);
            assert_eq!(data.entries.len(), 1);
            assert_eq!(data.version(b"k"), version);
            assert_eq!(data.version(b"absent"), None);
            assert_eq!(data.payload_bytes, 9);
            assert_eq!(data.journal_seq(), sequence);
            assert_eq!(data.changes_since(sequence), (vec![], false));
            assert_eq!(data.take_rewrite_changes(), (vec![], false));
            assert!(data.expirations.contains(&(123, b"k".to_vec())));
        }
    }

    #[test]
    fn append_and_setrange_keep_live_expiry() {
        let clock = Arc::new(TestClock(AtomicI64::new(1000)));
        let store = ShardExecutor::with_clock(64, clock.clone());
        let mut out = Vec::new();
        store.execute(&[b"SET", b"k", b"a", b"PX", b"5"], &mut out);
        assert_eq!(out, b"+OK\r\n");
        out.clear();
        assert!(store.execute_string_ranges(&[b"APPEND", b"k", b"b"], &mut out));
        assert_eq!(out, b":2\r\n");
        out.clear();
        store.execute_string_ranges(&[b"SETRANGE", b"k", b"1", b"c"], &mut out);
        assert_eq!(out, b":2\r\n");
        clock.0.store(1006, Ordering::SeqCst);
        out.clear();
        store.execute_string_ranges(&[b"STRLEN", b"k"], &mut out);
        assert_eq!(out, b":0\r\n");
        assert_eq!(store.data.try_borrow().unwrap().payload_bytes, 0);
    }
}
