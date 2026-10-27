#![forbid(unsafe_code)]

use rillcache::{engine::Store, session::Session};

fn raw(store: &Store, args: &[&[u8]]) -> Vec<u8> {
    let mut reply = Vec::new();
    store.execute(args, &mut reply);
    reply
}

fn statistics(store: &Store) -> String {
    String::from_utf8(raw(store, &[b"INFO", b"commandstats"])).unwrap()
}

#[test]
fn blocking_attempts_count_once_without_counting_empty_polls_or_completion_again() {
    for (command, producer) in [
        (
            b"BLPOP".as_slice(),
            vec![b"RPUSH".as_slice(), b"key", b"value"],
        ),
        (
            b"BZPOPMIN",
            vec![b"ZADD".as_slice(), b"key", b"1", b"value"],
        ),
        (
            b"BZPOPMAX",
            vec![b"ZADD".as_slice(), b"key", b"1", b"value"],
        ),
    ] {
        let store = Store::new(4096);
        assert_eq!(
            raw(
                &store,
                &[b"CONFIG", b"SET", b"slowlog-log-slower-than", b"-1"]
            ),
            b"+OK\r\n"
        );
        assert_eq!(
            raw(
                &store,
                &[b"CONFIG", b"SET", b"commandstats-enabled", b"yes"]
            ),
            b"+OK\r\n"
        );
        let mut session = Session::new();
        let mut reply = Vec::new();
        session.execute(&store, &[command, b"key", b"0"], &mut reply);
        assert!(reply.is_empty());
        assert!(session.poll_blocked(&store).is_none());
        let before = statistics(&store);
        let prefix = format!(
            "cmdstat_{}:calls=1,",
            String::from_utf8_lossy(command).to_ascii_lowercase()
        );
        assert!(before.contains(&prefix), "{before}");
        assert_eq!(raw(&store, &producer), b":1\r\n");
        let completed = session
            .poll_blocked(&store)
            .expect("producer wakes consumer");
        assert!(completed.starts_with(b"*"));
        assert!(session.poll_blocked(&store).is_none());
        assert_eq!(
            statistics(&store),
            before,
            "completion is not another dispatch"
        );
    }
}
