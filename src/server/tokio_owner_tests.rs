use super::*;

fn context() -> (Arc<Context>, mpsc::Receiver<Job>, mpsc::Receiver<ColdJob>) {
    let (sender, receive) = channel();
    let (cold, cold_receive) = mpsc::channel(1);
    (
        Arc::new(Context {
            config: Config::default(),
            store: Arc::new(Store::new(4 * 1024 * 1024)),
            stopping: Arc::new(AtomicBool::new(false)),
            failed: Arc::new(AtomicBool::new(false)),
            stop_replies: Arc::new(AtomicUsize::new(0)),
            controls: Arc::new(Mutex::new(HashMap::new())),
            cold: ColdExecutor {
                sender: cold,
                permit: Arc::new(Semaphore::new(1)),
            },
            owner: Some(sender),
            task_limit: 128,
        }),
        receive,
        cold_receive,
    )
}

fn run(future: impl std::future::Future<Output = ()>) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    tokio::task::LocalSet::new().block_on(&runtime, future);
}

#[test]
fn a_pipeline_is_one_owned_turn_preserving_session_and_buffer_capacity() {
    run(async {
        let (context, mut receive, _cold) = context();
        let (executor, _bridge) = context.store.install_local().unwrap();
        let client = Arc::clone(&context);
        let task = tokio::task::spawn_local(async move {
            let mut session = SessionSlot::new(Session::new());
            let id = session.id();
            let mut input = b"PING\r\n".repeat(16);
            let mut output = Vec::with_capacity(8192);
            let outcome = client
                .owner
                .as_ref()
                .unwrap()
                .run(
                    &mut session,
                    &mut input,
                    &mut output,
                    &client,
                    Operation::Batch,
                )
                .await
                .unwrap();
            assert_eq!(session.id(), id);
            assert_eq!(outcome, Outcome::Read);
            assert!(input.is_empty());
            assert_eq!(output, b"+PONG\r\n".repeat(16));
            assert!(output.capacity() >= 8192);
        });
        let deadline = Instant::now() + Duration::from_secs(3);
        while receive.is_empty() {
            assert!(Instant::now() < deadline);
            tokio::task::yield_now().await;
        }
        assert_eq!(
            receive.len(),
            1,
            "pipeline entries must not be separate IPC requests"
        );
        receive.try_recv().unwrap()(&executor);
        task.await.unwrap();
        assert_eq!(
            context.owner.as_ref().unwrap().permit.available_permits(),
            CAPACITY
        );
        assert_eq!(context.store.test_services().bridge_count(), 0);
    });
}

