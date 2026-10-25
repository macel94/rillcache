use super::*;

#[test]
fn isolated_replies_do_not_activate_a_lease_and_bursts_have_an_exact_boundary() {
    let mut burst = Burst::default();
    let now = Instant::now();
    let window = Duration::from_micros(50);
    assert!(!burst.observe(now, window));
    assert!(!burst.observe(now + Duration::from_millis(1), window));
    assert!(burst.observe(now + Duration::from_millis(1) + window / 2, window));
    assert!(!burst.observe(now + Duration::from_millis(1) + window * 3 / 2, window));
    assert!(burst.observe(now + Duration::from_millis(1) + window * 3 / 2, window));
    assert!(
        !burst.observe(now, window),
        "an invalid backwards observation is not a burst"
    );
}

#[test]
fn an_active_epoch_accepts_a_client_without_a_recent_local_burst() {
    let stop = Arc::new(AtomicBool::new(false));
    let sharing = Sharing::new(Config::default(), stop, 50);
    assert!(!sharing.should_lend());
    let (sender, receiver) = mpsc::channel();
    *sharing.leader.lock().unwrap() = Some(sender);
    *sharing.recent.lock().unwrap() = Burst::default();
    assert!(sharing.should_lend());
    drop(Leadership {
        sharing: &sharing,
        receiver,
    });
    *sharing.recent.lock().unwrap() = Burst {
        previous: Some(Instant::now() - Duration::from_secs(1)),
    };
    assert!(!sharing.should_lend());
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
