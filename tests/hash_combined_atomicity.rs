#![forbid(unsafe_code)]

use rillcache::engine::{Clock, Store};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Barrier};

struct TestClock(AtomicI64);
impl Clock for TestClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::Relaxed)
    }
}

fn fixed_store(budget: usize) -> Store {
    Store::with_clock(budget, Arc::new(TestClock(AtomicI64::new(100))))
}

fn call(store: &Store, args: &[&[u8]]) -> Vec<u8> {
    let mut output = Vec::new();
    store.execute(args, &mut output);
    output
}

fn payload_is(store: &Store, bytes: usize) {
    let info = call(store, &[b"INFO", b"memory"]);
    let needle = format!("rillcache_payload_bytes:{bytes}\r\n");
    assert!(
        info.windows(needle.len())
            .any(|window| window == needle.as_bytes())
    );
}

#[test]
fn due_hgetex_deadlines_delete_sequential_duplicates_and_reclaim_payload() {
    let store = fixed_store(64);
    assert_eq!(
        call(
            &store,
            &[b"HSETEX", b"h", b"FIELDS", b"2", b"a", b"1", b"b", b"2"]
        ),
        b":1\r\n"
    );
    assert_eq!(call(&store, &[b"PEXPIRE", b"h", b"1000"]), b":1\r\n");
    assert_eq!(
        call(
            &store,
            &[
                b"HGETEX", b"h", b"PXAT", b"100", b"FIELDS", b"3", b"a", b"a", b"missing"
            ]
        ),
        b"*3\r\n$1\r\n1\r\n$-1\r\n$-1\r\n"
    );
    assert_eq!(call(&store, &[b"HGET", b"h", b"b"]), b"$1\r\n2\r\n");
    assert_eq!(
        call(&store, &[b"HPTTL", b"h", b"FIELDS", b"2", b"a", b"b"]),
        b"*2\r\n:-2\r\n:-1\r\n"
    );
    assert_eq!(call(&store, &[b"PTTL", b"h"]), b":1000\r\n");
    payload_is(&store, 3);
    assert_eq!(
        call(
            &store,
            &[b"HGETEX", b"h", b"PX", b"0", b"FIELDS", b"2", b"b", b"b"]
        ),
        b"*2\r\n$1\r\n2\r\n$-1\r\n"
    );
    assert_eq!(call(&store, &[b"EXISTS", b"h"]), b":0\r\n");
    payload_is(&store, 0);
}

#[test]
fn due_hsetex_does_not_reserve_discarded_values_and_preserves_other_fields() {
    let store = fixed_store(16);
    assert_eq!(
        call(
            &store,
            &[
                b"HSETEX", b"h", b"PX", b"1000", b"FIELDS", b"2", b"a", b"1", b"b", b"2"
            ]
        ),
        b":1\r\n"
    );
    assert_eq!(call(&store, &[b"PEXPIRE", b"h", b"2000"]), b":1\r\n");
    let large = vec![b'x'; 1024];
    assert_eq!(
        call(
            &store,
            &[
                b"HSETEX", b"h", b"PX", b"0", b"FIELDS", b"2", b"a", &large, b"c", &large
            ]
        ),
        b":1\r\n"
    );
    payload_is(&store, 3);
    assert_eq!(call(&store, &[b"HGET", b"h", b"b"]), b"$1\r\n2\r\n");
    assert_eq!(
        call(&store, &[b"HPTTL", b"h", b"FIELDS", b"3", b"a", b"c", b"b"]),
        b"*3\r\n:-2\r\n:-2\r\n:1000\r\n"
    );
    assert_eq!(call(&store, &[b"PTTL", b"h"]), b":2000\r\n");
    assert_eq!(
        call(
            &store,
            &[
                b"HSETEX", b"h", b"FXX", b"EX", b"0", b"FIELDS", b"2", b"b", b"3", b"b", b"4"
            ]
        ),
        b":1\r\n"
    );
    assert_eq!(call(&store, &[b"EXISTS", b"h"]), b":0\r\n");
    payload_is(&store, 0);
}

