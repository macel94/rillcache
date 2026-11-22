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
fn rewrite_setup_failure_is_visible_and_does_not_latch_live_writes() {
    let path = aof_path("rewrite-setup-failure");
    let staging = path.with_file_name(format!(
        ".{}.rewrite-{}",
        path.file_name().unwrap().to_str().unwrap(),
        std::process::id()
    ));
    let store = open(&path, FsyncPolicy::Always);
    assert_eq!(run(&store, &[b"SET", b"first", b"old\0"]), b"+OK\r\n");
    let original = std::fs::read(&path).unwrap();
    // A directory cannot be opened as the rewrite scratch file. This only
    // faults rewrite setup, without changing permissions or the live journal.
    std::fs::create_dir(&staging).unwrap();
    assert_eq!(
        run(&store, &[b"BGREWRITEAOF"]),
        b"-ERR append-only journal is not enabled or cannot be rewritten\r\n"
    );
    let info = run(&store, &[b"INFO", b"persistence"]);
    let text = std::str::from_utf8(&info).unwrap();
    assert!(text.contains("aof_last_bgrewrite_status:err\r\n"), "{text}");
    assert!(text.contains("aof_last_write_status:ok\r\n"), "{text}");
    assert!(text.contains("aof_rewrite_in_progress:0\r\n"), "{text}");
    assert!(!text.contains("aof_last_rewrite_start_usec:"), "{text}");
    assert!(!text.contains("aof_last_rewrite_finish_usec:"), "{text}");
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert_eq!(run(&store, &[b"SET", b"second", b"new\xff"]), b"+OK\r\n");
    std::fs::remove_dir(&staging).unwrap();
    assert_eq!(
        run(&store, &[b"BGREWRITEAOF"]),
        b"+Background append only file rewriting started\r\n"
    );
    store
        .call(|executor| {
            for _ in 0..8 {
                executor.rewrite_step()?;
                if !executor.rewrite_in_progress() {
                    return Ok(());
                }
            }
            Err(std::io::Error::other("fixture rewrite did not finish"))
        })
        .unwrap();
    let info = run(&store, &[b"INFO", b"persistence"]);
    let text = std::str::from_utf8(&info).unwrap();
    assert!(text.contains("aof_last_bgrewrite_status:ok\r\n"), "{text}");
    let finish_usec = text
        .lines()
        .find_map(|line| line.strip_prefix("aof_last_rewrite_finish_usec:"))
        .expect("successful finalization timing");
    assert!(finish_usec.parse::<u64>().is_ok(), "{text}");
    std::fs::create_dir(&staging).unwrap();
    assert_eq!(
        run(&store, &[b"BGREWRITEAOF"]),
        b"-ERR append-only journal is not enabled or cannot be rewritten\r\n"
    );
    let failed_info = run(&store, &[b"INFO", b"persistence"]);
    let failed_text = std::str::from_utf8(&failed_info).unwrap();
    assert!(
        failed_text.contains("aof_last_bgrewrite_status:err\r\n"),
        "{failed_text}"
    );
    assert_eq!(
        failed_text
            .lines()
            .find_map(|line| line.strip_prefix("aof_last_rewrite_finish_usec:")),
        Some(finish_usec)
    );
    std::fs::remove_dir(staging).unwrap();
    drop(store);
    let reopened = open(&path, FsyncPolicy::Always);
    assert_eq!(run(&reopened, &[b"GET", b"first"]), b"$4\r\nold\0\r\n");
    assert_eq!(run(&reopened, &[b"GET", b"second"]), b"$4\r\nnew\xff\r\n");
    drop(reopened);
    std::fs::remove_file(path).unwrap();
}

#[cfg(unix)]
#[test]
fn rewrite_publish_failure_preserves_live_journal_and_allows_retry() {
    let path = aof_path("rewrite-publish-failure");
    let staging = path.with_file_name(format!(
        ".{}.rewrite-{}",
        path.file_name().unwrap().to_str().unwrap(),
        std::process::id()
    ));
    let store = open(&path, FsyncPolicy::Always);
    assert_eq!(run(&store, &[b"SET", b"first", b"old\0"]), b"+OK\r\n");
    let original = std::fs::read(&path).unwrap();
    store
        .call(move |executor| -> std::io::Result<()> {
            executor.rewrite_journal().unwrap();
            for _ in 0..8 {
                if executor.persistence_status().rewrite_phase == "commit" {
                    break;
                }
                executor.rewrite_step()?;
            }
            assert_eq!(executor.persistence_status().rewrite_phase, "commit");
            // The opened staging file remains usable on Unix, but cannot be
            // renamed into place. This injects a publication failure only.
            std::fs::remove_file(staging)?;
            executor.journal_sync()?;
            let status = executor.persistence_status();
            assert!(!status.rewrite_in_progress);
            assert!(status.rewrite_failed);
            assert!(!status.write_failed);
            assert_eq!(status.last_rewrite_finish_usec, None);
            Ok(())
        })
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert_eq!(run(&store, &[b"SET", b"second", b"new\xff"]), b"+OK\r\n");
    drop(store);

    let reopened = open(&path, FsyncPolicy::Always);
    assert_eq!(run(&reopened, &[b"GET", b"first"]), b"$4\r\nold\0\r\n");
    assert_eq!(run(&reopened, &[b"GET", b"second"]), b"$4\r\nnew\xff\r\n");
    reopened
        .call(|executor| -> std::io::Result<()> {
            executor.rewrite_journal().unwrap();
            for _ in 0..8 {
                executor.rewrite_step()?;
                if !executor.rewrite_in_progress() {
                    break;
                }
            }
            let status = executor.persistence_status();
            assert!(!status.rewrite_in_progress);
            assert!(!status.rewrite_failed);
            assert!(status.last_rewrite_finish_usec.is_some());
            Ok(())
        })
        .unwrap();
    drop(reopened);
    let rewritten = open(&path, FsyncPolicy::Always);
    assert_eq!(run(&rewritten, &[b"GET", b"first"]), b"$4\r\nold\0\r\n");
    assert_eq!(run(&rewritten, &[b"GET", b"second"]), b"$4\r\nnew\xff\r\n");
    drop(rewritten);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn rewrite_setup_timing_is_recorded_only_when_a_rewrite_starts() {
    let path = aof_path("rewrite-setup-timing");
    let store = open(&path, FsyncPolicy::Always);
    store
        .call(|executor| {
            assert_eq!(executor.persistence_status().last_rewrite_start_usec, None);
            executor.rewrite_journal().unwrap();
            assert!(executor.rewrite_in_progress());
            let measured = executor.persistence_status().last_rewrite_start_usec;
            assert!(measured.is_some());
            executor.rewrite_journal().unwrap();
            assert_eq!(
                executor.persistence_status().last_rewrite_start_usec,
                measured
            );
            executor.journal_sync()
        })
        .unwrap();
    drop(store);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn rewrite_byte_progress_matches_temporary_file_and_resets() {
    for authenticate in [false, true] {
        let path = aof_path("rewrite-byte-progress");
        let auth = authenticate.then(|| JournalAuth::new(AUTH_KEY.to_vec(), false).unwrap());
        let mut journal =
            super::Journal::open(&path, FsyncPolicy::Always, Limits::default(), auth).unwrap();
        assert_eq!(journal.status().rewrite_bytes_written, Some(0));

        journal.begin_rewrite().unwrap();
        let header_bytes = journal.status().rewrite_bytes_written.unwrap();
        assert!(header_bytes > 0);
        journal.abort_rewrite();
        assert_eq!(journal.status().rewrite_bytes_written, Some(0));

        journal.begin_rewrite().unwrap();
        let header_bytes = journal.status().rewrite_bytes_written.unwrap();
        let record = b"*3\r\n$3\r\nSET\r\n$1\r\nk\r\n$1\r\nv\r\n";
        journal.write_rewrite(record).unwrap();
        let written = journal.status().rewrite_bytes_written.unwrap();
        let temp_len = journal
            .rewrite
            .as_ref()
            .unwrap()
            .file
            .metadata()
            .unwrap()
            .len();
        assert_eq!(written, temp_len);
        if authenticate {
            assert!(written > header_bytes + record.len() as u64);
        } else {
            assert_eq!(written, header_bytes + record.len() as u64);
        }
        journal.sync_rewrite().unwrap();
        assert_eq!(journal.status().rewrite_bytes_written, Some(written));
        let published = journal.commit_rewrite().unwrap();
        assert!(
            published > written,
            "commit adds the final integrity trailer"
        );
        assert_eq!(journal.status().current_size, Some(published));
        assert_eq!(journal.status().rewrite_bytes_written, Some(0));
        drop(journal);
        std::fs::remove_file(path).unwrap();
    }
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
    // write as hset, but the expired field itself is still reported exactly
    // once as `hexpired` because the removal really happened.
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
        let mut channel = b"__keyevent@0__:".to_vec();
        channel.extend_from_slice(b"hexpired");
        let mut expired = Vec::new();
        crate::protocol::encode_request(
            &[b"pmessage", b"__keyevent@0__:*", &channel, b"h\0\xff"],
            &mut expired,
        );
        assert_eq!(receiver.try_recv().unwrap(), expired);
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
fn expired_hash_field_emits_hexpired_then_del_for_empty_hash() {
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
            &[b"HSETEX", b"h", b"PX", b"1", b"FIELDS", b"1", b"f", b"v"]
        ),
        b":1\r\n"
    );
    while receiver.try_recv().is_ok() {}
    clock.0.fetch_add(2, Ordering::SeqCst);
    assert_eq!(run(&store, &[b"HGET", b"h", b"f"]), b"$-1\r\n");
    for event in [b"hexpired".as_slice(), b"del"] {
        let mut channel = b"__keyevent@0__:".to_vec();
        channel.extend_from_slice(event);
        let mut frame = Vec::new();
        crate::protocol::encode_request(
            &[b"pmessage", b"__keyevent@0__:*", &channel, b"h"],
            &mut frame,
        );
        assert_eq!(receiver.try_recv().unwrap(), frame);
    }
    assert!(
        receiver.try_recv().is_err(),
        "field expiry is reported once per batch"
    );
    assert_eq!(run(&store, &[b"EXISTS", b"h"]), b":0\r\n");
}

#[test]
fn hash_ttl_updates_emit_hexpire_and_hpersist_only_when_changed() {
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
        run(&store, &[b"HSET", b"h", b"f", b"v", b"g", b"w"]),
        b":2\r\n"
    );
    while receiver.try_recv().is_ok() {}

    let expect_event = |event: &[u8]| {
        let mut channel = b"__keyevent@0__:".to_vec();
        channel.extend_from_slice(event);
        let mut frame = Vec::new();
        crate::protocol::encode_request(
            &[b"pmessage", b"__keyevent@0__:*", &channel, b"h"],
            &mut frame,
        );
        assert_eq!(receiver.try_recv().unwrap(), frame);
        assert!(
            receiver.try_recv().is_err(),
            "one event per changed command"
        );
    };

    assert_eq!(
        run(
            &store,
            &[b"HEXPIRE", b"h", b"10", b"FIELDS", b"2", b"f", b"g"]
        ),
        b"*2\r\n:1\r\n:1\r\n"
    );
    expect_event(b"hexpire");

    // NX finds an existing field TTL for both fields, so nothing changed and no
    // notification (or WATCH invalidation) may be produced.
    assert_eq!(
        run(
            &store,
            &[b"HEXPIRE", b"h", b"10", b"NX", b"FIELDS", b"2", b"f", b"g"]
        ),
        b"*2\r\n:0\r\n:0\r\n"
    );
    assert!(receiver.try_recv().is_err());

    assert_eq!(
        run(&store, &[b"HPERSIST", b"h", b"FIELDS", b"2", b"f", b"g"]),
        b"*2\r\n:1\r\n:1\r\n"
    );
    expect_event(b"hpersist");

    assert_eq!(
        run(&store, &[b"HPERSIST", b"h", b"FIELDS", b"2", b"f", b"g"]),
        b"*2\r\n:-1\r\n:-1\r\n"
    );
    assert!(receiver.try_recv().is_err());

    assert_eq!(
        run(
            &store,
            &[b"HEXPIRE", b"h", b"1", b"FIELDS", b"1", b"missing"]
        ),
        b"*1\r\n:-2\r\n"
    );
    assert!(receiver.try_recv().is_err());
}

