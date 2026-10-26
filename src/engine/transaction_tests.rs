use super::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

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

fn changed(store: &Store, watched: &WatchState, now: i64) -> bool {
    let data = store.data.read().unwrap();
    watched.changed(&data, now)
}

#[test]
fn watch_detects_writes_deletes_expiry_and_flush() {
    let clock = Arc::new(TestClock(AtomicI64::new(1000)));
    let store = Store::with_clock(1024, clock.clone());

    let watched = store.watch_snapshot(&[b"k"]).unwrap();
    assert_eq!(call(&store, &[b"SET", b"k", b"v"]), b"+OK\r\n");
    assert!(changed(&store, &watched, 1000), "SET after WATCH aborts");

    let watched = store.watch_snapshot(&[b"k"]).unwrap();
    assert_eq!(call(&store, &[b"GET", b"k"]), b"$1\r\nv\r\n");
    assert!(!changed(&store, &watched, 1000), "reads must not abort");

    assert_eq!(call(&store, &[b"PEXPIRE", b"k", b"50"]), b":1\r\n");
    assert!(changed(&store, &watched, 1000), "TTL change aborts");

    let watched = store.watch_snapshot(&[b"k"]).unwrap();
    clock.0.store(1100, Ordering::SeqCst);
    assert!(
        changed(&store, &watched, 1100),
        "logical expiry must abort without polling"
    );

    let watched = store.watch_snapshot(&[b"absent"]).unwrap();
    assert_eq!(call(&store, &[b"FLUSHALL"]), b"+OK\r\n");
    assert!(changed(&store, &watched, 1100), "FLUSHALL aborts");
}

#[test]
fn aborted_transaction_emits_one_null_array_and_executes_nothing() {
    let store = Store::new(1024);
    assert_eq!(call(&store, &[b"SET", b"k", b"before"]), b"+OK\r\n");
    let watched = store.watch_snapshot(&[b"k"]).unwrap();
    assert_eq!(call(&store, &[b"SET", b"k", b"other"]), b"+OK\r\n");
    let mut out = Vec::new();
    let outcome = store.execute_transaction(
        &[
            vec![b"SET".to_vec(), b"k".to_vec(), b"queued".to_vec()],
            vec![b"INCR".to_vec(), b"n".to_vec()],
        ],
        &watched,
        &mut out,
    );
    assert_eq!(outcome, TransactionOutcome::Aborted);
    assert_eq!(out, b"*-1\r\n");
    assert_eq!(call(&store, &[b"GET", b"k"]), b"$5\r\nother\r\n");
    assert_eq!(call(&store, &[b"GET", b"n"]), b"$-1\r\n");
}

#[test]
fn transaction_replies_are_one_array_in_order_and_failures_do_not_roll_back() {
    let store = Store::new(1024);
    let watched = store.watch_snapshot(&[]).unwrap();
    let mut out = Vec::new();
    let outcome = store.execute_transaction(
        &[
            vec![b"SET".to_vec(), b"a".to_vec(), b"abc".to_vec()],
            vec![b"INCR".to_vec(), b"a".to_vec()],
            vec![b"INCR".to_vec(), b"a".to_vec()],
            vec![b"SET".to_vec(), b"b".to_vec(), b"ok".to_vec()],
            vec![b"GET".to_vec(), b"missing".to_vec()],
        ],
        &watched,
        &mut out,
    );
    assert_eq!(outcome, TransactionOutcome::Completed(Action::Continue));
    assert_eq!(
        out,
        b"*5\r\n+OK\r\n-ERR value is not an integer or out of range\r\n-ERR value is not an integer or out of range\r\n+OK\r\n$-1\r\n".to_vec()
    );
    assert_eq!(call(&store, &[b"GET", b"a"]), b"$3\r\nabc\r\n");
    assert_eq!(call(&store, &[b"GET", b"b"]), b"$2\r\nok\r\n");
}

#[test]
fn exec_is_exclusive_with_ordinary_reads() {
    let store = Arc::new(Store::new(16 * 1024 * 1024));
    const UPDATES: usize = 4000;
    let watched = store.watch_snapshot(&[]).unwrap();
    let mut commands = (0..UPDATES)
        .map(|value| {
            vec![
                b"SET".to_vec(),
                b"key".to_vec(),
                value.to_string().into_bytes(),
            ]
        })
        .collect::<Vec<_>>();
    commands.push(vec![b"SET".to_vec(), b"key".to_vec(), b"final".to_vec()]);

    let done = Arc::new(AtomicBool::new(false));
    let reader = {
        let store = Arc::clone(&store);
        let done = Arc::clone(&done);
        std::thread::spawn(move || {
            while !done.load(Ordering::Acquire) {
                let mut out = Vec::new();
                store.execute(&[b"GET", b"key"], &mut out);
                assert!(
                    out == b"$-1\r\n" || out == b"$5\r\nfinal\r\n",
                    "EXEC exposed an intermediate value: {out:?}"
                );
            }
        })
    };
    let mut out = Vec::new();
    let outcome = store.execute_transaction(&commands, &watched, &mut out);
    done.store(true, Ordering::Release);
    reader.join().unwrap();
    assert_eq!(outcome, TransactionOutcome::Completed(Action::Continue));
    assert_eq!(call(&store, &[b"GET", b"key"]), b"$5\r\nfinal\r\n");
}

#[test]
fn version_tracking_is_bounded_and_invalidates_watches() {
    let store = Store::new(1024);
    let watched = store.watch_snapshot(&[b"first"]).unwrap();
    {
        let mut data = store.data.write().unwrap();
        for index in 0..(super::data::MAX_TRACKED_VERSIONS + 8) {
            data.touch(format!("key-{index}").as_bytes());
        }
    }
    assert!(
        changed(&store, &watched, 0),
        "bounded version eviction must fail closed"
    );
    assert!(store.data.read().unwrap().versions.len() <= super::data::MAX_TRACKED_VERSIONS);
}
