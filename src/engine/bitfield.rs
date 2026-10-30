//! Atomic arbitrary-width bitfield operations over string values.
//!
//! `BITFIELD` parses every subcommand and overflow policy before touching the
//! key, then executes the operations left-to-right against one owned value. The
//! final value is payload-admitted once, so a later syntax/offset/admission error
//! cannot expose a partially written bitfield.

use super::options::{NOT_INTEGER, OOM, parse_integer};
use super::{ShardExecutor, arity, integer, internal_error, wrong_type};
use std::io::Write;

const MAX_STRING_BYTES: usize = 512 * 1024 * 1024;
const OFFSET_ERROR: &[u8] = b"-ERR bit offset is not an integer or out of range\r\n";
const TYPE_ERROR: &[u8] = b"-ERR Invalid bitfield type. Use something like i16 u8. Note that u64 is not supported but i64 is.\r\n";
const OVERFLOW_ERROR: &[u8] = b"-ERR Invalid OVERFLOW type specified\r\n";
const SYNTAX_ERROR: &[u8] = b"-ERR syntax error\r\n";
const RO_ERROR: &[u8] = b"-ERR BITFIELD_RO only supports the GET subcommand\r\n";
const MAX_OFFSET: u64 = u32::MAX as u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Overflow {
    Wrap,
    Sat,
    Fail,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Field {
    width: u32,
    signed: bool,
}

impl Field {
    fn bounds(self) -> (i128, i128) {
        if self.signed {
            let limit = 1i128 << (self.width - 1);
            (-limit, limit - 1)
        } else {
            (0, (1i128 << self.width) - 1)
        }
    }

    fn wrap(self, value: i128) -> i128 {
        let modulus = 1i128 << self.width;
        let mut wrapped = value.rem_euclid(modulus);
        if self.signed && wrapped >= (1i128 << (self.width - 1)) {
            wrapped -= modulus;
        }
        wrapped
    }

    fn apply(self, value: i128, overflow: Overflow) -> Option<i128> {
        let (minimum, maximum) = self.bounds();
        if (minimum..=maximum).contains(&value) {
            return Some(value);
        }
        match overflow {
            Overflow::Wrap => Some(self.wrap(value)),
            Overflow::Sat => Some(value.clamp(minimum, maximum)),
            Overflow::Fail => None,
        }
    }

    fn read(self, bytes: &[u8], offset: u64) -> i64 {
        let mut bits = 0u64;
        for index in 0..self.width {
            let position = offset + u64::from(index);
            let byte = position / 8;
            let bit = (position % 8) as u8;
            let value = bytes
                .get(byte as usize)
                .map_or(0, |byte| (byte >> (7 - bit)) & 1);
            bits = (bits << 1) | u64::from(value);
        }
        if self.signed && self.width < 64 && (bits & (1 << (self.width - 1))) != 0 {
            (i128::from(bits) - (1i128 << self.width)) as i64
        } else {
            bits as i64
        }
    }

    fn write(self, bytes: &mut [u8], offset: u64, value: i128) {
        let modulus = 1i128 << self.width;
        let bits = value.rem_euclid(modulus) as u64;
        for index in 0..self.width {
            let position = offset + u64::from(index);
            let byte = position / 8;
            let bit = (position % 8) as u8;
            let mask = 1u8 << (7 - bit);
            let next = ((bits >> (self.width - 1 - index)) & 1) as u8;
            if next == 0 {
                bytes[byte as usize] &= !mask;
            } else {
                bytes[byte as usize] |= mask;
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Get {
        field: Field,
        offset: u64,
    },
    Set {
        field: Field,
        offset: u64,
        value: i64,
        overflow: Overflow,
    },
    Incr {
        field: Field,
        offset: u64,
        increment: i64,
        overflow: Overflow,
    },
}

impl Op {
    fn is_write(self) -> bool {
        !matches!(self, Self::Get { .. })
    }
}

fn parse_field(raw: &[u8]) -> Option<Field> {
    let (signed, digits) = match raw.first()? {
        b'i' | b'I' => (true, &raw[1..]),
        b'u' | b'U' => (false, &raw[1..]),
        _ => return None,
    };
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let width = std::str::from_utf8(digits).ok()?.parse::<u32>().ok()?;
    if width == 0 || (signed && width > 64) || (!signed && width > 63) {
        return None;
    }
    Some(Field { width, signed })
}

fn parse_offset(raw: &[u8], width: u32) -> Option<u64> {
    let (raw, multiplied) = if raw.first() == Some(&b'#') {
        (&raw[1..], true)
    } else {
        (raw, false)
    };
    let value = parse_integer(raw)?;
    if value < 0 {
        return None;
    }
    let offset = if multiplied {
        (value as u64).checked_mul(u64::from(width))?
    } else {
        value as u64
    };
    (offset <= MAX_OFFSET).then_some(offset)
}

fn parse_overflow(raw: &[u8]) -> Option<Overflow> {
    if raw.eq_ignore_ascii_case(b"WRAP") {
        Some(Overflow::Wrap)
    } else if raw.eq_ignore_ascii_case(b"SAT") {
        Some(Overflow::Sat)
    } else if raw.eq_ignore_ascii_case(b"FAIL") {
        Some(Overflow::Fail)
    } else {
        None
    }
}

fn parse_operations(args: &[&[u8]], read_only: bool) -> Result<Vec<Op>, &'static [u8]> {
    let mut operations = Vec::new();
    let mut overflow = Overflow::Wrap;
    let mut index = 2;
    while index < args.len() {
        let token = args[index];
        if token.eq_ignore_ascii_case(b"OVERFLOW") {
            let Some(value) = args.get(index + 1) else {
                return Err(SYNTAX_ERROR);
            };
            overflow = parse_overflow(value).ok_or(OVERFLOW_ERROR)?;
            index += 2;
        } else if token.eq_ignore_ascii_case(b"GET") {
            let Some(type_raw) = args.get(index + 1) else {
                return Err(SYNTAX_ERROR);
            };
            let field = parse_field(type_raw).ok_or(TYPE_ERROR)?;
            let Some(offset_raw) = args.get(index + 2) else {
                return Err(SYNTAX_ERROR);
            };
            let offset = parse_offset(offset_raw, field.width).ok_or(OFFSET_ERROR)?;
            operations.push(Op::Get { field, offset });
            index += 3;
        } else if token.eq_ignore_ascii_case(b"SET") || token.eq_ignore_ascii_case(b"INCRBY") {
            if read_only {
                return Err(RO_ERROR);
            }
            let increment = token.eq_ignore_ascii_case(b"INCRBY");
            let Some(type_raw) = args.get(index + 1) else {
                return Err(SYNTAX_ERROR);
            };
            let field = parse_field(type_raw).ok_or(TYPE_ERROR)?;
            let Some(offset_raw) = args.get(index + 2) else {
                return Err(SYNTAX_ERROR);
            };
            let offset = parse_offset(offset_raw, field.width).ok_or(OFFSET_ERROR)?;
            let Some(value_raw) = args.get(index + 3) else {
                return Err(SYNTAX_ERROR);
            };
            let value = parse_integer(value_raw).ok_or(NOT_INTEGER)?;
            operations.push(if increment {
                Op::Incr {
                    field,
                    offset,
                    increment: value,
                    overflow,
                }
            } else {
                Op::Set {
                    field,
                    offset,
                    value,
                    overflow,
                }
            });
            index += 4;
        } else {
            return Err(SYNTAX_ERROR);
        }
    }
    Ok(operations)
}

fn required_length(offset: u64, width: u32) -> Option<usize> {
    let bits = offset.checked_add(u64::from(width))?;
    let bytes = bits.checked_add(7)? / 8;
    usize::try_from(bytes)
        .ok()
        .filter(|bytes| *bytes <= MAX_STRING_BYTES)
}

fn append_reply(results: &[Option<i64>], output: &mut Vec<u8>) {
    write!(output, "*{}\r\n", results.len()).expect("Vec write");
    for result in results {
        match result {
            Some(value) => integer(*value, output),
            None => output.extend_from_slice(b"$-1\r\n"),
        }
    }
}

impl ShardExecutor {
    pub(super) fn execute_bitfield(&self, args: &[&[u8]], output: &mut Vec<u8>) -> bool {
        let Some(command) = args.first() else {
            return false;
        };
        let read_only = if command.eq_ignore_ascii_case(b"BITFIELD_RO") {
            true
        } else if command.eq_ignore_ascii_case(b"BITFIELD") {
            false
        } else {
            return false;
        };
        if args.len() < 2 {
            arity(if read_only { "bitfield_ro" } else { "bitfield" }, output);
            return true;
        }
        let operations = match parse_operations(args, read_only) {
            Ok(operations) => operations,
            Err(error) => {
                output.extend_from_slice(error);
                return true;
            }
        };
        if operations.is_empty() {
            output.extend_from_slice(b"*0\r\n");
            return true;
        }
        let key = args[1];
        let Ok(mut data) = self.data.try_borrow_mut() else {
            internal_error(output);
            return true;
        };
        let now = self.now(&data);
        data.expire_key(key, now);
        let needs_write = operations.iter().any(|operation| operation.is_write());
        let expires = data.entries.get(key).and_then(|entry| entry.expires);
        let previous = data.entries.get(key).map(|entry| entry.payload_len());
        if data
            .entries
            .get(key)
            .is_some_and(|entry| !entry.is_string())
        {
            wrong_type(output);
            return true;
        }
        if !needs_write {
            let bytes = data
                .entries
                .get(key)
                .map_or(&[][..], |entry| entry.value.as_slice());
            let results = operations
                .iter()
                .map(|operation| match operation {
                    Op::Get { field, offset } => Some(field.read(bytes, *offset)),
                    _ => unreachable!("read-only path contains a write"),
                })
                .collect::<Vec<_>>();
            append_reply(&results, output);
            return true;
        }
        let mut bytes = data
            .entries
            .get(key)
            .map_or_else(Vec::new, |entry| entry.value.clone());
        let mut results = Vec::with_capacity(operations.len());
        let mut changed = false;
        for operation in operations {
            match operation {
                Op::Get { field, offset } => {
                    if required_length(offset, field.width).is_none() {
                        output.extend_from_slice(OFFSET_ERROR);
                        return true;
                    }
                    results.push(Some(field.read(&bytes, offset)));
                }
                Op::Set {
                    field,
                    offset,
                    value,
                    overflow,
                } => {
                    let Some(length) = required_length(offset, field.width) else {
                        output.extend_from_slice(OFFSET_ERROR);
                        return true;
                    };
                    let old = field.read(&bytes, offset);
                    let Some(value) = field.apply(i128::from(value), overflow) else {
                        results.push(None);
                        continue;
                    };
                    if bytes.len() < length {
                        bytes.resize(length, 0);
                    }
                    field.write(&mut bytes, offset, value);
                    changed = true;
                    results.push(Some(old));
                }
                Op::Incr {
                    field,
                    offset,
                    increment,
                    overflow,
                } => {
                    let Some(length) = required_length(offset, field.width) else {
                        output.extend_from_slice(OFFSET_ERROR);
                        return true;
                    };
                    let old = field.read(&bytes, offset);
                    let Some(value) =
                        field.apply(i128::from(old) + i128::from(increment), overflow)
                    else {
                        results.push(None);
                        continue;
                    };
                    if bytes.len() < length {
                        bytes.resize(length, 0);
                    }
                    field.write(&mut bytes, offset, value);
                    changed = true;
                    results.push(Some(value as i64));
                }
            }
        }
        if changed {
            let Some(payload) = data.reserve(key, bytes.len(), previous, self.payload_limit(), now)
            else {
                output.extend_from_slice(OOM);
                return true;
            };
            data.put(key, &bytes, expires, payload, now);
        }
        append_reply(&results, output);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Action;

    fn run(store: &ShardExecutor, args: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        assert_eq!(store.execute(args, &mut out), Action::Continue);
        out
    }

    #[test]
    fn get_set_incr_signed_unsigned_and_hash_offsets() {
        let store = ShardExecutor::new(1024);
        assert_eq!(run(&store, &[b"BITFIELD", b"k"]), b"*0\r\n");
        assert_eq!(
            run(&store, &[b"BITFIELD", b"k", b"GET", b"u8", b"0"]),
            b"*1\r\n:0\r\n"
        );
        assert_eq!(
            run(
                &store,
                &[
                    b"BITFIELD",
                    b"k",
                    b"SET",
                    b"u8",
                    b"0",
                    b"255",
                    b"GET",
                    b"u8",
                    b"0"
                ]
            ),
            b"*2\r\n:0\r\n:255\r\n"
        );
        assert_eq!(
            run(
                &store,
                &[
                    b"BITFIELD",
                    b"k",
                    b"SET",
                    b"i8",
                    b"0",
                    b"-1",
                    b"GET",
                    b"i8",
                    b"0"
                ]
            ),
            b"*2\r\n:-1\r\n:-1\r\n"
        );
        assert_eq!(
            run(
                &store,
                &[
                    b"BITFIELD",
                    b"k",
                    b"SET",
                    b"u8",
                    b"#1",
                    b"7",
                    b"GET",
                    b"u8",
                    b"8"
                ]
            ),
            b"*2\r\n:0\r\n:7\r\n"
        );
        assert_eq!(
            run(&store, &[b"BITFIELD", b"k", b"INCRBY", b"u8", b"0", b"1"]),
            b"*1\r\n:0\r\n"
        );
    }

    #[test]
    fn overflow_policies_apply_to_set_and_incr_without_partial_writes() {
        let store = ShardExecutor::new(1024);
        assert_eq!(
            run(
                &store,
                &[
                    b"BITFIELD",
                    b"k",
                    b"SET",
                    b"u8",
                    b"0",
                    b"200",
                    b"OVERFLOW",
                    b"SAT",
                    b"INCRBY",
                    b"u8",
                    b"0",
                    b"100",
                    b"GET",
                    b"u8",
                    b"0",
                    b"OVERFLOW",
                    b"FAIL",
                    b"INCRBY",
                    b"u8",
                    b"0",
                    b"100",
                    b"GET",
                    b"u8",
                    b"0",
                    b"OVERFLOW",
                    b"WRAP",
                    b"INCRBY",
                    b"u8",
                    b"0",
                    b"100"
                ]
            ),
            b"*6\r\n:0\r\n:255\r\n:255\r\n$-1\r\n:255\r\n:99\r\n"
        );
        assert_eq!(run(&store, &[b"GET", b"k"]), b"$1\r\nc\r\n");
    }

    #[test]
    fn validation_errors_do_not_mutate_and_ro_rejects_writes() {
        let store = ShardExecutor::new(1024);
        assert_eq!(
            run(&store, &[b"BITFIELD", b"k", b"GET", b"u64", b"0"]),
            TYPE_ERROR
        );
        assert_eq!(
            run(&store, &[b"BITFIELD", b"k", b"GET", b"u8", b"x"]),
            OFFSET_ERROR
        );
        assert_eq!(
            run(
                &store,
                &[
                    b"BITFIELD",
                    b"k",
                    b"OVERFLOW",
                    b"BOGUS",
                    b"GET",
                    b"u8",
                    b"0"
                ]
            ),
            OVERFLOW_ERROR
        );
        assert_eq!(
            run(&store, &[b"BITFIELD", b"k", b"BOGUS", b"u8", b"0"]),
            SYNTAX_ERROR
        );
        assert_eq!(
            run(&store, &[b"BITFIELD_RO", b"k", b"SET", b"u8", b"0", b"1"]),
            RO_ERROR
        );
        assert_eq!(run(&store, &[b"EXISTS", b"k"]), b":0\r\n");
        assert_eq!(
            run(&store, &[b"BITFIELD_RO", b"k", b"GET", b"u8", b"0"]),
            b"*1\r\n:0\r\n"
        );
        assert_eq!(
            run(
                &store,
                &[
                    b"BITFIELD",
                    b"k",
                    b"SET",
                    b"i64",
                    b"0",
                    b"-9223372036854775808"
                ]
            ),
            b"*1\r\n:0\r\n"
        );
        assert_eq!(
            run(
                &store,
                &[
                    b"BITFIELD",
                    b"k",
                    b"SET",
                    b"i64",
                    b"0",
                    b"9223372036854775808"
                ]
            ),
            NOT_INTEGER
        );
    }

    #[test]
    fn writes_extend_zero_fill_preserve_ttl_and_fail_atomically_on_oom() {
        let store = ShardExecutor::new(4);
        assert_eq!(
            run(&store, &[b"BITFIELD", b"k", b"SET", b"u8", b"8", b"1"]),
            b"*1\r\n:0\r\n"
        );
        assert_eq!(run(&store, &[b"STRLEN", b"k"]), b":2\r\n");
        assert_eq!(run(&store, &[b"GET", b"k"]), b"$2\r\n\0\x01\r\n");
        assert_eq!(run(&store, &[b"PEXPIRE", b"k", b"10000"]), b":1\r\n");
        assert_eq!(
            run(&store, &[b"BITFIELD", b"k", b"INCRBY", b"u8", b"0", b"1"]),
            b"*1\r\n:1\r\n"
        );
        assert!(!run(&store, &[b"PTTL", b"k"]).starts_with(b":-"));
        assert!(
            run(&store, &[b"BITFIELD", b"k", b"SET", b"u8", b"100", b"1"]).starts_with(b"-OOM")
        );
        assert_eq!(run(&store, &[b"GET", b"k"]), b"$2\r\n\x01\x01\r\n");
    }
}
