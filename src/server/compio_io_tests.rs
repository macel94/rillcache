use super::*;

fn native_required() -> bool {
    std::env::var("RILLCACHE_REQUIRE_COMPIO").as_deref() == Ok("1")
        || std::env::var("RILLCACHE_TEST_THREADS").as_deref() == Ok("compio")
}

fn kernel_available() -> bool {
    let supported = available();
    assert!(
        supported || !native_required(),
        "native Compio driver is required"
    );
    if !supported {
        eprintln!("Compio kernel test unavailable; automatic Mio fallback remains covered");
    }
    supported
}

fn require_or_skip() -> Option<Runtime> {
    match Runtime::new() {
        Ok(runtime) => {
            assert!(runtime.driver_type().is_iouring());
            Some(runtime)
        }
        Err(error) => {
            assert!(
                !native_required(),
                "required native Compio driver is unavailable: {error}"
            );
            eprintln!(
                "Compio kernel test unavailable: {error}; automatic Mio fallback remains covered"
            );
            None
        }
    }
}

#[test]
fn availability_probe_preserves_calling_thread_affinity() {
    let before = core_affinity::get_core_ids();
    let enabled = available();
    assert_eq!(before, core_affinity::get_core_ids());
    eprintln!("Compio io_uring available: {enabled}");
}

#[test]
fn availability_probe_does_not_interrupt_unrelated_caller_socket_io() {
    if !available() {
        return;
    }
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let (mut socket, _) = listener.accept().unwrap();
    // Probe after setup so no other syscall drains a delayed notification from
    // a temporary ring before this unrelated blocking read.
    assert!(available());
    let writer = thread::spawn(move || {
        thread::sleep(Duration::from_millis(250));
        socket.write_all(b"x").unwrap();
    });
    let result = peer.read(&mut [0]);
    writer.join().unwrap();
    assert_eq!(result.unwrap(), 1);
}

#[test]
fn selection_is_explicit_and_unsupported_auto_configuration_keeps_fallback() {
    let config = Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        ..Config::default()
    };
    let auto = Server::bind(config.clone()).unwrap();
    let supported = available();
    assert_eq!(auto.policy.completion_transport, supported);
    assert_eq!(auto.policy.readiness_io, !supported);
    let explicit = Server::bind(Config {
        io_mode: IoMode::Mio,
        ..config.clone()
    })
    .unwrap();
    assert!(!explicit.policy.completion_transport);
    assert!(explicit.policy.readiness_io);
    let fixed = Config {
        worker_threads: Some(1),
        ..config.clone()
    };
    assert!(!completion_configuration_eligible(&fixed));
    assert!(
        !Server::bind(fixed.clone())
            .unwrap()
            .policy
            .completion_transport
    );
    assert!(
        matches!(Server::bind(Config { io_mode: IoMode::Compio, ..fixed }),
        Err(error) if error.kind() == io::ErrorKind::Unsupported)
    );
    for config in [
        Config {
            appendonly: true,
            ..config.clone()
        },
        Config {
            storage_owners: Some(2),
            ..config.clone()
        },
        Config {
            aclfile: Some("not-opened".into()),
            ..config.clone()
        },
        Config {
            audit_log: Some("not-opened".into()),
            ..config.clone()
        },
        Config {
            tls_bind_addr: Some("127.0.0.1:0".parse().unwrap()),
            ..config
        },
    ] {
        assert!(!completion_configuration_eligible(&config));
    }
}

