#![forbid(unsafe_code)]

use rillcache::engine::{Clock, Store};
use rillcache::session::Session;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

struct CountingClock {
    now: AtomicI64,
    reads: AtomicU64,
}
impl Clock for CountingClock {
    fn now_ms(&self) -> i64 {
        self.reads.fetch_add(1, Ordering::Relaxed);
        self.now.load(Ordering::Relaxed)
    }
}
fn fixture() -> (Store, Arc<CountingClock>) {
    let clock = Arc::new(CountingClock {
        now: AtomicI64::new(100),
        reads: AtomicU64::new(0),
    });
    let store = Store::with_clock(1024, clock.clone());
    let mut output = Vec::new();
    store.execute(
        &[b"CONFIG", b"SET", b"slowlog-log-slower-than", b"-1"],
        &mut output,
    );
    assert_eq!(output, b"+OK\r\n");
    clock.reads.store(0, Ordering::Relaxed);
    (store, clock)
}
fn call(session: &mut Session, store: &Store, args: &[&[u8]]) -> Vec<u8> {
    let mut output = Vec::new();
    session.execute(store, args, &mut output);
    output
}

#[test]
fn inactive_pause_adds_no_clock_reads_to_plain_session_dispatch() {
    let (store, clock) = fixture();
    let mut session = Session::new();
    for _ in 0..32 {
        assert_eq!(call(&mut session, &store, &[b"PING"]), b"+PONG\r\n");
    }
    assert_eq!(clock.reads.load(Ordering::Relaxed), 0);
}

#[test]
fn unpause_after_exact_expiry_restores_zero_clock_dispatch() {
    let (store, clock) = fixture();
    let mut admin = Session::new();
    let mut client = Session::new();
    assert_eq!(
        call(&mut admin, &store, &[b"CLIENT", b"PAUSE", b"100", b"ALL"]),
        b"+OK\r\n"
    );
    assert!(call(&mut client, &store, &[b"PING"]).is_empty());
    assert!(client.is_paused());
    clock.now.store(200, Ordering::Relaxed);
    assert_eq!(client.poll_paused(&store).unwrap().0, b"+PONG\r\n");
    assert_eq!(
        call(&mut admin, &store, &[b"CLIENT", b"UNPAUSE"]),
        b"+OK\r\n"
    );
    clock.reads.store(0, Ordering::Relaxed);
    for _ in 0..32 {
        assert_eq!(call(&mut client, &store, &[b"PING"]), b"+PONG\r\n");
    }
    assert_eq!(clock.reads.load(Ordering::Relaxed), 0);
    clock.now.store(100, Ordering::Relaxed);
    assert_eq!(call(&mut client, &store, &[b"PING"]), b"+PONG\r\n");
    assert!(
        !client.is_paused(),
        "an explicitly cleared pause must not resurrect on clock rollback"
    );
}

#[test]
fn maintenance_expires_once_before_a_paused_conditional_write_resumes() {
    for mode in [b"ALL".as_slice(), b"WRITE"] {
        let (store, clock) = fixture();
        let mut admin = Session::new();
        let mut writer = Session::new();
        let mut subscriber = Session::new();
        let (sender, events) = std::sync::mpsc::sync_channel(4);
        subscriber.set_pubsub_sender(sender);
        assert_eq!(
            call(
                &mut admin,
                &store,
                &[b"CONFIG", b"SET", b"notify-keyspace-events", b"Ex"]
            ),
            b"+OK\r\n"
        );
        assert_eq!(
            call(
                &mut subscriber,
                &store,
                &[b"SUBSCRIBE", b"__keyevent@0__:expired"]
            ),
            b"*3\r\n$9\r\nsubscribe\r\n$22\r\n__keyevent@0__:expired\r\n:1\r\n"
        );
        assert_eq!(
            call(&mut writer, &store, &[b"SET", b"k", b"old", b"PX", b"10"]),
            b"+OK\r\n"
        );
        assert_eq!(
            call(&mut admin, &store, &[b"CLIENT", b"PAUSE", b"100", mode]),
            b"+OK\r\n"
        );
        assert!(call(&mut writer, &store, &[b"SET", b"k", b"new", b"NX"]).is_empty());
        assert!(writer.is_paused());

        clock.now.store(109, Ordering::Relaxed);
        assert_eq!(store.purge_expired(16), 0);
        assert!(events.try_recv().is_err());
        clock.now.store(111, Ordering::Relaxed);
        assert_eq!(store.purge_expired(16), 1);
        assert_eq!(
            events.try_recv().unwrap(),
            b"*3\r\n$7\r\nmessage\r\n$22\r\n__keyevent@0__:expired\r\n$1\r\nk\r\n"
        );
        assert!(writer.poll_paused(&store).is_none());
        assert_eq!(store.purge_expired(16), 0);
        assert_eq!(
            call(&mut admin, &store, &[b"CLIENT", b"UNPAUSE"]),
            b"+OK\r\n"
        );
        assert_eq!(writer.poll_paused(&store).unwrap().0, b"+OK\r\n");
        assert!(writer.poll_paused(&store).is_none());
        assert_eq!(call(&mut writer, &store, &[b"GET", b"k"]), b"$3\r\nnew\r\n");
        assert!(events.try_recv().is_err());
    }
}

#[test]
fn a_fresh_pause_after_expiry_is_enforced_and_unpause_clears_it() {
    let (store, clock) = fixture();
    let mut admin = Session::new();
    let mut client = Session::new();
    assert_eq!(
        call(&mut admin, &store, &[b"CLIENT", b"PAUSE", b"1", b"ALL"]),
        b"+OK\r\n"
    );
    assert!(call(&mut client, &store, &[b"PING"]).is_empty());
    clock.now.store(101, Ordering::Relaxed);
    assert_eq!(client.poll_paused(&store).unwrap().0, b"+PONG\r\n");
    assert_eq!(
        call(&mut admin, &store, &[b"CLIENT", b"PAUSE", b"100", b"ALL"]),
        b"+OK\r\n"
    );
    assert!(call(&mut client, &store, &[b"PING"]).is_empty());
    assert_eq!(
        call(&mut admin, &store, &[b"CLIENT", b"UNPAUSE"]),
        b"+OK\r\n"
    );
    assert_eq!(client.poll_paused(&store).unwrap().0, b"+PONG\r\n");
    clock.reads.store(0, Ordering::Relaxed);
    assert_eq!(call(&mut client, &store, &[b"PING"]), b"+PONG\r\n");
    assert_eq!(clock.reads.load(Ordering::Relaxed), 0);
}
