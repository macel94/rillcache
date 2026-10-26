use super::*;

pub(super) fn pair(input: Vec<u8>) -> (Client, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    peer.set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    peer.set_nodelay(true).unwrap();
    let (stream, _) = listener.accept().unwrap();
    stream.set_nonblocking(true).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    (
        Client {
            stream,
            session: Session::new(),
            input,
            output: Vec::new(),
            sent: 0,
            closing: false,
            eof: false,
            activity: Instant::now(),
            _permit: None,
            shutdown_on_drop: true,
        },
        peer,
    )
}

pub(super) fn loan(client: Client, stopping: &Arc<AtomicBool>) -> (Loan, Receiver<Option<Client>>) {
    let (home, returned) = mpsc::channel();
    (
        Loan {
            client: Some(client),
            home,
            stopping: Arc::clone(stopping),
            coordinator: false,
        },
        returned,
    )
}

#[test]
fn loans_return_once_and_late_senders_recover_after_epoch_closure() {
    let stop = Arc::new(AtomicBool::new(false));
    let sharing = Sharing::new(Config::default(), Arc::clone(&stop), 50);
    let (sender, receiver) = mpsc::channel();
    *sharing.leader.lock().unwrap() = Some(sender.clone());
    let leadership = Leadership {
        sharing: &sharing,
        receiver,
    };
    let (client, mut peer) = pair(b"partial".to_vec());
    let (queued, returned) = loan(client, &stop);
    sender.send(queued).ok().unwrap();
    drop(leadership);
    assert!(sharing.leader.lock().unwrap().is_none());
    let mut client = returned
        .recv_timeout(Duration::from_secs(3))
        .unwrap()
        .unwrap();
    assert_eq!(client.input, b"partial");
    assert!(matches!(
        returned.try_recv(),
        Err(mpsc::TryRecvError::Disconnected)
    ));
    let (late, home) = loan(client, &stop);
    let error = sender.send(late).expect_err("epoch receiver is closed");
    drop(error);
    client = home.recv_timeout(Duration::from_secs(3)).unwrap().unwrap();
    client.stream.write_all(b"+PONG\r\n").unwrap();
    let mut bytes = [0; 7];
    peer.read_exact(&mut bytes).unwrap();
    assert_eq!(&bytes, b"+PONG\r\n");
    assert!(matches!(
        home.try_recv(),
        Err(mpsc::TryRecvError::Disconnected)
    ));
}

#[test]
fn cancelled_queued_and_active_loans_close_before_waking_owners() {
    let stop = Arc::new(AtomicBool::new(false));
    let sharing = Sharing::new(Config::default(), Arc::clone(&stop), 50);
    let (sender, receiver) = mpsc::channel();
    *sharing.leader.lock().unwrap() = Some(sender.clone());
    let (first, mut first_peer) = pair(Vec::new());
    let (second, mut second_peer) = pair(Vec::new());
    let (first, first_home) = loan(first, &stop);
    let (second, second_home) = loan(second, &stop);
    sender.send(second).ok().unwrap();
    stop.store(true, Ordering::Release);
    sharing.run(
        first,
        Leadership {
            sharing: &sharing,
            receiver,
        },
        &Store::new(1024),
    );
    for home in [first_home, second_home] {
        assert!(home.recv_timeout(Duration::from_secs(3)).unwrap().is_none());
    }
    assert_eq!(first_peer.read(&mut [0]).unwrap(), 0);
    assert_eq!(second_peer.read(&mut [0]).unwrap(), 0);
    assert!(sharing.leader.lock().unwrap().is_none());
}

#[test]
fn leased_clients_share_existing_execute_once_and_command_quanta() {
    let stop = Arc::new(AtomicBool::new(false));
    let sharing = Sharing::new(Config::default(), Arc::clone(&stop), 50);
    let (sender, receiver) = mpsc::channel();
    *sharing.leader.lock().unwrap() = Some(sender.clone());
    let request = b"*2\r\n$4\r\nINCR\r\n$1\r\nk\r\n";
    let (first, mut first_peer) = pair(request.repeat(65));
    let (second, mut second_peer) = pair(request.repeat(65));
    let (first, first_home) = loan(first, &stop);
    let (second, second_home) = loan(second, &stop);
    sender.send(second).ok().unwrap();
    let store = Store::new(1024);
    sharing.run(
        first,
        Leadership {
            sharing: &sharing,
            receiver,
        },
        &store,
    );
    let first = first_home
        .recv_timeout(Duration::from_secs(3))
        .unwrap()
        .unwrap();
    let second = second_home
        .recv_timeout(Duration::from_secs(3))
        .unwrap()
        .unwrap();
    assert!(first.input.is_empty() && second.input.is_empty());
    let expected_first: Vec<u8> = (1..=64)
        .chain([129])
        .flat_map(|n| format!(":{n}\r\n").into_bytes())
        .collect();
    let expected_second: Vec<u8> = (65..=128)
        .chain([130])
        .flat_map(|n| format!(":{n}\r\n").into_bytes())
        .collect();
    for (peer, expected) in [
        (&mut first_peer, expected_first),
        (&mut second_peer, expected_second),
    ] {
        let mut received = vec![0; expected.len()];
        peer.read_exact(&mut received).unwrap();
        assert_eq!(received, expected);
    }
    let mut reply = Vec::new();
    store.execute(&[b"GET", b"k"], &mut reply);
    assert_eq!(reply, b"$3\r\n130\r\n");
}

