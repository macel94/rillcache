use super::*;

static READ_WAITS: AtomicUsize = AtomicUsize::new(0);
static READ_PENDING_WAITS: AtomicUsize = AtomicUsize::new(0);
// Manual diagnostic only: no counters/wrapper in production builds. A pending
// read causes Timeout to poll Sleep, but cooperative budget exhaustion means
// this is an upper bound on registrations, NOT an exact timer-driver counter.
pub(super) async fn observe_read_wait<F: std::future::Future>(future: F) -> F::Output {
    READ_WAITS.fetch_add(1, Ordering::Relaxed);
    let mut future = std::pin::pin!(future);
    let mut observed_pending = false;
    std::future::poll_fn(|cx| {
        let result = future.as_mut().poll(cx);
        if result.is_pending() && !observed_pending {
            observed_pending = true;
            READ_PENDING_WAITS.fetch_add(1, Ordering::Relaxed);
        }
        result
    })
    .await
}

#[test]
#[ignore = "manual mechanism diagnostic; not a throughput benchmark"]
fn diagnose_read_timeout_construction_ping_p1_and_p16() {
    const CLIENTS: usize = 8;
    const PER_CLIENT: usize = 8192;
    for pipeline in [1, 16] {
        let mut server = TestServer::start(1, 128);
        let ready = Arc::new(std::sync::Barrier::new(CLIENTS + 1));
        let start = Arc::new(std::sync::Barrier::new(CLIENTS + 1));
        let mut clients = Vec::new();
        for _ in 0..CLIENTS {
            let mut stream = server.connect();
            let ready = Arc::clone(&ready);
            let start = Arc::clone(&start);
            clients.push(thread::spawn(move || {
                ping(&mut stream);
                ready.wait();
                start.wait();
                let request = b"*1\r\n$4\r\nPING\r\n".repeat(pipeline);
                let expected = b"+PONG\r\n".repeat(pipeline);
                let mut actual = vec![0; expected.len()];
                for _ in 0..PER_CLIENT / pipeline {
                    stream.write_all(&request).unwrap();
                    stream.read_exact(&mut actual).unwrap();
                    assert_eq!(actual, expected);
                }
                stream // keep alive until after counter snapshot
            }));
        }
        ready.wait();
        let waits_before = READ_WAITS.load(Ordering::Relaxed);
        let pending_before = READ_PENDING_WAITS.load(Ordering::Relaxed);
        start.wait();
        let streams: Vec<_> = clients
            .into_iter()
            .map(|client| client.join().unwrap())
            .collect();
        let waits = READ_WAITS.load(Ordering::Relaxed) - waits_before;
        let pending = READ_PENDING_WAITS.load(Ordering::Relaxed) - pending_before;
        println!(
            "READ_DIAGNOSTIC {{\"pipeline\":{pipeline},\"commands\":{},\"read_timeout_constructions\":{waits},\"unique_pending_read_waits\":{pending},\"scope\":\"test-instrumented counts, not throughput or exact timer registrations\"}}",
            CLIENTS * PER_CLIENT
        );
        server.shutdown();
        drop(streams);
    }
}

struct TestServer {
    addr: SocketAddr,
    store: Arc<Store>,
    stopping: Arc<AtomicBool>,
    done: Receiver<io::Result<()>>,
    thread: Option<JoinHandle<()>>,
}

