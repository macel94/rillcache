use super::options::{NOT_INTEGER, OOM, parse_integer};
use super::{Store, arity, integer, internal_error, wrong_type};
use crate::protocol::encode_bulk;
use std::io::Write;

const MAX_STRING_BYTES: usize = 512 * 1024 * 1024;
const BIT_ERROR: &[u8] = b"-ERR bit is not an integer or out of range\r\n";
const OFFSET_ERROR: &[u8] = b"-ERR bit offset is not an integer or out of range\r\n";
const FLOAT_ERROR: &[u8] = b"-ERR value is not a valid float\r\n";
const LCS_LIMIT: usize = 4 * 1024 * 1024;

impl Store {
    pub(super) fn execute_string_algorithms(&self, args: &[&[u8]], out: &mut Vec<u8>) -> bool {
        let Some(command) = args.first() else {
            return false;
        };
        if command.eq_ignore_ascii_case(b"SETBIT") {
            if args.len() != 4 {
                arity("setbit", out)
            } else {
                self.setbit(args, out)
            }
        } else if command.eq_ignore_ascii_case(b"GETBIT") {
            if args.len() != 3 {
                arity("getbit", out)
            } else {
                self.getbit(args, out)
            }
        } else if command.eq_ignore_ascii_case(b"INCRBYFLOAT") {
            if args.len() != 3 {
                arity("incrbyfloat", out)
            } else {
                self.incrbyfloat(args, out)
            }
        } else if command.eq_ignore_ascii_case(b"LCS") {
            if args.len() < 3 {
                arity("lcs", out)
            } else {
                self.lcs(args, out)
            }
        } else {
            return false;
        }
        true
    }

    fn getbit(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let Some(offset) = parse_integer(args[2])
            .and_then(|n| u64::try_from(n).ok())
            .filter(|n| *n <= u32::MAX as u64)
        else {
            out.extend_from_slice(OFFSET_ERROR);
            return;
        };
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(args[1], now);
        if data
            .entries
            .get(args[1])
            .is_some_and(|entry| !entry.is_string())
        {
            wrong_type(out);
            return;
        }
        let byte = (offset / 8) as usize;
        let bit = 7 - (offset % 8) as u8;
        let value = data
            .entries
            .get(args[1])
            .and_then(|e| e.value.get(byte))
            .is_some_and(|v| v & (1 << bit) != 0);
        integer(i64::from(value), out);
    }

    fn setbit(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let Some(offset) = parse_integer(args[2])
            .and_then(|n| u64::try_from(n).ok())
            .filter(|n| *n <= u32::MAX as u64)
        else {
            out.extend_from_slice(OFFSET_ERROR);
            return;
        };
        let Some(value) = parse_integer(args[3]).filter(|n| *n == 0 || *n == 1) else {
            out.extend_from_slice(BIT_ERROR);
            return;
        };
        let byte = (offset / 8) as usize;
        let length = byte + 1;
        if length > MAX_STRING_BYTES {
            out.extend_from_slice(OFFSET_ERROR);
            return;
        }
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(args[1], now);
        let entry = data.entries.get(args[1]);
        if entry.is_some_and(|entry| !entry.is_string()) {
            wrong_type(out);
            return;
        }
        let previous = entry.map(|e| e.payload_len);
        let expires = entry.and_then(|e| e.expires);
        let old = entry.and_then(|e| e.value.get(byte)).copied().unwrap_or(0);
        let shift = 7 - (offset % 8) as u8;
        let old_bit = (old >> shift) & 1;
        let new_len = previous.unwrap_or(0).max(length);
        let Some(payload) = data.reserve(args[1], new_len, previous, self.max_payload_bytes, now)
        else {
            out.extend_from_slice(OOM);
            return;
        };
        let mut next = Vec::new();
        if next.try_reserve_exact(new_len).is_err() {
            out.extend_from_slice(OOM);
            return;
        }
        if let Some(entry) = data.entries.get(args[1]) {
            next.extend_from_slice(&entry.value);
        }
        next.resize(new_len, 0);
        next[byte] = (next[byte] & !(1 << shift)) | ((value as u8) << shift);
        data.put(args[1], &next, expires, payload);
        integer(i64::from(old_bit), out);
    }

    fn incrbyfloat(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let Ok(increment) = std::str::from_utf8(args[2])
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .ok_or(())
        else {
            out.extend_from_slice(FLOAT_ERROR);
            return;
        };
        if !increment.is_finite() {
            out.extend_from_slice(b"-ERR increment would produce NaN or Infinity\r\n");
            return;
        }
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(args[1], now);
        let entry = data.entries.get(args[1]);
        if entry.is_some_and(|entry| !entry.is_string()) {
            wrong_type(out);
            return;
        }
        let current = match entry {
            Some(entry) => match std::str::from_utf8(&entry.value)
                .ok()
                .and_then(|s| s.parse::<f64>().ok())
            {
                Some(v) if v.is_finite() => v,
                _ => {
                    out.extend_from_slice(FLOAT_ERROR);
                    return;
                }
            },
            None => 0.0,
        };
        let value = current + increment;
        if !value.is_finite() {
            out.extend_from_slice(b"-ERR increment would produce NaN or Infinity\r\n");
            return;
        }
        let formatted = if value == 0.0 {
            "0".to_owned()
        } else {
            value.to_string()
        };
        let previous = entry.map(|e| e.payload_len);
        let expires = entry.and_then(|e| e.expires);
        let Some(payload) = data.reserve(
            args[1],
            formatted.len(),
            previous,
            self.max_payload_bytes,
            now,
        ) else {
            out.extend_from_slice(OOM);
            return;
        };
        data.put(args[1], formatted.as_bytes(), expires, payload);
        encode_bulk(Some(formatted.as_bytes()), out);
    }

