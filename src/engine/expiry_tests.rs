use super::*;
use std::sync::atomic::{AtomicI64, Ordering};

struct TestClock(AtomicI64);
impl Clock for TestClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}
fn setup(maximum: usize) -> (Store, Arc<TestClock>) {
    let clock = Arc::new(TestClock(AtomicI64::new(1000)));
    (Store::with_clock(maximum, clock.clone()), clock)
}
fn exec(store: &Store, args: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    assert_eq!(store.execute(args, &mut out), Action::Continue);
    out
}
fn bulk(value: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    encode_bulk(Some(value), &mut out);
    out
}

#[test]
fn expiry_is_logical_without_sleeps_and_matches_the_millisecond_boundary() {
    let (store, clock) = setup(100);
    assert_eq!(
        exec(&store, &[b"SET", b"k", b"v", b"PX", b"10"]),
        b"+OK\r\n"
    );
    assert_eq!(exec(&store, &[b"PTTL", b"k"]), b":10\r\n");
    clock.0.store(1010, Ordering::SeqCst);
    assert_eq!(exec(&store, &[b"GET", b"k"]), bulk(b"v"));
    assert_eq!(exec(&store, &[b"PTTL", b"k"]), b":0\r\n");
    clock.0.store(1011, Ordering::SeqCst);
    assert_eq!(exec(&store, &[b"GET", b"k"]), b"$-1\r\n");
    assert_eq!(exec(&store, &[b"EXISTS", b"k", b"k"]), b":0\r\n");
    assert_eq!(exec(&store, &[b"TTL", b"k"]), b":-2\r\n");
    assert_eq!(exec(&store, &[b"DEL", b"k", b"k"]), b":0\r\n");
    assert_eq!(store.data.read().unwrap().payload_bytes, 0);
    assert_eq!(exec(&store, &[b"SET", b"k", b"v", b"NX"]), b"+OK\r\n");
    assert_eq!(exec(&store, &[b"TTL", b"k"]), b":-1\r\n");
}

#[test]
fn observed_expiration_cannot_resurrect_on_a_later_wall_clock_rollback() {
    for command in [b"GET".as_slice(), b"EXISTS", b"PTTL"] {
        let (store, clock) = setup(100);
        exec(&store, &[b"SET", b"k", b"v", b"PX", b"1"]);
        clock.0.store(1002, Ordering::SeqCst);
        let reply = exec(&store, &[command, b"k"]);
        assert!(reply == b"$-1\r\n" || reply == b":0\r\n" || reply == b":-2\r\n");
        clock.0.store(1000, Ordering::SeqCst);
        assert_eq!(exec(&store, &[b"GET", b"k"]), b"$-1\r\n");
        assert!(store.data.read().unwrap().expirations.is_empty());
    }
}

#[test]
fn set_get_conditionals_keep_or_clear_ttl_without_mutating_rejected_writes() {
    let (store, _) = setup(100);
    assert_eq!(
        exec(&store, &[b"SET", b"k", b"old", b"PX", b"100"]),
        b"+OK\r\n"
    );
    assert_eq!(
        exec(&store, &[b"SET", b"k", b"bad", b"NX", b"GET"]),
        bulk(b"old")
    );
    assert_eq!(exec(&store, &[b"PTTL", b"k"]), b":100\r\n");
    assert_eq!(
        exec(&store, &[b"SET", b"k", b"new", b"XX", b"GET", b"KEEPTTL"]),
        bulk(b"old")
    );
    assert_eq!(exec(&store, &[b"PTTL", b"k"]), b":100\r\n");
    assert_eq!(
        exec(&store, &[b"SET", b"missing", b"v", b"XX", b"GET"]),
        b"$-1\r\n"
    );
    assert_eq!(exec(&store, &[b"SET", b"k", b"plain"]), b"+OK\r\n");
    assert_eq!(exec(&store, &[b"PTTL", b"k"]), b":-1\r\n");
    assert!(store.data.read().unwrap().expirations.is_empty());
}

