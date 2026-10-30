use super::*;

fn services() -> Arc<ServerServices> {
    Arc::new(ServerServices::with_clock_and_auth(
        4 * 1024 * 1024,
        Arc::new(super::super::SystemClock),
        super::super::Auth::default(),
    ))
}

fn router(owners: usize) -> Arc<MultiOwner> {
    MultiOwner::start(services(), owners, RouterSizing::for_limits(64, 64 * 1024))
        .expect("router starts")
}

fn call(router: &MultiOwner, args: &[&[u8]]) -> Vec<u8> {
    let mut output = Vec::new();
    let action = router.execute_blocking(args, &mut output);
    assert_eq!(action, Action::Continue, "command closed the connection");
    output
}

#[test]
fn layout_keeps_binary_keys_on_one_owner_and_stays_stable() {
    for owners in [1, 2, 3, 4, 7, 64] {
        let router = router(owners);
        let layout = router.layout();
        assert_eq!(layout.owner_count(), owners);
        assert_eq!(layout.partition_count(), if owners == 1 { 1 } else { 64 });
        for key in [b"".as_slice(), b"\0\xff\r\n", b"alpha", &vec![b'k'; 4096]] {
            assert_eq!(layout.owner_for(key), layout.owner_for(key));
            assert!(layout.owner_for(key).index() < owners);
        }
        router.shutdown().unwrap();
    }
}

#[test]
fn single_owner_commands_round_trip_and_isolate_keys_by_owner() {
    let router = router(3);
    assert_eq!(call(&router, &[b"SET", b"alpha", b"1"]), b"+OK\r\n");
    assert_eq!(call(&router, &[b"GET", b"alpha"]), b"$1\r\n1\r\n");
    assert_eq!(call(&router, &[b"INCR", b"alpha"]), b":2\r\n");
    assert_eq!(call(&router, &[b"GET", b"alpha"]), b"$1\r\n2\r\n");
    router.shutdown().unwrap();
}

#[test]
fn atomic_mset_and_mget_across_owners_are_all_or_nothing() {
    let router = router(4);
    let layout = router.layout();
    // Choose keys that deliberately span every owner.
    let mut keys: Vec<Vec<u8>> = Vec::new();
    for owner in layout.owners() {
        let key = (0..100_000_u64)
            .map(|n| format!("span:{n}").into_bytes())
            .find(|key| layout.owner_for(key) == owner)
            .unwrap();
        keys.push(key);
    }
    let mut args: Vec<&[u8]> = vec![b"MSET"];
    for key in &keys {
        args.push(key);
        args.push(b"initial");
    }
    assert_eq!(call(&router, &args), b"+OK\r\n");

    let mut mget: Vec<&[u8]> = vec![b"MGET"];
    mget.extend(keys.iter().map(Vec::as_slice));
    assert_eq!(
        call(&router, &mget),
        b"*4\r\n$7\r\ninitial\r\n$7\r\ninitial\r\n$7\r\ninitial\r\n$7\r\ninitial\r\n"
    );

    // A malformed MSET must not commit any pair.
    let mut bad: Vec<&[u8]> = vec![b"MSET"];
    for key in &keys {
        bad.push(key);
    }
    bad.push(b"dangling");
    assert!(call(&router, &bad).starts_with(b"-ERR"));
    assert_eq!(
        call(&router, &mget),
        b"*4\r\n$7\r\ninitial\r\n$7\r\ninitial\r\n$7\r\ninitial\r\n$7\r\ninitial\r\n"
    );
    router.shutdown().unwrap();
}

#[test]
fn rename_across_owners_is_atomic_and_visible_once() {
    let router = router(2);
    let layout = router.layout();
    let source = b"rename:source";
    let destination = (0..100_000_u64)
        .map(|n| format!("rename:{n}").into_bytes())
        .find(|key| layout.owner_for(key) != layout.owner_for(source))
        .unwrap();
    assert_eq!(call(&router, &[b"SET", source, b"value"]), b"+OK\r\n");
    assert_eq!(call(&router, &[b"DEL", &destination]), b":0\r\n");
    assert_eq!(
        call(&router, &[b"RENAME", source, &destination]),
        b"+OK\r\n"
    );
    assert_eq!(call(&router, &[b"GET", source]), b"$-1\r\n");
    assert_eq!(call(&router, &[b"GET", &destination]), b"$5\r\nvalue\r\n");
    router.shutdown().unwrap();
}

#[test]
fn duplicate_keys_keep_argument_order_and_counts() {
    let router = router(3);
    let first = b"dup:a";
    let second = b"dup:b";
    assert_eq!(
        call(
            &router,
            &[b"MSET", first, b"first", second, b"other", first, b"last"]
        ),
        b"+OK\r\n"
    );
    assert_eq!(
        call(&router, &[b"MGET", second, first, first, b"dup:absent"]),
        b"*4\r\n$5\r\nother\r\n$4\r\nlast\r\n$4\r\nlast\r\n$-1\r\n"
    );
    assert_eq!(call(&router, &[b"EXISTS", first, first, second]), b":3\r\n");
    assert_eq!(call(&router, &[b"DEL", first, first, second]), b":2\r\n");
    router.shutdown().unwrap();
}

