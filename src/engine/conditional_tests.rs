use super::*;
use std::sync::Barrier;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::thread;

struct TestClock(AtomicI64);
impl Clock for TestClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}
fn store(maximum: usize) -> (Store, Arc<TestClock>) {
    let clock = Arc::new(TestClock(AtomicI64::new(100)));
    (Store::with_clock(maximum, clock.clone()), clock)
}
fn run(store: &Store, args: &[&[u8]]) -> Vec<u8> {
    let mut reply = Vec::new();
    assert_eq!(store.execute(args, &mut reply), Action::Continue);
    reply
}

#[test]
fn value_and_digest_conditions_preserve_ttl_and_budget_on_rejection_and_error() {
    for condition in [b"IFEQ".as_slice(), b"IFNE", b"IFDEQ", b"IFDNE"] {
        let (store, clock) = store(16);
        assert_eq!(
            run(&store, &[b"SET", b"k", b"\0\xff", b"PX", b"50"]),
            b"+OK\r\n"
        );
        let digest_reply = run(&store, &[b"DIGEST", b"k"]);
        let digest = &digest_reply[5..21];
        let (matching, different): (&[u8], &[u8]) = if condition.starts_with(b"IFD") {
            (digest, b"xxxxxxxxxxxxxxxx")
        } else {
            (b"\0\xff", b"wrong")
        };
        let unequal = condition == b"IFNE" || condition == b"IFDNE";
        let rejected = if unequal { matching } else { different };
        let admitted = if unequal { different } else { matching };
        assert_eq!(
            run(
                &store,
                &[
                    b"SET",
                    b"k",
                    b"too large for admission",
                    condition,
                    rejected
                ]
            ),
            b"$-1\r\n"
        );
        assert_eq!(
            run(&store, &[b"SET", b"k", b"no", condition, rejected, b"GET"]),
            b"$2\r\n\0\xff\r\n"
        );
        assert_eq!(
            run(
                &store,
                &[
                    b"SET",
                    b"k",
                    b"too large for admission",
                    condition,
                    admitted,
                    b"GET"
                ]
            ),
            options::OOM
        );
        assert_eq!(run(&store, &[b"GET", b"k"]), b"$2\r\n\0\xff\r\n");
        assert_eq!(run(&store, &[b"PTTL", b"k"]), b":50\r\n");
        assert_eq!(store.data.read().unwrap().payload_bytes, 3);
        for other in [
            b"NX".as_slice(),
            b"XX",
            b"IFEQ",
            b"IFNE",
            b"IFDEQ",
            b"IFDNE",
        ] {
            if other == condition {
                continue;
            }
            let mut args: Vec<&[u8]> = vec![b"SET", b"k", b"no", condition, admitted, other];
            if other != b"NX" && other != b"XX" {
                args.push(b"argument");
            }
            assert_eq!(run(&store, &args), options::SYNTAX);
        }
        // Repeating the same comparison kind replaces its operand, like Redis.
        assert_eq!(
            run(
                &store,
                &[
                    b"SET", b"k", b"ok", condition, rejected, condition, admitted, b"KEEPTTL"
                ]
            ),
            b"+OK\r\n"
        );
        assert_eq!(run(&store, &[b"PTTL", b"k"]), b":50\r\n");
        clock.0.store(151, Ordering::SeqCst);
        assert_eq!(run(&store, &[b"DIGEST", b"k"]), b"$-1\r\n");
        assert_eq!(run(&store, &[b"DELEX", b"k", b"IFDEQ", b"bad"]), b":0\r\n");
        assert_eq!(store.data.read().unwrap().payload_bytes, 0);
        assert!(store.data.read().unwrap().expirations.is_empty());
    }
}