#[test]
fn cancelling_a_waiter_retains_credit_until_queued_work_is_dropped() {
    run(async {
        let (context, receive, _cold) = context();
        let mut tasks = Vec::new();
        for _ in 0..=CAPACITY {
            let client = Arc::clone(&context);
            tasks.push(tokio::task::spawn_local(async move {
                let mut session = SessionSlot::new(Session::new());
                let mut input = b"PING\r\n".to_vec();
                let mut output = Vec::new();
                client
                    .owner
                    .as_ref()
                    .unwrap()
                    .run(
                        &mut session,
                        &mut input,
                        &mut output,
                        &client,
                        Operation::Batch,
                    )
                    .await
            }));
            if tasks.len() == 1 {
                let deadline = Instant::now() + Duration::from_secs(3);
                while receive.is_empty() {
                    assert!(Instant::now() < deadline);
                    tokio::task::yield_now().await;
                }
            }
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        while receive.len() < CAPACITY {
            assert!(Instant::now() < deadline);
            tokio::task::yield_now().await;
        }
        let permit = &context.owner.as_ref().unwrap().permit;
        assert_eq!(permit.available_permits(), 0);
        let cancelled = tasks.remove(0);
        cancelled.abort();
        assert!(cancelled.await.unwrap_err().is_cancelled());
        assert_eq!(receive.len(), CAPACITY);
        assert_eq!(
            permit.available_permits(),
            0,
            "cancelled reply does not release admitted bytes early"
        );
        drop(receive);
        for task in tasks {
            assert!(task.await.unwrap().is_err());
        }
        assert_eq!(permit.available_permits(), CAPACITY);
    });
}

#[test]
fn a_cancelled_queued_turn_does_not_execute_later_data_side_effects() {
    run(async {
        let (context, mut receive, _cold) = context();
        let (executor, _bridge) = context.store.install_local().unwrap();
        let client = Arc::clone(&context);
        let task = tokio::task::spawn_local(async move {
            let mut session = SessionSlot::new(Session::new());
            let mut input = b"INCR cancelled\r\n".to_vec();
            let mut output = Vec::new();
            client
                .owner
                .as_ref()
                .unwrap()
                .run(
                    &mut session,
                    &mut input,
                    &mut output,
                    &client,
                    Operation::Batch,
                )
                .await
        });
        let deadline = Instant::now() + Duration::from_secs(3);
        while receive.is_empty() {
            assert!(Instant::now() < deadline);
            tokio::task::yield_now().await;
        }
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        receive.try_recv().unwrap()(&executor);
        let mut output = Vec::new();
        executor.execute(&[b"GET", b"cancelled"], &mut output);
        assert_eq!(
            output, b"$-1\r\n",
            "revoked queued request performed a data write"
        );
        assert_eq!(
            context.owner.as_ref().unwrap().permit.available_permits(),
            CAPACITY
        );
    });
}

#[test]
fn protocol_reply_ownership_survives_delayed_remote_scheduling() {
    run(async {
        let (context, mut receive, _cold) = context();
        let (executor, _bridge) = context.store.install_local().unwrap();
        let client = Arc::clone(&context);
        let (entered_send, entered) = oneshot::channel();
        let (release_send, release) = oneshot::channel();
        let task = tokio::task::spawn_local(async move {
            let mut session = SessionSlot::new(Session::new());
            let mut input = b"PING\r\nSHUTDOWN\r\n".to_vec();
            let mut output = Vec::new();
            let outcome = client
                .owner
                .as_ref()
                .unwrap()
                .run(
                    &mut session,
                    &mut input,
                    &mut output,
                    &client,
                    Operation::Batch,
                )
                .await
                .unwrap();
            assert_eq!(outcome, Outcome::Close);
            assert_eq!(output, b"+PONG\r\n");
            entered_send.send(()).unwrap();
            release.await.unwrap();
            drop(session);
        });
        let deadline = Instant::now() + Duration::from_secs(3);
        while receive.is_empty() {
            assert!(Instant::now() < deadline);
            tokio::task::yield_now().await;
        }
        receive.try_recv().unwrap()(&executor);
        entered.await.unwrap();
        // Deliberately model a scheduling delay greater than the former grace,
        // not a sleep used for startup/readiness or a throughput measurement.
        tokio::time::sleep(POLL_TICK * 3).await;
        assert!(context.store.shutdown_requested());
        assert_eq!(context.stop_replies.load(Ordering::Acquire), 1);
        release_send.send(()).unwrap();
        task.await.unwrap();
        assert_eq!(context.stop_replies.load(Ordering::Acquire), 0);
    });
}

#[test]
fn batch_quanta_and_atomic_reply_backpressure_preserve_buffered_side_effects() {
    let store = Store::new(4 * 1024 * 1024);
    let (executor, _bridge) = store.install_local().unwrap();
    let mut session = Session::new();
    let mut input = b"INCR counter\r\n".repeat(COMMAND_QUANTUM + 1);
    let mut output = Vec::new();
    let stopping = AtomicBool::new(false);
    assert_eq!(
        batch(
            &mut session,
            &mut input,
            &mut output,
            Limits::default(),
            &executor,
            &stopping
        ),
        Outcome::Buffered
    );
    assert_eq!(input, b"INCR counter\r\n");
    assert!(output.ends_with(b":64\r\n"));
    output.clear();
    let mut setup = Vec::new();
    executor.execute(
        &[b"SET", b"large", &vec![b'x'; OUTPUT_QUANTUM + 1]],
        &mut setup,
    );
    input = b"GET large\r\nINCR after-reply\r\n".to_vec();
    assert_eq!(
        batch(
            &mut session,
            &mut input,
            &mut output,
            Limits::default(),
            &executor,
            &stopping
        ),
        Outcome::Buffered
    );
    assert!(output.len() > OUTPUT_QUANTUM);
    assert_eq!(input, b"INCR after-reply\r\n");
    let mut missing = Vec::new();
    executor.execute(&[b"GET", b"after-reply"], &mut missing);
    assert_eq!(missing, b"$-1\r\n");
}
