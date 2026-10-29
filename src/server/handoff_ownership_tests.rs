use super::*;
use std::io::{BufRead, BufReader};
use std::sync::Barrier;

#[test]
fn idle_backoff_is_capped_and_actual_progress_resets_it() {
    let mut backoff = IdleBackoff::default();
    for expected in [1, 2, 4, 8, 16, 32, 64] {
        assert_eq!(backoff.observe(false), expected);
    }
    for _ in 0..1000 {
        assert_eq!(backoff.observe(false), 64);
    }
    assert_eq!(backoff.observe(true), 0);
    assert_eq!(backoff.observe(false), 1);
    assert_eq!(backoff.observe(true), 0);
    assert_eq!(backoff.observe(true), 0);
    assert_eq!(backoff.observe(false), 1);
}

#[test]
fn admission_rotates_stably_and_epoch_closure_is_lane_local() {
    let stop = Arc::new(AtomicBool::new(false));
    for capacity in [None, Some(0), Some(1), Some(2), Some(3), Some(4), Some(5)] {
        let policy = SchedulingPolicy::new(None, capacity);
        let mut lanes = Lanes::new(&Config::default(), &stop, policy);
        assert_eq!(lanes.lanes.len(), policy.client_handoff_lanes);
        if lanes.lanes.is_empty() {
            assert!(lanes.assign().is_none());
            continue;
        }
        for index in 0..17 {
            let assigned = lanes.assign().unwrap();
            assert_eq!(assigned.adaptive_poll, !policy.single_reply_yield);
            assert!(Arc::ptr_eq(
                &assigned,
                &lanes.lanes[index % lanes.lanes.len()]
            ));
        }
        if lanes.lanes.len() == 2 {
            let first = &lanes.lanes[0];
            let second = &lanes.lanes[1];
            let (sender, receiver) = mpsc::channel();
            *first.leader.lock().unwrap() = Some(sender);
            first.leader_active.store(true, Ordering::Release);
            let (sender, _second_receiver) = mpsc::channel();
            *second.leader.lock().unwrap() = Some(sender);
            second.leader_active.store(true, Ordering::Release);
            drop(Leadership {
                sharing: first,
                receiver,
            });
            assert!(!first.active());
            assert!(first.leader.lock().unwrap().is_none());
            assert!(second.active());
            assert!(second.leader.lock().unwrap().is_some());
        }
    }
    for workers in [1, 4, 64] {
        let mut lanes = Lanes::new(
            &Config::default(),
            &stop,
            SchedulingPolicy::new(Some(workers), Some(4)),
        );
        assert!(lanes.assign().is_none());
    }
}

#[test]
fn independent_coordinators_preserve_shared_transaction_atomicity() {
    let stop = Arc::new(AtomicBool::new(false));
    let mut lanes = Lanes::new(
        &Config::default(),
        &stop,
        SchedulingPolicy::new(None, Some(4)),
    );
    for lane in &mut lanes.lanes {
        Arc::get_mut(lane).unwrap().max_epoch = Duration::ZERO;
    }
    let request = b"MULTI\r\nINCR counter\r\nINCR counter\r\nEXEC\r\n";
    let mut peers = Vec::new();
    let mut homes = Vec::new();
    let barrier = Barrier::new(2);
    let store = Store::new(1024);
    thread::scope(|scope| {
        for _ in 0..2 {
            let lane = lanes.assign().unwrap();
            let (client, peer) = super::tests::pair(request.repeat(16));
            let (loan, home) = super::tests::loan(client, &stop);
            peers.push(peer);
            homes.push(home);
            let barrier = &barrier;
            let store = &store;
            scope.spawn(move || {
                let (sender, receiver) = mpsc::channel();
                *lane.leader.lock().unwrap() = Some(sender);
                barrier.wait();
                lane.run(
                    loan,
                    Leadership {
                        sharing: &lane,
                        receiver,
                    },
                    store,
                );
            });
        }
    });
    for home in homes {
        let client = home.recv_timeout(Duration::from_secs(3)).unwrap().unwrap();
        assert!(client.input.is_empty());
        assert!(client.output.is_empty());
    }
    let mut increments = Vec::new();
    for peer in peers {
        let mut reader = BufReader::new(peer);
        for _ in 0..16 {
            for expected in ["+OK\r\n", "+QUEUED\r\n", "+QUEUED\r\n", "*2\r\n"] {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                assert_eq!(line, expected);
            }
            let mut pair = Vec::new();
            for _ in 0..2 {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                pair.push(
                    line.strip_prefix(':')
                        .unwrap()
                        .trim()
                        .parse::<usize>()
                        .unwrap(),
                );
            }
            assert_eq!(pair[0] % 2, 1, "transactions may not interleave");
            assert_eq!(pair[1], pair[0] + 1);
            increments.extend(pair);
        }
    }
    increments.sort_unstable();
    assert_eq!(increments, (1..=64).collect::<Vec<_>>());
    let mut reply = Vec::new();
    store.execute(&[b"GET", b"counter"], &mut reply);
    assert_eq!(reply, b"$2\r\n64\r\n");
}