impl TestServer {
    fn start(workers: usize, task_limit: usize) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let store = Arc::new(Store::new(64 * 1024 * 1024));
        let stopping = Arc::new(AtomicBool::new(false));
        let config = Config {
            bind_addr: addr,
            idle_timeout: Duration::from_secs(30),
            ..Config::default()
        };
        let (sender, done) = sync_channel(1);
        let client_store = Arc::clone(&store);
        let client_stop = Arc::clone(&stopping);
        let thread = thread::spawn(move || {
            let result = serve_with_task_limit(
                listener,
                config,
                client_store,
                client_stop,
                workers,
                task_limit,
            );
            sender.send(result).unwrap();
        });
        Self {
            addr,
            store,
            stopping,
            done,
            thread: Some(thread),
        }
    }

    fn connect(&self) -> TcpStream {
        let stream = TcpStream::connect(self.addr).unwrap();
        stream.set_nodelay(true).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream
    }

    fn wait_clients(&self, expected: usize) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while self.store.connected_clients() != expected {
            assert!(
                Instant::now() < deadline,
                "expected {expected} clients, got {}",
                self.store.connected_clients()
            );
            thread::yield_now();
        }
    }

    fn shutdown(&mut self) {
        if let Some(thread) = self.thread.take() {
            self.stopping.store(true, Ordering::Release);
            self.done
                .recv_timeout(Duration::from_secs(3))
                .expect("transport must join even with saturated admission")
                .unwrap();
            thread.join().unwrap();
            assert_eq!(self.store.connected_clients(), 0);
        }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn ping(stream: &mut TcpStream) {
    stream.write_all(b"*1\r\n$4\r\nPING\r\n").unwrap();
    pong(stream);
}

fn pong(stream: &mut TcpStream) {
    let mut reply = [0u8; 7];
    stream.read_exact(&mut reply).unwrap();
    assert_eq!(&reply, b"+PONG\r\n");
}

#[test]
fn saturated_local_admission_retains_pending_client_until_capacity_returns() {
    let mut server = TestServer::start(1, 1);
    let mut active = server.connect();
    ping(&mut active);
    let mut pending = server.connect();
    pending.write_all(b"*1\r\n$4\r\nPING\r\n").unwrap();
    server.wait_clients(2); // one active task, one admitted but not routed
    for _ in 0..16 {
        ping(&mut active); // pending admission must not stall existing tasks
    }
    drop(active);
    pong(&mut pending); // the original code dropped this connection after join
    ping(&mut pending);
    server.shutdown();
}

#[test]
fn saturated_local_admission_stops_without_waiting_for_an_idle_client() {
    let mut server = TestServer::start(1, 1);
    let mut active = server.connect();
    ping(&mut active);
    let _pending = server.connect();
    server.wait_clients(2);
    ping(&mut active);
    server.shutdown(); // no client has disconnected to make join_next complete
    assert_eq!(active.read(&mut [0]).unwrap(), 0);
}

#[test]
fn saturated_remote_queue_and_local_tasks_keep_progress_and_join_every_registration() {
    let mut server = TestServer::start(2, 1);
    let mut local = server.connect();
    ping(&mut local);
    let mut remote = server.connect();
    ping(&mut remote);
    let mut queued = Vec::new();
    for _ in 0..=ROUTE_CAPACITY {
        let mut stream = server.connect();
        stream.write_all(b"*1\r\n$4\r\nPING\r\n").unwrap();
        queued.push(stream);
    }
    server.wait_clients(ROUTE_CAPACITY + 3);
    for _ in 0..16 {
        ping(&mut local);
        ping(&mut remote);
    }
    // Releasing remote capacity must admit the FIFO head without losing the
    // separately retained pending connection. Shutdown must drop every queue.
    drop(remote);
    pong(&mut queued[0]);
    ping(&mut local);
    server.shutdown();
}

#[test]
fn slow_reply_stops_pipeline_side_effects_and_joins_on_one_runtime() {
    let mut server = TestServer::start(1, 128);
    let value = vec![b'x'; 1024 * 1024];
    let mut setup = server.connect();
    let mut request = Vec::new();
    crate::protocol::encode_request(&[b"SET", b"large", &value], &mut request);
    setup.write_all(&request).unwrap();
    let mut ok = [0; 5];
    setup.read_exact(&mut ok).unwrap();
    assert_eq!(&ok, b"+OK\r\n");
    let mut slow = server.connect();
    request.clear();
    crate::protocol::encode_request(&[b"GET", b"large"], &mut request);
    let mut pipeline = request.repeat(1024);
    crate::protocol::encode_request(&[b"SET", b"must-not-run", b"v"], &mut pipeline);
    slow.write_all(&pipeline).unwrap();
    for _ in 0..32 {
        ping(&mut setup);
    }
    request.clear();
    crate::protocol::encode_request(&[b"GET", b"must-not-run"], &mut request);
    setup.write_all(&request).unwrap();
    let mut missing = [0; 5];
    setup.read_exact(&mut missing).unwrap();
    assert_eq!(&missing, b"$-1\r\n");
    server.shutdown();
}

#[test]
fn cold_io_saturation_leaves_runtime_runnable_and_shutdown_joins_the_service() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();
    let (sender, mut receiver) = mpsc::channel::<ColdJob>(1);
    let (entered_sender, entered) = sync_channel(1);
    let (release_sender, release) = sync_channel(1);
    let service = thread::spawn(move || {
        // A controlled stand-in for a slow filesystem syscall. Never runs on
        // the network runtime, and cannot be cancelled by dropping its handle.
        entered_sender.send(()).unwrap();
        release.recv().unwrap();
        while let Some(job) = receiver.blocking_recv() {
            job();
        }
    });
    entered.recv().unwrap();
    let store = Arc::new(Store::new(1024));
    let stopping = Arc::new(AtomicBool::new(false));
    let permit = Arc::new(Semaphore::new(1));
    let context = Arc::new(Context {
        config: Config::default(),
        store: Arc::clone(&store),
        stopping: Arc::clone(&stopping),
        controls: Arc::new(Mutex::new(HashMap::new())),
        cold: ColdExecutor {
            sender,
            permit: Arc::clone(&permit),
        },
        task_limit: 128,
    });
    let (done_sender, done) = sync_channel(1);
    let network = thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = tokio::task::LocalSet::new()
            .block_on(&runtime, accept_with_storage(listener, context, &[]));
        service.join().unwrap();
        done_sender.send(result).unwrap();
    });
    let connect = || {
        let stream = TcpStream::connect(addr).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream
    };
    let mut first = connect();
    first
        .write_all(b"*2\r\n$3\r\nACL\r\n$7\r\nGENPASS\r\n")
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while permit.available_permits() != 0 {
        assert!(Instant::now() < deadline);
        thread::yield_now();
    }
    let mut second = connect();
    second
        .write_all(b"*2\r\n$3\r\nACL\r\n$7\r\nGENPASS\r\n")
        .unwrap();
    let mut healthy = connect();
    for _ in 0..16 {
        // GET takes Store's transaction/data gate; unlike PING it exposes
        // indirect blocking behind an offloaded operation holding that gate.
        healthy
            .write_all(b"*2\r\n$3\r\nGET\r\n$7\r\nmissing\r\n")
            .unwrap();
        let mut reply = [0; 5];
        healthy.read_exact(&mut reply).unwrap();
        assert_eq!(&reply, b"$-1\r\n");
    }
    stopping.store(true, Ordering::Release);
    // Stop is pending on the real admitted service work, not detached/aborted.
    assert!(done.try_recv().is_err());
    release_sender.send(()).unwrap();
    done.recv_timeout(Duration::from_secs(3)).unwrap().unwrap();
    network.join().unwrap();
    assert_eq!(store.connected_clients(), 0);
    assert_eq!(permit.available_permits(), 1);
}

