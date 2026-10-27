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
        } else if command.eq_ignore_ascii_case(b"BITCOUNT") {
            if args.len() < 2 {
                arity("bitcount", out)
            } else {
                self.bitcount(args, out)
            }
        } else if command.eq_ignore_ascii_case(b"BITPOS") {
            self.bitpos(args, out)
        } else if command.eq_ignore_ascii_case(b"BITOP") {
            self.bitop(args, out)
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
        let Some(payload) = data.reserve(args[1], new_len, previous, self.payload_limit(), now)
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
        data.put(args[1], &next, expires, payload, now);
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
            self.payload_limit(),
            now,
        ) else {
            out.extend_from_slice(OOM);
            return;
        };
        data.put(args[1], formatted.as_bytes(), expires, payload, now);
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

const BIT_POS_ERROR: &[u8] = b"-ERR The bit argument must be 1 or 0.\r\n";
const BITOP_NOT_ERROR: &[u8] = b"-ERR BITOP NOT must be called with a single source key.\r\n";

#[derive(Clone, Copy, PartialEq, Eq)]
enum BitUnit {
    Byte,
    Bit,
}

fn parse_bit_unit(raw: &[u8]) -> Option<BitUnit> {
    if raw.eq_ignore_ascii_case(b"BYTE") {
        Some(BitUnit::Byte)
    } else if raw.eq_ignore_ascii_case(b"BIT") {
        Some(BitUnit::Bit)
    } else {
        None
    }
}

fn normalize_bit_range(start: i64, end: i64, length: usize) -> Option<(usize, usize)> {
    if length == 0 {
        return None;
    }
    let length = length as i128;
    let mut start = if start < 0 {
        length + start as i128
    } else {
        start as i128
    };
    let mut end = if end < 0 {
        length + end as i128
    } else {
        end as i128
    };
    start = start.max(0);
    end = end.min(length - 1);
    if start > end || start >= length || end < 0 {
        None
    } else {
        Some((start as usize, end as usize))
    }
}

fn count_bits(bytes: &[u8], start_bit: usize, end_bit: usize) -> u64 {
    let first = start_bit / 8;
    let last = end_bit / 8;
    if first >= bytes.len() {
        return 0;
    }
    let last = last.min(bytes.len() - 1);
    if first == last {
        let from = start_bit % 8;
        let to = end_bit % 8;
        let mask = (0xffu8 >> from) & (0xffu8 << (7 - to));
        return u64::from((bytes[first] & mask).count_ones());
    }
    let mut total = u64::from((bytes[first] & (0xffu8 >> (start_bit % 8))).count_ones());
    total += bytes[first + 1..last]
        .iter()
        .map(|byte| u64::from(byte.count_ones()))
        .sum::<u64>();
    total += u64::from((bytes[last] & (0xffu8 << (7 - (end_bit % 8)))).count_ones());
    total
}

fn find_bit(bytes: &[u8], bit: u8, start_bit: usize, end_bit: usize) -> Option<usize> {
    let first = start_bit / 8;
    let last = (end_bit / 8).min(bytes.len().checked_sub(1)?);
    if first > last {
        return None;
    }
    for (byte_index, byte) in bytes.iter().enumerate().take(last + 1).skip(first) {
        let from = if byte_index == first {
            start_bit % 8
        } else {
            0
        };
        let to = if byte_index == last { end_bit % 8 } else { 7 };
        let byte = *byte;
        for offset in from..=to {
            let value = (byte >> (7 - offset)) & 1;
            if value == bit {
                return Some(byte_index * 8 + offset);
            }
        }
    }
    None
}

impl Store {
    fn bitcount(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let (start, end, unit) = match args.len() {
            2 => (None, None, BitUnit::Byte),
            4 | 5 => {
                let (Some(start), Some(end)) = (parse_integer(args[2]), parse_integer(args[3]))
                else {
                    out.extend_from_slice(NOT_INTEGER);
                    return;
                };
                let unit = if args.len() == 5 {
                    let Some(unit) = parse_bit_unit(args[4]) else {
                        out.extend_from_slice(b"-ERR syntax error\r\n");
                        return;
                    };
                    unit
                } else {
                    BitUnit::Byte
                };
                (Some(start), Some(end), unit)
            }
            _ => {
                out.extend_from_slice(b"-ERR syntax error\r\n");
                return;
            }
        };
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(args[1], now);
        let bytes = match data.entries.get(args[1]) {
            None => {
                integer(0, out);
                return;
            }
            Some(entry) if entry.is_string() => entry.value.as_slice(),
            Some(_) => {
                wrong_type(out);
                return;
            }
        };
        let (Some(start), Some(end)) = (start, end) else {
            let count: u64 = bytes.iter().map(|byte| u64::from(byte.count_ones())).sum();
            integer(count as i64, out);
            return;
        };
        let length = if unit == BitUnit::Byte {
            bytes.len()
        } else {
            bytes.len().saturating_mul(8)
        };
        let Some((start, end)) = normalize_bit_range(start, end, length) else {
            integer(0, out);
            return;
        };
        let count = if unit == BitUnit::Byte {
            bytes[start..=end]
                .iter()
                .map(|byte| u64::from(byte.count_ones()))
                .sum::<u64>()
        } else {
            count_bits(bytes, start, end)
        };
        integer(count as i64, out);
    }

