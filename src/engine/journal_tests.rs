use crate::engine::{Auth, FsyncPolicy, JournalAuth, PopOutcome, Store};
use crate::protocol::Limits;
use std::io::Write;

#[test]
fn crc32_matches_the_standard_check_vector() {
    assert_eq!(!super::crc32_update(!0u32, b"123456789"), 0xcbf4_3926);
}

#[test]
fn rewrite_progress_reports_snapshot_reconcile_sync_and_commit() {
    let mut job = super::RewriteJob {
        keys: vec![b"a".to_vec(), b"b".to_vec()],
        cursor: 0,
        pending: std::collections::VecDeque::new(),
        synced: false,
        rounds: 0,
    };
    assert_eq!(job.progress(), ("snapshot", 0, 2));
    job.cursor = 2;
    assert_eq!(job.progress(), ("sync", 2, 2));
    job.pending.push_back(b"a".to_vec());
    assert_eq!(job.progress(), ("reconcile", 2, 2));
    job.pending.clear();
    job.synced = true;
    assert_eq!(job.progress(), ("commit", 2, 2));
}

fn aof_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rillcache-aof-{label}-{}-{:?}.aof",
        std::process::id(),
        std::thread::current().id()
    ))
}

fn open(path: &std::path::Path, policy: FsyncPolicy) -> Store {
    Store::with_auth_and_journal(
        16 * 1024 * 1024,
        Auth::default(),
        path,
        policy,
        Limits::default(),
        None,
    )
    .unwrap()
}

#[test]
fn clock_rollback_after_restart_can_resurrect_a_persisted_expiry() {
    use crate::engine::Clock;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicI64, Ordering};

    struct TestClock(AtomicI64);
    impl Clock for TestClock {
        fn now_ms(&self) -> i64 {
            self.0.load(Ordering::SeqCst)
        }
    }

    let path = aof_path("clock-rollback");
    let initial_time = 1_700_000_000_000_i64;
    let clock = Arc::new(TestClock(AtomicI64::new(initial_time)));
    {
        let store =
            Store::with_clock_and_journal(&path, FsyncPolicy::Always, clock.clone()).unwrap();
        assert_eq!(
            run(&store, &[b"SET", b"ttl", b"value", b"PX", b"1000"]),
            b"+OK\r\n"
        );
    }

    // Replay after the deadline, but do not access the key: with no persisted
    // clock high-water mark, the original absolute deadline remains in the AOF.
    clock.0.store(initial_time + 2_000, Ordering::SeqCst);
    {
        let store =
            Store::with_clock_and_journal(&path, FsyncPolicy::Always, clock.clone()).unwrap();
        assert_eq!(run(&store, &[b"PING"]), b"+PONG\r\n");
    }

    // A backward wall-clock step before expiry is observed by a command or the
    // background reaper lets the same persisted deadline become live on replay.
    clock.0.store(initial_time + 500, Ordering::SeqCst);
    let store = Store::with_clock_and_journal(&path, FsyncPolicy::Always, clock).unwrap();
    assert_eq!(run(&store, &[b"GET", b"ttl"]), b"$5\r\nvalue\r\n");
    drop(store);
    let _ = std::fs::remove_file(path);
}

const AUTH_KEY: &[u8] = b"0123456789abcdef";

fn open_with_auth(path: &std::path::Path, key: &[u8], migrate: bool) -> std::io::Result<Store> {
    Store::with_auth_and_journal(
        16 * 1024 * 1024,
        Auth::default(),
        path,
        FsyncPolicy::Always,
        Limits::default(),
        Some(JournalAuth::new(key.to_vec(), migrate)?),
    )
}

fn run(store: &Store, arguments: &[&[u8]]) -> Vec<u8> {
    let mut output = Vec::new();
    store.execute(arguments, &mut output);
    output
}