#[test]
fn hash_delete_events_report_hdel_then_del_only_for_real_removals() {
    use crate::engine::NotifyFlags;
    let store = Store::new(12);
    store.set_notify_flags(NotifyFlags::parse(b"EA").unwrap());
    let (sender, receiver) = std::sync::mpsc::sync_channel(Store::pubsub_queue_capacity());
    assert!(store.pubsub_subscribe(true, b"__keyevent@0__:*", 1, sender, false));
    let expect = |event: &[u8], key: &[u8]| {
        let mut channel = b"__keyevent@0__:".to_vec();
        channel.extend_from_slice(event);
        let mut frame = Vec::new();
        crate::protocol::encode_request(
            &[b"pmessage", b"__keyevent@0__:*", &channel, key],
            &mut frame,
        );
        assert_eq!(receiver.try_recv().unwrap(), frame);
    };

    assert_eq!(run(&store, &[b"HSET", b"h", b"a", b"1"]), b":1\r\n");
    while receiver.try_recv().is_ok() {}
    assert_eq!(run(&store, &[b"HDEL", b"h", b"a"]), b":1\r\n");
    expect(b"hdel", b"h");
    expect(b"del", b"h");

    assert_eq!(
        run(&store, &[b"HSET", b"h2", b"a", b"1", b"b", b"2"]),
        b":2\r\n"
    );
    while receiver.try_recv().is_ok() {}
    assert_eq!(run(&store, &[b"HDEL", b"h2", b"a"]), b":1\r\n");
    expect(b"hdel", b"h2");
    assert_eq!(run(&store, &[b"HDEL", b"h2", b"missing"]), b":0\r\n");
    assert!(receiver.try_recv().is_err(), "a no-op HDEL emits nothing");

    assert_eq!(run(&store, &[b"HSET", b"h3", b"a", b"1"]), b":1\r\n");
    while receiver.try_recv().is_ok() {}
    assert_eq!(
        run(&store, &[b"HGETDEL", b"h3", b"FIELDS", b"1", b"a"]),
        b"*1\r\n$1\r\n1\r\n"
    );
    expect(b"hdel", b"h3");
    expect(b"del", b"h3");

    assert_eq!(run(&store, &[b"HSET", b"h4", b"a", b"1"]), b":1\r\n");
    while receiver.try_recv().is_ok() {}
    assert_eq!(
        run(&store, &[b"HGETDEL", b"h4", b"FIELDS", b"1", b"missing"]),
        b"*1\r\n$-1\r\n"
    );
    assert!(
        receiver.try_recv().is_err(),
        "a missed HGETDEL emits nothing"
    );
}

#[test]
fn hsetex_emits_its_exact_write_or_delete_event_pair() {
    use crate::engine::NotifyFlags;
    let store = Store::new(12);
    store.set_notify_flags(NotifyFlags::parse(b"EA").unwrap());
    let (sender, receiver) = std::sync::mpsc::sync_channel(Store::pubsub_queue_capacity());
    assert!(store.pubsub_subscribe(true, b"__keyevent@0__:*", 1, sender, false));
    let expect = |event: &[u8], key: &[u8]| {
        let mut channel = b"__keyevent@0__:".to_vec();
        channel.extend_from_slice(event);
        let mut frame = Vec::new();
        crate::protocol::encode_request(
            &[b"pmessage", b"__keyevent@0__:*", &channel, key],
            &mut frame,
        );
        assert_eq!(receiver.try_recv().unwrap(), frame);
    };

    assert_eq!(
        run(&store, &[b"HSETEX", b"h", b"FIELDS", b"1", b"a", b"1"]),
        b":1\r\n"
    );
    expect(b"hset", b"h");
    assert!(receiver.try_recv().is_err(), "no TTL option, no hexpire");

    assert_eq!(
        run(
            &store,
            &[b"HSETEX", b"h", b"PX", b"1000", b"FIELDS", b"1", b"a", b"2"]
        ),
        b":1\r\n"
    );
    expect(b"hset", b"h");
    expect(b"hexpire", b"h");

    assert_eq!(
        run(
            &store,
            &[b"HSETEX", b"h", b"KEEPTTL", b"FIELDS", b"1", b"a", b"3"]
        ),
        b":1\r\n"
    );
    expect(b"hset", b"h");
    assert!(receiver.try_recv().is_err(), "KEEPTTL emits no hexpire");

    // A past absolute TTL removes the last field, so the hash and its key go.
    assert_eq!(
        run(
            &store,
            &[b"HSETEX", b"h", b"PXAT", b"1", b"FIELDS", b"1", b"a", b"4"]
        ),
        b":1\r\n"
    );
    expect(b"hdel", b"h");
    expect(b"del", b"h");
    assert_eq!(run(&store, &[b"EXISTS", b"h"]), b":0\r\n");

    // No field existed and nothing was created; the successful no-op emits
    // nothing and does not touch the key's WATCH version.
    assert_eq!(
        run(
            &store,
            &[
                b"HSETEX", b"missing", b"PXAT", b"1", b"FIELDS", b"1", b"a", b"1"
            ]
        ),
        b":1\r\n"
    );
    assert!(receiver.try_recv().is_err());
    assert_eq!(run(&store, &[b"EXISTS", b"missing"]), b":0\r\n");

    assert_eq!(
        run(
            &store,
            &[b"HSETEX", b"missing", b"FXX", b"FIELDS", b"1", b"a", b"1"]
        ),
        b":0\r\n"
    );
    assert!(receiver.try_recv().is_err());
}