#[test]
fn set_validates_every_option_before_condition_and_preserves_value_ttl_on_error() {
    let (store, _) = setup(100);
    exec(&store, &[b"SET", b"k", b"old", b"PX", b"100"]);
    for flags in [
        vec![b"NX".as_slice(), b"PX", b"0"],
        vec![b"PX", b"-1"],
        vec![b"EX", b"9223372036854775807"],
        vec![b"PX", b"9223372036854775807"],
    ] {
        let mut args = vec![b"SET".as_slice(), b"k", b"new"];
        args.extend(flags);
        assert!(exec(&store, &args).starts_with(b"-ERR invalid expire time"));
    }
    for flags in [
        vec![b"EX".as_slice(), b"01"],
        vec![b"PX", b"+1"],
        vec![b"EX", b"bad"],
    ] {
        let mut args = vec![b"SET".as_slice(), b"k", b"new"];
        args.extend(flags);
        assert_eq!(exec(&store, &args), options::NOT_INTEGER);
    }
    for flags in [
        vec![b"EX".as_slice(), b"1", b"PX", b"1"],
        vec![b"KEEPTTL", b"EX", b"1"],
        vec![b"EX", b"1", b"KEEPTTL"],
        vec![b"NX", b"XX"],
        vec![b"PX"],
        vec![b"EX\0", b"1"],
    ] {
        let mut args = vec![b"SET".as_slice(), b"k", b"new"];
        args.extend(flags);
        assert_eq!(exec(&store, &args), options::SYNTAX);
    }
    assert_eq!(exec(&store, &[b"GET", b"k"]), bulk(b"old"));
    assert_eq!(exec(&store, &[b"PTTL", b"k"]), b":100\r\n");
    assert_eq!(
        exec(
            &store,
            &[b"SET", b"k", b"new", b"EX", b"ignored", b"EX", b"2"]
        ),
        b"+OK\r\n"
    );
    assert_eq!(exec(&store, &[b"PTTL", b"k"]), b":2000\r\n");
}

#[test]
fn aliases_getex_getdel_and_absolute_expiry_preserve_exact_wire_semantics() {
    let (store, _) = setup(100);
    assert_eq!(exec(&store, &[b"SETNX", b"k", b"old"]), b":1\r\n");
    assert_eq!(exec(&store, &[b"SETNX", b"k", b"bad"]), b":0\r\n");
    assert_eq!(exec(&store, &[b"SETEX", b"k", b"2", b"new"]), b"+OK\r\n");
    assert_eq!(exec(&store, &[b"GETEX", b"k", b"PX", b"50"]), bulk(b"new"));
    assert_eq!(exec(&store, &[b"PTTL", b"k"]), b":50\r\n");
    assert_eq!(exec(&store, &[b"GETEX", b"k", b"PERSIST"]), bulk(b"new"));
    assert_eq!(exec(&store, &[b"PTTL", b"k"]), b":-1\r\n");
    assert_eq!(exec(&store, &[b"PSETEX", b"k", b"50", b"x"]), b"+OK\r\n");
    assert_eq!(exec(&store, &[b"GETSET", b"k", b"last"]), bulk(b"x"));
    assert_eq!(exec(&store, &[b"PTTL", b"k"]), b":-1\r\n");
    assert_eq!(
        exec(&store, &[b"GETEX", b"absent", b"EX", b"not-a-number"]),
        b"$-1\r\n"
    );
    assert_eq!(
        exec(&store, &[b"GETEX", b"k", b"PXAT", b"1"]),
        bulk(b"last")
    );
    assert_eq!(exec(&store, &[b"GET", b"k"]), b"$-1\r\n");
    exec(&store, &[b"SET", b"k", b"x", b"EXAT", b"2"]);
    assert_eq!(exec(&store, &[b"PEXPIRETIME", b"k"]), b":2000\r\n");
    assert_eq!(exec(&store, &[b"GETDEL", b"k"]), bulk(b"x"));
    assert!(store.data.read().unwrap().expirations.is_empty());
    assert_eq!(
        exec(&store, &[b"SET", b"k", b"x", b"PXAT", b"1000"]),
        b"+OK\r\n"
    );
    assert_eq!(exec(&store, &[b"GET", b"k"]), b"$-1\r\n");
}