#[test]
fn counted_fields_allow_trailing_options_without_parsing_binary_fields_as_options() {
    let store = fixed_store(64);
    let field = b"\0\r\n\xff";
    assert_eq!(
        call(
            &store,
            &[
                b"HSETEX", b"h", b"FIELDS", b"1", field, b"\xff", b"FNX", b"PX", b"1000"
            ]
        ),
        b":1\r\n"
    );
    assert_eq!(
        call(
            &store,
            &[b"HGETEX", b"h", b"FIELDS", b"2", field, field, b"PERSIST"]
        ),
        b"*2\r\n$1\r\n\xff\r\n$1\r\n\xff\r\n"
    );
    assert_eq!(
        call(&store, &[b"HPTTL", b"h", b"FIELDS", b"1", field]),
        b"*1\r\n:-1\r\n"
    );
    assert_eq!(
        call(
            &store,
            &[
                b"HGETEX", b"h", b"FIELDS", b"1", field, b"FIELDS", b"1", field
            ]
        ),
        b"-ERR FIELDS keyword specified multiple times\r\n"
    );
}

#[test]
fn amplified_hgetex_reply_is_bounded_and_rejection_preserves_ttls() {
    let store = fixed_store(2 * 1024 * 1024);
    let large = vec![b'x'; 1024 * 1024];
    assert_eq!(
        call(
            &store,
            &[
                b"HSETEX", b"h", b"PX", b"1000", b"FIELDS", b"1", b"a", &large
            ]
        ),
        b":1\r\n"
    );
    let response = call(
        &store,
        &[
            b"HGETEX", b"h", b"PX", b"5000", b"FIELDS", b"3", b"a", b"a", b"a",
        ],
    );
    assert!(
        response == b"-ERR HGETEX reply exceeds configured limit\r\n",
        "unexpected reply length {}",
        response.len()
    );
    assert_eq!(
        call(&store, &[b"HPEXPIRETIME", b"h", b"FIELDS", b"1", b"a"]),
        b"*1\r\n:1100\r\n"
    );
    payload_is(&store, large.len() + 2);
    // A due deadline returns the large value only once; duplicates then see null.
    let response = call(
        &store,
        &[
            b"HGETEX", b"h", b"PX", b"0", b"FIELDS", b"3", b"a", b"a", b"a",
        ],
    );
    assert_eq!(
        response.len(),
        large.len() + b"*3\r\n$1048576\r\n\r\n$-1\r\n$-1\r\n".len()
    );
    assert!(response.ends_with(b"\r\n$-1\r\n$-1\r\n"));
    payload_is(&store, 0);
}

#[test]
fn hgetex_reply_budget_includes_framing_and_preserves_prior_output_on_error() {
    let store = fixed_store(3 * 1024 * 1024);
    let exact = vec![b'x'; 2 * 1024 * 1024 - 16];
    assert_eq!(
        call(&store, &[b"HSETEX", b"h", b"FIELDS", b"1", b"a", &exact]),
        b":1\r\n"
    );
    assert_eq!(
        call(&store, &[b"HGETEX", b"h", b"FIELDS", b"1", b"a"]).len(),
        2 * 1024 * 1024
    );
    let larger = vec![b'x'; exact.len() + 1];
    assert_eq!(
        call(
            &store,
            &[
                b"HSETEX", b"h", b"PX", b"1000", b"FIELDS", b"1", b"a", &larger
            ]
        ),
        b":1\r\n"
    );
    let mut output = b"previous reply".to_vec();
    store.execute(
        &[b"HGETEX", b"h", b"PERSIST", b"FIELDS", b"1", b"a"],
        &mut output,
    );
    assert_eq!(
        output,
        b"previous reply-ERR HGETEX reply exceeds configured limit\r\n"
    );
    assert_eq!(
        call(&store, &[b"HPTTL", b"h", b"FIELDS", b"1", b"a"]),
        b"*1\r\n:1000\r\n"
    );
}

