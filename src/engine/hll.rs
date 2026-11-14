//! Bounded HyperLogLog commands.
//!
//! A HyperLogLog value is stored as a Redis-style string payload so TYPE, GET,
//! MGET, TTL, RENAME, COPY and the append-only journal keep treating it as one
//! ordinary key. The payload has a private, versioned exact/dense representation:
//! small cardinalities keep an exact sorted 64-bit hash set, then promote to a
//! 2^14-register dense sketch. It is deliberately not Redis's sparse/dense byte
//! encoding, so cross-version HLL payload interoperability is not claimed.

use super::data::Data;
use super::options::OOM;
use super::{ShardExecutor, arity, integer, internal_error, wrong_type};
use std::collections::BTreeSet;

const MAGIC: &[u8; 4] = b"HYLL";
const VERSION: u8 = 1;
const ENCODING_EXACT: u8 = 0;
const ENCODING_DENSE: u8 = 1;
const HEADER_BYTES: usize = 8;
const CACHE_BYTES: usize = 8;
const DENSE_REGISTERS: usize = 1 << 14;
const DENSE_PAYLOAD: usize = HEADER_BYTES + DENSE_REGISTERS + CACHE_BYTES;
const EXACT_LIMIT: usize = 4096;
const INVALID_HLL: &[u8] = b"-WRONGTYPE Key is not a valid HyperLogLog string value.\r\n";

#[derive(Clone, Debug, PartialEq, Eq)]
enum Hll {
    Exact(BTreeSet<u64>),
    Dense(Vec<u8>),
}

impl Hll {
    fn empty() -> Self {
        Self::Exact(BTreeSet::new())
    }

    /// Validate the header and encoding length without copying or scanning the
    /// sketch. Register bytes are opaque (a dense register is a 6-bit rank), so
    /// the hot add/count paths stay O(1) in the register count.
    fn shape(bytes: &[u8]) -> Option<()> {
        if bytes.len() < HEADER_BYTES
            || &bytes[..4] != MAGIC
            || bytes[5] != VERSION
            || bytes[6] > 1
            || bytes[7] > 1
        {
            return None;
        }
        let trailer = if bytes[7] == 1 { CACHE_BYTES } else { 0 };
        match bytes[4] {
            ENCODING_EXACT => {
                if bytes.len() < 12 {
                    return None;
                }
                let count = u32::from_le_bytes(bytes[8..12].try_into().ok()?) as usize;
                if count > EXACT_LIMIT {
                    return None;
                }
                let end = 12usize
                    .checked_add(count.checked_mul(8)?)?
                    .checked_add(trailer)?;
                (bytes.len() == end).then_some(())
            }
            ENCODING_DENSE => {
                let end = HEADER_BYTES
                    .checked_add(DENSE_REGISTERS)?
                    .checked_add(trailer)?;
                (bytes.len() == end).then_some(())
            }
            _ => None,
        }
    }

    fn parse(bytes: &[u8]) -> Option<Self> {
        Self::shape(bytes)?;
        match bytes[4] {
            ENCODING_EXACT => {
                let count = u32::from_le_bytes(bytes[8..12].try_into().ok()?) as usize;
                let end = 12usize.checked_add(count.checked_mul(8)?)?;
                let mut hashes = BTreeSet::new();
                let mut previous = None;
                for chunk in bytes[12..end].as_chunks::<8>().0 {
                    let hash = u64::from_le_bytes(*chunk);
                    if previous.is_some_and(|previous| previous >= hash) {
                        return None;
                    }
                    previous = Some(hash);
                    hashes.insert(hash);
                }
                Some(Self::Exact(hashes))
            }
            ENCODING_DENSE => Some(Self::Dense(
                bytes[HEADER_BYTES..HEADER_BYTES + DENSE_REGISTERS].to_vec(),
            )),
            _ => None,
        }
    }