#[test]
fn hgetex_reports_its_ttl_events_without_a_write_event() {
    use crate::engine::NotifyFlags;
    let store = Store::new(12);
    store.set_notify_flags(NotifyFlags::parse(b"EA").unwrap());
    let (sender, receiver) = std::sync::mpsc::sync_channel(Store::pubsub_queue_capacity());
    assert!(store.pubsub_subscribe(true, b"__keyevent@0__:*", 1, sender, false));
    let expect = |event: &[u8], key: &[u8]| {
        let mut channel = b"__keyevent@0__:".to_vec();
        channel.extend_from_slice(event);
        let mut frame = Vec::new();
        crate::protocol::encode_request(
            &[b"pmessage", b"__keyevent@0__:*", &channel, key],
            &mut frame,
        );
        assert_eq!(receiver.try_recv().unwrap(), frame);
    };

    assert_eq!(
        run(&store, &[b"HSET", b"h", b"a", b"1", b"b", b"2"]),
        b":2\r\n"
    );
    while receiver.try_recv().is_ok() {}

    // Persisting a field with no TTL changes nothing and emits nothing.
    assert_eq!(
        run(
            &store,
            &[b"HGETEX", b"h", b"PERSIST", b"FIELDS", b"1", b"a"]
        ),
        b"*1\r\n$1\r\n1\r\n"
    );
    assert!(receiver.try_recv().is_err());

    assert_eq!(
        run(
            &store,
            &[b"HGETEX", b"h", b"EX", b"10", b"FIELDS", b"1", b"a"]
        ),
        b"*1\r\n$1\r\n1\r\n"
    );
    expect(b"hexpire", b"h");
    assert!(
        receiver.try_recv().is_err(),
        "HGETEX must not emit a write event"
    );

    assert_eq!(
        run(
            &store,
            &[b"HGETEX", b"h", b"PERSIST", b"FIELDS", b"1", b"a"]
        ),
        b"*1\r\n$1\r\n1\r\n"
    );
    expect(b"hpersist", b"h");

    // A missing field is a read miss, not a TTL change.
    assert_eq!(
        run(
            &store,
            &[b"HGETEX", b"h", b"EX", b"10", b"FIELDS", b"1", b"missing"]
        ),
        b"*1\r\n$-1\r\n"
    );
    assert!(receiver.try_recv().is_err());
}

#[test]
fn first_get_after_expiry_emits_expired_then_keymiss() {
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
    let store = Store::with_clock(64, clock.clone());
    store.set_notify_flags(NotifyFlags::parse(b"Exm").unwrap());
    let (sender, receiver) = std::sync::mpsc::sync_channel(Store::pubsub_queue_capacity());
    assert!(store.pubsub_subscribe(true, b"__keyevent@0__:*", 1, sender, false));
    let key = b"expiry\0\xff";
    let expect_event = |channel: &[u8]| {
        let mut frame = Vec::new();
        crate::protocol::encode_request(
            &[b"pmessage", b"__keyevent@0__:*", channel, key],
            &mut frame,
        );
        assert_eq!(receiver.try_recv().unwrap(), frame);
    };

    assert_eq!(run(&store, &[b"SET", key, b"v", b"PX", b"2"]), b"+OK\r\n");
    // The absolute deadline itself is still live.
    clock.0.store(102, Ordering::SeqCst);
    assert_eq!(run(&store, &[b"GET", key]), b"$1\r\nv\r\n");
    assert!(receiver.try_recv().is_err());
    clock.0.store(103, Ordering::SeqCst);
    assert_eq!(run(&store, &[b"GET", key]), b"$-1\r\n");
    expect_event(b"__keyevent@0__:expired");
    expect_event(b"__keyevent@0__:keymiss");
    assert_eq!(run(&store, &[b"GET", key]), b"$-1\r\n");
    expect_event(b"__keyevent@0__:keymiss");
    assert!(receiver.try_recv().is_err());
}

#[test]
fn read_misses_emit_keymiss_only_for_missing_keys() {
    use crate::engine::NotifyFlags;
    let store = Store::new(12);
    store.set_notify_flags(NotifyFlags::parse(b"Em").unwrap());
    let (sender, receiver) = std::sync::mpsc::sync_channel(Store::pubsub_queue_capacity());
    assert!(store.pubsub_subscribe(true, b"__keyevent@0__:*", 1, sender, false));
    let expect_miss = |key: &[u8]| {
        let mut frame = Vec::new();
        crate::protocol::encode_request(
            &[
                b"pmessage",
                b"__keyevent@0__:*",
                b"__keyevent@0__:keymiss",
                key,
            ],
            &mut frame,
        );
        assert_eq!(receiver.try_recv().unwrap(), frame);
    };

    assert_eq!(run(&store, &[b"GET", b"missing"]), b"$-1\r\n");
    expect_miss(b"missing");
    assert!(receiver.try_recv().is_err(), "one miss per lookup");

    assert_eq!(run(&store, &[b"SET", b"k", b"v"]), b"+OK\r\n");
    assert!(
        receiver.try_recv().is_err(),
        "m does not include set events"
    );
    assert_eq!(run(&store, &[b"GET", b"k"]), b"$1\r\nv\r\n");
    assert!(receiver.try_recv().is_err(), "a hit is not a key miss");

    assert_eq!(
        run(&store, &[b"MGET", b"k", b"missing", b"missing"]),
        b"*3\r\n$1\r\nv\r\n$-1\r\n$-1\r\n"
    );
    expect_miss(b"missing");
    expect_miss(b"missing");

    assert_eq!(run(&store, &[b"LPUSH", b"w", b"a"]), b":1\r\n");
    assert!(
        receiver.try_recv().is_err(),
        "m does not include list events"
    );
    assert_eq!(run(&store, &[b"MGET", b"w"]), b"*1\r\n$-1\r\n");
    assert!(
        receiver.try_recv().is_err(),
        "an existing non-string is not a key miss"
    );

    store.set_notify_flags(NotifyFlags::parse(b"EA").unwrap());
    assert_eq!(run(&store, &[b"GET", b"missing"]), b"$-1\r\n");
    assert!(receiver.try_recv().is_err(), "A deliberately excludes m");
}

#[test]
fn replay_reconstructs_known_group_entries_read() {
    let path = aof_path("entries-read-replay");
    {
        let store = open(&path, FsyncPolicy::Always);
        assert!(run(&store, &[b"XADD", b"s", b"1-0", b"f", b"v"]).starts_with(b"$"));
        assert!(run(&store, &[b"XADD", b"s", b"2-0", b"f", b"v"]).starts_with(b"$"));
        assert_eq!(
            run(
                &store,
                &[b"XGROUP", b"CREATE", b"s", b"g", b"0", b"ENTRIESREAD", b"9"]
            ),
            b"+OK\r\n"
        );
        assert!(
            run(
                &store,
                &[
                    b"XREADGROUP",
                    b"GROUP",
                    b"g",
                    b"c",
                    b"COUNT",
                    b"2",
                    b"STREAMS",
                    b"s",
                    b">"
                ]
            )
            .starts_with(b"*1\r\n")
        );
    }
    let store = open(&path, FsyncPolicy::Always);
    let info = run(&store, &[b"XINFO", b"GROUPS", b"s"]);
    assert!(
        info.windows(b"$12\r\nentries-read\r\n:11\r\n".len())
            .any(|window| window == b"$12\r\nentries-read\r\n:11\r\n"),
        "{info:?}"
    );
}

