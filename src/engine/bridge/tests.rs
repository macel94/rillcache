use super::*;
use std::sync::atomic::AtomicUsize;
use std::sync::{Barrier, mpsc};
use std::time::Duration;

impl Drop for ShardExecutor {
    fn drop(&mut self) {
        let mut events = self.services.owner_events.lock().unwrap();
        let current = std::thread::current().id();
        assert_eq!(events[0], ("construct", current));
        assert!(self.data.try_borrow_mut().is_ok(), "borrow survived a turn");
        events.push(("drop", current));
    }
}

#[test]
fn executor_is_not_send_or_sync_but_store_is_both() {
    // Inference becomes ambiguous (a compile error) if either forbidden bound
    // starts being satisfied. No unsafe assertion or extra test crate is used.
    trait NotSend<A> {
        fn check() {}
    }
    impl<T: ?Sized> NotSend<()> for T {}
    struct SendWitness;
    impl<T: ?Sized + Send> NotSend<SendWitness> for T {}
    let _ = <ShardExecutor as NotSend<_>>::check;
    trait NotSync<A> {
        fn check() {}
    }
    impl<T: ?Sized> NotSync<()> for T {}
    struct SyncWitness;
    impl<T: ?Sized + Sync> NotSync<SyncWitness> for T {}
    let _ = <ShardExecutor as NotSync<_>>::check;
    fn shared<T: Send + Sync>() {}
    shared::<Store>();
}

#[test]
fn local_sessions_and_external_calls_use_identical_complete_dispatch() {
    let store = Store::new(1024 * 1024);
    let (local, inbox) = store.install_local().unwrap();
    let external = Store::new(1024 * 1024);
    let mut left = crate::session::Session::new();
    let mut right = crate::session::Session::new();
    let commands: &[&[&[u8]]] = &[
        &[b"PING"],
        &[b"SET", b"\0key", b"\xffvalue", b"PX", b"900000"],
        &[b"GET", b"\0key"],
        &[b"HSET", b"hash", b"field", b"value"],
        &[b"HGET", b"hash", b"field"],
        &[b"RPUSH", b"list", b"a", b"b"],
        &[b"BLPOP", b"list", b"0"],
        &[b"SADD", b"set", b"a", b"b"],
        &[b"SISMEMBER", b"set", b"b"],
        &[b"ZADD", b"sorted", b"1", b"a"],
        &[b"ZRANGE", b"sorted", b"0", b"-1"],
        &[b"XADD", b"stream", b"1-0", b"field", b"value"],
        &[b"XRANGE", b"stream", b"-", b"+"],
        &[b"PFADD", b"hll", b"a", b"b"],
        &[b"PFCOUNT", b"hll"],
        &[b"GEOADD", b"geo", b"13", b"38", b"member"],
        &[b"GEOPOS", b"geo", b"member"],
        &[b"SETBIT", b"bits", b"3", b"1"],
        &[b"GETBIT", b"bits", b"3"],
        &[b"MSET", b"a", b"old", b"b", b"old"],
        &[b"WATCH", b"a"],
        &[b"SET", b"a", b"changed"],
        &[b"MULTI"],
        &[b"MSET", b"a", b"new", b"b", b"new"],
        &[b"EXEC"],
        &[b"MGET", b"a", b"b"],
        &[b"MULTI"],
        &[b"SET", b"a", b"committed"],
        &[b"INCR", b"a"],
        &[b"SET", b"b", b"later"],
        &[b"EXEC"],
        &[b"MGET", b"a", b"b"],
        &[b"COPY", b"hash", b"hash-copy"],
        &[b"RENAME", b"hash-copy", b"renamed"],
        &[b"TYPE", b"renamed"],
        &[b"CONFIG", b"SET", b"maxmemory-policy", b"allkeys-random"],
        &[b"CONFIG", b"GET", b"maxmemory-policy"],
        &[b"FLUSHALL"],
        &[b"DBSIZE"],
    ];
    for arguments in commands {
        let mut actual = Vec::new();
        let mut expected = Vec::new();
        assert_eq!(
            left.execute_on(&local, arguments, &mut actual),
            right.execute(&external, arguments, &mut expected)
        );
        assert_eq!(actual, expected, "{arguments:?}");
        assert!(local.data.try_borrow_mut().is_ok());
    }
    assert_eq!(store.services.bridge_submissions.load(Ordering::Relaxed), 0);
    assert_eq!(
        local.stats().total_commands_processed,
        commands.len() as u64
    );
    assert_eq!(
        local.stats().total_commands_processed,
        external.stats().total_commands_processed
    );
    drop(inbox);
    drop(local);
    let events = store.services.owner_events.lock().unwrap();
    assert_eq!(
        events.as_slice(),
        &[
            ("construct", std::thread::current().id()),
            ("drop", std::thread::current().id())
        ]
    );
}