#[test]
fn expire_flags_persist_rounding_and_overflow_are_atomic() {
    let (store, _) = setup(100);
    exec(&store, &[b"SET", b"k", b"v"]);
    assert_eq!(exec(&store, &[b"EXPIRE", b"k", b"1", b"XX"]), b":0\r\n");
    assert_eq!(exec(&store, &[b"PEXPIRE", b"k", b"100", b"GT"]), b":0\r\n");
    assert_eq!(exec(&store, &[b"PEXPIRE", b"k", b"1500", b"NX"]), b":1\r\n");
    assert_eq!(exec(&store, &[b"TTL", b"k"]), b":2\r\n");
    assert_eq!(
        exec(&store, &[b"PEXPIRE", b"k", b"1499", b"XX", b"LT"]),
        b":1\r\n"
    );
    assert_eq!(exec(&store, &[b"TTL", b"k"]), b":1\r\n");
    assert_eq!(
        exec(&store, &[b"EXPIRE", b"k", b"3", b"XX", b"GT"]),
        b":1\r\n"
    );
    assert_eq!(exec(&store, &[b"EXPIREAT", b"k", b"4"]), b":1\r\n");
    assert_eq!(exec(&store, &[b"EXPIRETIME", b"k"]), b":4\r\n");
    assert!(exec(&store, &[b"EXPIRE", b"k", b"1", b"NX", b"LT"]).starts_with(b"-ERR NX"));
    assert!(
        exec(&store, &[b"EXPIRE", b"k", b"9223372036854775807"])
            .starts_with(b"-ERR invalid expire")
    );
    assert_eq!(exec(&store, &[b"PEXPIRETIME", b"k"]), b":4000\r\n");
    assert_eq!(exec(&store, &[b"PERSIST", b"k"]), b":1\r\n");
    assert_eq!(exec(&store, &[b"PERSIST", b"k"]), b":0\r\n");
    assert_eq!(exec(&store, &[b"PEXPIREAT", b"k", b"0", b"LT"]), b":1\r\n");
    assert_eq!(exec(&store, &[b"PTTL", b"k"]), b":-2\r\n");
}

#[test]
fn expiry_metadata_is_indexed_and_reclaimed_without_a_false_payload_oom() {
    let (store, clock) = setup(4);
    exec(&store, &[b"SET", b"k", b"123", b"PX", b"10"]);
    for value in 20..1020 {
        exec(&store, &[b"PEXPIRE", b"k", value.to_string().as_bytes()]);
        assert_eq!(store.data.read().unwrap().expirations.len(), 1);
    }
    clock.0.store(5000, Ordering::SeqCst);
    assert_eq!(exec(&store, &[b"SET", b"new", b"x"]), b"+OK\r\n");
    assert_eq!(store.data.read().unwrap().payload_bytes, 4);
    assert!(store.data.read().unwrap().expirations.is_empty());
    let (store, clock) = setup(10000);
    for key in 0..400 {
        exec(
            &store,
            &[b"SET", key.to_string().as_bytes(), b"", b"PX", b"1"],
        );
    }
    clock.0.store(1002, Ordering::SeqCst);
    assert_eq!(store.purge_expired(128), 128);
    assert_eq!(store.purge_expired(1000), 272);
    assert_eq!(store.data.read().unwrap().payload_bytes, 0);
}