#[test]
fn malformed_digest_length_and_missing_key_precedence_are_native() {
    let (store, _) = store(128);
    for condition in [b"IFDEQ".as_slice(), b"IFDNE"] {
        for bad in [b"".as_slice(), b"a", b"12345678901234567", b"\xff"] {
            assert_eq!(run(&store, &[b"SET", b"k", b"v"]), b"+OK\r\n");
            assert_eq!(
                run(&store, &[b"SET", b"k", b"no", condition, bad]),
                b"-ERR must be exactly 16 hexadecimal characters\r\n"
            );
            assert_eq!(
                run(&store, &[b"DELEX", b"k", condition, bad]),
                b"-ERR must be exactly 16 hexadecimal characters\r\n"
            );
            assert_eq!(run(&store, &[b"GET", b"k"]), b"$1\r\nv\r\n");
            assert_eq!(run(&store, &[b"DEL", b"k"]), b":1\r\n");
            assert_eq!(run(&store, &[b"DELEX", b"k", condition, bad]), b":0\r\n");
            let expected: &[u8] = if condition == b"IFDEQ" {
                b"$-1\r\n"
            } else {
                b"+OK\r\n"
            };
            assert_eq!(
                run(&store, &[b"SET", b"k", b"new", condition, bad]),
                expected
            );
            run(&store, &[b"DEL", b"k"]);
        }
    }
    assert_eq!(
        run(&store, &[b"DELEX", b"k", b"BAD", b"ignored"]),
        b":0\r\n"
    );
}

#[test]
fn typed_comparisons_fail_without_mutation_and_msetex_reclaims_typed_payload() {
    let (store, _) = store(32);
    assert_eq!(
        run(
            &store,
            &[b"XADD", b"k", b"1-0", b"f", b"12345678901234567890"]
        ),
        b"$3\r\n1-0\r\n"
    );
    assert_eq!(run(&store, &[b"PEXPIREAT", b"k", b"150"]), b":1\r\n");
    assert_eq!(
        run(&store, &[b"MGET", b"k", b"missing"]),
        b"*2\r\n$-1\r\n$-1\r\n"
    );
    for args in [
        vec![b"DIGEST".as_slice(), b"k"],
        vec![b"DELIFEQ", b"k", b""],
        vec![b"SET", b"k", b"new", b"IFEQ", b""],
        vec![b"SET", b"k", b"new", b"IFNE", b"x"],
        vec![b"SET", b"k", b"new", b"IFDEQ", b"bad"],
        vec![b"SET", b"k", b"new", b"IFDNE", b"bad", b"GET"],
    ] {
        assert_eq!(
            run(&store, &args),
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n"
        );
    }
    assert_eq!(
        run(&store, &[b"DELEX", b"k", b"BAD", b"ignored"]),
        b"-ERR Key should be of string type if conditions are specified\r\n"
    );
    assert_eq!(
        run(
            &store,
            &[b"MSETEX", b"2", b"k", b"new", b"b", b"new", b"XX"]
        ),
        b":0\r\n"
    );
    assert_eq!(
        run(
            &store,
            &[
                b"MSETEX",
                b"2",
                b"k",
                b"new",
                b"b",
                b"123456789012345678901234567890"
            ]
        ),
        options::OOM
    );
    assert_eq!(run(&store, &[b"XLEN", b"k"]), b":1\r\n");
    assert_eq!(run(&store, &[b"PTTL", b"k"]), b":50\r\n");
    assert_eq!(store.data.read().unwrap().payload_bytes, 22);
    assert_eq!(
        run(
            &store,
            &[
                b"MSETEX",
                b"3",
                b"k",
                b"discarded",
                b"k",
                b"x",
                b"b",
                b"1234",
                b"KEEPTTL"
            ]
        ),
        b":1\r\n"
    );
    assert_eq!(run(&store, &[b"GET", b"k"]), b"$1\r\nx\r\n");
    assert_eq!(run(&store, &[b"PTTL", b"k"]), b":50\r\n");
    assert_eq!(run(&store, &[b"PTTL", b"b"]), b":-1\r\n");
    assert_eq!(store.data.read().unwrap().payload_bytes, 7);
    assert_eq!(run(&store, &[b"DEL", b"k", b"b"]), b":2\r\n");
    assert_eq!(store.data.read().unwrap().payload_bytes, 0);
    run(&store, &[b"XADD", b"k", b"1-0", b"f", b"v"]);
    assert_eq!(run(&store, &[b"DELEX", b"k"]), b":1\r\n");
    assert_eq!(store.data.read().unwrap().payload_bytes, 0);
}

