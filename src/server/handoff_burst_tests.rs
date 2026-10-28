use super::*;

#[test]
fn isolated_replies_do_not_activate_a_lease_and_leader_presence_does() {
    let stop = Arc::new(AtomicBool::new(false));
    let sharing = Sharing::new(Config::default(), stop, 50);
    assert!(!sharing.should_lend());
    let (sender, _receiver) = mpsc::channel();
    *sharing.leader.lock().unwrap() = Some(sender);
    sharing.leader_active.store(true, Ordering::Release);
    assert!(sharing.should_lend());
    sharing.leader_active.store(false, Ordering::Release);
    std::thread::sleep(Duration::from_micros(100));
    sharing.recent_micros.store(0, Ordering::Relaxed);
    assert!(!sharing.should_lend());
}

#[test]
fn atomic_reply_observations_reject_stale_times_and_keep_the_exact_boundary() {
    assert!(!Sharing::recent_burst(0, 1, 50));
    assert!(Sharing::recent_burst(100, 100, 50));
    assert!(Sharing::recent_burst(100, 149, 50));
    assert!(!Sharing::recent_burst(100, 150, 50));
    assert!(!Sharing::recent_burst(200, 100, 50));
    let stop = Arc::new(AtomicBool::new(false));
    let sharing = Sharing::new(Config::default(), stop, 50);
    let (sender, receiver) = mpsc::channel();
    *sharing.leader.lock().unwrap() = Some(sender);
    sharing.leader_active.store(true, Ordering::Release);
    drop(Leadership {
        sharing: &sharing,
        receiver,
    });
    assert!(sharing.leader.lock().unwrap().is_none());
    assert!(!sharing.leader_active.load(Ordering::Acquire));
}

#[test]
fn active_reflects_leader_presence_for_single_reply_joins() {
    let sharing = Sharing::new(
        Config::default(),
        Arc::new(AtomicBool::new(false)),
        CLIENT_HANDOFF_IDLE_US,
    );
    assert!(!sharing.active());
    sharing.leader_active.store(true, Ordering::Release);
    assert!(sharing.active());
    assert!(sharing.should_lend());
}

#[test]
fn progress_never_extends_an_epochs_maximum_lifetime() {
    let now = Instant::now();
    let idle = Duration::from_micros(50);
    let maximum = Duration::from_millis(100);
    let mut window = EpochWindow::new(now, idle, maximum);
    assert!(window.active(now, true));
    assert!(!window.active(now + idle, false));
    assert!(window.active(now + maximum - Duration::from_nanos(1), true));
    assert!(!window.active(now + maximum, true));
}

#[test]
fn quiet_borrowers_return_but_the_coordinators_socket_stays_serviced() {
    let stop = Arc::new(AtomicBool::new(false));
    let (client, _peer) = super::tests::pair(Vec::new());
    let now = client.activity;
    let (mut loan, _home) = super::tests::loan(client, &stop);
    let idle = Duration::from_micros(50);
    assert!(!loan.quiet(now + idle - Duration::from_nanos(1), idle));
    assert!(loan.quiet(now + idle, idle));
    loan.coordinator = true;
    assert!(!loan.quiet(now + Duration::from_secs(1), idle));
}

#[test]
fn epoch_limit_returns_ready_input_without_executing_it_twice() {
    let stop = Arc::new(AtomicBool::new(false));
    let mut sharing = Sharing::new(Config::default(), Arc::clone(&stop), 50);
    sharing.max_epoch = Duration::ZERO;
    let (sender, receiver) = mpsc::channel();
    *sharing.leader.lock().unwrap() = Some(sender.clone());
    let request = b"*2\r\n$4\r\nINCR\r\n$1\r\nk\r\n";
    let (first, _first_peer) = super::tests::pair(request.repeat(65));
    let (second, _second_peer) = super::tests::pair(request.repeat(65));
    let (first, first_home) = super::tests::loan(first, &stop);
    let (second, second_home) = super::tests::loan(second, &stop);
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
    for home in [first_home, second_home] {
        assert_eq!(
            home.recv_timeout(Duration::from_secs(3))
                .unwrap()
                .unwrap()
                .input,
            request
        );
    }
    let mut reply = Vec::new();
    store.execute(&[b"GET", b"k"], &mut reply);
    assert_eq!(reply, b"$3\r\n128\r\n");
}