#[test]
fn external_commands_execute_and_drop_data_on_the_same_joined_owner() {
    let store = Store::new(1024);
    let services = Arc::clone(&store.services);
    let caller = std::thread::current().id();
    let owner = store
        .try_submit(|| |_: &ShardExecutor| std::thread::current().id())
        .unwrap();
    assert_ne!(owner, caller);
    let mut reply = Vec::new();
    store.execute(&[b"SET", b"key", b"value"], &mut reply);
    assert_eq!(reply, b"+OK\r\n");
    assert!(
        store.install_local().is_none(),
        "live ownership cannot migrate"
    );
    drop(store);
    assert_eq!(
        services.owner_events.lock().unwrap().as_slice(),
        &[("construct", owner), ("drop", owner)]
    );
}

#[test]
fn external_request_and_reply_are_serviced_between_local_turns() {
    let store = Arc::new(Store::new(1024));
    let (local, inbox) = store.install_local().unwrap();
    let (entered, observed) = mpsc::sync_channel(1);
    let bridge = Arc::clone(&store);
    let remote = std::thread::spawn(move || {
        bridge
            .try_submit(|| {
                move |executor: &ShardExecutor| {
                    entered.send(std::thread::current().id()).unwrap();
                    let mut reply = Vec::new();
                    executor.execute(&[b"MSET", b"a", b"same", b"b", b"same"], &mut reply);
                    reply
                }
            })
            .unwrap()
    });
    // Receive the admitted request explicitly; no scheduling sleep is needed.
    let request = inbox.0.recv_timeout(Duration::from_secs(3)).unwrap();
    local.run_request(request);
    assert_eq!(observed.recv().unwrap(), std::thread::current().id());
    assert_eq!(remote.join().unwrap(), b"+OK\r\n");
    let mut reply = Vec::new();
    local.execute(&[b"MGET", b"a", b"b"], &mut reply);
    assert_eq!(reply, b"*2\r\n$4\r\nsame\r\n$4\r\nsame\r\n");
    assert_eq!(store.services.bridge_submissions.load(Ordering::Relaxed), 1);
}

#[test]
fn admission_precedes_copy_and_bounds_retained_external_work() {
    const CALLERS: usize = 8;
    let store = Arc::new(Store::new(1024));
    let gate = store.admission.lock().unwrap();
    let start = Arc::new(Barrier::new(CALLERS + 1));
    let release = Arc::new(Barrier::new(2));
    let prepared = Arc::new(AtomicUsize::new(0));
    let live = Arc::new(AtomicUsize::new(0));
    let maximum = Arc::new(AtomicUsize::new(0));
    let (entered, observed) = mpsc::channel();
    let mut threads = Vec::new();
    for _ in 0..CALLERS {
        let (store, start, release, prepared, live, maximum, entered) = (
            Arc::clone(&store),
            Arc::clone(&start),
            Arc::clone(&release),
            Arc::clone(&prepared),
            Arc::clone(&live),
            Arc::clone(&maximum),
            entered.clone(),
        );
        threads.push(std::thread::spawn(move || {
            start.wait();
            store
                .try_submit(|| {
                    prepared.fetch_add(1, Ordering::SeqCst);
                    let retained = vec![42_u8; 1024 * 1024];
                    maximum.fetch_max(
                        live.fetch_add(retained.len(), Ordering::SeqCst) + retained.len(),
                        Ordering::SeqCst,
                    );
                    move |_: &ShardExecutor| {
                        entered.send(()).unwrap();
                        release.wait();
                        live.fetch_sub(retained.len(), Ordering::SeqCst);
                        retained.len()
                    }
                })
                .unwrap()
        }));
    }
    start.wait();
    assert_eq!(
        prepared.load(Ordering::SeqCst),
        0,
        "copy ran before admission"
    );
    drop(gate);
    for expected in 1..=CALLERS {
        observed.recv_timeout(Duration::from_secs(3)).unwrap();
        assert_eq!(prepared.load(Ordering::SeqCst), expected);
        assert_eq!(live.load(Ordering::SeqCst), 1024 * 1024);
        release.wait();
    }
    for thread in threads {
        assert_eq!(thread.join().unwrap(), 1024 * 1024);
    }
    assert_eq!(maximum.load(Ordering::SeqCst), 1024 * 1024);
    assert_eq!(live.load(Ordering::SeqCst), 0);
}

#[test]
fn closed_local_owner_reports_failure_without_panicking_or_stranding_calls() {
    let store = Store::new(1024);
    let (local, inbox) = store.install_local().unwrap();
    drop(inbox);
    drop(local);
    assert_eq!(
        store.try_submit(|| |_: &ShardExecutor| ()),
        Err(BridgeError::Stopped)
    );
    let mut reply = b"prefix".to_vec();
    assert_eq!(
        store.execute(&[b"GET", b"key"], &mut reply),
        Action::Continue
    );
    assert_eq!(reply, b"prefix-ERR internal store error\r\n");
    assert!(store.shutdown_requested());
    assert_eq!(store.watch_snapshot(&[b"key"]), None);
    assert_eq!(
        store.blocking_pop(&[b"key"], true),
        PopOutcome::JournalError
    );
    assert!(store.journal_flush().is_err());
}

