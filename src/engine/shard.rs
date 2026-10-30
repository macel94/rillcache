//! Fixed-layout deterministic key routing. The active executor owns one shard.
//!
//! Layouts are stable for a process lifetime. Seeded XXH3 disperses ordinary
//! keys, but is not a cryptographic HashDoS guarantee. Data ownership and the
//! external operation bridge live in the executor/bridge, not in a guard facade.

/// Hard upper bound shared with the server's worker-count limit.
pub const MAX_SHARDS: usize = 64;

/// Why a requested shard layout was rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShardError {
    /// A keyspace needs at least one shard.
    Zero,
    /// Owners are selected with a mask, so the count must be a power of two.
    NotPowerOfTwo(usize),
    /// The count exceeds [`MAX_SHARDS`].
    TooMany(usize),
}

impl std::fmt::Display for ShardError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Zero => formatter.write_str("shard count must be at least one"),
            Self::NotPowerOfTwo(count) => {
                write!(formatter, "shard count {count} is not a power of two")
            }
            Self::TooMany(count) => {
                write!(
                    formatter,
                    "shard count {count} exceeds the limit of {MAX_SHARDS}"
                )
            }
        }
    }
}

impl std::error::Error for ShardError {}

/// Deterministic key-to-shard routing for one process lifetime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShardRouter {
    mask: usize,
    seed: u64,
}

impl ShardRouter {
    /// Build a router with a fresh process-random seed.
    pub fn new(shards: usize) -> Result<Self, ShardError> {
        Self::with_seed(shards, process_seed())
    }

    /// Build a router with an explicit seed; intended for tests and replay.
    pub fn with_seed(shards: usize, seed: u64) -> Result<Self, ShardError> {
        if shards == 0 {
            return Err(ShardError::Zero);
        }
        if shards > MAX_SHARDS {
            return Err(ShardError::TooMany(shards));
        }
        if !shards.is_power_of_two() {
            return Err(ShardError::NotPowerOfTwo(shards));
        }
        Ok(Self {
            mask: shards - 1,
            seed,
        })
    }

    /// Number of shards owned by this layout.
    pub fn count(self) -> usize {
        self.mask + 1
    }

    /// Shard that owns `key`. Always below [`Self::count`].
    pub fn shard_for(self, key: &[u8]) -> usize {
        (seeded_hash(self.seed, key) as usize) & self.mask
    }
}

fn process_seed() -> u64 {
    use std::hash::{BuildHasher, RandomState};
    RandomState::new().hash_one(0_u64)
}