#[test]
fn msetex_reaping_does_not_publish_false_write_events_on_rejection_or_oom() {
    use crate::engine::{Clock, NotifyFlags};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicI64, Ordering};

    struct TestClock(AtomicI64);
    impl Clock for TestClock {
        fn now_ms(&self) -> i64 {
            self.0.load(Ordering::SeqCst)
        }
    }
    let clock = Arc::new(TestClock(AtomicI64::new(100)));
    let store = Store::with_clock(12, clock.clone());
    store.set_notify_flags(NotifyFlags::parse(b"EA").unwrap());
    let (sender, receiver) = std::sync::mpsc::sync_channel(Store::pubsub_queue_capacity());
    assert!(store.pubsub_subscribe(true, b"__keyevent@0__:*", 1, sender, false));
    assert_eq!(run(&store, &[b"SET", b"k", b"v"]), b"+OK\r\n");
    assert_eq!(run(&store, &[b"SET", b"s", b"v", b"PX", b"1"]), b"+OK\r\n");
    while receiver.try_recv().is_ok() {}

    for (arguments, expected) in [
        (
            vec![b"MSETEX".as_slice(), b"1", b"k", b"new", b"NX", b"EX", b"1"],
            b":0\r\n".as_slice(),
        ),
        (
            vec![
                b"MSETEX".as_slice(),
                b"1",
                b"missing",
                b"new",
                b"XX",
                b"EX",
                b"1",
            ],
            b":0\r\n".as_slice(),
        ),
        (
            vec![
                b"MSETEX".as_slice(),
                b"1",
                b"k",
                b"too-large-value",
                b"EX",
                b"1",
            ],
            b"-OOM configured payload limit exceeded\r\n".as_slice(),
        ),
    ] {
        clock.0.fetch_add(2, Ordering::SeqCst);
        assert_eq!(run(&store, &arguments), expected);
        let mut expired = Vec::new();
        crate::protocol::encode_request(
            &[
                b"pmessage",
                b"__keyevent@0__:*",
                b"__keyevent@0__:expired",
                b"s",
            ],
            &mut expired,
        );
        assert_eq!(receiver.try_recv().unwrap(), expired);
        assert!(
            receiver.try_recv().is_err(),
            "rejected MSETEX must not emit set/expire"
        );
        assert_eq!(run(&store, &[b"GET", b"k"]), b"$1\r\nv\r\n");
        assert_eq!(run(&store, &[b"SET", b"s", b"v", b"PX", b"1"]), b"+OK\r\n");
        while receiver.try_recv().is_ok() {}
    }
    clock.0.fetch_add(2, Ordering::SeqCst);
    assert_eq!(
        run(&store, &[b"MSETEX", b"1", b"k", b"new", b"PX", b"100"]),
        b":1\r\n"
    );
    for (event, key) in [
        (b"expired".as_slice(), b"s".as_slice()),
        (b"set", b"k"),
        (b"expire", b"k"),
    ] {
        let mut channel = b"__keyevent@0__:".to_vec();
        channel.extend_from_slice(event);
        let mut expected = Vec::new();
        crate::protocol::encode_request(
            &[b"pmessage", b"__keyevent@0__:*", &channel, key],
            &mut expected,
        );
        assert_eq!(receiver.try_recv().unwrap(), expected);
    }
    assert!(
        receiver.try_recv().is_err(),
        "unrelated reaped key has no set event"
    );
}

