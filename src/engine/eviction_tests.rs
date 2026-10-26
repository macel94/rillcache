use super::*;
use std::sync::Arc;
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

fn store_with_policy(maximum: usize, policy: EvictionPolicy) -> (Store, Arc<TestClock>) {
    let clock = Arc::new(TestClock(AtomicI64::new(1000)));
    let store = Store::with_clock(maximum, clock.clone());
    assert!(store.set_eviction_policy(policy));
    (store, clock)
}

#[test]
fn noeviction_keeps_the_atomic_oom_contract() {
    let (store, _) = store_with_policy(10, EvictionPolicy::NoEviction);
    assert_eq!(call(&store, &[b"SET", b"a", b"1234"]), b"+OK\r\n");
    assert_eq!(call(&store, &[b"SET", b"b", b"1234"]), b"+OK\r\n");
    assert_eq!(call(&store, &[b"DBSIZE"]), b":2\r\n");
    assert!(call(&store, &[b"SET", b"c", b"v"]).starts_with(b"-OOM"));
    assert_eq!(call(&store, &[b"DBSIZE"]), b":2\r\n");
    assert_eq!(store.stats().evicted_keys, 0);
}

#[test]
fn allkeys_random_evicts_only_after_reaping_and_succeeds() {
    let (store, _) = store_with_policy(10, EvictionPolicy::AllKeysRandom);
    assert_eq!(call(&store, &[b"SET", b"a", b"1234"]), b"+OK\r\n");
    assert_eq!(call(&store, &[b"SET", b"b", b"1234"]), b"+OK\r\n");
    assert_eq!(call(&store, &[b"SET", b"c", b"v"]), b"+OK\r\n");
    assert_eq!(call(&store, &[b"DBSIZE"]), b":2\r\n");
    assert_eq!(call(&store, &[b"GET", b"c"]), b"$1\r\nv\r\n");
    assert_eq!(store.stats().evicted_keys, 1);
    let survivors = [
        call(&store, &[b"EXISTS", b"a"]),
        call(&store, &[b"EXISTS", b"b"]),
    ]
    .iter()
    .filter(|reply| reply.as_slice() == b":0\r\n")
    .count();
    assert_eq!(survivors, 1);
}

#[test]
fn volatile_policies_do_not_delete_persistent_keys() {
    let (store, _) = store_with_policy(10, EvictionPolicy::VolatileRandom);
    assert_eq!(call(&store, &[b"SET", b"a", b"1234"]), b"+OK\r\n");
    assert_eq!(call(&store, &[b"SET", b"b", b"1234"]), b"+OK\r\n");
    assert!(call(&store, &[b"SET", b"c", b"v"]).starts_with(b"-OOM"));
    assert_eq!(store.stats().evicted_keys, 0);

    let (store, _) = store_with_policy(10, EvictionPolicy::VolatileRandom);
    assert_eq!(
        call(&store, &[b"SET", b"a", b"1234", b"PX", b"10000"]),
        b"+OK\r\n"
    );
    assert_eq!(call(&store, &[b"SET", b"b", b"1234"]), b"+OK\r\n");
    assert_eq!(call(&store, &[b"SET", b"c", b"v"]), b"+OK\r\n");
    assert_eq!(call(&store, &[b"EXISTS", b"a"]), b":0\r\n");
    assert_eq!(call(&store, &[b"EXISTS", b"b"]), b":1\r\n");
    assert_eq!(store.stats().evicted_keys, 1);
}

#[test]
fn volatile_ttl_and_lru_choose_their_documented_candidates() {
    let (store, _clock) = store_with_policy(15, EvictionPolicy::VolatileTtl);
    for (key, ttl) in [
        (b"a".as_slice(), b"1000".as_slice()),
        (b"b", b"5000"),
        (b"c", b"2000"),
    ] {
        assert_eq!(
            call(&store, &[b"SET", key, b"value", b"PX", ttl]),
            b"+OK\r\n"
        );
    }
    assert_eq!(call(&store, &[b"SET", b"d", b"value"]), b"+OK\r\n");
    assert_eq!(call(&store, &[b"EXISTS", b"a"]), b":0\r\n");
    assert_eq!(call(&store, &[b"EXISTS", b"b"]), b":1\r\n");

    let (store, clock) = store_with_policy(10, EvictionPolicy::AllKeysLru);
    assert_eq!(call(&store, &[b"SET", b"a", b"1234"]), b"+OK\r\n");
    clock.0.store(2000, Ordering::SeqCst);
    assert_eq!(call(&store, &[b"SET", b"b", b"1234"]), b"+OK\r\n");
    clock.0.store(3000, Ordering::SeqCst);
    assert_eq!(call(&store, &[b"SET", b"c", b"v"]), b"+OK\r\n");
    assert_eq!(call(&store, &[b"EXISTS", b"a"]), b":0\r\n");
    assert_eq!(call(&store, &[b"EXISTS", b"b"]), b":1\r\n");
    assert_eq!(call(&store, &[b"EXISTS", b"c"]), b":1\r\n");
}