    fn bitpos(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        if args.len() < 3 {
            arity("bitpos", out);
            return;
        }
        if args.len() > 6 {
            out.extend_from_slice(b"-ERR syntax error\r\n");
            return;
        }
        let bit = match parse_integer(args[2]) {
            Some(0) => 0,
            Some(1) => 1,
            Some(_) => {
                out.extend_from_slice(BIT_POS_ERROR);
                return;
            }
            None => {
                out.extend_from_slice(NOT_INTEGER);
                return;
            }
        };
        let (start, end, unit) = match args.len() {
            3 => (None, None, BitUnit::Byte),
            4 => {
                let Some(start) = parse_integer(args[3]) else {
                    out.extend_from_slice(NOT_INTEGER);
                    return;
                };
                (Some(start), None, BitUnit::Byte)
            }
            5 | 6 => {
                let (Some(start), Some(end)) = (parse_integer(args[3]), parse_integer(args[4]))
                else {
                    out.extend_from_slice(NOT_INTEGER);
                    return;
                };
                let unit = if args.len() == 6 {
                    let Some(unit) = parse_bit_unit(args[5]) else {
                        out.extend_from_slice(b"-ERR syntax error\r\n");
                        return;
                    };
                    unit
                } else {
                    BitUnit::Byte
                };
                (Some(start), Some(end), unit)
            }
            _ => unreachable!(),
        };
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(args[1], now);
        let bytes = match data.entries.get(args[1]) {
            None => {
                integer(if bit == 0 { 0 } else { -1 }, out);
                return;
            }
            Some(entry) if entry.is_string() => entry.value.as_slice(),
            Some(_) => {
                wrong_type(out);
                return;
            }
        };
        let units = if unit == BitUnit::Byte {
            bytes.len()
        } else {
            bytes.len().saturating_mul(8)
        };
        if units == 0 {
            integer(-1, out);
            return;
        }
        let start_unit = match start {
            None => 0,
            Some(start) => {
                if start < 0 {
                    (units as i128 + start as i128).max(0) as usize
                } else if start as u128 >= units as u128 {
                    integer(-1, out);
                    return;
                } else {
                    start as usize
                }
            }
        };
        if start_unit >= units {
            integer(-1, out);
            return;
        }
        let (end_unit, explicit_end) = match end {
            None => (units - 1, false),
            Some(end) => {
                let end = if end < 0 {
                    (units as i128 + end as i128).max(0) as usize
                } else {
                    (end as u128).min((units - 1) as u128) as usize
                };
                if end < start_unit {
                    integer(-1, out);
                    return;
                }
                (end, true)
            }
        };
        let (start_bit, end_bit) = if unit == BitUnit::Byte {
            (start_unit * 8, end_unit * 8 + 7)
        } else {
            (start_unit, end_unit)
        };
        match find_bit(bytes, bit, start_bit, end_bit) {
            Some(position) => integer(position as i64, out),
            None if bit == 0 && !explicit_end => integer(bytes.len().saturating_mul(8) as i64, out),
            None => integer(-1, out),
        }
    }

    fn bitop(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        if args.len() < 4 {
            arity("bitop", out);
            return;
        }
        let operation = args[1];
        let not = operation.eq_ignore_ascii_case(b"NOT");
        if !not
            && !operation.eq_ignore_ascii_case(b"AND")
            && !operation.eq_ignore_ascii_case(b"OR")
            && !operation.eq_ignore_ascii_case(b"XOR")
        {
            out.extend_from_slice(b"-ERR syntax error\r\n");
            return;
        }
        if not && args.len() != 4 {
            out.extend_from_slice(BITOP_NOT_ERROR);
            return;
        }
        let destination = args[2];
        let sources = &args[3..];
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(destination, now);
        for source in sources {
            data.expire_key(source, now);
        }
        let mut values = Vec::with_capacity(sources.len());
        for source in sources {
            match data.entries.get(*source) {
                None => values.push(None),
                Some(entry) if entry.is_string() => values.push(Some(entry.value.as_slice())),
                Some(_) => {
                    wrong_type(out);
                    return;
                }
            }
        }
        let result = if not {
            let source = values[0].unwrap_or(&[]);
            source.iter().map(|byte| !byte).collect::<Vec<u8>>()
        } else {
            let length = values
                .iter()
                .map(|value| value.map_or(0, <[u8]>::len))
                .max()
                .unwrap_or(0);
            let and = operation.eq_ignore_ascii_case(b"AND");
            let mut result = Vec::with_capacity(length);
            for index in 0..length {
                let mut byte = if and { 0xff } else { 0 };
                for value in &values {
                    let source = value
                        .and_then(|value| value.get(index).copied())
                        .unwrap_or(0);
                    if and {
                        byte &= source;
                    } else if operation.eq_ignore_ascii_case(b"OR") {
                        byte |= source;
                    } else {
                        byte ^= source;
                    }
                }
                result.push(byte);
            }
            result
        };
        if result.is_empty() {
            data.remove(destination);
            integer(0, out);
            return;
        }
        let previous = data
            .entries
            .get(destination)
            .map(|entry| entry.payload_len());
        let Some(payload) = data.reserve(
            destination,
            result.len(),
            previous,
            self.payload_limit(),
            now,
        ) else {
            out.extend_from_slice(OOM);
            return;
        };
        data.put(destination, &result, None, payload, now);
        integer(result.len() as i64, out);
    }
}