#[test]
fn same_value_write_invalidates_a_watch_across_owners() {
    let router = router(2);
    let layout = router.layout();
    let watched = b"watch:key";
    let other = (0..100_000_u64)
        .map(|n| format!("watch:{n}").into_bytes())
        .find(|key| layout.owner_for(key) != layout.owner_for(watched))
        .unwrap();
    assert_eq!(call(&router, &[b"SET", watched, b"same"]), b"+OK\r\n");
    let snapshot = router
        .watch_snapshot_blocking(&[watched.as_slice()])
        .unwrap();
    assert_eq!(call(&router, &[b"SET", watched, b"same"]), b"+OK\r\n");
    let mut output = Vec::new();
    let outcome = router.execute_transaction_blocking(
        &[vec![b"INCR".to_vec(), other.clone()]],
        &snapshot,
        &mut output,
    );
    assert_eq!(outcome, TransactionOutcome::Aborted);
    assert_eq!(output, b"*-1\r\n");
    router.shutdown().unwrap();
}

#[test]
fn exec_runtime_error_is_not_a_rollback_across_owners() {
    let router = router(2);
    let layout = router.layout();
    let first = b"exec:a";
    let second = (0..100_000_u64)
        .map(|n| format!("exec:{n}").into_bytes())
        .find(|key| layout.owner_for(key) != layout.owner_for(first))
        .unwrap();
    let commands = vec![
        vec![b"SET".to_vec(), first.to_vec(), b"not-an-integer".to_vec()],
        vec![b"INCR".to_vec(), first.to_vec()],
        vec![b"SET".to_vec(), second.clone(), b"after-error".to_vec()],
    ];
    let mut output = Vec::new();
    let outcome =
        router.execute_transaction_blocking(&commands, &WatchState::default(), &mut output);
    assert_eq!(outcome, TransactionOutcome::Completed(Action::Continue));
    assert_eq!(
        output,
        b"*3\r\n+OK\r\n-ERR value is not an integer or out of range\r\n+OK\r\n"
    );
    assert_eq!(
        call(&router, &[b"GET", first]),
        b"$14\r\nnot-an-integer\r\n"
    );
    assert_eq!(call(&router, &[b"GET", &second]), b"$11\r\nafter-error\r\n");
    router.shutdown().unwrap();
}

#[test]
fn blocking_pop_wakes_across_owners_and_disconnect_reclaims_registration() {
    let router = router(2);
    let layout = router.layout();
    let a = b"block:a";
    let b = (0..100_000_u64)
        .map(|n| format!("block:{n}").into_bytes())
        .find(|key| layout.owner_for(key) != layout.owner_for(a))
        .unwrap();
    let keys = [a.as_slice(), b.as_slice()];
    let outcome = futures_lite_block_on(router.blocking_pop(&keys, true));
    assert_eq!(outcome, super::super::PopOutcome::Empty);
    assert_eq!(call(&router, &[b"LPUSH", &b, b"wakeup"]), b":1\r\n");
    let outcome = futures_lite_block_on(router.blocking_pop(&keys, true));
    match outcome {
        super::super::PopOutcome::Value { key, value } => {
            assert_eq!(key, b);
            assert_eq!(value, b"wakeup");
        }
        other => panic!("unexpected outcome: {other:?}"),
    }
    assert_eq!(call(&router, &[b"LLEN", &b]), b":0\r\n");
    router.shutdown().unwrap();
}

#[test]
fn global_commands_cover_every_owner() {
    let router = router(4);
    for index in 0..64 {
        let key = format!("global:{index}");
        assert_eq!(call(&router, &[b"SET", key.as_bytes(), b"v"]), b"+OK\r\n");
    }
    assert_eq!(call(&router, &[b"DBSIZE"]), b":64\r\n");
    let keys = call(&router, &[b"KEYS", b"global:*"]);
    let mut parsed = parse_bulk_array(&keys).unwrap();
    parsed.sort_unstable();
    assert_eq!(parsed.len(), 64);
    assert_eq!(call(&router, &[b"FLUSHALL"]), b"+OK\r\n");
    assert_eq!(call(&router, &[b"DBSIZE"]), b":0\r\n");
    router.shutdown().unwrap();
}

#[test]
fn scan_walks_every_owner_and_terminates_at_zero() {
    let router = router(4);
    for index in 0..100 {
        let key = format!("scan:{index}");
        assert_eq!(call(&router, &[b"SET", key.as_bytes(), b"v"]), b"+OK\r\n");
    }
    let mut cursor = b"0".to_vec();
    let mut seen = 0usize;
    for _ in 0..10_000 {
        let reply = call(&router, &[b"SCAN", &cursor, b"COUNT", b"1000"]);
        let (next, keys) = parse_scan_reply(&reply).expect("scan reply");
        seen += keys.len();
        cursor = next.to_string().into_bytes();
        if next == 0 {
            break;
        }
    }
    assert_eq!(seen, 100);
    assert_eq!(cursor, b"0");
    router.shutdown().unwrap();
}

#[test]
fn shutdown_joins_every_owner_thread_once() {
    let router = router(4);
    router.shutdown().unwrap();
    router.shutdown().unwrap();
    assert!(router.stopped.load(Ordering::Acquire));
    for link in &router.owners {
        assert!(link.thread.lock().unwrap().is_none());
    }
}

fn futures_lite_block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(future)
}