#[test]
fn hsetex_oom_and_failed_condition_preserve_values_key_and_field_ttls() {
    let store = fixed_store(8);
    assert_eq!(
        call(
            &store,
            &[b"HSETEX", b"h", b"PX", b"50", b"FIELDS", b"1", b"a", b"v"]
        ),
        b":1\r\n"
    );
    assert_eq!(call(&store, &[b"PEXPIRE", b"h", b"200"]), b":1\r\n");
    let large = b"0123456789";
    assert_eq!(
        call(
            &store,
            &[
                b"HSETEX", b"h", b"FXX", b"PX", b"5000", b"FIELDS", b"2", b"a", large, b"b", large
            ]
        ),
        b":0\r\n"
    );
    assert!(
        call(
            &store,
            &[
                b"HSETEX", b"h", b"PX", b"5000", b"FIELDS", b"2", b"a", large, b"b", large
            ]
        )
        .starts_with(b"-OOM")
    );
    assert_eq!(call(&store, &[b"HGET", b"h", b"a"]), b"$1\r\nv\r\n");
    assert_eq!(
        call(&store, &[b"HPTTL", b"h", b"FIELDS", b"2", b"a", b"b"]),
        b"*2\r\n:50\r\n:-2\r\n"
    );
    assert_eq!(call(&store, &[b"PTTL", b"h"]), b":200\r\n");
    payload_is(&store, 3);
}

#[test]
fn concurrent_hash_fnx_writers_have_exactly_one_winner() {
    let store = Arc::new(fixed_store(4096));
    let barrier = Arc::new(Barrier::new(8));
    let handles: Vec<_> = (0..8)
        .map(|index| {
            let store = store.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let value = index.to_string();
                barrier.wait();
                let response = call(
                    &store,
                    &[
                        b"HSETEX",
                        b"h",
                        b"FNX",
                        b"FIELDS",
                        b"1",
                        b"a",
                        value.as_bytes(),
                    ],
                );
                assert!(response == b":1\r\n" || response == b":0\r\n");
                (value, response == b":1\r\n")
            })
        })
        .collect();
    let outcomes: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    let winners: Vec<_> = outcomes.iter().filter(|(_, won)| *won).collect();
    assert_eq!(winners.len(), 1);
    assert_eq!(
        call(&store, &[b"HGET", b"h", b"a"]),
        format!("$1\r\n{}\r\n", winners[0].0).as_bytes()
    );
}

#[test]
fn combined_hash_deadlines_enforce_the_verified_redis_ceiling() {
    let store = fixed_store(64);
    let maximum = ((1i64 << 48) - 1) >> 2;
    let accepted = maximum.to_string();
    let rejected = (maximum + 1).to_string();
    assert_eq!(
        call(
            &store,
            &[
                b"HSETEX",
                b"h",
                b"PXAT",
                accepted.as_bytes(),
                b"FIELDS",
                b"1",
                b"a",
                b"v"
            ]
        ),
        b":1\r\n"
    );
    assert_eq!(
        call(
            &store,
            &[
                b"HGETEX",
                b"h",
                b"PXAT",
                rejected.as_bytes(),
                b"FIELDS",
                b"1",
                b"a"
            ]
        ),
        b"-ERR invalid expire time in 'hgetex' command\r\n"
    );
    assert_eq!(
        call(
            &store,
            &[
                b"HSETEX",
                b"absent",
                b"PX",
                b"9223372036854775807",
                b"FIELDS",
                b"1",
                b"a",
                b"v"
            ]
        ),
        b"-ERR invalid expire time in 'hsetex' command\r\n"
    );
    assert_eq!(call(&store, &[b"EXISTS", b"absent"]), b":0\r\n");
    assert_eq!(
        call(&store, &[b"HPEXPIRETIME", b"h", b"FIELDS", b"1", b"a"]),
        format!("*1\r\n:{maximum}\r\n").as_bytes()
    );
}

#[test]
fn expired_field_is_absent_for_fnx_at_its_exact_deadline() {
    let clock = Arc::new(TestClock(AtomicI64::new(100)));
    let store = Store::with_clock(64, clock.clone());
    assert_eq!(
        call(
            &store,
            &[b"HSETEX", b"h", b"PX", b"10", b"FIELDS", b"1", b"a", b"old"]
        ),
        b":1\r\n"
    );
    clock.0.store(110, Ordering::Relaxed);
    assert_eq!(
        call(
            &store,
            &[b"HSETEX", b"h", b"FNX", b"FIELDS", b"1", b"a", b"new"]
        ),
        b":1\r\n"
    );
    assert_eq!(call(&store, &[b"HGET", b"h", b"a"]), b"$3\r\nnew\r\n");
}