    fn lcs(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let mut len_only = false;
        let mut indexes = false;
        let mut with_len = false;
        let mut min_len = 0usize;
        let mut i = 3;
        while i < args.len() {
            if args[i].eq_ignore_ascii_case(b"LEN") {
                len_only = true;
            } else if args[i].eq_ignore_ascii_case(b"IDX") {
                indexes = true;
            } else if args[i].eq_ignore_ascii_case(b"WITHMATCHLEN") {
                with_len = true;
            } else if args[i].eq_ignore_ascii_case(b"MINMATCHLEN") && i + 1 < args.len() {
                let Some(n) = parse_integer(args[i + 1]) else {
                    out.extend_from_slice(NOT_INTEGER);
                    return;
                };
                min_len = usize::try_from(n.max(0)).unwrap_or(usize::MAX);
                i += 1;
            } else {
                out.extend_from_slice(b"-ERR syntax error\r\n");
                return;
            }
            i += 1;
        }
        if len_only && indexes {
            out.extend_from_slice(
                b"-ERR If you want both the length and indexes, please just use IDX.\r\n",
            );
            return;
        }
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(args[1], now);
        data.expire_key(args[2], now);
        if [args[1], args[2]].iter().any(|key| {
            data.entries
                .get(*key)
                .is_some_and(|entry| !entry.is_string())
        }) {
            wrong_type(out);
            return;
        }
        let a = data
            .entries
            .get(args[1])
            .map_or(&[][..], |e| e.value.as_slice());
        let b = data
            .entries
            .get(args[2])
            .map_or(&[][..], |e| e.value.as_slice());
        let Some(cells) = (a.len() + 1)
            .checked_mul(b.len() + 1)
            .filter(|n| *n <= LCS_LIMIT)
        else {
            out.extend_from_slice(b"-ERR Insufficient memory, transient memory for LCS exceeds proto-max-bulk-len\r\n");
            return;
        };
        let mut table = Vec::<u32>::new();
        if table.try_reserve_exact(cells).is_err() {
            out.extend_from_slice(
                b"-ERR Insufficient memory, failed allocating transient memory for LCS\r\n",
            );
            return;
        }
        table.resize(cells, 0);
        let width = b.len() + 1;
        for x in 1..=a.len() {
            for y in 1..=b.len() {
                let at = x * width + y;
                table[at] = if a[x - 1] == b[y - 1] {
                    table[at - width - 1] + 1
                } else {
                    table[at - width].max(table[at - 1])
                };
            }
        }
        let mut x = a.len();
        let mut y = b.len();
        let mut matches = Vec::new();
        let mut result = Vec::new();
        if matches
            .try_reserve_exact(table[a.len() * width + b.len()] as usize)
            .is_err()
            || result
                .try_reserve_exact(table[a.len() * width + b.len()] as usize)
                .is_err()
        {
            out.extend_from_slice(
                b"-ERR Insufficient memory, failed allocating transient memory for LCS\r\n",
            );
            return;
        }
        while x > 0 && y > 0 {
            if a[x - 1] == b[y - 1] {
                result.push(a[x - 1]);
                matches.push((x - 1, y - 1));
                x -= 1;
                y -= 1;
            } else if table[(x - 1) * width + y] > table[x * width + y - 1] {
                x -= 1
            } else {
                y -= 1
            }
        }
        result.reverse();
        matches.reverse();
        if len_only {
            integer(result.len() as i64, out);
            return;
        }
        if !indexes {
            encode_bulk(Some(&result), out);
            return;
        }
        let mut ranges: Vec<(usize, usize, usize)> = Vec::new();
        if ranges.try_reserve_exact(matches.len()).is_err() {
            out.extend_from_slice(
                b"-ERR Insufficient memory, failed allocating transient memory for LCS\r\n",
            );
            return;
        }
        for (x, y) in matches {
            if let Some(last) = ranges.last_mut()
                && last.0 + last.2 == x
                && last.1 + last.2 == y
            {
                last.2 += 1;
                continue;
            }
            ranges.push((x, y, 1));
        }
        ranges.retain(|r| r.2 >= min_len);
        let fields = if with_len { 3 } else { 2 };
        let _ = write!(out, "*4\r\n$7\r\nmatches\r\n*{}\r\n", ranges.len());
        for (a, b, n) in ranges.into_iter().rev() {
            let _ = write!(
                out,
                "*{}\r\n*2\r\n:{}\r\n:{}\r\n*2\r\n:{}\r\n:{}\r\n",
                fields,
                a,
                a + n - 1,
                b,
                b + n - 1
            );
            if with_len {
                let _ = write!(out, ":{}\r\n", n);
            }
        }
        let _ = write!(
            out,
            "$3\r\nlen\r\n:{}\r\n",
            table[a.len() * width + b.len()]
        );
    }
}