#[test]
fn repeated_leases_reuse_the_shell_and_drop_it_without_shutting_down_the_owner() {
    let sharing = Sharing::new(Config::default(), Arc::new(AtomicBool::new(false)), 50);
    let (sender, loans) = mpsc::channel();
    *sharing.leader.lock().unwrap() = Some(sender);
    let (mut client, mut peer) = super::tests::pair(Vec::new());
    let mut original = client.stream.try_clone().unwrap();
    original.set_nonblocking(false).unwrap();
    client.shutdown_on_drop = false;
    drop(client);
    let (push, mut receiver) = mpsc::sync_channel(8);
    thread::scope(|scope| {
        let owner = scope.spawn(|| {
            let mut channel = LeaseChannel::new();
            let mut session = Session::new();
            let id = session.id();
            let mut input = Vec::with_capacity(128);
            let mut output = Vec::with_capacity(128);
            let mut shell_id = None;
            for _ in 0..4 {
                assert!(
                    !sharing
                        .exchange(
                            &mut original,
                            &mut input,
                            &mut output,
                            &mut session,
                            &mut LeaseIO {
                                pubsub: &mut receiver,
                                channel: &mut channel,
                            },
                            &Store::new(1024),
                        )
                        .unwrap()
                );
                assert_eq!(session.id(), id);
                assert_eq!(input, b"partial");
                assert!(input.capacity() >= 128);
                assert!(output.is_empty());
                assert!(output.capacity() >= 128);
                assert_eq!(
                    receiver.recv_timeout(Duration::from_secs(3)).unwrap(),
                    b"queued"
                );
                let shell = channel.cached.as_ref().unwrap();
                assert!(!shell.shutdown_on_drop);
                assert_eq!(shell.sent, 0);
                assert!(shell.input.is_empty());
                assert!(shell.output.is_empty());
                assert!(matches!(
                    shell.pubsub.try_recv(),
                    Err(mpsc::TryRecvError::Disconnected)
                ));
                let previous = shell_id.get_or_insert(shell.session.id());
                assert_eq!(
                    *previous,
                    shell.session.id(),
                    "reuse the placeholder session"
                );
            }
            drop(channel); // Closing the cached handle must not call shutdown.
            original.write_all(b"+alive\r\n").unwrap();
            let mut next = [0; 4];
            original.read_exact(&mut next).unwrap();
            assert_eq!(&next, b"next");
        });
        for _ in 0..4 {
            let mut loan = loans.recv_timeout(Duration::from_secs(3)).unwrap();
            let client = loan.client.as_mut().unwrap();
            client.input.clear();
            client.input.extend_from_slice(b"partial");
            // Exercise suffix compaction on every reuse, not just the first loan.
            client.stream.write_all(b"+PO").unwrap();
            client.output.extend_from_slice(b"+PONG\r\n");
            client.sent = 3;
            push.send(b"queued".to_vec()).unwrap();
            drop(loan);
            let mut response = [0; 7];
            peer.read_exact(&mut response).unwrap();
            assert_eq!(&response, b"+PONG\r\n");
        }
        let mut response = [0; 8];
        peer.read_exact(&mut response).unwrap();
        assert_eq!(&response, b"+alive\r\n");
        peer.write_all(b"next").unwrap();
        owner.join().unwrap();
    });
}