    fn serialize(&self, cached: Option<u64>) -> Vec<u8> {
        let mut bytes = match self {
            Self::Exact(hashes) => {
                let mut bytes = Vec::with_capacity(12 + hashes.len() * 8 + CACHE_BYTES);
                bytes.extend_from_slice(MAGIC);
                bytes.extend_from_slice(&[ENCODING_EXACT, VERSION, u8::from(cached.is_some()), 1]);
                bytes.extend_from_slice(&(hashes.len() as u32).to_le_bytes());
                for hash in hashes {
                    bytes.extend_from_slice(&hash.to_le_bytes());
                }
                bytes
            }
            Self::Dense(registers) => {
                let mut bytes = Vec::with_capacity(DENSE_PAYLOAD);
                bytes.extend_from_slice(MAGIC);
                bytes.extend_from_slice(&[ENCODING_DENSE, VERSION, u8::from(cached.is_some()), 1]);
                bytes.extend_from_slice(registers);
                bytes
            }
        };
        bytes.extend_from_slice(&cached.unwrap_or(0).to_le_bytes());
        bytes
    }

    /// A cached count is only trusted when the trailer marker and validity byte
    /// are both set; the caller has already validated the payload shape.
    fn cached_count(bytes: &[u8]) -> Option<u64> {
        if bytes.get(6) != Some(&1) || bytes.get(7) != Some(&1) {
            return None;
        }
        bytes
            .get(bytes.len().checked_sub(CACHE_BYTES)?..)
            .and_then(|trailer| trailer.try_into().ok())
            .map(u64::from_le_bytes)
    }

    fn dense_registers_mut(bytes: &mut [u8]) -> Option<&mut [u8]> {
        if bytes[4] != ENCODING_DENSE || bytes[7] != 1 || bytes.len() != DENSE_PAYLOAD {
            return None;
        }
        Some(&mut bytes[HEADER_BYTES..HEADER_BYTES + DENSE_REGISTERS])
    }

    fn add_element(&mut self, element: &[u8]) -> bool {
        self.add_hash(super::xxh3::xxh3_64(element))
    }

    fn add_hash(&mut self, hash: u64) -> bool {
        match self {
            Self::Exact(hashes) => {
                let inserted = hashes.insert(hash);
                if hashes.len() > EXACT_LIMIT {
                    let hashes = std::mem::take(hashes);
                    let mut registers = vec![0; DENSE_REGISTERS];
                    for hash in hashes {
                        update_register(&mut registers, hash);
                    }
                    *self = Self::Dense(registers);
                }
                inserted
            }
            Self::Dense(registers) => update_register(registers, hash),
        }
    }

    fn merge(&mut self, other: &Self) {
        match (&mut *self, other) {
            (Self::Exact(hashes), Self::Exact(other)) => {
                hashes.extend(other.iter().copied());
                if hashes.len() > EXACT_LIMIT {
                    let hashes = std::mem::take(hashes);
                    let mut registers = vec![0; DENSE_REGISTERS];
                    for hash in hashes {
                        update_register(&mut registers, hash);
                    }
                    *self = Self::Dense(registers);
                }
            }
            (Self::Dense(registers), Self::Exact(hashes)) => {
                for hash in hashes {
                    update_register(registers, *hash);
                }
            }
            (Self::Exact(hashes), Self::Dense(other)) => {
                let own = std::mem::take(hashes);
                let mut registers = other.clone();
                for hash in own {
                    update_register(&mut registers, hash);
                }
                *self = Self::Dense(registers);
            }
            (Self::Dense(registers), Self::Dense(other)) => {
                for (register, value) in registers.iter_mut().zip(other) {
                    *register = (*register).max(*value);
                }
            }
        }
    }

    fn count(&self) -> u64 {
        match self {
            Self::Exact(hashes) => hashes.len() as u64,
            Self::Dense(registers) => dense_count(registers),
        }
    }
}