#[test]
fn oom_returns_one_error_and_preserves_the_existing_value_and_expiry() {
    let (store, _) = setup(5);
    exec(&store, &[b"SET", b"k", b"1234", b"PX", b"100"]);
    assert_eq!(
        exec(&store, &[b"SET", b"k", b"12345", b"GET", b"PX", b"200"]),
        options::OOM
    );
    assert_eq!(exec(&store, &[b"GET", b"k"]), bulk(b"1234"));
    assert_eq!(exec(&store, &[b"PTTL", b"k"]), b":100\r\n");
}

#[test]
fn integers_are_canonical_signed_64_bit_and_failures_preserve_ttl() {
    let (store, _) = setup(100);
    for bad in [
        b"".as_slice(),
        b"+1",
        b"01",
        b"-0",
        b" 1",
        b"1.0",
        b"1\0",
        b"\xff",
        b"9223372036854775808",
    ] {
        exec(&store, &[b"SET", b"k", bad, b"PX", b"100"]);
        assert_eq!(exec(&store, &[b"INCR", b"k"]), options::NOT_INTEGER);
        assert_eq!(exec(&store, &[b"GET", b"k"]), bulk(bad));
        assert_eq!(exec(&store, &[b"PTTL", b"k"]), b":100\r\n");
    }
    exec(
        &store,
        &[b"SET", b"k", b"9223372036854775807", b"PX", b"100"],
    );
    assert_eq!(
        exec(&store, &[b"INCR", b"k"]),
        b"-ERR increment or decrement would overflow\r\n"
    );
    assert_eq!(
        exec(&store, &[b"DECRBY", b"k", b"-9223372036854775808"]),
        b"-ERR decrement would overflow\r\n"
    );
    assert_eq!(exec(&store, &[b"GET", b"k"]), bulk(b"9223372036854775807"));
    exec(&store, &[b"DEL", b"k"]);
    assert_eq!(
        exec(&store, &[b"INCRBY", b"k", b"-9223372036854775808"]),
        b":-9223372036854775808\r\n"
    );
    assert_eq!(
        exec(&store, &[b"DECR", b"k"]),
        b"-ERR increment or decrement would overflow\r\n"
    );
}

#[test]
fn deterministic_mixed_commands_preserve_payload_and_expiry_index_invariants() {
    let (store, clock) = setup(512);
    let mut seed = 0x245698ab_u64;
    for iteration in 0..5000 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        clock.0.store(1000 + (seed % 40) as i64, Ordering::SeqCst);
        let key = format!("k:{}", seed % 32);
        let value = (seed / 100).to_string();
        let ttl = (seed % 8 + 1).to_string();
        let args: Vec<&[u8]> = match (seed >> 32) % 8 {
            0 => vec![b"SET", key.as_bytes(), value.as_bytes()],
            1 => vec![
                b"SET",
                key.as_bytes(),
                value.as_bytes(),
                b"PX",
                ttl.as_bytes(),
            ],
            2 => vec![b"PEXPIRE", key.as_bytes(), ttl.as_bytes()],
            3 => vec![b"GET", key.as_bytes()],
            4 => vec![b"INCR", key.as_bytes()],
            5 => vec![b"GETEX", key.as_bytes(), b"PERSIST"],
            6 => vec![b"SET", key.as_bytes(), value.as_bytes(), b"GET", b"KEEPTTL"],
            _ => vec![b"DEL", key.as_bytes()],
        };
        exec(&store, &args);
        if iteration % 11 == 0 {
            store.purge_expired(7);
        }
        let data = store.data.read().unwrap();
        assert_eq!(
            data.payload_bytes,
            data.entries
                .iter()
                .map(|(key, entry)| key.len() + entry.value.len())
                .sum::<usize>()
        );
        assert!(data.payload_bytes <= 512);
        let indexed: std::collections::BTreeSet<_> = data
            .entries
            .iter()
            .filter_map(|(key, entry)| entry.expires.map(|at| (at, key.clone())))
            .collect();
        assert_eq!(data.expirations, indexed);
    }
}