#[test]
fn keyspace_read_misses_emit_keymiss() {
    use crate::engine::NotifyFlags;
    let store = Store::new(128);
    store.set_notify_flags(NotifyFlags::parse(b"Em").unwrap());
    let (sender, receiver) = std::sync::mpsc::sync_channel(Store::pubsub_queue_capacity());
    assert!(store.pubsub_subscribe(true, b"__keyevent@0__:*", 1, sender, false));
    let expect_miss = |key: &[u8]| {
        let mut frame = Vec::new();
        crate::protocol::encode_request(
            &[
                b"pmessage",
                b"__keyevent@0__:*",
                b"__keyevent@0__:keymiss",
                key,
            ],
            &mut frame,
        );
        assert_eq!(receiver.try_recv().unwrap(), frame);
    };

    assert_eq!(run(&store, &[b"EXISTS", b"missing", b"missing"]), b":0\r\n");
    expect_miss(b"missing");
    expect_miss(b"missing");
    assert_eq!(run(&store, &[b"TYPE", b"missing"]), b"+none\r\n");
    expect_miss(b"missing");
    assert_eq!(run(&store, &[b"TTL", b"missing"]), b":-2\r\n");
    expect_miss(b"missing");
    assert_eq!(run(&store, &[b"PTTL", b"missing"]), b":-2\r\n");
    expect_miss(b"missing");

    assert_eq!(run(&store, &[b"RPUSH", b"list", b"a"]), b":1\r\n");
    assert!(receiver.try_recv().is_err(), "m covers only read misses");
    assert_eq!(run(&store, &[b"EXISTS", b"list"]), b":1\r\n");
    assert_eq!(run(&store, &[b"TYPE", b"list"]), b"+list\r\n");
    assert_eq!(run(&store, &[b"TTL", b"list"]), b":-1\r\n");
    assert!(receiver.try_recv().is_err(), "existing keys are not misses");
}

#[test]
fn hash_and_string_read_misses_emit_keymiss() {
    use crate::engine::NotifyFlags;
    let store = Store::new(128);
    store.set_notify_flags(NotifyFlags::parse(b"Em").unwrap());
    let (sender, receiver) = std::sync::mpsc::sync_channel(Store::pubsub_queue_capacity());
    assert!(store.pubsub_subscribe(true, b"__keyevent@0__:*", 1, sender, false));
    let expect_miss = |key: &[u8]| {
        let mut frame = Vec::new();
        crate::protocol::encode_request(
            &[
                b"pmessage",
                b"__keyevent@0__:*",
                b"__keyevent@0__:keymiss",
                key,
            ],
            &mut frame,
        );
        assert_eq!(receiver.try_recv().unwrap(), frame);
    };

    assert_eq!(run(&store, &[b"HGET", b"missing", b"f"]), b"$-1\r\n");
    expect_miss(b"missing");
    assert_eq!(run(&store, &[b"STRLEN", b"missing"]), b":0\r\n");
    expect_miss(b"missing");
    assert_eq!(
        run(&store, &[b"GETRANGE", b"missing", b"0", b"1"]),
        b"$0\r\n\r\n"
    );
    expect_miss(b"missing");
    assert_eq!(run(&store, &[b"GETBIT", b"missing", b"0"]), b":0\r\n");
    expect_miss(b"missing");

    assert_eq!(run(&store, &[b"HSET", b"h", b"f", b"v"]), b":1\r\n");
    assert_eq!(run(&store, &[b"SET", b"s", b"x"]), b"+OK\r\n");
    assert!(receiver.try_recv().is_err(), "m covers only read misses");
    assert_eq!(run(&store, &[b"HGET", b"h", b"absent"]), b"$-1\r\n");
    assert_eq!(run(&store, &[b"STRLEN", b"s"]), b":1\r\n");
    assert_eq!(
        run(&store, &[b"GETRANGE", b"s", b"0", b"0"]),
        b"$1\r\nx\r\n"
    );
    assert_eq!(run(&store, &[b"GETBIT", b"s", b"0"]), b":0\r\n");
    assert!(receiver.try_recv().is_err(), "existing keys are not misses");

    assert_eq!(run(&store, &[b"RPUSH", b"l", b"a"]), b":1\r\n");
    assert_eq!(
        run(&store, &[b"HGET", b"l", b"f"]),
        b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n"
    );
    assert!(receiver.try_recv().is_err(), "wrong type is not a key miss");
}

#[test]
fn list_read_misses_emit_keymiss() {
    use crate::engine::NotifyFlags;
    let store = Store::new(128);
    store.set_notify_flags(NotifyFlags::parse(b"Em").unwrap());
    let (sender, receiver) = std::sync::mpsc::sync_channel(Store::pubsub_queue_capacity());
    assert!(store.pubsub_subscribe(true, b"__keyevent@0__:*", 1, sender, false));
    let expect_miss = |key: &[u8]| {
        let mut frame = Vec::new();
        crate::protocol::encode_request(
            &[
                b"pmessage",
                b"__keyevent@0__:*",
                b"__keyevent@0__:keymiss",
                key,
            ],
            &mut frame,
        );
        assert_eq!(receiver.try_recv().unwrap(), frame);
    };

    assert_eq!(run(&store, &[b"LLEN", b"missing"]), b":0\r\n");
    expect_miss(b"missing");
    assert_eq!(run(&store, &[b"LINDEX", b"missing", b"0"]), b"$-1\r\n");
    expect_miss(b"missing");
    assert_eq!(run(&store, &[b"LRANGE", b"missing", b"0", b"1"]), b"*0\r\n");
    expect_miss(b"missing");
    assert_eq!(run(&store, &[b"LPOS", b"missing", b"x"]), b"$-1\r\n");
    expect_miss(b"missing");

    assert_eq!(run(&store, &[b"RPUSH", b"l", b"a", b"b"]), b":2\r\n");
    assert!(receiver.try_recv().is_err(), "m covers only read misses");
    assert_eq!(run(&store, &[b"LLEN", b"l"]), b":2\r\n");
    assert_eq!(run(&store, &[b"LINDEX", b"l", b"0"]), b"$1\r\na\r\n");
    assert_eq!(
        run(&store, &[b"LRANGE", b"l", b"0", b"0"]),
        b"*1\r\n$1\r\na\r\n"
    );
    assert_eq!(run(&store, &[b"LPOS", b"l", b"absent"]), b"$-1\r\n");
    assert!(
        receiver.try_recv().is_err(),
        "existing lists are not misses"
    );
}

#[test]
fn random_member_and_stream_read_misses_emit_keymiss() {
    use crate::engine::NotifyFlags;
    let store = Store::new(128);
    store.set_notify_flags(NotifyFlags::parse(b"Em").unwrap());
    let (sender, receiver) = std::sync::mpsc::sync_channel(Store::pubsub_queue_capacity());
    assert!(store.pubsub_subscribe(true, b"__keyevent@0__:*", 1, sender, false));
    let expect_miss = |key: &[u8]| {
        let mut frame = Vec::new();
        crate::protocol::encode_request(
            &[
                b"pmessage",
                b"__keyevent@0__:*",
                b"__keyevent@0__:keymiss",
                key,
            ],
            &mut frame,
        );
        assert_eq!(receiver.try_recv().unwrap(), frame);
    };

    assert_eq!(run(&store, &[b"SRANDMEMBER", b"missing"]), b"$-1\r\n");
    expect_miss(b"missing");
    assert_eq!(run(&store, &[b"SRANDMEMBER", b"missing", b"2"]), b"*0\r\n");
    expect_miss(b"missing");
    assert_eq!(run(&store, &[b"ZRANDMEMBER", b"missing"]), b"$-1\r\n");
    expect_miss(b"missing");
    assert_eq!(run(&store, &[b"ZRANDMEMBER", b"missing", b"2"]), b"*0\r\n");
    expect_miss(b"missing");
    assert_eq!(run(&store, &[b"HRANDFIELD", b"missing"]), b"$-1\r\n");
    expect_miss(b"missing");
    assert_eq!(run(&store, &[b"HRANDFIELD", b"missing", b"2"]), b"*0\r\n");
    expect_miss(b"missing");
    assert_eq!(
        run(&store, &[b"HRANDFIELD", b"missing", b"2", b"WITHVALUES"]),
        b"*0\r\n"
    );
    expect_miss(b"missing");
    assert_eq!(run(&store, &[b"XLEN", b"missing"]), b":0\r\n");
    expect_miss(b"missing");
    assert_eq!(run(&store, &[b"XRANGE", b"missing", b"-", b"+"]), b"*0\r\n");
    expect_miss(b"missing");
    assert_eq!(
        run(&store, &[b"XREVRANGE", b"missing", b"+", b"-"]),
        b"*0\r\n"
    );
    expect_miss(b"missing");

    assert_eq!(run(&store, &[b"SADD", b"s", b"a"]), b":1\r\n");
    assert_eq!(run(&store, &[b"ZADD", b"z", b"1", b"a"]), b":1\r\n");
    assert_eq!(run(&store, &[b"HSET", b"h", b"f", b"v"]), b":1\r\n");
    assert_eq!(
        run(&store, &[b"XADD", b"x", b"1-1", b"f", b"v"]),
        b"$3\r\n1-1\r\n"
    );
    assert!(receiver.try_recv().is_err(), "m covers only read misses");
    assert_eq!(run(&store, &[b"SRANDMEMBER", b"s"]), b"$1\r\na\r\n");
    assert_eq!(run(&store, &[b"ZRANDMEMBER", b"z"]), b"$1\r\na\r\n");
    assert_eq!(run(&store, &[b"HRANDFIELD", b"h"]), b"$1\r\nf\r\n");
    assert_eq!(run(&store, &[b"XLEN", b"x"]), b":1\r\n");
    assert!(run(&store, &[b"XRANGE", b"x", b"-", b"+"]).starts_with(b"*1\r\n"));
    assert!(receiver.try_recv().is_err(), "existing keys are not misses");
}