#[test]
fn msetex_strict_counts_expiry_and_option_errors_preserve_the_whole_batch() {
    let (store, clock) = store(32);
    run(&store, &[b"SET", b"k", b"old", b"PX", b"50"]);
    for count in [
        b"0".as_slice(),
        b"-1",
        b"01",
        b"+1",
        b"1.5",
        b"2147483648",
        b"9223372036854775808",
    ] {
        assert_eq!(
            run(&store, &[b"MSETEX", count, b"k", b"new"]),
            b"-ERR invalid numkeys value or out of range\r\n"
        );
    }
    for count in [b"2".as_slice(), b"1073741823", b"2147483647"] {
        assert_eq!(
            run(&store, &[b"MSETEX", count, b"k", b"new"]),
            b"-ERR wrong number of key-value pairs\r\n"
        );
    }
    for invalid in [
        vec![b"GET".as_slice()],
        vec![b"IFEQ", b"old"],
        vec![b"NX", b"XX"],
        vec![b"XX", b"NX"],
        vec![b"PX"],
        vec![b"EX", b"1", b"PX", b"1"],
        vec![b"KEEPTTL", b"PX", b"1"],
        vec![b"NX\0"],
        vec![b"KEEPTTL\0"],
    ] {
        let mut args: Vec<&[u8]> = vec![b"MSETEX", b"2", b"k", b"new", b"b", b"other"];
        args.extend(invalid);
        assert_eq!(run(&store, &args), options::SYNTAX);
    }
    for kind in [b"EX".as_slice(), b"PX", b"EXAT", b"PXAT"] {
        for bad in [b"0".as_slice(), b"-1"] {
            assert_eq!(
                run(&store, &[b"MSETEX", b"1", b"k", b"new", b"NX", kind, bad]),
                b"-ERR invalid expire time in 'msetex' command\r\n"
            );
        }
        assert_eq!(
            run(&store, &[b"MSETEX", b"1", b"k", b"new", kind, b"1.5"]),
            options::NOT_INTEGER
        );
    }
    assert_eq!(
        run(
            &store,
            &[b"MSETEX", b"1", b"k", b"new", b"EX", b"9223372036854775807"]
        ),
        b"-ERR invalid expire time in 'msetex' command\r\n"
    );
    assert_eq!(run(&store, &[b"GET", b"k"]), b"$3\r\nold\r\n");
    assert_eq!(run(&store, &[b"GET", b"b"]), b"$-1\r\n");
    assert_eq!(run(&store, &[b"PTTL", b"k"]), b":50\r\n");
    clock.0.store(151, Ordering::SeqCst);
    assert_eq!(
        run(&store, &[b"MSETEX", b"1", b"k", b"new", b"XX"]),
        b":0\r\n"
    );
    assert_eq!(
        run(
            &store,
            &[
                b"MSETEX", b"1", b"k", b"new", b"NX", b"PX", b"5", b"PX", b"10"
            ]
        ),
        b":1\r\n"
    );
    assert_eq!(run(&store, &[b"PTTL", b"k"]), b":10\r\n");
}

#[test]
fn concurrent_value_and_digest_compares_and_deletes_have_one_winner() {
    for condition in [b"IFEQ".as_slice(), b"IFDEQ"] {
        let (store, _) = store(128);
        run(&store, &[b"SET", b"k", b"old"]);
        let digest = run(&store, &[b"DIGEST", b"k"]);
        let expected: &[u8] = if condition == b"IFEQ" {
            b"old"
        } else {
            &digest[5..21]
        };
        for delete in [false, true] {
            run(&store, &[b"SET", b"k", b"old"]);
            let barrier = Barrier::new(8);
            let winners = AtomicUsize::new(0);
            thread::scope(|scope| {
                for _ in 0..8 {
                    let (store, barrier, winners) = (&store, &barrier, &winners);
                    scope.spawn(move || {
                        barrier.wait();
                        let reply = if delete {
                            run(store, &[b"DELEX", b"k", condition, expected])
                        } else {
                            run(store, &[b"SET", b"k", b"new", condition, expected])
                        };
                        if reply
                            == if delete {
                                b":1\r\n".as_slice()
                            } else {
                                b"+OK\r\n"
                            }
                        {
                            winners.fetch_add(1, Ordering::SeqCst);
                        } else {
                            assert_eq!(
                                reply,
                                if delete {
                                    b":0\r\n".as_slice()
                                } else {
                                    b"$-1\r\n"
                                }
                            );
                        }
                    });
                }
            });
            assert_eq!(winners.load(Ordering::SeqCst), 1);
        }
    }
}
