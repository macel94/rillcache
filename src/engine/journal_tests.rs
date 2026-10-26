use crate::engine::{Auth, FsyncPolicy, PopOutcome, Store};
use crate::protocol::Limits;
use std::io::Write;

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
    )
    .unwrap()
}

fn run(store: &Store, arguments: &[&[u8]]) -> Vec<u8> {
    let mut output = Vec::new();
    store.execute(arguments, &mut output);
    output
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
        assert_eq!(run(&store, &[b"BGREWRITEAOF"]), b"+OK\r\n");
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
            Limits::default()
        )
        .is_err()
    );
    let _ = std::fs::remove_file(&path);
}