#[test]
fn set_and_sorted_set_algebra_misses_emit_keymiss() {
    use crate::engine::NotifyFlags;
    let store = Store::new(128);
    store.set_notify_flags(NotifyFlags::parse(b"Em").unwrap());
    let (sender, receiver) = std::sync::mpsc::sync_channel(Store::pubsub_queue_capacity());
    assert!(store.pubsub_subscribe(true, b"__keyevent@0__:*", 1, sender, false));
    let expect_miss = |key: &[u8]| {
        let mut frame = Vec::new();
        crate::protocol::encode_request(
            &[
                b"pmessage",
                b"__keyevent@0__:*",
                b"__keyevent@0__:keymiss",
                key,
            ],
            &mut frame,
        );
        assert_eq!(receiver.try_recv().unwrap(), frame);
    };

    assert_eq!(run(&store, &[b"SINTER", b"missing"]), b"*0\r\n");
    expect_miss(b"missing");
    assert_eq!(run(&store, &[b"SUNION", b"missing"]), b"*0\r\n");
    expect_miss(b"missing");
    assert_eq!(run(&store, &[b"SDIFF", b"missing"]), b"*0\r\n");
    expect_miss(b"missing");
    assert_eq!(run(&store, &[b"SINTERCARD", b"1", b"missing"]), b":0\r\n");
    expect_miss(b"missing");
    assert_eq!(
        run(&store, &[b"SINTERSTORE", b"dst", b"missing"]),
        b":0\r\n"
    );
    expect_miss(b"missing");
    assert_eq!(
        run(&store, &[b"SUNIONSTORE", b"dst", b"missing"]),
        b":0\r\n"
    );
    expect_miss(b"missing");
    // Every source is looked up, so each missing key reports once.
    assert_eq!(run(&store, &[b"SINTER", b"one", b"two"]), b"*0\r\n");
    expect_miss(b"one");
    expect_miss(b"two");
    assert_eq!(run(&store, &[b"ZUNION", b"1", b"missing"]), b"*0\r\n");
    expect_miss(b"missing");
    assert_eq!(run(&store, &[b"ZINTER", b"1", b"missing"]), b"*0\r\n");
    expect_miss(b"missing");
    assert_eq!(run(&store, &[b"ZDIFF", b"1", b"missing"]), b"*0\r\n");
    expect_miss(b"missing");
    assert_eq!(run(&store, &[b"ZINTERCARD", b"1", b"missing"]), b":0\r\n");
    expect_miss(b"missing");
    assert_eq!(
        run(&store, &[b"ZUNIONSTORE", b"dst", b"1", b"missing"]),
        b":0\r\n"
    );
    expect_miss(b"missing");
    assert_eq!(
        run(&store, &[b"ZDIFFSTORE", b"dst", b"1", b"missing"]),
        b":0\r\n"
    );
    expect_miss(b"missing");

    assert_eq!(run(&store, &[b"SADD", b"s", b"a"]), b":1\r\n");
    assert_eq!(run(&store, &[b"ZADD", b"z", b"1", b"a"]), b":1\r\n");
    assert!(receiver.try_recv().is_err(), "m covers only read misses");
    assert_eq!(run(&store, &[b"SINTER", b"s", b"s"]), b"*1\r\n$1\r\na\r\n");
    assert_eq!(run(&store, &[b"SUNION", b"s"]), b"*1\r\n$1\r\na\r\n");
    assert_eq!(run(&store, &[b"SINTERCARD", b"1", b"s"]), b":1\r\n");
    assert_eq!(run(&store, &[b"ZUNION", b"1", b"z"]), b"*1\r\n$1\r\na\r\n");
    assert_eq!(run(&store, &[b"ZDIFF", b"1", b"z"]), b"*1\r\n$1\r\na\r\n");
    assert_eq!(run(&store, &[b"ZINTERCARD", b"1", b"z"]), b":1\r\n");
    assert!(receiver.try_recv().is_err(), "existing keys are not misses");
}

#[test]
fn sort_misses_emit_keymiss() {
    use crate::engine::NotifyFlags;
    let store = Store::new(128);
    store.set_notify_flags(NotifyFlags::parse(b"Em").unwrap());
    let (sender, receiver) = std::sync::mpsc::sync_channel(Store::pubsub_queue_capacity());
    assert!(store.pubsub_subscribe(true, b"__keyevent@0__:*", 1, sender, false));
    let expect_miss = |key: &[u8]| {
        let mut frame = Vec::new();
        crate::protocol::encode_request(
            &[
                b"pmessage",
                b"__keyevent@0__:*",
                b"__keyevent@0__:keymiss",
                key,
            ],
            &mut frame,
        );
        assert_eq!(receiver.try_recv().unwrap(), frame);
    };

    assert_eq!(run(&store, &[b"SORT", b"missing"]), b"*0\r\n");
    expect_miss(b"missing");
    assert_eq!(run(&store, &[b"SORT_RO", b"missing"]), b"*0\r\n");
    expect_miss(b"missing");
    assert_eq!(
        run(&store, &[b"SORT", b"missing", b"STORE", b"dst"]),
        b":0\r\n"
    );
    expect_miss(b"missing");

    assert_eq!(run(&store, &[b"RPUSH", b"l", b"2", b"1"]), b":2\r\n");
    assert!(receiver.try_recv().is_err(), "m covers only read misses");
    assert_eq!(
        run(&store, &[b"SORT", b"l"]),
        b"*2\r\n$1\r\n1\r\n$1\r\n2\r\n"
    );
    assert_eq!(
        run(&store, &[b"SORT_RO", b"l"]),
        b"*2\r\n$1\r\n1\r\n$1\r\n2\r\n"
    );
    assert!(receiver.try_recv().is_err(), "existing keys are not misses");
}

#[test]
fn xread_misses_emit_keymiss() {
    use crate::engine::NotifyFlags;
    let store = Store::new(128);
    store.set_notify_flags(NotifyFlags::parse(b"Em").unwrap());
    let (sender, receiver) = std::sync::mpsc::sync_channel(Store::pubsub_queue_capacity());
    assert!(store.pubsub_subscribe(true, b"__keyevent@0__:*", 1, sender, false));
    let expect_miss = |key: &[u8]| {
        let mut frame = Vec::new();
        crate::protocol::encode_request(
            &[
                b"pmessage",
                b"__keyevent@0__:*",
                b"__keyevent@0__:keymiss",
                key,
            ],
            &mut frame,
        );
        assert_eq!(receiver.try_recv().unwrap(), frame);
    };

    assert_eq!(
        run(
            &store,
            &[b"XREAD", b"COUNT", b"1", b"STREAMS", b"missing", b"0"]
        ),
        b"*-1\r\n"
    );
    expect_miss(b"missing");
    let reply = run(
        &store,
        &[
            b"XREADGROUP",
            b"GROUP",
            b"g",
            b"c",
            b"COUNT",
            b"1",
            b"STREAMS",
            b"missing",
            b"0",
        ],
    );
    assert!(reply.starts_with(b"-NOGROUP"), "{reply:?}");
    expect_miss(b"missing");

    assert_eq!(
        run(&store, &[b"XADD", b"x", b"1-1", b"f", b"v"]),
        b"$3\r\n1-1\r\n"
    );
    assert_eq!(
        run(&store, &[b"XGROUP", b"CREATE", b"x", b"g", b"0"]),
        b"+OK\r\n"
    );
    assert!(receiver.try_recv().is_err(), "m covers only read misses");
    assert!(
        run(&store, &[b"XREAD", b"COUNT", b"1", b"STREAMS", b"x", b"0"]).starts_with(b"*1\r\n")
    );
    assert!(
        run(
            &store,
            &[
                b"XREADGROUP",
                b"GROUP",
                b"g",
                b"c",
                b"COUNT",
                b"1",
                b"STREAMS",
                b"x",
                b"0",
            ],
        )
        .starts_with(b"*1\r\n")
    );
    assert!(
        receiver.try_recv().is_err(),
        "existing streams are not misses"
    );
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
    let max_key = vec![b'x'; super::MAX_AUTH_KEY_BYTES];
    let with_crlf = [max_key.as_slice(), b"\r\n"].concat();
    std::fs::write(&path, &with_crlf).unwrap();
    assert_eq!(super::read_auth_key_file(&path).unwrap(), max_key);
    std::fs::write(&path, [with_crlf.as_slice(), b"x"].concat()).unwrap();
    assert!(super::read_auth_key_file(&path).is_err());
    std::fs::write(&path, vec![b'x'; super::MAX_AUTH_KEY_BYTES + 1]).unwrap();
    assert!(super::read_auth_key_file(&path).is_err());
    let _ = std::fs::remove_file(&path);
    assert!(super::read_auth_key_file(&path).is_err());
}