#[test]
fn panic_publishes_stop_and_closes_controls_before_task_reaping() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let (control, _) = listener.accept().unwrap();
    let controls = Arc::new(Mutex::new(HashMap::from([(1, control)])));
    let stopping = Arc::new(AtomicBool::new(false));
    let guard = FailureStop {
        stopping: Arc::clone(&stopping),
        controls,
    };
    let result = std::panic::catch_unwind(move || {
        let _guard = guard;
        panic!("injected owner task failure");
    });
    assert!(result.is_err());
    assert!(stopping.load(Ordering::Acquire));
    assert_eq!(peer.read(&mut [0]).unwrap(), 0);
}

#[test]
fn a_panicked_task_does_not_skip_joining_other_tasks() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let completed = Arc::new(AtomicBool::new(false));
    runtime.block_on(async {
        let mut tasks = Tasks::new();
        tasks.spawn(async {
            panic!("injected client panic");
            #[allow(unreachable_code)]
            Ok(())
        });
        let mark = Arc::clone(&completed);
        tasks.spawn(async move {
            tokio::task::yield_now().await;
            mark.store(true, Ordering::Release);
            Ok(())
        });
        assert!(join_tasks(&mut tasks).await.is_err());
        assert!(tasks.is_empty());
    });
    assert!(completed.load(Ordering::Acquire));
}

#[test]
fn cold_command_classification_keeps_exec_local_and_binary_case_safe_acl_cold() {
    assert!(cold_command(&[b"aCl", b"GENPASS"]));
    for args in [
        vec![],
        vec![b"PING".as_slice()],
        vec![b"EXEC".as_slice()],
        vec![b"ACL\0".as_slice()],
    ] {
        assert!(!cold_command(&args));
    }
}

#[test]
fn acl_filesystem_commands_remain_rejected_inside_exec_on_one_runtime() {
    let mut server = TestServer::start(1, 128);
    let mut transaction = server.connect();
    let mut healthy = server.connect();
    for subcommand in [b"SAVE".as_slice(), b"LOAD", b"GENPASS"] {
        transaction.write_all(b"*1\r\n$5\r\nMULTI\r\n").unwrap();
        let mut ok = [0; 5];
        transaction.read_exact(&mut ok).unwrap();
        assert_eq!(&ok, b"+OK\r\n");
        let mut request = Vec::new();
        crate::protocol::encode_request(&[b"ACL", subcommand], &mut request);
        transaction.write_all(&request).unwrap();
        let expected = b"-ERR session commands are not supported inside MULTI\r\n";
        let mut actual = vec![0; expected.len()];
        transaction.read_exact(&mut actual).unwrap();
        assert_eq!(actual, expected);
        transaction.write_all(b"*1\r\n$4\r\nEXEC\r\n").unwrap();
        let expected = b"-EXECABORT Transaction discarded because of previous errors.\r\n";
        let mut actual = vec![0; expected.len()];
        transaction.read_exact(&mut actual).unwrap();
        assert_eq!(actual, expected);
        healthy
            .write_all(b"*2\r\n$3\r\nGET\r\n$7\r\nmissing\r\n")
            .unwrap();
        let mut missing = [0; 5];
        healthy.read_exact(&mut missing).unwrap();
        assert_eq!(&missing, b"$-1\r\n");
    }
    server.shutdown();
}