#[test]
fn cancelled_owned_read_returns_the_initialized_prefix_before_close() {
    let Some(runtime) = require_or_skip() else {
        return;
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let _peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (socket, _) = listener.accept().unwrap();
    runtime.block_on(async move {
        let stream = CompletionStream::from_std(socket).unwrap();
        let cancel = CancelToken::new();
        let token = cancel.clone();
        let task = compio::runtime::spawn(async move {
            let mut input = b"retained-prefix".to_vec();
            let begin = input.len();
            input.reserve(MAX_READ);
            let mut reader = &stream;
            let BufResult(result, returned) = reader
                .read(compio::buf::IoBuf::slice(input, begin..begin + MAX_READ))
                .with_cancel(token)
                .await;
            assert_eq!(result.unwrap_err().raw_os_error(), Some(125));
            assert_eq!(returned.into_inner(), b"retained-prefix");
            stream.close().await.unwrap();
        });
        compio::time::sleep(CONTROL_TICK).await;
        cancel.cancel();
        compio::time::timeout(Duration::from_secs(3), task)
            .await
            .expect("cancelled read and socket close must finish")
            .unwrap();
    });
}

#[test]
fn control_ticks_retain_the_same_accept_until_a_peer_arrives() {
    let Some(runtime) = require_or_skip() else {
        return;
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    runtime.block_on(async move {
        let listener = CompletionListener::from_std(listener).unwrap();
        let (stream, mut peer) = {
            let mut accepting = pin!(listener.accept());
            for _ in 0..3 {
                assert!(
                    ready_or_tick(accepting.as_mut(), CONTROL_TICK)
                        .await
                        .is_none()
                );
            }
            let mut peer = TcpStream::connect(address).unwrap();
            peer.write_all(b"retained-accept").unwrap();
            let (stream, _) = ready_or_tick(accepting.as_mut(), Duration::from_secs(3))
                .await
                .expect("original accept must still deliver this peer")
                .unwrap();
            (stream, peer)
        };
        let mut socket = &stream;
        let BufResult(result, input) =
            compio::io::AsyncReadExt::read_exact(&mut socket, Vec::with_capacity(15)).await;
        result.unwrap();
        assert_eq!(input, b"retained-accept");
        stream.close().await.unwrap();
        listener.close().await.unwrap();
        assert_eq!(peer.read(&mut [0]).unwrap(), 0);
    });
}

#[test]
fn panic_guard_publishes_fail_stop_and_closes_owned_controls() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let (socket, _) = listener.accept().unwrap();
    let controls = Arc::new(Mutex::new(HashMap::from([(1, socket)])));
    let stopping = Arc::new(AtomicBool::new(false));
    let failed = Arc::new(AtomicBool::new(false));
    let guard = FailureStop {
        stopping: Arc::clone(&stopping),
        failed: Arc::clone(&failed),
        controls,
    };
    assert!(
        thread::spawn(move || {
            let _failure = guard;
            panic!("controlled completion task failure");
        })
        .join()
        .is_err()
    );
    assert!(failed.load(Ordering::Acquire));
    assert!(stopping.load(Ordering::Acquire));
    assert_eq!(peer.read(&mut [0]).unwrap(), 0);
}

#[test]
fn cold_service_saturation_keeps_network_runnable_and_bridge_alive_through_join() {
    if !kernel_available() {
        return;
    }
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let store = Arc::new(Store::new(1024 * 1024));
    let stopping = Arc::new(AtomicBool::new(false));
    let network_store = Arc::clone(&store);
    let cold_store = Arc::clone(&store);
    let network_stop = Arc::clone(&stopping);
    let (entered_send, entered) = sync_channel(1);
    let (release_send, release) = sync_channel(1);
    let (bridge_send, bridge_reply) = sync_channel(1);
    let (done_send, done) = sync_channel(1);
    let network = thread::spawn(move || {
        let result = serve_with_cold_service(
            listener,
            Config::default(),
            network_store,
            network_stop,
            move |receive| {
                if let Ok(job) = receive.recv() {
                    // The actual admitted job owns the only cold permit while this
                    // controlled stand-in blocks like a filesystem operation.
                    entered_send.send(()).unwrap();
                    release.recv().unwrap();
                    let mut output = Vec::new();
                    cold_store.execute(&[b"SET", b"cold-finished", b"v"], &mut output);
                    bridge_send.send(output).unwrap();
                    job();
                }
                while let Ok(job) = receive.recv() {
                    job();
                }
            },
        );
        done_send.send(result).unwrap();
    });
    let connect = || {
        let socket = TcpStream::connect(address).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        socket
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        socket
    };
    let mut healthy = connect();
    healthy.write_all(b"*1\r\n$4\r\nPING\r\n").unwrap();
    let mut pong = [0; 7];
    healthy.read_exact(&mut pong).unwrap();
    assert_eq!(&pong, b"+PONG\r\n");
    let mut cold = connect();
    cold.write_all(b"*2\r\n$3\r\nACL\r\n$6\r\nWHOAMI\r\n")
        .unwrap();
    entered
        .recv_timeout(Duration::from_secs(3))
        .expect("cold command must acquire bounded service credit");
    for _ in 0..8 {
        healthy.write_all(b"*1\r\n$4\r\nPING\r\n").unwrap();
        healthy.read_exact(&mut pong).unwrap();
        assert_eq!(&pong, b"+PONG\r\n");
    }
    stopping.store(true, Ordering::Release);
    let premature = done.recv_timeout(Duration::from_millis(20));
    release_send.send(()).unwrap();
    let completion = done.recv_timeout(Duration::from_secs(3));
    let bridge = bridge_reply.recv_timeout(Duration::from_secs(3));
    network.join().unwrap();
    assert!(
        matches!(premature, Err(std::sync::mpsc::RecvTimeoutError::Timeout)),
        "server finished before admitted cold work joined: {premature:?}"
    );
    completion
        .expect("socket tasks, bridge and cold service must finish")
        .unwrap();
    assert_eq!(bridge.unwrap(), b"+OK\r\n");
    assert_eq!(store.connected_clients(), 0);
}

#[test]
fn slow_completion_reply_blocks_later_side_effects_and_shutdown_joins() {
    if !kernel_available() {
        return;
    }
    let server = Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        io_mode: IoMode::Compio,
        ..Config::default()
    })
    .unwrap()
    .spawn()
    .unwrap();
    assert!(server.scheduling_policy().completion_transport);
    let mut healthy = TcpStream::connect(server.local_addr()).unwrap();
    healthy
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    healthy
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut request = Vec::new();
    crate::protocol::encode_request(&[b"SET", b"large", &vec![b'x'; 1024 * 1024]], &mut request);
    healthy.write_all(&request).unwrap();
    let mut ok = [0; 5];
    healthy.read_exact(&mut ok).unwrap();
    assert_eq!(&ok, b"+OK\r\n");
    let mut slow = TcpStream::connect(server.local_addr()).unwrap();
    slow.set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    request.clear();
    crate::protocol::encode_request(&[b"GET", b"large"], &mut request);
    let mut pipeline = request.repeat(1024);
    crate::protocol::encode_request(&[b"SET", b"must-not-run", b"v"], &mut pipeline);
    slow.write_all(&pipeline).unwrap();
    for _ in 0..16 {
        healthy.write_all(b"*1\r\n$4\r\nPING\r\n").unwrap();
        let mut pong = [0; 7];
        healthy.read_exact(&mut pong).unwrap();
        assert_eq!(&pong, b"+PONG\r\n");
    }
    healthy
        .write_all(b"*2\r\n$3\r\nGET\r\n$12\r\nmust-not-run\r\n")
        .unwrap();
    let mut missing = [0; 5];
    healthy.read_exact(&mut missing).unwrap();
    assert_eq!(&missing, b"$-1\r\n");
    let (send, done) = sync_channel(1);
    let joined = thread::spawn(move || {
        send.send(server.shutdown()).unwrap();
    });
    done.recv_timeout(Duration::from_secs(3))
        .expect("pending read/write must join on shutdown")
        .unwrap();
    joined.join().unwrap();
}