fn update_register(registers: &mut [u8], hash: u64) -> bool {
    let index = (hash & ((1 << 14) - 1)) as usize;
    let remaining = hash >> 14;
    let rank = (remaining.leading_zeros() as u8).saturating_sub(13);
    if rank > registers[index] {
        registers[index] = rank;
        true
    } else {
        false
    }
}

fn dense_count(registers: &[u8]) -> u64 {
    let m = DENSE_REGISTERS as f64;
    let sum = registers
        .iter()
        .map(|register| 2f64.powi(-(*register as i32)))
        .sum::<f64>();
    let alpha = 0.7213 / (1.0 + 1.079 / m);
    let mut estimate = alpha * m * m / sum;
    let zeros = registers.iter().filter(|register| **register == 0).count() as f64;
    if estimate <= 2.5 * m && zeros > 0.0 {
        estimate = m * (m / zeros).ln();
    }
    estimate.round().max(0.0).min(i64::MAX as f64) as u64
}

enum Shape {
    Missing,
    Exact,
    Dense,
    WrongType,
    Invalid,
}

/// Cheap classification for the in-place paths: validates the payload without
/// copying the sketch.
fn inspect_shape(data: &mut Data, key: &[u8], now: i64) -> Shape {
    data.expire_key(key, now);
    let Some(entry) = data.entries.get(key) else {
        return Shape::Missing;
    };
    if !entry.is_string() {
        return Shape::WrongType;
    }
    if Hll::shape(&entry.value).is_none() {
        return Shape::Invalid;
    }
    match entry.value[4] {
        ENCODING_EXACT => Shape::Exact,
        ENCODING_DENSE => Shape::Dense,
        _ => Shape::Invalid,
    }
}

enum KeyState {
    Missing,
    Hll {
        hll: Hll,
        expires: Option<i64>,
        previous: usize,
    },
    WrongType,
    Invalid,
}

fn inspect(data: &mut Data, key: &[u8], now: i64) -> KeyState {
    data.expire_key(key, now);
    let Some(entry) = data.entries.get(key) else {
        return KeyState::Missing;
    };
    if !entry.is_string() {
        return KeyState::WrongType;
    }
    match Hll::parse(&entry.value) {
        Some(hll) => KeyState::Hll {
            hll,
            expires: entry.expires,
            previous: entry.payload_len(),
        },
        None => KeyState::Invalid,
    }
}

impl ShardExecutor {
    pub(super) fn execute_hll(&self, args: &[&[u8]], out: &mut Vec<u8>) -> bool {
        let Some(command) = args.first() else {
            return false;
        };
        if command.eq_ignore_ascii_case(b"PFADD") {
            self.pfadd(args, out);
        } else if command.eq_ignore_ascii_case(b"PFCOUNT") {
            self.pfcount(args, out);
        } else if command.eq_ignore_ascii_case(b"PFMERGE") {
            self.pfmerge(args, out);
        } else {
            return false;
        }
        true
    }

