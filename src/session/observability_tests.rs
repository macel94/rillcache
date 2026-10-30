//! Optional observers must not change plain-command replies or error accounting.
use super::*;

fn configured_store(mask: u8) -> Store {
    let store = Store::new(4096);
    store.set_slowlog_threshold_us(if mask & 1 == 0 { -1 } else { 0 });
    store.set_commandstats_enabled(mask & 2 != 0);
    let mut reply = Vec::new();
    store.execute(
        &[
            b"CONFIG",
            b"SET",
            b"latency-monitor-threshold",
            if mask & 4 == 0 { b"0" } else { b"1" },
        ],
        &mut reply,
    );
    assert_eq!(reply, b"+OK\r\n");
    store
}

fn assert_commandstats(store: &Store, names: &[(&str, usize)], enabled: bool) {
    let mut info = Vec::new();
    store.execute(&[b"INFO", b"commandstats"], &mut info);
    for (name, count) in names {
        let marker = format!("cmdstat_{name}:calls={count},");
        assert_eq!(
            info.windows(marker.len())
                .any(|part| part == marker.as_bytes()),
            enabled
        );
    }
}

#[test]
fn plain_errors_are_counted_with_every_optional_collector_configuration() {
    for mask in 0..8 {
        for protocol in [Protocol::Resp2, Protocol::Resp3] {
            let store = configured_store(mask);
            let mut seeded = Vec::new();
            store.execute(&[b"LPUSH", b"list", b"x"], &mut seeded);
            assert_eq!(seeded, b":1\r\n");
            let mut session = Session {
                protocol,
                ..Session::new()
            };
            let cases: &[(&[&[u8]], &[u8])] = &[
                (
                    &[b"PING", b"one", b"two"],
                    b"-ERR wrong number of arguments for 'ping' command\r\n",
                ),
                (
                    &[b"GET", b"list"],
                    b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
                ),
                (
                    &[b"SET", b"key"],
                    b"-ERR wrong number of arguments for 'set' command\r\n",
                ),
                (
                    &[b"ECHO"],
                    b"-ERR wrong number of arguments for 'echo' command\r\n",
                ),
            ];
            for (args, expected) in cases {
                let mut output = b"prefix:".to_vec();
                assert_eq!(session.execute(&store, args, &mut output), Action::Continue);
                assert_eq!(&output[..7], b"prefix:");
                assert_eq!(&output[7..], *expected);
            }
            let errors: std::collections::BTreeMap<_, _> =
                store.error_stats().into_iter().collect();
            assert_eq!(
                errors.get(b"ERR".as_slice()),
                Some(&3),
                "mask={mask}, protocol={protocol:?}"
            );
            assert_eq!(errors.get(b"WRONGTYPE".as_slice()), Some(&1));
            assert_eq!(errors.len(), 2);
            assert_eq!(store.stats().total_commands_processed, 4);
            assert_eq!(store.slowlog_len(), if mask & 1 == 0 { 0 } else { 4 });
            assert_commandstats(
                &store,
                &[("ping", 1), ("get", 1), ("set", 1), ("echo", 1)],
                mask & 2 != 0,
            );
        }
    }
}

#[test]
fn plain_success_replies_and_observers_remain_consistent_in_resp2_and_resp3() {
    for mask in 0..8 {
        for protocol in [Protocol::Resp2, Protocol::Resp3] {
            let store = configured_store(mask);
            let mut session = Session {
                protocol,
                ..Session::new()
            };
            let null = if protocol == Protocol::Resp3 {
                b"_\r\n".as_slice()
            } else {
                b"$-1\r\n".as_slice()
            };
            let cases: &[(&[&[u8]], &[u8])] = &[
                (&[b"PING"], b"+PONG\r\n"),
                (&[b"ECHO", b"a\0b"], b"$3\r\na\0b\r\n"),
                (&[b"SET", b"k", b"v"], b"+OK\r\n"),
                (&[b"GET", b"k"], b"$1\r\nv\r\n"),
                (&[b"SET", b"k", b"no", b"NX"], null),
                (&[b"SET", b"k", b"new", b"XX"], b"+OK\r\n"),
                (&[b"GET", b"k"], b"$3\r\nnew\r\n"),
            ];
            for (args, expected) in cases {
                let mut output = Vec::new();
                assert_eq!(session.execute(&store, args, &mut output), Action::Continue);
                assert_eq!(output, *expected);
            }
            assert!(store.error_stats().is_empty());
            assert_eq!(store.stats().total_commands_processed, 7);
            assert_eq!(store.slowlog_len(), if mask & 1 == 0 { 0 } else { 7 });
            assert_commandstats(
                &store,
                &[("ping", 1), ("echo", 1), ("set", 3), ("get", 2)],
                mask & 2 != 0,
            );
        }
    }
}