#[test]
fn journal_key_reader_stops_at_the_oversize_sentinel() {
    let cap = super::MAX_AUTH_KEY_BYTES + 3;
    let mut source = std::io::Cursor::new(vec![b'x'; cap + 32]);
    assert_eq!(
        super::read_auth_key_file_bytes(&mut source).unwrap().len(),
        cap
    );
    assert_eq!(source.position(), cap as u64);
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

    let active = super::consumer_active_record(key, group, consumer, 1_700_000_000_456);
    let mut command = None;
    let mut payload = None;
    let parsed = crate::protocol::dispatch_request(&active, Limits::default(), |args| {
        command = Some(args[0].to_vec());
        payload = Some(args[1].to_vec());
        Some(())
    });
    assert!(parsed.unwrap().is_some());
    assert_eq!(command.as_deref(), Some(b"ECHO".as_slice()));
    let payload = payload.expect("consumer-active payload");
    assert_eq!(
        super::parse_consumer_active(&payload),
        Some((
            key.as_slice(),
            group.as_slice(),
            consumer.as_slice(),
            1_700_000_000_456
        ))
    );
    let mut truncated = payload.clone();
    truncated.pop();
    assert!(super::parse_consumer_active(&truncated).is_none());
    let mut trailing = payload;
    trailing.push(b'x');
    assert!(super::parse_consumer_active(&trailing).is_none());
    assert!(super::parse_consumer_active(b"__RILLCACHE_CONSUMER_ACTIVE_V1__:").is_none());
}
#[test]
fn new_key_events_fire_once_for_real_creations() {
    use crate::engine::NotifyFlags;
    let store = Store::new(1024);
    store.set_notify_flags(NotifyFlags::parse(b"En").unwrap());
    let (sender, receiver) = std::sync::mpsc::sync_channel(Store::pubsub_queue_capacity());
    assert!(store.pubsub_subscribe(true, b"__keyevent@0__:*", 1, sender, false));
    let expect_new = |key: &[u8]| {
        let mut frame = Vec::new();
        crate::protocol::encode_request(
            &[b"pmessage", b"__keyevent@0__:*", b"__keyevent@0__:new", key],
            &mut frame,
        );
        assert_eq!(receiver.try_recv().unwrap(), frame);
    };

    assert_eq!(run(&store, &[b"GET", b"missing"]), b"$-1\r\n");
    assert!(
        receiver.try_recv().is_err(),
        "a read miss does not create a key"
    );

    assert_eq!(run(&store, &[b"SET", b"k", b"v"]), b"+OK\r\n");
    expect_new(b"k");
    assert_eq!(run(&store, &[b"SET", b"k", b"v2"]), b"+OK\r\n");
    assert!(receiver.try_recv().is_err(), "overwrite is not a new key");

    assert_eq!(run(&store, &[b"HSET", b"h", b"f", b"v"]), b":1\r\n");
    expect_new(b"h");
    assert_eq!(
        run(&store, &[b"XADD", b"s", b"1-0", b"f", b"v"]),
        b"$3\r\n1-0\r\n"
    );
    expect_new(b"s");
    assert_eq!(
        run(&store, &[b"XADD", b"s", b"2-0", b"f", b"v"]),
        b"$3\r\n2-0\r\n"
    );
    assert!(
        receiver.try_recv().is_err(),
        "second entry is not a new key"
    );

    assert_eq!(run(&store, &[b"COPY", b"k", b"copy"]), b":1\r\n");
    expect_new(b"copy");
    assert_eq!(run(&store, &[b"COPY", b"k", b"copy"]), b":0\r\n");
    assert!(receiver.try_recv().is_err());
}

#[test]
fn additional_hash_read_misses_emit_keymiss() {
    use crate::engine::NotifyFlags;
    let store = Store::new(128);
    store.set_notify_flags(NotifyFlags::parse(b"Em").unwrap());
    let (sender, receiver) = std::sync::mpsc::sync_channel(Store::pubsub_queue_capacity());
    assert!(store.pubsub_subscribe(true, b"__keyevent@0__:*", 1, sender, false));
    let expect_miss = |key: &[u8]| {
        let mut frame = Vec::new();
        crate::protocol::encode_request(
            &[
                b"pmessage",
                b"__keyevent@0__:*",
                b"__keyevent@0__:keymiss",
                key,
            ],
            &mut frame,
        );
        assert_eq!(receiver.try_recv().unwrap(), frame);
    };

    assert_eq!(run(&store, &[b"HEXISTS", b"missing", b"f"]), b":0\r\n");
    expect_miss(b"missing");
    assert_eq!(
        run(&store, &[b"HMGET", b"missing", b"a", b"b"]),
        b"*2\r\n$-1\r\n$-1\r\n"
    );
    expect_miss(b"missing");
    assert_eq!(run(&store, &[b"HKEYS", b"missing"]), b"*0\r\n");
    expect_miss(b"missing");
    assert_eq!(run(&store, &[b"HVALS", b"missing"]), b"*0\r\n");
    expect_miss(b"missing");
    assert_eq!(run(&store, &[b"HSTRLEN", b"missing", b"f"]), b":0\r\n");
    expect_miss(b"missing");

    assert_eq!(run(&store, &[b"HSET", b"h", b"f", b"v"]), b":1\r\n");
    assert!(receiver.try_recv().is_err(), "m covers only read misses");
    assert_eq!(run(&store, &[b"HEXISTS", b"h", b"absent"]), b":0\r\n");
    assert_eq!(run(&store, &[b"HMGET", b"h", b"absent"]), b"*1\r\n$-1\r\n");
    assert_eq!(run(&store, &[b"HSTRLEN", b"h", b"absent"]), b":0\r\n");
    assert_eq!(run(&store, &[b"HKEYS", b"h"]), b"*1\r\n$1\r\nf\r\n");
    assert!(
        receiver.try_recv().is_err(),
        "existing hash fields/keys are not misses"
    );
}