#[test]
fn protocol_and_metadata_survive_a_complete_lease_round_trip() {
    let stop = Arc::new(AtomicBool::new(false));
    let mut sharing = Sharing::new(Config::default(), Arc::clone(&stop), 50);
    sharing.max_epoch = Duration::ZERO;
    let (sender, receiver) = mpsc::channel();
    *sharing.leader.lock().unwrap() = Some(sender);
    let request = b"*2\r\n$5\r\nHELLO\r\n$1\r\n3\r\n*3\r\n$6\r\nCLIENT\r\n$7\r\nSETNAME\r\n$8\r\nborrowed\r\n";
    let (client, mut peer) = pair(request.to_vec());
    let (loan, home) = loan(client, &stop);
    let store = Store::new(1024);
    sharing.run(
        loan,
        Leadership {
            sharing: &sharing,
            receiver,
        },
        &store,
    );
    let mut client = home.recv_timeout(Duration::from_secs(3)).unwrap().unwrap();
    let mut wire = Vec::new();
    let mut chunk = [0; 128];
    while !wire.ends_with(b"+OK\r\n") {
        let count = peer.read(&mut chunk).unwrap();
        assert_ne!(count, 0, "handoff closed before replies were flushed");
        wire.extend_from_slice(&chunk[..count]);
        assert!(wire.len() <= 128);
    }
    assert!(wire.starts_with(b"%5\r\n"));
    let mut reply = Vec::new();
    client
        .session
        .execute(&store, &[b"GET", b"missing"], &mut reply);
    assert_eq!(reply, b"_\r\n");
    reply.clear();
    client
        .session
        .execute(&store, &[b"CLIENT", b"GETNAME"], &mut reply);
    assert_eq!(reply, b"$8\r\nborrowed\r\n");
}

#[test]
fn returned_partial_reply_is_not_resent_and_blocking_mode_is_restored() {
    let stop = Arc::new(AtomicBool::new(false));
    let sharing = Sharing::new(Config::default(), Arc::clone(&stop), 50);
    let (sender, receiver) = mpsc::channel();
    *sharing.leader.lock().unwrap() = Some(sender);
    let (client, mut peer) = pair(Vec::new());
    // This handle is the original worker's handle, not a leased Client.
    let mut original = client.stream.try_clone().unwrap();
    original.set_nonblocking(false).unwrap();
    thread::scope(|scope| {
        let (ready, wait) = mpsc::channel();
        let worker = scope.spawn(move || {
            let mut input = Vec::new();
            let mut output = Vec::new();
            assert!(
                !sharing
                    .exchange(
                        &mut original,
                        &mut input,
                        &mut output,
                        &mut Session::new(),
                        &Store::new(100),
                    )
                    .unwrap()
            );
            assert_eq!(input, b"partial");
            assert!(output.is_empty());
            ready.send(()).unwrap();
            let mut bytes = [0; 4];
            original.read_exact(&mut bytes).unwrap();
            assert_eq!(&bytes, b"next");
        });
        let mut pending = receiver.recv_timeout(Duration::from_secs(3)).unwrap();
        let state = pending.client.as_mut().unwrap();
        state.stream.write_all(b"+PO").unwrap();
        state.output.extend_from_slice(b"+PONG\r\n");
        state.sent = 3;
        state.input.extend_from_slice(b"partial");
        drop(pending);
        let mut reply = [0; 7];
        peer.read_exact(&mut reply).unwrap();
        assert_eq!(&reply, b"+PONG\r\n");
        wait.recv_timeout(Duration::from_secs(3)).unwrap();
        thread::sleep(Duration::from_millis(2));
        peer.write_all(b"next").unwrap();
        worker.join().unwrap();
    });
}

#[test]
fn leadership_unwind_returns_queued_ownership_without_a_stale_epoch() {
    let stop = Arc::new(AtomicBool::new(false));
    let sharing = Sharing::new(Config::default(), Arc::clone(&stop), 50);
    let (sender, receiver) = mpsc::channel();
    *sharing.leader.lock().unwrap() = Some(sender.clone());
    let (client, _peer) = pair(Vec::new());
    let (queued, home) = loan(client, &stop);
    sender.send(queued).ok().unwrap();
    thread::scope(|scope| {
        assert!(
            scope
                .spawn(|| {
                    let _leadership = Leadership {
                        sharing: &sharing,
                        receiver,
                    };
                    panic!("simulated leader unwind");
                })
                .join()
                .is_err()
        );
    });
    assert!(home.recv_timeout(Duration::from_secs(3)).unwrap().is_some());
    assert!(sharing.leader.lock().unwrap().is_none());
}