#[test]
fn hsetex_reaping_emits_only_real_successful_target_write_events() {
    use crate::engine::{Clock, NotifyFlags};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicI64, Ordering};
    struct TestClock(AtomicI64);
    impl Clock for TestClock {
        fn now_ms(&self) -> i64 {
            self.0.load(Ordering::SeqCst)
        }
    }
    let clock = Arc::new(TestClock(AtomicI64::new(100)));
    let store = Store::with_clock(12, clock.clone());
    store.set_notify_flags(NotifyFlags::parse(b"EA").unwrap());
    let (sender, receiver) = std::sync::mpsc::sync_channel(Store::pubsub_queue_capacity());
    assert!(store.pubsub_subscribe(true, b"__keyevent@0__:*", 1, sender, false));
    assert_eq!(
        run(
            &store,
            &[b"HSETEX", b"h\0\xff", b"FIELDS", b"1", b"a", b"v"]
        ),
        b":1\r\n"
    );
    let cases: &[(&[&[u8]], &[u8])] = &[
        (
            &[b"HSETEX", b"h\0\xff", b"FNX", b"FIELDS", b"1", b"a", b"new"],
            b":0\r\n",
        ),
        (
            &[
                b"HSETEX", b"h\0\xff", b"FXX", b"FIELDS", b"1", b"missing", b"new",
            ],
            b":0\r\n",
        ),
        (
            &[
                b"HSETEX",
                b"h\0\xff",
                b"FIELDS",
                b"1",
                b"a",
                b"too-large-value",
            ],
            b"-OOM configured payload limit exceeded\r\n",
        ),
        (
            &[b"HSETEX", b"h\0\xff", b"FIELDS", b"1", b"a", b"newnew"],
            b":1\r\n",
        ),
    ];
    for (arguments, expected) in cases {
        assert_eq!(
            run(&store, &[b"SET", b"s", b"vvvv", b"PX", b"1"]),
            b"+OK\r\n"
        );
        while receiver.try_recv().is_ok() {}
        clock.0.fetch_add(2, Ordering::SeqCst);
        assert_eq!(run(&store, arguments), *expected);
        let events = if *expected == b":1\r\n" {
            vec![
                (b"expired".as_slice(), b"s".as_slice()),
                (b"hset", b"h\0\xff"),
            ]
        } else if expected.starts_with(b"-OOM") {
            vec![(b"expired".as_slice(), b"s".as_slice())]
        } else {
            Vec::new()
        };
        for (event, key) in events {
            let mut channel = b"__keyevent@0__:".to_vec();
            channel.extend_from_slice(event);
            let mut frame = Vec::new();
            crate::protocol::encode_request(
                &[b"pmessage", b"__keyevent@0__:*", &channel, key],
                &mut frame,
            );
            assert_eq!(receiver.try_recv().unwrap(), frame);
        }
        assert!(
            receiver.try_recv().is_err(),
            "no hset event for rejected writes or unrelated expired keys"
        );
    }
    // Lazy expiry of a field on the target must not label a rejected FNX/FXX
    // write as hset either; field lifecycle notifications are not implemented.
    for arguments in [
        vec![
            b"HSETEX".as_slice(),
            b"h\0\xff",
            b"FNX",
            b"FIELDS",
            b"1",
            b"a",
            b"new",
        ],
        vec![
            b"HSETEX".as_slice(),
            b"h\0\xff",
            b"FXX",
            b"FIELDS",
            b"2",
            b"a",
            b"new",
            b"b",
            b"new",
        ],
    ] {
        assert_eq!(
            run(
                &store,
                &[
                    b"HSETEX", b"h\0\xff", b"PX", b"1", b"FIELDS", b"1", b"b", b"v"
                ]
            ),
            b":1\r\n"
        );
        while receiver.try_recv().is_ok() {}
        clock.0.fetch_add(2, Ordering::SeqCst);
        assert_eq!(run(&store, &arguments), b":0\r\n");
        assert!(
            receiver.try_recv().is_err(),
            "rejected field expiry must not emit hset"
        );
        assert_eq!(
            run(&store, &[b"HGET", b"h\0\xff", b"a"]),
            b"$6\r\nnewnew\r\n"
        );
    }
}