#[test]
fn set_and_sorted_set_read_misses_emit_keymiss() {
    use crate::engine::NotifyFlags;
    let store = Store::new(128);
    store.set_notify_flags(NotifyFlags::parse(b"Em").unwrap());
    let (sender, receiver) = std::sync::mpsc::sync_channel(Store::pubsub_queue_capacity());
    assert!(store.pubsub_subscribe(true, b"__keyevent@0__:*", 1, sender, false));
    let expect_miss = |key: &[u8]| {
        let mut frame = Vec::new();
        crate::protocol::encode_request(
            &[
                b"pmessage",
                b"__keyevent@0__:*",
                b"__keyevent@0__:keymiss",
                key,
            ],
            &mut frame,
        );
        assert_eq!(receiver.try_recv().unwrap(), frame);
    };

    assert_eq!(run(&store, &[b"SCARD", b"missing"]), b":0\r\n");
    expect_miss(b"missing");
    assert_eq!(run(&store, &[b"SISMEMBER", b"missing", b"m"]), b":0\r\n");
    expect_miss(b"missing");
    assert_eq!(run(&store, &[b"SMEMBERS", b"missing"]), b"*0\r\n");
    expect_miss(b"missing");
    assert_eq!(run(&store, &[b"ZCARD", b"missing"]), b":0\r\n");
    expect_miss(b"missing");
    assert_eq!(run(&store, &[b"ZSCORE", b"missing", b"m"]), b"$-1\r\n");
    expect_miss(b"missing");
    assert_eq!(
        run(&store, &[b"ZMSCORE", b"missing", b"a", b"b"]),
        b"*2\r\n$-1\r\n$-1\r\n"
    );
    expect_miss(b"missing");
    assert_eq!(
        run(&store, &[b"ZRANGE", b"missing", b"0", b"-1"]),
        b"*0\r\n"
    );
    expect_miss(b"missing");
    assert_eq!(run(&store, &[b"ZRANK", b"missing", b"m"]), b"$-1\r\n");
    expect_miss(b"missing");
    assert_eq!(run(&store, &[b"ZREVRANK", b"missing", b"m"]), b"$-1\r\n");
    expect_miss(b"missing");
    assert_eq!(
        run(&store, &[b"ZCOUNT", b"missing", b"-inf", b"+inf"]),
        b":0\r\n"
    );
    expect_miss(b"missing");
    assert_eq!(
        run(&store, &[b"ZRANGEBYSCORE", b"missing", b"-inf", b"+inf"]),
        b"*0\r\n"
    );
    expect_miss(b"missing");
    assert_eq!(
        run(&store, &[b"ZREVRANGEBYSCORE", b"missing", b"+inf", b"-inf"]),
        b"*0\r\n"
    );
    expect_miss(b"missing");
    assert_eq!(
        run(&store, &[b"ZREVRANGE", b"missing", b"0", b"-1"]),
        b"*0\r\n"
    );
    expect_miss(b"missing");
    assert_eq!(
        run(&store, &[b"ZLEXCOUNT", b"missing", b"-", b"+"]),
        b":0\r\n"
    );
    expect_miss(b"missing");

    assert_eq!(run(&store, &[b"SADD", b"s", b"a"]), b":1\r\n");
    assert_eq!(run(&store, &[b"ZADD", b"z", b"1", b"a"]), b":1\r\n");
    assert!(receiver.try_recv().is_err(), "m covers only read misses");
    assert_eq!(run(&store, &[b"SCARD", b"s"]), b":1\r\n");
    assert_eq!(run(&store, &[b"SISMEMBER", b"s", b"absent"]), b":0\r\n");
    assert_eq!(run(&store, &[b"SMEMBERS", b"s"]), b"*1\r\n$1\r\na\r\n");
    assert_eq!(run(&store, &[b"ZCARD", b"z"]), b":1\r\n");
    assert_eq!(run(&store, &[b"ZSCORE", b"z", b"absent"]), b"$-1\r\n");
    assert_eq!(run(&store, &[b"ZRANK", b"z", b"absent"]), b"$-1\r\n");
    assert_eq!(run(&store, &[b"ZCOUNT", b"z", b"5", b"6"]), b":0\r\n");
    assert_eq!(
        run(&store, &[b"ZRANGEBYSCORE", b"z", b"5", b"6"]),
        b"*0\r\n"
    );
    assert!(receiver.try_recv().is_err(), "existing keys are not misses");
}

#[test]
fn collection_scan_misses_emit_keymiss() {
    use crate::engine::NotifyFlags;
    let store = Store::new(128);
    store.set_notify_flags(NotifyFlags::parse(b"Em").unwrap());
    let (sender, receiver) = std::sync::mpsc::sync_channel(Store::pubsub_queue_capacity());
    assert!(store.pubsub_subscribe(true, b"__keyevent@0__:*", 1, sender, false));
    let expect_miss = |key: &[u8]| {
        let mut frame = Vec::new();
        crate::protocol::encode_request(
            &[
                b"pmessage",
                b"__keyevent@0__:*",
                b"__keyevent@0__:keymiss",
                key,
            ],
            &mut frame,
        );
        assert_eq!(receiver.try_recv().unwrap(), frame);
    };

    assert_eq!(
        run(&store, &[b"HSCAN", b"missing", b"0"]),
        b"*2\r\n$1\r\n0\r\n*0\r\n"
    );
    expect_miss(b"missing");
    assert_eq!(
        run(&store, &[b"SSCAN", b"missing", b"0"]),
        b"*2\r\n$1\r\n0\r\n*0\r\n"
    );
    expect_miss(b"missing");
    assert_eq!(
        run(&store, &[b"ZSCAN", b"missing", b"0"]),
        b"*2\r\n$1\r\n0\r\n*0\r\n"
    );
    expect_miss(b"missing");

    assert_eq!(run(&store, &[b"HSET", b"h", b"f", b"v"]), b":1\r\n");
    assert_eq!(run(&store, &[b"SADD", b"s", b"a"]), b":1\r\n");
    assert_eq!(run(&store, &[b"ZADD", b"z", b"1", b"a"]), b":1\r\n");
    assert!(receiver.try_recv().is_err(), "m covers only read misses");
    assert!(run(&store, &[b"HSCAN", b"h", b"0"]).starts_with(b"*2\r\n$1\r\n0\r\n"));
    assert!(run(&store, &[b"SSCAN", b"s", b"0"]).starts_with(b"*2\r\n$1\r\n0\r\n"));
    assert!(run(&store, &[b"ZSCAN", b"z", b"0"]).starts_with(b"*2\r\n$1\r\n0\r\n"));
    assert!(receiver.try_recv().is_err(), "existing keys are not misses");
}

#[test]
fn stream_metadata_read_misses_emit_keymiss() {
    use crate::engine::NotifyFlags;
    let store = Store::new(128);
    store.set_notify_flags(NotifyFlags::parse(b"Em").unwrap());
    let (sender, receiver) = std::sync::mpsc::sync_channel(Store::pubsub_queue_capacity());
    assert!(store.pubsub_subscribe(true, b"__keyevent@0__:*", 1, sender, false));
    let expect_miss = |key: &[u8]| {
        let mut frame = Vec::new();
        crate::protocol::encode_request(
            &[
                b"pmessage",
                b"__keyevent@0__:*",
                b"__keyevent@0__:keymiss",
                key,
            ],
            &mut frame,
        );
        assert_eq!(receiver.try_recv().unwrap(), frame);
    };

    assert_eq!(run(&store, &[b"XACK", b"missing", b"g", b"1-1"]), b":0\r\n");
    expect_miss(b"missing");
    assert_eq!(
        run(&store, &[b"XPENDING", b"missing", b"g"]),
        b"-NOGROUP No such key or consumer group\r\n"
    );
    expect_miss(b"missing");
    assert_eq!(
        run(&store, &[b"XCLAIM", b"missing", b"g", b"c", b"0", b"1-1"]),
        b"-NOGROUP No such key or consumer group\r\n"
    );
    expect_miss(b"missing");
    assert_eq!(
        run(&store, &[b"XAUTOCLAIM", b"missing", b"g", b"c", b"0", b"0"]),
        b"-NOGROUP No such key or consumer group\r\n"
    );
    expect_miss(b"missing");
    assert_eq!(
        run(&store, &[b"XINFO", b"STREAM", b"missing"]),
        b"-ERR no such key\r\n"
    );
    expect_miss(b"missing");
    assert_eq!(
        run(&store, &[b"XINFO", b"GROUPS", b"missing"]),
        b"-ERR no such key\r\n"
    );
    expect_miss(b"missing");
    assert_eq!(
        run(&store, &[b"XINFO", b"CONSUMERS", b"missing", b"g"]),
        b"-ERR no such key\r\n"
    );
    expect_miss(b"missing");

    assert_eq!(
        run(&store, &[b"XADD", b"x", b"1-1", b"f", b"v"]),
        b"$3\r\n1-1\r\n"
    );
    assert_eq!(
        run(&store, &[b"XGROUP", b"CREATE", b"x", b"g", b"0"]),
        b"+OK\r\n"
    );
    assert!(receiver.try_recv().is_err(), "m covers only read misses");
    assert_eq!(run(&store, &[b"XACK", b"x", b"g", b"1-1"]), b":0\r\n");
    assert!(run(&store, &[b"XPENDING", b"x", b"g"]).starts_with(b"*4\r\n"));
    let info = run(&store, &[b"XINFO", b"STREAM", b"x"]);
    assert!(info.starts_with(b"*"));
    assert!(info.windows(6).any(|part| part == b"length"));
    assert!(run(&store, &[b"XINFO", b"GROUPS", b"x"]).starts_with(b"*1\r\n"));
    assert!(run(&store, &[b"XINFO", b"CONSUMERS", b"x", b"g"]).starts_with(b"*0\r\n"));
    assert!(receiver.try_recv().is_err(), "existing keys are not misses");
}