/// XXH3 already spreads key bytes well; adding the process seed and running a
/// SplitMix64 finalizer disperses routing indices without a second map.
fn seeded_hash(seed: u64, key: &[u8]) -> u64 {
    let mut value = super::xxh3::xxh3_64(key).wrapping_add(seed);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Action, Store};

    #[test]
    fn layout_validation_accepts_only_powers_of_two_up_to_the_limit() {
        assert_eq!(ShardRouter::with_seed(0, 1), Err(ShardError::Zero));
        assert_eq!(
            ShardRouter::with_seed(3, 1),
            Err(ShardError::NotPowerOfTwo(3))
        );
        assert_eq!(ShardRouter::with_seed(65, 1), Err(ShardError::TooMany(65)));
        assert_eq!(
            ShardRouter::with_seed(MAX_SHARDS + 1, 1),
            Err(ShardError::TooMany(MAX_SHARDS + 1))
        );
        for shards in [1, 2, 4, 8, 16, 32, MAX_SHARDS] {
            let router = ShardRouter::with_seed(shards, 1).expect("valid layout");
            assert_eq!(router.count(), shards);
        }
    }

    #[test]
    fn routing_is_stable_within_a_layout_and_seed_sensitive_across_layouts() {
        let first = ShardRouter::with_seed(16, 0x5eed).unwrap();
        let second = ShardRouter::with_seed(16, 0x5eee).unwrap();
        let mut differences = 0;
        for index in 0..256_u32 {
            let key = index.to_le_bytes();
            assert_eq!(first.shard_for(&key), first.shard_for(&key));
            differences += usize::from(first.shard_for(&key) != second.shard_for(&key));
        }
        assert!(differences > 0, "seed must influence ownership");
    }

    #[test]
    fn fixed_seed_spreads_keys_across_every_shard() {
        let router = ShardRouter::with_seed(8, 0x5eed).unwrap();
        let mut counts = [0_usize; 8];
        for index in 0..4096_u32 {
            counts[router.shard_for(&index.to_le_bytes())] += 1;
        }
        assert!(counts.iter().all(|count| *count > 0), "{counts:?}");
        assert_eq!(counts.iter().sum::<usize>(), 4096);
    }

    #[test]
    fn ownership_is_in_range_for_empty_binary_and_long_keys() {
        let router = ShardRouter::with_seed(4, 9).unwrap();
        let long = vec![0xaa; 4096];
        for key in [b"".as_slice(), b"\0", b"\xff\xfe\0", long.as_slice()] {
            assert!(router.shard_for(key) < router.count());
        }
    }

    #[test]
    fn current_store_layout_reports_one_shard_and_routes_every_key_to_zero() {
        let store = Store::new(1024);
        assert_eq!(store.shard_count(), 1);
        let long = vec![0x5a; 4096];
        for key in [b"".as_slice(), b"key", b"\xff\x00\xfe", long.as_slice()] {
            assert_eq!(store.shard_for(key), 0);
        }
        let mut output = Vec::new();
        assert_eq!(
            store.execute(&[b"SET", b"key", b"value"], &mut output),
            Action::Continue
        );
        assert_eq!(store.shard_for(b"key"), 0);
    }

    #[test]
    fn stores_isolate_data_even_when_the_same_key_routes_to_zero() {
        let first = Store::new(1024);
        let second = Store::new(1024);
        assert_eq!(first.shard_for(b"shared"), second.shard_for(b"shared"));

        let mut output = Vec::new();
        first.execute(&[b"SET", b"shared", b"first"], &mut output);
        assert_eq!(output, b"+OK\r\n");
        output.clear();
        second.execute(&[b"GET", b"shared"], &mut output);
        assert_eq!(output, b"$-1\r\n");
        output.clear();
        second.execute(&[b"SET", b"shared", b"second"], &mut output);
        assert_eq!(output, b"+OK\r\n");
        output.clear();
        first.execute(&[b"GET", b"shared"], &mut output);
        assert_eq!(output, b"$5\r\nfirst\r\n");
    }

    #[test]
    fn concurrent_store_callers_keep_independent_keys_and_atomic_multi_key_views() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use std::sync::{Arc, Barrier};

        let store = Arc::new(Store::new(1024 * 1024));
        let start = Arc::new(Barrier::new(5));
        let done = Arc::new(AtomicBool::new(false));
        let observations = Arc::new(AtomicUsize::new(0));

        let observer = {
            let store = Arc::clone(&store);
            let start = Arc::clone(&start);
            let done = Arc::clone(&done);
            let observations = Arc::clone(&observations);
            std::thread::spawn(move || {
                start.wait();
                while !done.load(Ordering::Acquire) {
                    let mut output = Vec::new();
                    store.execute(&[b"MGET", b"left", b"right"], &mut output);
                    assert!(
                        output == b"*2\r\n$1\r\n0\r\n$1\r\n0\r\n"
                            || output == b"*2\r\n$1\r\n1\r\n$1\r\n1\r\n",
                        "reader observed a partial MSET: {output:?}"
                    );
                    observations.fetch_add(1, Ordering::Relaxed);
                }
            })
        };

        let mut output = Vec::new();
        store.execute(&[b"MSET", b"left", b"0", b"right", b"0"], &mut output);
        assert_eq!(output, b"+OK\r\n");

        let writers = (0..4)
            .map(|worker| {
                let store = Arc::clone(&store);
                let start = Arc::clone(&start);
                std::thread::spawn(move || {
                    start.wait();
                    for index in 0..250 {
                        let value = ((index + worker) % 2).to_string();
                        let mut output = Vec::new();
                        store.execute(
                            &[
                                b"MSET",
                                b"left",
                                value.as_bytes(),
                                b"right",
                                value.as_bytes(),
                            ],
                            &mut output,
                        );
                        assert_eq!(output, b"+OK\r\n");

                        let key = format!("worker:{worker}:{index}");
                        output.clear();
                        store.execute(&[b"SET", key.as_bytes(), value.as_bytes()], &mut output);
                        assert_eq!(output, b"+OK\r\n");
                    }
                })
            })
            .collect::<Vec<_>>();
        for writer in writers {
            writer.join().unwrap();
        }
        done.store(true, Ordering::Release);
        observer.join().unwrap();
        assert!(observations.load(Ordering::Relaxed) > 0);

        for worker in 0..4 {
            for index in 0..250 {
                let key = format!("worker:{worker}:{index}");
                let mut output = Vec::new();
                store.execute(&[b"GET", key.as_bytes()], &mut output);
                assert!(output == b"$1\r\n0\r\n" || output == b"$1\r\n1\r\n");
            }
        }
    }
}