#[test]
fn replay_restores_strings_collections_streams_and_expiry() {
    let path = aof_path("roundtrip");
    let deadline = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64)
        + 3_600_000;
    {
        let store = open(&path, FsyncPolicy::Always);
        assert_eq!(
            run(&store, &[b"SET", b"string\0", b"value\xff"]),
            b"+OK\r\n"
        );
        assert_eq!(
            run(&store, &[b"RPUSH", b"list", b"one", b"two", b"three"]),
            b":3\r\n"
        );
        assert_eq!(run(&store, &[b"SADD", b"set", b"b", b"a"]), b":2\r\n");
        assert_eq!(
            run(
                &store,
                &[b"HSET", b"hash", b"field", b"value", b"other", b"x"]
            ),
            b":2\r\n"
        );
        assert_eq!(
            run(&store, &[b"ZADD", b"zset", b"2.5", b"b", b"-1", b"a"]),
            b":2\r\n"
        );
        assert_eq!(
            run(&store, &[b"XADD", b"stream", b"1-0", b"f", b"v"]),
            b"$3\r\n1-0\r\n"
        );
        assert_eq!(
            run(&store, &[b"XADD", b"stream", b"2-0", b"g", b"w"]),
            b"$3\r\n2-0\r\n"
        );
        assert_eq!(
            run(&store, &[b"XGROUP", b"CREATE", b"stream", b"group", b"1-0"]),
            b"+OK\r\n"
        );
        let deadline = deadline.to_string();
        assert_eq!(
            run(&store, &[b"PEXPIREAT", b"string\0", deadline.as_bytes()]),
            b":1\r\n"
        );
    }

    let store = open(&path, FsyncPolicy::Always);
    assert_eq!(run(&store, &[b"GET", b"string\0"]), b"$6\r\nvalue\xff\r\n");
    assert_eq!(
        run(&store, &[b"LRANGE", b"list", b"0", b"-1"]),
        b"*3\r\n$3\r\none\r\n$3\r\ntwo\r\n$5\r\nthree\r\n"
    );
    assert_eq!(
        run(&store, &[b"SMEMBERS", b"set"]),
        b"*2\r\n$1\r\na\r\n$1\r\nb\r\n"
    );
    assert_eq!(
        run(&store, &[b"HGETALL", b"hash"]),
        b"*4\r\n$5\r\nfield\r\n$5\r\nvalue\r\n$5\r\nother\r\n$1\r\nx\r\n"
    );
    assert_eq!(
        run(&store, &[b"ZRANGE", b"zset", b"0", b"-1", b"WITHSCORES"]),
        b"*4\r\n$1\r\na\r\n$2\r\n-1\r\n$1\r\nb\r\n$3\r\n2.5\r\n"
    );
    assert_eq!(run(&store, &[b"XLEN", b"stream"]), b":2\r\n");
    let info = run(&store, &[b"XINFO", b"STREAM", b"stream", b"FULL"]);
    assert!(info.windows(5).any(|window| window == b"group"));
    let ttl = run(&store, &[b"PTTL", b"string\0"]);
    assert!(ttl.starts_with(b":") && ttl != b":-1\r\n" && ttl != b":-2\r\n");

    let text = std::fs::read(&path).unwrap();
    assert!(text.windows(3).any(|window| window == b"DEL"));
    assert!(text.windows(4).any(|window| window == b"PEXP"));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn rejected_writes_do_not_grow_the_log_and_flush_is_recorded() {
    let path = aof_path("flush");
    {
        let store = open(&path, FsyncPolicy::Always);
        assert_eq!(run(&store, &[b"SET", b"k", b"v"]), b"+OK\r\n");
        let before = std::fs::metadata(&path).unwrap().len();
        assert_eq!(run(&store, &[b"SET", b"k", b"new", b"NX"]), b"$-1\r\n");
        assert_eq!(
            run(&store, &[b"INCR", b"k"]),
            b"-ERR value is not an integer or out of range\r\n"
        );
        assert_eq!(std::fs::metadata(&path).unwrap().len(), before);
        assert_eq!(run(&store, &[b"FLUSHALL"]), b"+OK\r\n");
    }
    let store = open(&path, FsyncPolicy::Always);
    assert_eq!(run(&store, &[b"DBSIZE"]), b":0\r\n");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn transaction_batch_and_concurrent_writers_replay_to_the_final_state() {
    let path = aof_path("concurrent");
    {
        let store = std::sync::Arc::new(open(&path, FsyncPolicy::Always));
        let watched = store.watch_snapshot(&[]).unwrap();
        let mut output = Vec::new();
        store.execute_transaction(
            &[
                vec![b"SET".to_vec(), b"batch:a".to_vec(), b"1".to_vec()],
                vec![b"SET".to_vec(), b"batch:b".to_vec(), b"2".to_vec()],
            ],
            &watched,
            &mut output,
        );
        assert_eq!(output, b"*2\r\n+OK\r\n+OK\r\n");
        let mut handles = Vec::new();
        for worker in 0..4 {
            let store = std::sync::Arc::clone(&store);
            handles.push(std::thread::spawn(move || {
                for index in 0..50 {
                    let key = format!("worker:{worker}");
                    let value = index.to_string();
                    run(&store, &[b"SET", key.as_bytes(), value.as_bytes()]);
                }
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }
    }
    let store = open(&path, FsyncPolicy::Always);
    assert_eq!(run(&store, &[b"GET", b"batch:a"]), b"$1\r\n1\r\n");
    assert_eq!(run(&store, &[b"GET", b"batch:b"]), b"$1\r\n2\r\n");
    for worker in 0..4 {
        let key = format!("worker:{worker}");
        assert_eq!(run(&store, &[b"GET", key.as_bytes()]), b"$2\r\n49\r\n");
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn rewrite_compacts_history_and_still_replays_the_final_state() {
    let path = aof_path("rewrite");
    {
        let store = open(&path, FsyncPolicy::Always);
        for index in 0..200 {
            let value = format!("value-{index}");
            assert_eq!(run(&store, &[b"SET", b"key", value.as_bytes()]), b"+OK\r\n");
        }
        assert_eq!(run(&store, &[b"RPUSH", b"list", b"a", b"b"]), b":2\r\n");
        let before = std::fs::metadata(&path).unwrap().len();
        assert_eq!(
            run(&store, &[b"BGREWRITEAOF"]),
            b"+Background append only file rewriting started\r\n"
        );
        for _ in 0..10_000 {
            store.journal_sync().unwrap();
            if !store.rewrite_in_progress() {
                break;
            }
        }
        assert!(!store.rewrite_in_progress(), "rewrite did not finish");
        let after = std::fs::metadata(&path).unwrap().len();
        assert!(after < before, "rewrite must compact {before} -> {after}");
        assert_eq!(run(&store, &[b"GET", b"key"]), b"$9\r\nvalue-199\r\n");
    }
    let store = open(&path, FsyncPolicy::Always);
    assert_eq!(run(&store, &[b"GET", b"key"]), b"$9\r\nvalue-199\r\n");
    assert_eq!(
        run(&store, &[b"LRANGE", b"list", b"0", b"-1"]),
        b"*2\r\n$1\r\na\r\n$1\r\nb\r\n"
    );
    assert_eq!(run(&store, &[b"DBSIZE"]), b":2\r\n");
    let _ = std::fs::remove_file(&path);

    let path = aof_path("rewrite-disabled");
    let store = Store::new(1024);
    assert!(run(&store, &[b"BGREWRITEAOF"]).starts_with(b"-ERR append-only"));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn blocking_pop_is_journaled_and_survives_restart() {
    let path = aof_path("blocking");
    {
        let store = open(&path, FsyncPolicy::Always);
        assert_eq!(run(&store, &[b"RPUSH", b"queue", b"a", b"b"]), b":2\r\n");
        assert_eq!(
            store.blocking_pop(&[b"queue"], true),
            PopOutcome::Value {
                key: b"queue".to_vec(),
                value: b"a".to_vec()
            }
        );
        assert_eq!(
            store.blocking_pop(&[b"queue"], false),
            PopOutcome::Value {
                key: b"queue".to_vec(),
                value: b"b".to_vec()
            }
        );
        assert_eq!(store.blocking_pop(&[b"queue"], true), PopOutcome::Empty);
    }
    let store = open(&path, FsyncPolicy::Always);
    assert_eq!(run(&store, &[b"EXISTS", b"queue"]), b":0\r\n");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn eviction_is_journaled_and_replays_to_the_same_survivors() {
    let path = aof_path("eviction");
    {
        let store = Store::with_auth_and_journal(
            11,
            Auth::default(),
            &path,
            FsyncPolicy::Always,
            Limits::default(),
            None,
        )
        .unwrap();
        store.set_eviction_policy(crate::engine::EvictionPolicy::AllKeysRandom);
        assert_eq!(run(&store, &[b"SET", b"a", b"1234"]), b"+OK\r\n");
        assert_eq!(run(&store, &[b"SET", b"b", b"1234"]), b"+OK\r\n");
        assert_eq!(run(&store, &[b"SET", b"c", b"v"]), b"+OK\r\n");
        assert_eq!(store.stats().evicted_keys, 1);
    }
    let store = Store::with_auth_and_journal(
        11,
        Auth::default(),
        &path,
        FsyncPolicy::Always,
        Limits::default(),
        None,
    )
    .unwrap();
    assert_eq!(run(&store, &[b"DBSIZE"]), b":2\r\n");
    assert_eq!(run(&store, &[b"GET", b"c"]), b"$1\r\nv\r\n");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_partial_tail_is_truncated_but_mid_file_corruption_fails() {
    let path = aof_path("truncated");
    {
        let store = open(&path, FsyncPolicy::Always);
        assert_eq!(run(&store, &[b"SET", b"k", b"v"]), b"+OK\r\n");
    }
    let complete = std::fs::metadata(&path).unwrap().len();
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    file.write_all(b"*3\r\n$3\r\nSET\r\n$1\r\nx").unwrap();
    drop(file);
    let store = open(&path, FsyncPolicy::Always);
    assert_eq!(run(&store, &[b"GET", b"k"]), b"$1\r\nv\r\n");
    assert_eq!(std::fs::metadata(&path).unwrap().len(), complete);
    let _ = std::fs::remove_file(&path);

    let path = aof_path("corrupt");
    {
        let store = open(&path, FsyncPolicy::Always);
        assert_eq!(run(&store, &[b"SET", b"k", b"v"]), b"+OK\r\n");
    }
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[0] = b'X';
    std::fs::write(&path, bytes).unwrap();
    assert!(
        Store::with_auth_and_journal(
            1024,
            Auth::default(),
            &path,
            FsyncPolicy::Always,
            Limits::default(),
            None
        )
        .is_err()
    );
    let _ = std::fs::remove_file(&path);
}
#[test]
fn runtime_appendfsync_updates_the_open_journal_and_survives_restart() {
    let path = aof_path("runtime-fsync");
    {
        let store = open(&path, FsyncPolicy::EverySec);
        assert_eq!(store.journal_policy(), Some(FsyncPolicy::EverySec));
        assert_eq!(
            run(&store, &[b"CONFIG", b"GET", b"appendfsync"]),
            b"*2\r\n$11\r\nappendfsync\r\n$8\r\neverysec\r\n"
        );
        assert_eq!(
            run(&store, &[b"CONFIG", b"SET", b"appendfsync", b"no"]),
            b"+OK\r\n"
        );
        assert_eq!(store.journal_policy(), Some(FsyncPolicy::No));
        assert_eq!(run(&store, &[b"SET", b"k", b"v"]), b"+OK\r\n");
        assert_eq!(
            run(&store, &[b"CONFIG", b"SET", b"appendfsync", b"always"]),
            b"+OK\r\n"
        );
        assert_eq!(store.journal_policy(), Some(FsyncPolicy::Always));
        assert_eq!(run(&store, &[b"SET", b"k", b"v2"]), b"+OK\r\n");
        assert_eq!(
            run(&store, &[b"CONFIG", b"GET", b"appendonly"]),
            b"*2\r\n$10\r\nappendonly\r\n$3\r\nyes\r\n"
        );
    }
    let store = open(&path, FsyncPolicy::EverySec);
    assert_eq!(run(&store, &[b"GET", b"k"]), b"$2\r\nv2\r\n");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn automatic_rewrite_compacts_after_configured_growth() {
    let path = aof_path("auto-rewrite");
    {
        let store = open(&path, FsyncPolicy::Always);
        store.set_auto_aof_rewrite_percentage(1);
        store.set_auto_aof_rewrite_min_size(128);
        for index in 0..300 {
            let value = format!("value-{index}");
            assert_eq!(run(&store, &[b"SET", b"key", value.as_bytes()]), b"+OK\r\n");
        }
        let before = std::fs::metadata(&path).unwrap().len();
        assert!(store.auto_rewrite_if_due());
        for _ in 0..10_000 {
            store.journal_sync().unwrap();
            if !store.rewrite_in_progress() {
                break;
            }
        }
        assert!(!store.rewrite_in_progress(), "auto rewrite did not finish");
        let after = std::fs::metadata(&path).unwrap().len();
        assert!(
            after < before,
            "auto rewrite must compact {before} -> {after}"
        );
        assert!(!store.auto_rewrite_if_due());
        for _ in 0..300 {
            assert_eq!(run(&store, &[b"SET", b"key", b"again"]), b"+OK\r\n");
        }
        assert!(store.auto_rewrite_if_due());
    }
    let store = open(&path, FsyncPolicy::Always);
    assert_eq!(run(&store, &[b"GET", b"key"]), b"$5\r\nagain\r\n");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn authenticated_journal_replays_with_the_key_and_rejects_modified_tags() {
    let path = aof_path("auth-replay");
    {
        let store = open_with_auth(&path, AUTH_KEY, false).unwrap();
        assert_eq!(run(&store, &[b"SET", b"one", b"1"]), b"+OK\r\n");
        assert_eq!(run(&store, &[b"SET", b"two", b"2"]), b"+OK\r\n");
    }
    {
        let store = open_with_auth(&path, AUTH_KEY, false).unwrap();
        assert_eq!(run(&store, &[b"GET", b"one"]), b"$1\r\n1\r\n");
        assert_eq!(run(&store, &[b"GET", b"two"]), b"$1\r\n2\r\n");
    }
    assert!(
        open_with_auth(&path, b"fedcba9876543210", false).is_err(),
        "a different key must fail the anchor"
    );
    let mut bytes = std::fs::read(&path).unwrap();
    let marker = b"__RILLCACHE_RECORD_HMAC256_V1__:";
    let position = bytes
        .windows(marker.len())
        .position(|window| window == marker)
        .expect("authentication marker");
    let tag_start = position + marker.len();
    bytes[tag_start] = if bytes[tag_start] == b'0' { b'1' } else { b'0' };
    std::fs::write(&path, bytes).unwrap();
    let error = open_with_auth(&path, AUTH_KEY, false)
        .err()
        .expect("modified keyed tag must fail");
    assert!(
        error.to_string().contains("authentication"),
        "unexpected error: {error}"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn authenticated_journal_is_still_readable_without_a_key() {
    let path = aof_path("auth-no-key");
    {
        let store = open_with_auth(&path, AUTH_KEY, false).unwrap();
        assert_eq!(run(&store, &[b"SET", b"k", b"v"]), b"+OK\r\n");
    }
    let store = open(&path, FsyncPolicy::Always);
    assert_eq!(run(&store, &[b"GET", b"k"]), b"$1\r\nv\r\n");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn enabling_authentication_on_an_existing_journal_requires_explicit_migration() {
    let path = aof_path("auth-migrate");
    {
        let store = open(&path, FsyncPolicy::Always);
        assert_eq!(run(&store, &[b"SET", b"before", b"value"]), b"+OK\r\n");
    }
    let error = open_with_auth(&path, AUTH_KEY, false)
        .err()
        .expect("unauthenticated journal must not be silently sealed");
    assert!(
        error.to_string().contains("--aof-hmac-migrate"),
        "unexpected error: {error}"
    );
    {
        let store = open_with_auth(&path, AUTH_KEY, true).unwrap();
        assert_eq!(run(&store, &[b"GET", b"before"]), b"$5\r\nvalue\r\n");
        assert_eq!(run(&store, &[b"SET", b"after", b"value"]), b"+OK\r\n");
    }
    {
        let store = open_with_auth(&path, AUTH_KEY, false).unwrap();
        assert_eq!(run(&store, &[b"GET", b"before"]), b"$5\r\nvalue\r\n");
        assert_eq!(run(&store, &[b"GET", b"after"]), b"$5\r\nvalue\r\n");
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn authenticated_rewrite_and_posts_rewrite_appends_replay() {
    let path = aof_path("auth-rewrite");
    {
        let store = open_with_auth(&path, AUTH_KEY, false).unwrap();
        for index in 0..50 {
            let value = format!("value-{index}");
            assert_eq!(run(&store, &[b"SET", b"key", value.as_bytes()]), b"+OK\r\n");
        }
        assert_eq!(
            run(&store, &[b"BGREWRITEAOF"]),
            b"+Background append only file rewriting started\r\n"
        );
        let mut during = 0;
        for _ in 0..10_000 {
            if store.rewrite_in_progress() && during < 5 {
                let value = format!("during-{during}");
                assert_eq!(
                    run(&store, &[b"SET", b"during", value.as_bytes()]),
                    b"+OK\r\n"
                );
                during += 1;
            }
            store.journal_sync().unwrap();
            if !store.rewrite_in_progress() {
                break;
            }
        }
        assert!(!store.rewrite_in_progress(), "rewrite did not finish");
        assert_eq!(run(&store, &[b"SET", b"after", b"rewrite"]), b"+OK\r\n");
        if during == 0 {
            assert_eq!(run(&store, &[b"SET", b"during", b"none"]), b"+OK\r\n");
        }
    }
    let store = open_with_auth(&path, AUTH_KEY, false).unwrap();
    assert_eq!(run(&store, &[b"GET", b"key"]), b"$8\r\nvalue-49\r\n");
    assert_eq!(run(&store, &[b"GET", b"after"]), b"$7\r\nrewrite\r\n");
    assert!(
        run(&store, &[b"GET", b"during"]).starts_with(b"$"),
        "mutations during rewrite must survive authenticated replay"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_torn_final_authentication_marker_discards_only_the_unacknowledged_transaction() {
    let path = aof_path("auth-torn");
    {
        let store = open_with_auth(&path, AUTH_KEY, false).unwrap();
        assert_eq!(run(&store, &[b"SET", b"kept", b"1"]), b"+OK\r\n");
        assert_eq!(run(&store, &[b"SET", b"torn", b"2"]), b"+OK\r\n");
    }
    let mut bytes = std::fs::read(&path).unwrap();
    let marker = b"__RILLCACHE_RECORD_HMAC256_V1__:";
    let last = bytes
        .windows(marker.len())
        .rposition(|window| window == marker)
        .expect("last authentication marker");
    bytes.truncate(last + 5);
    std::fs::write(&path, bytes).unwrap();
    let store = open_with_auth(&path, AUTH_KEY, false).unwrap();
    assert_eq!(run(&store, &[b"GET", b"kept"]), b"$1\r\n1\r\n");
    assert_eq!(run(&store, &[b"GET", b"torn"]), b"$-1\r\n");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_partial_unauthenticated_next_append_preserves_the_previous_authenticated_transaction() {
    let path = aof_path("auth-partial-next");
    {
        let store = open_with_auth(&path, AUTH_KEY, false).unwrap();
        assert_eq!(run(&store, &[b"SET", b"durable", b"value"]), b"+OK\r\n");
    }
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    file.write_all(b"*3\r\n$3\r\nSET\r\n$5\r\nbrok").unwrap();
    drop(file);
    let store = open_with_auth(&path, AUTH_KEY, false).unwrap();
    assert_eq!(run(&store, &[b"GET", b"durable"]), b"$5\r\nvalue\r\n");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn journal_authentication_key_files_are_trimmed_and_bounded() {
    let path = std::env::temp_dir().join(format!(
        "rillcache-auth-key-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::write(&path, b"0123456789abcdef\r\n").unwrap();
    assert_eq!(super::read_auth_key_file(&path).unwrap(), AUTH_KEY);
    std::fs::write(&path, b"0123456789abcde\n").unwrap();
    assert!(super::read_auth_key_file(&path).is_err());
    std::fs::write(&path, vec![b'x'; 1025]).unwrap();
    assert!(super::read_auth_key_file(&path).is_err());
    let _ = std::fs::remove_file(&path);
    assert!(super::read_auth_key_file(&path).is_err());
}

#[test]
fn authentication_detects_modified_content_even_when_the_crc32_is_recomputed() {
    let path = aof_path("auth-recomputed-crc");
    {
        let store = open_with_auth(&path, AUTH_KEY, false).unwrap();
        assert_eq!(run(&store, &[b"SET", b"k", b"v"]), b"+OK\r\n");
    }
    let mut bytes = std::fs::read(&path).unwrap();
    let checksum_header = b"__RILLCACHE_AOF_CRC32_V1__:";
    let checksum_record = b"__RILLCACHE_RECORD_CRC32_V1__:";
    let header_at = bytes
        .windows(checksum_header.len())
        .position(|window| window == checksum_header)
        .unwrap();
    let header_end = bytes[header_at..]
        .windows(2)
        .position(|window| window == b"\r\n")
        .unwrap()
        + header_at
        + 2;
    let record_at = bytes
        .windows(checksum_record.len())
        .position(|window| window == checksum_record)
        .unwrap();
    let echo = b"*2\r\n$4\r\nECHO\r\n";
    let record_start = bytes[..record_at]
        .windows(echo.len())
        .rposition(|window| window == echo)
        .unwrap();
    let value_at = bytes
        .windows(7)
        .position(|window| window == b"$1\r\nv\r\n")
        .unwrap()
        + 4;
    bytes[value_at] = b"x"[0];
    let crc = !super::crc32_update(!0u32, &bytes[header_end..record_start]);
    let record_end = bytes[record_at..]
        .windows(2)
        .position(|window| window == b"\r\n")
        .unwrap()
        + record_at;
    bytes[record_end - 8..record_end].copy_from_slice(format!("{crc:08x}").as_bytes());
    std::fs::write(&path, bytes).unwrap();
    let error = open_with_auth(&path, AUTH_KEY, false)
        .err()
        .expect("recomputed CRC must not hide a keyed-chain mismatch");
    assert!(
        error.to_string().contains("authentication chain mismatch"),
        "unexpected error: {error}"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn consumer_seen_journal_records_are_binary_safe_and_strictly_framed() {
    let key = b"key\x00with:colon";
    let group = b"group\xffbinary";
    let consumer = b"consumer\nname";
    let record = super::consumer_seen_record(key, group, consumer, 1_700_000_000_123);
    let mut command = None;
    let mut payload = None;
    let parsed = crate::protocol::dispatch_request(&record, Limits::default(), |args| {
        command = Some(args[0].to_vec());
        payload = Some(args[1].to_vec());
        Some(())
    });
    assert!(parsed.unwrap().is_some());
    assert_eq!(command.as_deref(), Some(b"ECHO".as_slice()));
    let payload = payload.expect("consumer-seen payload");
    assert_eq!(
        super::parse_consumer_seen(&payload),
        Some((
            key.as_slice(),
            group.as_slice(),
            consumer.as_slice(),
            1_700_000_000_123
        ))
    );

    let mut truncated = payload.clone();
    truncated.pop();
    assert!(super::parse_consumer_seen(&truncated).is_none());
    let mut trailing = payload;
    trailing.push(b'x');
    assert!(super::parse_consumer_seen(&trailing).is_none());
    assert!(super::parse_consumer_seen(b"__RILLCACHE_CONSUMER_SEEN_V1__:").is_none());
}