#[test]
fn sort_pattern_misses_emit_keymiss() {
    use crate::engine::NotifyFlags;
    let store = Store::new(128);
    store.set_notify_flags(NotifyFlags::parse(b"Em").unwrap());
    let (sender, receiver) = std::sync::mpsc::sync_channel(Store::pubsub_queue_capacity());
    assert!(store.pubsub_subscribe(true, b"__keyevent@0__:*", 1, sender, false));
    let expect_miss = |key: &[u8]| {
        let mut frame = Vec::new();
        crate::protocol::encode_request(
            &[
                b"pmessage",
                b"__keyevent@0__:*",
                b"__keyevent@0__:keymiss",
                key,
            ],
            &mut frame,
        );
        assert_eq!(receiver.try_recv().unwrap(), frame);
    };

    assert_eq!(run(&store, &[b"RPUSH", b"l", b"a"]), b":1\r\n");
    assert!(receiver.try_recv().is_err(), "m covers only read misses");
    assert_eq!(
        run(&store, &[b"SORT", b"l", b"BY", b"weight_*"]),
        b"*1\r\n$1\r\na\r\n"
    );
    expect_miss(b"weight_a");
    assert_eq!(
        run(&store, &[b"SORT", b"l", b"GET", b"data_*", b"ALPHA"]),
        b"*1\r\n$-1\r\n"
    );
    expect_miss(b"data_a");
    assert_eq!(
        run(
            &store,
            &[b"SORT_RO", b"l", b"GET", b"data_*", b"BY", b"nosort"]
        ),
        b"*1\r\n$-1\r\n"
    );
    expect_miss(b"data_a");
    assert_eq!(
        run(&store, &[b"SORT", b"l", b"BY", b"nosort"]),
        b"*1\r\n$1\r\na\r\n"
    );
    assert!(
        receiver.try_recv().is_err(),
        "constant BY does not look up keys"
    );

    assert_eq!(run(&store, &[b"SET", b"weight_a", b"1"]), b"+OK\r\n");
    assert_eq!(run(&store, &[b"SET", b"data_a", b"v"]), b"+OK\r\n");
    assert!(receiver.try_recv().is_err(), "m covers only read misses");
    assert_eq!(
        run(
            &store,
            &[b"SORT", b"l", b"BY", b"weight_*", b"GET", b"data_*"]
        ),
        b"*1\r\n$1\r\nv\r\n"
    );
    assert!(
        receiver.try_recv().is_err(),
        "existing pattern keys are not misses"
    );
}

#[test]
fn bit_algorithm_read_misses_emit_keymiss() {
    use crate::engine::NotifyFlags;
    let store = Store::new(128);
    store.set_notify_flags(NotifyFlags::parse(b"Em").unwrap());
    let (sender, receiver) = std::sync::mpsc::sync_channel(Store::pubsub_queue_capacity());
    assert!(store.pubsub_subscribe(true, b"__keyevent@0__:*", 1, sender, false));
    let expect_miss = |key: &[u8]| {
        let mut frame = Vec::new();
        crate::protocol::encode_request(
            &[
                b"pmessage",
                b"__keyevent@0__:*",
                b"__keyevent@0__:keymiss",
                key,
            ],
            &mut frame,
        );
        assert_eq!(receiver.try_recv().unwrap(), frame);
    };

    assert_eq!(run(&store, &[b"BITCOUNT", b"missing"]), b":0\r\n");
    expect_miss(b"missing");
    assert_eq!(run(&store, &[b"BITPOS", b"missing", b"1"]), b":-1\r\n");
    expect_miss(b"missing");
    assert_eq!(
        run(&store, &[b"BITFIELD_RO", b"missing", b"GET", b"u8", b"0"]),
        b"*1\r\n:0\r\n"
    );
    expect_miss(b"missing");
    assert_eq!(run(&store, &[b"LCS", b"a", b"b", b"LEN"]), b":0\r\n");
    expect_miss(b"a");
    expect_miss(b"b");

    assert_eq!(run(&store, &[b"SET", b"a", b"x"]), b"+OK\r\n");
    assert!(receiver.try_recv().is_err(), "m covers only read misses");
    assert_eq!(run(&store, &[b"BITCOUNT", b"a"]), b":4\r\n");
    assert_eq!(run(&store, &[b"BITPOS", b"a", b"0"]), b":0\r\n");
    assert_eq!(
        run(&store, &[b"BITFIELD_RO", b"a", b"GET", b"u8", b"0"]),
        b"*1\r\n:120\r\n"
    );
    assert!(receiver.try_recv().is_err(), "existing keys are not misses");
    assert_eq!(run(&store, &[b"LCS", b"a", b"b", b"LEN"]), b":0\r\n");
    expect_miss(b"b");
    assert_eq!(run(&store, &[b"LCS", b"a", b"a", b"LEN"]), b":1\r\n");
    assert!(receiver.try_recv().is_err(), "two hits are not misses");

    assert_eq!(run(&store, &[b"RPUSH", b"l", b"a"]), b":1\r\n");
    assert_eq!(
        run(&store, &[b"BITCOUNT", b"l"]),
        b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n"
    );
    assert!(receiver.try_recv().is_err(), "wrong type is not a key miss");
    // Both operands are read before the type check.
    assert_eq!(
        run(&store, &[b"LCS", b"l", b"missing2", b"LEN"]),
        b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n"
    );
    expect_miss(b"missing2");
    assert!(receiver.try_recv().is_err(), "wrong type is not a key miss");
    // A missing first operand is reported before the second operand's type error.
    assert_eq!(
        run(&store, &[b"LCS", b"missing3", b"l", b"LEN"]),
        b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n"
    );
    expect_miss(b"missing3");
}

#[test]
fn introspection_and_hash_ttl_misses_emit_keymiss() {
    use crate::engine::NotifyFlags;
    let store = Store::new(128);
    store.set_notify_flags(NotifyFlags::parse(b"Em").unwrap());
    let (sender, receiver) = std::sync::mpsc::sync_channel(Store::pubsub_queue_capacity());
    assert!(store.pubsub_subscribe(true, b"__keyevent@0__:*", 1, sender, false));
    let expect_miss = |key: &[u8]| {
        let mut frame = Vec::new();
        crate::protocol::encode_request(
            &[
                b"pmessage",
                b"__keyevent@0__:*",
                b"__keyevent@0__:keymiss",
                key,
            ],
            &mut frame,
        );
        assert_eq!(receiver.try_recv().unwrap(), frame);
    };

    assert_eq!(
        run(&store, &[b"OBJECT", b"ENCODING", b"missing"]),
        b"$-1\r\n"
    );
    expect_miss(b"missing");
    assert_eq!(
        run(&store, &[b"OBJECT", b"REFCOUNT", b"missing"]),
        b"$-1\r\n"
    );
    expect_miss(b"missing");
    assert_eq!(run(&store, &[b"MEMORY", b"USAGE", b"missing"]), b"$-1\r\n");
    expect_miss(b"missing");
    for command in [
        b"HTTL".as_slice(),
        b"HPTTL",
        b"HEXPIRETIME",
        b"HPEXPIRETIME",
    ] {
        assert_eq!(
            run(&store, &[command, b"missing", b"FIELDS", b"1", b"f"]),
            b"*1\r\n:-2\r\n"
        );
        expect_miss(b"missing");
    }

    assert_eq!(run(&store, &[b"SET", b"s", b"v"]), b"+OK\r\n");
    assert_eq!(run(&store, &[b"HSET", b"h", b"f", b"v"]), b":1\r\n");
    assert!(receiver.try_recv().is_err(), "m covers only read misses");
    assert_eq!(
        run(&store, &[b"OBJECT", b"ENCODING", b"s"]),
        b"$3\r\nraw\r\n"
    );
    assert_eq!(run(&store, &[b"OBJECT", b"REFCOUNT", b"s"]), b":1\r\n");
    assert_eq!(run(&store, &[b"MEMORY", b"USAGE", b"s"]), b":18\r\n");
    assert_eq!(
        run(&store, &[b"OBJECT", b"IDLETIME", b"s"]),
        b"-ERR OBJECT IDLETIME is not supported by this bounded server: read-idle time is not tracked\r\n"
    );
    assert_eq!(
        run(&store, &[b"OBJECT", b"FREQ", b"s"]),
        b"-ERR An LFU maxmemory policy is not selected, access frequency not tracked. Please note that when switching between maxmemory policies at runtime LFU and LRU data will take some time to adjust.\r\n"
    );
    assert_eq!(
        run(&store, &[b"HTTL", b"h", b"FIELDS", b"1", b"absent"]),
        b"*1\r\n:-2\r\n"
    );
    assert_eq!(
        run(&store, &[b"HTTL", b"h", b"FIELDS", b"1", b"f"]),
        b"*1\r\n:-1\r\n"
    );
    assert!(
        receiver.try_recv().is_err(),
        "hits, absent fields and capability errors are not misses"
    );
}