#[test]
fn maximum_absolute_deadline_queries_do_not_overflow_rounding() {
    let (store, _) = setup(100);
    assert_eq!(
        exec(
            &store,
            &[b"SET", b"k", b"v", b"PXAT", b"9223372036854775807"]
        ),
        b"+OK\r\n"
    );
    assert_eq!(
        exec(&store, &[b"PEXPIRETIME", b"k"]),
        b":9223372036854775807\r\n"
    );
    assert_eq!(
        exec(&store, &[b"EXPIRETIME", b"k"]),
        b":9223372036854776\r\n"
    );
    assert_eq!(exec(&store, &[b"TTL", b"k"]), b":9223372036854775\r\n");
}

#[test]
fn admitted_overwrites_reuse_capacity_but_release_disproportionate_shrinks() {
    let (store, _) = setup(4096);
    let mut expected_capacity = 0;
    for (length, byte) in [
        (1024, b'a'),
        (768, b'b'),
        (1024, b'c'),
        (17, b'd'),
        (0, b'e'),
        (65, b'f'),
    ] {
        let value = vec![byte; length];
        assert_eq!(
            exec(&store, &[b"SET", b"k", &value, b"PX", b"100"]),
            b"+OK\r\n"
        );
        assert_eq!(exec(&store, &[b"GET", b"k"]), bulk(&value));
        assert_eq!(exec(&store, &[b"PTTL", b"k"]), b":100\r\n");
        let data = store.data.read().unwrap();
        let entry = &data.entries[b"k".as_slice()];
        let capacity = entry.value.capacity();
        if length == 768 {
            assert_eq!(
                capacity, expected_capacity,
                "overwrite should reuse the old buffer"
            );
        }
        if length == 17 || length == 0 {
            assert_eq!(
                capacity, length,
                "large/empty shrink should release old capacity"
            );
        }
        assert_eq!(data.payload_bytes, 1 + length);
        assert_eq!(data.expirations.len(), 1);
        expected_capacity = capacity;
    }
}

#[test]
fn cached_admission_metadata_survives_reaping_other_keys_and_preserves_live_oom_state() {
    let (store, clock) = setup(10);
    exec(&store, &[b"SET", b"live", b"1"]);
    exec(&store, &[b"SET", b"dead", b"x", b"PX", b"1"]);
    clock.0.store(1002, Ordering::SeqCst);
    assert_eq!(
        exec(&store, &[b"SET", b"live", b"123456", b"GET"]),
        bulk(b"1")
    );
    assert_eq!(store.data.read().unwrap().payload_bytes, 10);
    assert_eq!(
        exec(&store, &[b"SET", b"live", b"1234567", b"GET"]),
        options::OOM
    );
    assert_eq!(exec(&store, &[b"GET", b"live"]), bulk(b"123456"));
    assert!(store.data.read().unwrap().expirations.is_empty());
}

#[test]
fn increments_preserve_expiration_and_shared_key_updates_are_atomic() {
    let (store, clock) = setup(100);
    exec(&store, &[b"SET", b"count", b"0", b"PX", b"100"]);
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let store = &store;
            scope.spawn(move || {
                for _ in 0..1000 {
                    assert!(exec(store, &[b"INCR", b"count"]).starts_with(b":"));
                }
            });
        }
    });
    assert_eq!(exec(&store, &[b"GET", b"count"]), bulk(b"8000"));
    assert_eq!(exec(&store, &[b"PTTL", b"count"]), b":100\r\n");
    assert_eq!(exec(&store, &[b"DECRBY", b"count", b"-10"]), b":8010\r\n");
    clock.0.store(1101, Ordering::SeqCst);
    assert_eq!(exec(&store, &[b"DECR", b"count"]), b":-1\r\n");
    assert_eq!(exec(&store, &[b"PTTL", b"count"]), b":-1\r\n");
    let (store, _) = setup(2);
    exec(&store, &[b"SET", b"k", b"9"]);
    assert_eq!(exec(&store, &[b"INCR", b"k"]), options::OOM);
    assert_eq!(exec(&store, &[b"GET", b"k"]), bulk(b"9"));
}