#[test]
fn mixed_frontend_compatibility_owner_outlives_plain_exit_and_inflight_tls_work() {
    let store = Arc::new(Store::new(1024));
    store.ensure_external_owner();
    assert!(store.install_local().is_none());
    let services = Arc::clone(&store.services);
    let plain = Arc::clone(&store);
    let tls = Arc::clone(&store);
    let (entered, observed) = mpsc::sync_channel(1);
    let release = Arc::new(Barrier::new(2));
    let released = Arc::clone(&release);
    let tls_worker = std::thread::spawn(move || {
        tls.try_submit(|| {
            move |executor: &ShardExecutor| {
                entered.send(()).unwrap();
                released.wait();
                let mut output = Vec::new();
                executor.execute(&[b"SET", b"tls", b"complete"], &mut output);
                output
            }
        })
        .unwrap()
    });
    observed.recv_timeout(Duration::from_secs(3)).unwrap();
    drop(plain);
    drop(store); // Plain exit and supervisor release race admitted TLS work.
    assert_eq!(services.owner_events.lock().unwrap().len(), 1);
    release.wait();
    assert_eq!(tls_worker.join().unwrap(), b"+OK\r\n");
    let events = services.owner_events.lock().unwrap();
    assert_eq!(events.len(), 2, "last frontend joined the owner");
    assert_eq!(events[0].1, events[1].1);
}

#[test]
fn owned_argument_frame_preserves_empty_binary_and_large_arguments() {
    let large = vec![0xff; 1024 * 1024];
    let arguments: &[&[u8]] = &[b"", b"\0\xff\r\n", &large, b""];
    let frame = OwnedArguments::copy(arguments);
    assert_eq!(frame.borrowed(), arguments);
    assert_eq!(frame.bytes.len(), 4 + large.len());
}

#[test]
fn external_owner_panic_is_fail_stop_and_public_api_returns_internal_errors() {
    let store = Store::new(1024);
    assert_eq!(
        store.try_submit(|| |_: &ShardExecutor| -> () { panic!("injected owner failure") }),
        Err(BridgeError::Stopped)
    );
    let mut reply = Vec::new();
    store.execute(&[b"SET", b"key", b"value"], &mut reply);
    assert_eq!(reply, b"-ERR internal store error\r\n");
    assert!(store.shutdown_requested());
    assert!(store.owner_status().is_err());
}

#[test]
fn configuration_publication_reclaims_old_lifecycle_and_applies_eviction_before_budget_change() {
    let store = Store::new(1024);
    let (local, _inbox) = store.install_local().unwrap();
    let mut output = Vec::new();
    local.execute(&[b"SET", b"key", b"value"], &mut output);
    {
        let mut data = local.data.borrow_mut();
        data.set_expiry(b"key", Some(0));
        data.expire_key(b"key", 1);
    }
    // Exercise clearing via the owner's epoch even when flags are unchanged.
    local.set_notify_flags(NotifyFlags::default());
    local.refresh_data_config();
    assert_eq!(
        local.data.borrow_mut().drain_lifecycle(),
        (Vec::new(), Vec::new())
    );
    output.clear();
    local.execute(&[b"SET", b"key", b"value"], &mut output);
    local.set_eviction_policy(EvictionPolicy::AllKeysRandom);
    assert!(local.set_payload_limit(1));
    assert!(local.data.borrow().entries.is_empty());
}

#[test]
fn aclfile_keeps_compatibility_execution_and_genpass_is_outside_data_borrows() {
    let path = std::env::temp_dir().join(format!(
        "rillcache-owner-acl-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::write(&path, b"user default on nopass ~* &* +@all\n").unwrap();
    let store = Store::with_auth(1024, Auth::from_acl_file(&path).unwrap());
    assert!(store.install_local().is_none());
    let mut session = crate::session::Session::new();
    for args in [
        vec![b"ACL".as_slice(), b"SAVE"],
        vec![b"ACL".as_slice(), b"LOAD"],
    ] {
        let mut output = Vec::new();
        session.execute(&store, &args, &mut output);
        assert_eq!(output, b"+OK\r\n");
    }
    let services = Arc::clone(&store.services);
    let mut output = Vec::new();
    store.execute(&[b"SET", b"key", b"value"], &mut output);
    assert_eq!(output, b"+OK\r\n");
    drop(store);
    assert_eq!(services.owner_events.lock().unwrap().len(), 2);
    std::fs::remove_file(path).unwrap();

    let store = Store::new(1024);
    let (local, _inbox) = store.install_local().unwrap();
    let _data = local.data.borrow_mut();
    let mut output = Vec::new();
    session.execute_on(&local, &[b"ACL", b"GENPASS", b"256"], &mut output);
    assert_eq!(&output[..5], b"$64\r\n");
    assert!(output[5..69].iter().all(u8::is_ascii_hexdigit));
    assert_eq!(&output[69..], b"\r\n");
    assert_eq!(store.services.bridge_submissions.load(Ordering::Relaxed), 0);
}
