use super::options::{NOT_INTEGER, OOM, parse_integer};
use super::{Store, arity, integer, internal_error};
use crate::protocol::encode_bulk;

const MAX_STRING_BYTES: usize = 512 * 1024 * 1024;
const RANGE_LIMIT: &[u8] = b"-ERR string exceeds maximum allowed size (proto-max-bulk-len)\r\n";
const OFFSET_RANGE: &[u8] = b"-ERR offset is out of range\r\n";

impl Store {
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
        } else if command.eq_ignore_ascii_case(b"GETRANGE") {
            if arguments.len() != 4 {
                arity("getrange", output);
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
        let Ok(mut data) = self.data.write() else {
            internal_error(output);
            return;
        };
        let now = self.now(&data);
        data.expire_key(key, now);
        let entry = data.entries.get(key);
        let previous = entry.map(|entry| entry.value.len());
        let expires = entry.and_then(|entry| entry.expires);
        let Some(length) = previous
            .unwrap_or(0)
            .checked_add(suffix.len())
            .filter(|&n| n <= MAX_STRING_BYTES)
        else {
            output.extend_from_slice(RANGE_LIMIT);
            return;
        };
        let Some(payload) = data.reserve(key, length, previous, self.max_payload_bytes, now) else {
            output.extend_from_slice(OOM);
            return;
        };
        let mut value = Vec::new();
        if value.try_reserve_exact(length).is_err() {
            output.extend_from_slice(OOM);
            return;
        }
        if let Some(entry) = data.entries.get(key) {
            value.extend_from_slice(&entry.value);
        }
        value.extend_from_slice(suffix);
        data.put(key, &value, expires, payload);
        integer(length as i64, output);
    }

    fn strlen(&self, key: &[u8], output: &mut Vec<u8>) {
        let Ok(mut data) = self.data.write() else {
            internal_error(output);
            return;
        };
        let now = self.now(&data);
        data.expire_key(key, now);
        integer(
            data.entries.get(key).map_or(0, |entry| entry.value.len()) as i64,
            output,
        );
    }

    fn getrange(&self, key: &[u8], start: &[u8], end: &[u8], output: &mut Vec<u8>) {
        let (Some(start), Some(end)) = (parse_integer(start), parse_integer(end)) else {
            output.extend_from_slice(NOT_INTEGER);
            return;
        };
        let Ok(mut data) = self.data.write() else {
            internal_error(output);
            return;
        };
        let now = self.now(&data);
        data.expire_key(key, now);
        let value = data
            .entries
            .get(key)
            .map_or(&[][..], |entry| entry.value.as_slice());
        let len = value.len() as i128;
        let normalize = |i: i64| {
            if i < 0 {
                (len + i as i128).max(0)
            } else {
                i as i128
            }
        };
        let first = normalize(start);
        let last = normalize(end);
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
        let Ok(mut data) = self.data.write() else {
            internal_error(output);
            return;
        };
        let now = self.now(&data);
        data.expire_key(key, now);
        let entry = data.entries.get(key);
        let previous = entry.map(|entry| entry.value.len());
        let old_len = previous.unwrap_or(0);
        let expires = entry.and_then(|entry| entry.expires);
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
        let Some(payload) = data.reserve(key, length, previous, self.max_payload_bytes, now) else {
            output.extend_from_slice(OOM);
            return;
        };
        let mut value = Vec::new();
        if value.try_reserve_exact(length).is_err() {
            output.extend_from_slice(OOM);
            return;
        }
        if let Some(entry) = data.entries.get(key) {
            value.extend_from_slice(&entry.value);
        }
        value.resize(length, 0);
        value[offset..end].copy_from_slice(replacement);
        data.put(key, &value, expires, payload);
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
    fn append_and_setrange_keep_live_expiry() {
        let clock = Arc::new(TestClock(AtomicI64::new(1000)));
        let store = Store::with_clock(64, clock.clone());
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
        assert_eq!(store.data.read().unwrap().payload_bytes, 0);
    }
}