    fn pfadd(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        if args.len() < 2 {
            arity("pfadd", out);
            return;
        }
        let key = args[1];
        let Ok(mut data) = self.data.try_borrow_mut() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        let shape = inspect_shape(&mut data, key, now);
        if matches!(shape, Shape::WrongType) {
            wrong_type(out);
            return;
        }
        if matches!(shape, Shape::Invalid) {
            out.extend_from_slice(INVALID_HLL);
            return;
        }
        if matches!(shape, Shape::Dense) {
            // Dense registers stay in the payload: validate once, update the
            // registers in place, and invalidate the cached count. The payload
            // length and therefore the accounting do not change.
            let hashes: Vec<u64> = args[2..]
                .iter()
                .map(|element| super::xxh3::xxh3_64(element))
                .collect();
            let Some(entry) = data.entries.get_mut(key) else {
                integer(0, out);
                return;
            };
            let mut changed = false;
            {
                let Some(registers) = Hll::dense_registers_mut(&mut entry.value) else {
                    out.extend_from_slice(INVALID_HLL);
                    return;
                };
                for hash in hashes {
                    changed |= update_register(registers, hash);
                }
            }
            entry.value[6] = 0;
            if changed {
                data.touch(key);
                integer(1, out);
            } else {
                integer(0, out);
            }
            return;
        }
        let (mut hll, expires, previous, created, mut changed) = match inspect(&mut data, key, now)
        {
            KeyState::Missing => (Hll::empty(), None, None, true, false),
            KeyState::Hll {
                hll,
                expires,
                previous,
            } => (hll, expires, Some(previous), false, false),
            KeyState::WrongType => {
                wrong_type(out);
                return;
            }
            KeyState::Invalid => {
                out.extend_from_slice(INVALID_HLL);
                return;
            }
        };
        for element in &args[2..] {
            changed |= hll.add_element(element);
        }
        if !created && !changed {
            integer(0, out);
            return;
        }
        let bytes = hll.serialize(None);
        let Some(payload) = data.reserve(key, bytes.len(), previous, self.payload_limit(), now)
        else {
            out.extend_from_slice(OOM);
            return;
        };
        data.put(key, &bytes, expires, payload, now);
        integer(1, out);
    }

    fn pfcount(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        if args.len() < 2 {
            arity("pfcount", out);
            return;
        }
        let Ok(mut data) = self.data.try_borrow_mut() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        if args.len() == 2 {
            let key = args[1];
            match inspect_shape(&mut data, key, now) {
                Shape::Missing => {
                    integer(0, out);
                    return;
                }
                Shape::WrongType => {
                    wrong_type(out);
                    return;
                }
                Shape::Invalid => {
                    out.extend_from_slice(INVALID_HLL);
                    return;
                }
                Shape::Exact => {}
                Shape::Dense => {
                    let Some(entry) = data.entries.get(key) else {
                        integer(0, out);
                        return;
                    };
                    if let Some(count) = Hll::cached_count(&entry.value) {
                        integer(count.min(i64::MAX as u64) as i64, out);
                        return;
                    }
                    let count =
                        dense_count(&entry.value[HEADER_BYTES..HEADER_BYTES + DENSE_REGISTERS]);
                    // Cache the estimate in the existing trailer. This is a
                    // read-path optimisation: it changes no logical state, so
                    // it does not touch the WATCH version or journal.
                    let Some(entry) = data.entries.get_mut(key) else {
                        integer(count.min(i64::MAX as u64) as i64, out);
                        return;
                    };
                    let end = entry.value.len();
                    entry.value[6] = 1;
                    entry.value[end - CACHE_BYTES..].copy_from_slice(&count.to_le_bytes());
                    integer(count.min(i64::MAX as u64) as i64, out);
                    return;
                }
            }
        }
        let mut merged = Hll::empty();
        for key in &args[1..] {
            match inspect(&mut data, key, now) {
                KeyState::Missing => {}
                KeyState::Hll { hll, .. } => merged.merge(&hll),
                KeyState::WrongType => {
                    wrong_type(out);
                    return;
                }
                KeyState::Invalid => {
                    out.extend_from_slice(INVALID_HLL);
                    return;
                }
            }
        }
        integer(merged.count().min(i64::MAX as u64) as i64, out);
    }

