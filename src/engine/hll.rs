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
const DENSE_REGISTERS: usize = 1 << 14;
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

    fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < HEADER_BYTES
            || &bytes[..4] != MAGIC
            || bytes[4] == ENCODING_EXACT && bytes[5] != VERSION
            || bytes[4] == ENCODING_DENSE && bytes[5] != VERSION
            || bytes[6] != 0
            || bytes[7] != 0
        {
            return None;
        }
        match bytes[4] {
            ENCODING_EXACT => {
                let count = u32::from_le_bytes(bytes[8..12].try_into().ok()?) as usize;
                if count > EXACT_LIMIT {
                    return None;
                }
                let end = 12usize.checked_add(count.checked_mul(8)?)?;
                if bytes.len() != end {
                    return None;
                }
                let mut hashes = BTreeSet::new();
                let mut previous = None;
                for chunk in bytes[12..].as_chunks::<8>().0 {
                    let hash = u64::from_le_bytes(*chunk);
                    if previous.is_some_and(|previous| previous >= hash) {
                        return None;
                    }
                    previous = Some(hash);
                    hashes.insert(hash);
                }
                Some(Self::Exact(hashes))
            }
            ENCODING_DENSE => {
                if bytes.len() != HEADER_BYTES + DENSE_REGISTERS {
                    return None;
                }
                let registers = bytes[HEADER_BYTES..].to_vec();
                if registers.iter().any(|register| *register > 51) {
                    return None;
                }
                Some(Self::Dense(registers))
            }
            _ => None,
        }
    }

    fn serialize(&self) -> Vec<u8> {
        match self {
            Self::Exact(hashes) => {
                let mut bytes = Vec::with_capacity(12 + hashes.len() * 8);
                bytes.extend_from_slice(MAGIC);
                bytes.extend_from_slice(&[ENCODING_EXACT, VERSION, 0, 0]);
                bytes.extend_from_slice(&(hashes.len() as u32).to_le_bytes());
                for hash in hashes {
                    bytes.extend_from_slice(&hash.to_le_bytes());
                }
                bytes
            }
            Self::Dense(registers) => {
                let mut bytes = Vec::with_capacity(HEADER_BYTES + DENSE_REGISTERS);
                bytes.extend_from_slice(MAGIC);
                bytes.extend_from_slice(&[ENCODING_DENSE, VERSION, 0, 0]);
                bytes.extend_from_slice(registers);
                bytes
            }
        }
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
        let bytes = hll.serialize();
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
        let bytes = merged.serialize();
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
        let bytes = hll.serialize();
        assert_eq!(bytes.len(), HEADER_BYTES + DENSE_REGISTERS);
        let parsed = Hll::parse(&bytes).expect("valid dense payload");
        assert!(parsed.count() >= EXACT_LIMIT as u64);
        assert!(parsed.count() <= (EXACT_LIMIT + 1000) as u64);
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