    fn pfmerge(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        if args.len() < 2 {
            arity("pfmerge", out);
            return;
        }
        let destination = args[1];
        let Ok(mut data) = self.data.try_borrow_mut() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        let (mut merged, expires, previous, created) = match inspect(&mut data, destination, now) {
            KeyState::Missing => (Hll::empty(), None, None, true),
            KeyState::Hll {
                hll,
                expires,
                previous,
            } => (hll, expires, Some(previous), false),
            KeyState::WrongType => {
                wrong_type(out);
                return;
            }
            KeyState::Invalid => {
                out.extend_from_slice(INVALID_HLL);
                return;
            }
        };
        for source in &args[2..] {
            match inspect(&mut data, source, now) {
                KeyState::Missing => {}
                KeyState::Hll { hll, .. } => merged.merge(&hll),
                KeyState::WrongType => {
                    wrong_type(out);
                    return;
                }
                KeyState::Invalid => {
                    out.extend_from_slice(INVALID_HLL);
                    return;
                }
            }
        }
        if !created && args.len() == 2 {
            out.extend_from_slice(b"+OK\r\n");
            return;
        }
        let bytes = merged.serialize(None);
        let Some(payload) = data.reserve(
            destination,
            bytes.len(),
            previous,
            self.payload_limit(),
            now,
        ) else {
            out.extend_from_slice(OOM);
            return;
        };
        data.put(destination, &bytes, expires, payload, now);
        out.extend_from_slice(b"+OK\r\n");
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
    fn exact_small_counts_add_merge_and_empty_creation() {
        let store = ShardExecutor::new(1 << 20);
        assert_eq!(run(&store, &[b"PFADD", b"h"]), b":1\r\n");
        assert_eq!(run(&store, &[b"TYPE", b"h"]), b"+string\r\n");
        assert_eq!(run(&store, &[b"PFCOUNT", b"h"]), b":0\r\n");
        assert_eq!(run(&store, &[b"PFADD", b"h", b"a", b"b", b"c"]), b":1\r\n");
        assert_eq!(run(&store, &[b"PFADD", b"h", b"a", b"b"]), b":0\r\n");
        assert_eq!(run(&store, &[b"PFCOUNT", b"h"]), b":3\r\n");
        assert_eq!(run(&store, &[b"PFADD", b"h2", b"c", b"d"]), b":1\r\n");
        assert_eq!(run(&store, &[b"PFMERGE", b"out", b"h", b"h2"]), b"+OK\r\n");
        assert_eq!(run(&store, &[b"PFCOUNT", b"out"]), b":4\r\n");
        assert_eq!(
            run(&store, &[b"PFCOUNT", b"h", b"h2", b"missing"]),
            b":4\r\n"
        );
    }

    #[test]
    fn merge_preserves_existing_destination_and_ttl() {
        let store = ShardExecutor::new(1 << 20);
        assert_eq!(run(&store, &[b"PFADD", b"d", b"a"]), b":1\r\n");
        assert_eq!(run(&store, &[b"PEXPIRE", b"d", b"10000"]), b":1\r\n");
        assert_eq!(run(&store, &[b"PFADD", b"s", b"b"]), b":1\r\n");
        assert_eq!(run(&store, &[b"PFMERGE", b"d", b"s"]), b"+OK\r\n");
        assert_eq!(run(&store, &[b"PFCOUNT", b"d"]), b":2\r\n");
        assert!(!run(&store, &[b"PTTL", b"d"]).starts_with(b":-"));
        assert_eq!(run(&store, &[b"PFMERGE", b"d"]), b"+OK\r\n");
        assert_eq!(run(&store, &[b"PFCOUNT", b"d"]), b":2\r\n");
        assert_eq!(run(&store, &[b"PFMERGE", b"fresh"]), b"+OK\r\n");
        assert_eq!(run(&store, &[b"PFCOUNT", b"fresh"]), b":0\r\n");
    }

    #[test]
    fn invalid_strings_and_wrong_types_are_rejected_without_mutation() {
        let store = ShardExecutor::new(1 << 20);
        assert_eq!(run(&store, &[b"SET", b"bad", b"not-hll"]), b"+OK\r\n");
        for args in [
            &[b"PFADD".as_slice(), b"bad", b"x"][..],
            &[b"PFCOUNT".as_slice(), b"bad"][..],
        ] {
            assert_eq!(run(&store, args), INVALID_HLL);
        }
        assert_eq!(run(&store, &[b"PFMERGE", b"bad", b"other"]), INVALID_HLL);
        assert_eq!(run(&store, &[b"GET", b"bad"]), b"$7\r\nnot-hll\r\n");
        assert_eq!(run(&store, &[b"RPUSH", b"list", b"x"]), b":1\r\n");
        for args in [
            &[b"PFADD".as_slice(), b"list", b"x"][..],
            &[b"PFCOUNT".as_slice(), b"list"][..],
            &[b"PFMERGE".as_slice(), b"list"][..],
            &[b"PFMERGE".as_slice(), b"out", b"list"][..],
        ] {
            assert_eq!(
                run(&store, args),
                b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n"
            );
        }
    }

    #[test]
    fn dense_promotion_keeps_counts_stable_and_serialization_bounded() {
        let mut hll = Hll::empty();
        for index in 0..(EXACT_LIMIT + 100) {
            hll.add_hash(super::super::xxh3::xxh3_64(index.to_string().as_bytes()));
        }
        assert!(matches!(hll, Hll::Dense(_)));
        let bytes = hll.serialize(None);
        assert_eq!(bytes.len(), DENSE_PAYLOAD);
        let parsed = Hll::parse(&bytes).expect("valid dense payload");
        assert!(parsed.count() >= EXACT_LIMIT as u64);
        assert!(parsed.count() <= (EXACT_LIMIT + 1000) as u64);
    }

    #[test]
    fn dense_cache_is_written_invalidated_and_round_trips() {
        let store = ShardExecutor::new(1 << 20);
        for batch in 0..((EXACT_LIMIT + 100) / 100) {
            let members: Vec<String> = (0..100)
                .map(|offset| format!("element-{}", batch * 100 + offset))
                .collect();
            let mut args = vec![b"PFADD".as_slice(), b"h"];
            args.extend(members.iter().map(|member| member.as_bytes()));
            assert_eq!(run(&store, &args), b":1\r\n");
        }
        let first = run(&store, &[b"PFCOUNT", b"h"]);
        let data = store.data.try_borrow().unwrap();
        let entry = data.entries.get(b"h".as_slice()).expect("hll");
        assert_eq!(entry.value[6], 1, "PFCOUNT caches the dense estimate");
        assert_eq!(entry.value.len(), DENSE_PAYLOAD);
        let cached = Hll::cached_count(&entry.value).expect("valid cache");
        assert_eq!(first, format!(":{cached}\r\n").into_bytes());
        assert_eq!(
            Hll::parse(&entry.value).expect("parses").count(),
            cached,
            "the sketch and cached count agree"
        );
        drop(data);
        assert_eq!(run(&store, &[b"PFCOUNT", b"h"]), first);
        // A new element invalidates the cache and the next count refreshes it.
        assert_eq!(run(&store, &[b"PFADD", b"h", b"new-element"]), b":1\r\n");
        {
            let data = store.data.try_borrow().unwrap();
            assert_eq!(data.entries[b"h".as_slice()].value[6], 0);
        }
        let updated = run(&store, &[b"PFCOUNT", b"h"]);
        assert!(updated != b":0\r\n");
        let data = store.data.try_borrow().unwrap();
        let entry = data.entries.get(b"h".as_slice()).expect("hll");
        assert_eq!(entry.value[6], 1);
        assert_eq!(
            updated,
            format!(
                ":{}",
                Hll::cached_count(&entry.value).expect("refreshed cache")
            )
            .bytes()
            .chain(b"\r\n".iter().copied())
            .collect::<Vec<u8>>()
        );
        drop(data);
        // The serialized payload stays a normal string for GET/TYPE.
        assert_eq!(run(&store, &[b"TYPE", b"h"]), b"+string\r\n");
    }

    #[test]
    fn malformed_payloads_are_rejected() {
        for bytes in [
            b"".as_slice(),
            b"HYLL",
            b"Hyll\x01\x01\0\0",
            b"HYLL\x00\x01\0\0\x01\0\0\0",
            b"HYLL\x01\x01\0\0",
            b"HYLL\x02\x01\0\0",
        ] {
            assert!(Hll::parse(bytes).is_none(), "{bytes:?}");
        }
    }
}
