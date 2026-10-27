#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use rillcache::engine::{EvictionPolicy, NotifyFlags};
use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

fn start(flags: NotifyFlags) -> ServerHandle {
    let workers = match std::env::var("RILLCACHE_TEST_THREADS").as_deref() {
        Ok("0") | Err(_) => None,
        Ok("1") => Some(1),
        Ok("4") => Some(4),
        Ok(value) => panic!("invalid test worker mode {value}"),
    };
    Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        worker_threads: workers,
        notify_keyspace_events: flags,
        ..Config::default()
    })
    .unwrap()
    .spawn()
    .unwrap()
}

fn connect(server: &ServerHandle) -> TcpStream {
    let stream = TcpStream::connect(server.local_addr()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream.set_nodelay(true).unwrap();
    stream
}

fn send(stream: &mut TcpStream, arguments: &[&[u8]], expected: &[u8]) {
    let mut request = Vec::new();
    encode_request(arguments, &mut request);
    stream.write_all(&request).unwrap();
    if !expected.is_empty() {
        let mut actual = vec![0; expected.len()];
        stream.read_exact(&mut actual).unwrap();
        assert_eq!(actual, expected, "command {arguments:?}");
    }
}

fn read_line(stream: &mut TcpStream, output: &mut Vec<u8>) -> String {
    let mut line = Vec::new();
    let mut byte = [0];
    loop {
        stream.read_exact(&mut byte).unwrap();
        if byte[0] == b'\n' {
            assert_eq!(line.pop(), Some(b'\r'));
            output.extend_from_slice(&line);
            output.extend_from_slice(b"\r\n");
            return String::from_utf8(line).unwrap();
        }
        line.push(byte[0]);
    }
}

fn read_frame(stream: &mut TcpStream) -> Vec<u8> {
    let mut output = Vec::new();
    let mut prefix = [0];
    stream.read_exact(&mut prefix).unwrap();
    output.push(prefix[0]);
    match prefix[0] {
        b'+' | b'-' | b':' => {
            read_line(stream, &mut output);
        }
        b'$' => {
            let length = read_line(stream, &mut output).parse::<i64>().unwrap();
            if length >= 0 {
                let mut body = vec![0; length as usize + 2];
                stream.read_exact(&mut body).unwrap();
                output.extend_from_slice(&body);
            }
        }
        b'*' => {
            let count = read_line(stream, &mut output).parse::<i64>().unwrap();
            for _ in 0..count.max(0) {
                output.extend_from_slice(&read_frame(stream));
            }
        }
        other => panic!("unexpected prefix {other:?}"),
    }
    output
}

fn frame(parts: &[&[u8]]) -> Vec<u8> {
    let mut output = format!("*{}\r\n", parts.len()).into_bytes();
    for part in parts {
        output.extend_from_slice(format!("${}\r\n", part.len()).as_bytes());
        output.extend_from_slice(part);
        output.extend_from_slice(b"\r\n");
    }
    output
}

fn subscribe(stream: &mut TcpStream, kind: &[u8], pattern: &[u8]) {
    send(stream, &[kind, pattern], b"");
    let confirmation = read_frame(stream);
    assert!(confirmation.starts_with(b"*3\r\n"), "{confirmation:?}");
    assert!(
        confirmation
            .windows(kind.len())
            .any(|window| window == kind)
    );
}

fn expect_msetex_events(subscriber: &mut TcpStream, keys: &[&[u8]], expiry: bool) {
    for key in keys {
        let mut channel = b"__keyspace@0__:".to_vec();
        channel.extend_from_slice(key);
        let events: &[&[u8]] = if expiry {
            &[b"set", b"expire"]
        } else {
            &[b"set"]
        };
        for event in events {
            assert_eq!(
                read_frame(subscriber),
                frame(&[b"pmessage", b"__keyspace@*", &channel, event])
            );
        }
    }
}

#[test]
fn msetex_emits_set_then_expire_for_each_key_in_argument_order() {
    let server = start(NotifyFlags::parse(b"KEA").unwrap());
    let mut subscriber = connect(&server);
    subscribe(&mut subscriber, b"psubscribe", b"__keyspace@*");
    let mut writer = connect(&server);
    for (option, deadline) in [
        (b"EX".as_slice(), b"3600".as_slice()),
        (b"PX", b"3600000"),
        (b"EXAT", b"4000000000"),
        (b"PXAT", b"4000000000000"),
    ] {
        send(
            &mut writer,
            &[
                b"mSeTeX", b"2", b"z", b"one", b"a", b"two", option, deadline,
            ],
            b":1\r\n",
        );
        expect_msetex_events(&mut subscriber, &[b"z", b"a"], true);
    }
    // A confirmation on the same subscription connection is an ordered barrier:
    // an unexpected extra notification would be read before it.
    send(
        &mut subscriber,
        &[b"PING", b"done"],
        &frame(&[b"pong", b"done"]),
    );
    server.shutdown().unwrap();
}

#[test]
fn msetex_duplicate_binary_keys_keep_events_but_keepttl_adds_no_expire() {
    let server = start(NotifyFlags::parse(b"KEA").unwrap());
    let mut subscriber = connect(&server);
    subscribe(&mut subscriber, b"psubscribe", b"__keyspace@*");
    let mut writer = connect(&server);
    let binary = b"z\0\xff";
    send(
        &mut writer,
        &[
            b"MSETEX", b"3", binary, b"first", b"a", b"v", binary, b"last", b"PX", b"3600000",
        ],
        b":1\r\n",
    );
    expect_msetex_events(&mut subscriber, &[binary, b"a", binary], true);
    send(&mut writer, &[b"GET", binary], b"$4\r\nlast\r\n");
    send(
        &mut writer,
        &[
            b"MSETEX", b"2", binary, b"new", b"a", b"v", b"XX", b"KEEPTTL",
        ],
        b":1\r\n",
    );
    expect_msetex_events(&mut subscriber, &[binary, b"a"], false);
    send(
        &mut writer,
        &[b"MSETEX", b"2", b"a", b"v", binary, b"new"],
        b":1\r\n",
    );
    expect_msetex_events(&mut subscriber, &[b"a", binary], false);
    send(&mut writer, &[b"PTTL", binary], b":-1\r\n");
    send(
        &mut subscriber,
        &[b"PING", b"done"],
        &frame(&[b"pong", b"done"]),
    );
    server.shutdown().unwrap();
}

#[test]
fn msetex_filters_categories_and_rejections_and_preserves_multi_order() {
    let server = start(NotifyFlags::parse(b"KEg").unwrap());
    let mut subscriber = connect(&server);
    subscribe(&mut subscriber, b"psubscribe", b"__keyevent@0__:*");
    let mut writer = connect(&server);
    send(
        &mut writer,
        &[b"MSETEX", b"2", b"z", b"one", b"a", b"two", b"EX", b"3600"],
        b":1\r\n",
    );
    for key in [b"z", b"a"] {
        assert_eq!(
            read_frame(&mut subscriber),
            frame(&[
                b"pmessage",
                b"__keyevent@0__:*",
                b"__keyevent@0__:expire",
                key
            ])
        );
    }
    send(
        &mut writer,
        &[b"MSETEX", b"1", b"z", b"bad", b"NX", b"EX", b"3600"],
        b":0\r\n",
    );
    send(
        &mut writer,
        &[b"MSETEX", b"1", b"missing", b"bad", b"XX", b"EX", b"3600"],
        b":0\r\n",
    );
    send(
        &mut writer,
        &[b"MSETEX", b"1", b"z", b"bad", b"EX", b"0"],
        b"-ERR invalid expire time in 'msetex' command\r\n",
    );
    send(
        &mut writer,
        &[b"MSETEX", b"2", b"z", b"bad"],
        b"-ERR syntax error\r\n",
    );
    send(
        &mut writer,
        &[b"CONFIG", b"SET", b"notify-keyspace-events", b"E$"],
        b"+OK\r\n",
    );
    send(&mut writer, &[b"MULTI"], b"+OK\r\n");
    send(
        &mut writer,
        &[
            b"MSETEX", b"2", b"z", b"one", b"a", b"two", b"PX", b"3600000",
        ],
        b"+QUEUED\r\n",
    );
    send(
        &mut writer,
        &[b"MSETEX", b"1", b"z", b"three"],
        b"+QUEUED\r\n",
    );
    send(&mut writer, &[b"EXEC"], b"*2\r\n:1\r\n:1\r\n");
    for key in [b"z", b"a", b"z"] {
        assert_eq!(
            read_frame(&mut subscriber),
            frame(&[b"pmessage", b"__keyevent@0__:*", b"__keyevent@0__:set", key])
        );
    }
    send(
        &mut subscriber,
        &[b"PING", b"done"],
        &frame(&[b"pong", b"done"]),
    );
    server.shutdown().unwrap();
}

#[test]
fn keyspace_and_keyevent_notifications_follow_flags_and_categories() {
    let server = start(NotifyFlags::parse(b"KEA").unwrap());
    let mut subscriber = connect(&server);
    subscribe(&mut subscriber, b"psubscribe", b"__keyevent@0__:*");
    subscribe(&mut subscriber, b"subscribe", b"__keyspace@0__:k");
    let mut writer = connect(&server);

    send(&mut writer, &[b"SET", b"k", b"v"], b"+OK\r\n");
    let expected = frame(&[b"message", b"__keyspace@0__:k", b"set"]);
    assert_eq!(read_frame(&mut subscriber), expected);
    let expected = frame(&[
        b"pmessage",
        b"__keyevent@0__:*",
        b"__keyevent@0__:set",
        b"k",
    ]);
    assert_eq!(read_frame(&mut subscriber), expected);

    send(&mut writer, &[b"HSET", b"h", b"f", b"v"], b":1\r\n");
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:hset",
            b"h"
        ])
    );
    send(&mut writer, &[b"LPUSH", b"l", b"x"], b":1\r\n");
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:lpush",
            b"l"
        ])
    );
    send(&mut writer, &[b"ZADD", b"z", b"1", b"x"], b":1\r\n");
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:zadd",
            b"z"
        ])
    );
    send(
        &mut writer,
        &[b"XADD", b"x", b"1-0", b"f", b"v"],
        b"$3\r\n1-0\r\n",
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:xadd",
            b"x"
        ])
    );
    send(&mut writer, &[b"PFADD", b"p", b"a"], b":1\r\n");
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:pfadd",
            b"p"
        ])
    );

    send(&mut writer, &[b"PEXPIRE", b"h", b"10000"], b":1\r\n");
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:expire",
            b"h"
        ])
    );
    send(&mut writer, &[b"RENAME", b"h", b"h2"], b"+OK\r\n");
    let mut events = Vec::new();
    for _ in 0..2 {
        events.push(read_frame(&mut subscriber));
    }
    assert!(events.contains(&frame(&[
        b"pmessage",
        b"__keyevent@0__:*",
        b"__keyevent@0__:rename_from",
        b"h"
    ])));
    assert!(events.contains(&frame(&[
        b"pmessage",
        b"__keyevent@0__:*",
        b"__keyevent@0__:rename_to",
        b"h2"
    ])));

    send(
        &mut writer,
        &[b"CONFIG", b"SET", b"notify-keyspace-events", b"Eg"],
        b"+OK\r\n",
    );
    send(&mut writer, &[b"SET", b"quiet", b"v"], b"+OK\r\n");
    send(&mut writer, &[b"DEL", b"quiet"], b":1\r\n");
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:del",
            b"quiet"
        ])
    );
    send(
        &mut writer,
        &[b"CONFIG", b"SET", b"notify-keyspace-events", b""],
        b"+OK\r\n",
    );
    send(&mut writer, &[b"SET", b"silent", b"v"], b"+OK\r\n");
    send(&mut writer, &[b"DEL", b"silent"], b":1\r\n");
    let info = {
        send(&mut writer, &[b"INFO", b"memory"], b"");
        read_frame(&mut writer)
    };
    let info = String::from_utf8(info).unwrap();
    assert!(info.contains("notify_keyspace_events:"), "{info}");
    server.shutdown().unwrap();
}

#[test]
fn geo_mutations_emit_zset_store_and_deletion_events() {
    let server = start(NotifyFlags::parse(b"KEA").unwrap());
    let mut subscriber = connect(&server);
    subscribe(&mut subscriber, b"psubscribe", b"__keyevent@0__:*");
    let mut writer = connect(&server);

    send(&mut writer, &[b"GEOADD", b"g", b"0", b"0", b"a"], b":1\r\n");
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:zadd",
            b"g"
        ])
    );
    send(
        &mut writer,
        &[
            b"GEOSEARCHSTORE",
            b"out",
            b"g",
            b"FROMLONLAT",
            b"0",
            b"0",
            b"BYRADIUS",
            b"1",
            b"km",
        ],
        b":1\r\n",
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:geosearchstore",
            b"out"
        ])
    );
    send(
        &mut writer,
        &[
            b"GEOSEARCHSTORE",
            b"out",
            b"g",
            b"FROMLONLAT",
            b"90",
            b"0",
            b"BYRADIUS",
            b"1",
            b"km",
        ],
        b":0\r\n",
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:del",
            b"out"
        ])
    );
    server.shutdown().unwrap();
}

#[test]
fn stream_claim_mutations_emit_claim_events() {
    let server = start(NotifyFlags::parse(b"KEA").unwrap());
    let mut subscriber = connect(&server);
    subscribe(&mut subscriber, b"psubscribe", b"__keyevent@0__:*");
    let mut writer = connect(&server);

    send(
        &mut writer,
        &[b"XADD", b"s", b"1-0", b"f", b"a"],
        b"$3\r\n1-0\r\n",
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:xadd",
            b"s"
        ])
    );
    send(
        &mut writer,
        &[b"XGROUP", b"CREATE", b"s", b"g", b"0"],
        b"+OK\r\n",
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:xgroup",
            b"s"
        ])
    );
    send(
        &mut writer,
        &[
            b"XREADGROUP",
            b"GROUP",
            b"g",
            b"alice",
            b"STREAMS",
            b"s",
            b">",
        ],
        b"*1\r\n*2\r\n$1\r\ns\r\n*1\r\n*2\r\n$3\r\n1-0\r\n*2\r\n$1\r\nf\r\n$1\r\na\r\n",
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:xreadgroup",
            b"s"
        ])
    );
    send(
        &mut writer,
        &[b"XCLAIM", b"s", b"g", b"bob", b"0", b"1-0", b"JUSTID"],
        b"*1\r\n$3\r\n1-0\r\n",
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:xclaim",
            b"s"
        ])
    );
    send(
        &mut writer,
        &[b"XAUTOCLAIM", b"s", b"g", b"carol", b"0", b"0-0", b"JUSTID"],
        b"*3\r\n$3\r\n0-0\r\n*1\r\n$3\r\n1-0\r\n*0\r\n",
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:xautoclaim",
            b"s"
        ])
    );
    server.shutdown().unwrap();
}
#[test]
fn stream_trim_emits_xtrim_events() {
    let server = start(NotifyFlags::parse(b"KEA").unwrap());
    let mut subscriber = connect(&server);
    subscribe(&mut subscriber, b"psubscribe", b"__keyevent@0__:*");
    let mut writer = connect(&server);

    for (id, expected) in [
        (b"1-0".as_slice(), b"$3\r\n1-0\r\n".as_slice()),
        (b"2-0", b"$3\r\n2-0\r\n"),
    ] {
        send(&mut writer, &[b"XADD", b"s", id, b"f", b"v"], expected);
        assert_eq!(
            read_frame(&mut subscriber),
            frame(&[
                b"pmessage",
                b"__keyevent@0__:*",
                b"__keyevent@0__:xadd",
                b"s"
            ])
        );
    }
    send(&mut writer, &[b"XTRIM", b"s", b"MAXLEN", b"1"], b":1\r\n");
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:xtrim",
            b"s"
        ])
    );
    server.shutdown().unwrap();
}

#[test]
fn expired_and_evicted_flags_accept_config_and_eviction_publishes_events() {
    let workers = match std::env::var("RILLCACHE_TEST_THREADS").as_deref() {
        Ok("0") | Err(_) => None,
        Ok("1") => Some(1),
        Ok("4") => Some(4),
        Ok(value) => panic!("invalid test worker mode {value}"),
    };
    let server = Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        worker_threads: workers,
        max_data_bytes: 10,
        maxmemory_policy: EvictionPolicy::AllKeysRandom,
        notify_keyspace_events: NotifyFlags::parse(b"KEA").unwrap(),
        ..Config::default()
    })
    .unwrap()
    .spawn()
    .unwrap();
    let mut subscriber = connect(&server);
    subscribe(&mut subscriber, b"psubscribe", b"__keyevent@0__:*");
    let mut writer = connect(&server);

    send(&mut writer, &[b"SET", b"victim", b"123"], b"+OK\r\n");
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:set",
            b"victim"
        ])
    );
    send(&mut writer, &[b"SET", b"new", b"123"], b"+OK\r\n");
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:evicted",
            b"victim"
        ])
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:set",
            b"new"
        ])
    );

    send(
        &mut writer,
        &[b"CONFIG", b"SET", b"notify-keyspace-events", b"KEx"],
        b"+OK\r\n",
    );
    send(
        &mut writer,
        &[b"CONFIG", b"GET", b"notify-keyspace-events"],
        b"",
    );
    let reply = read_frame(&mut writer);
    assert_eq!(reply, frame(&[b"notify-keyspace-events", b"KEx"]));
    server.shutdown().unwrap();
}
#[test]
fn sorted_set_algebra_store_emits_destination_events() {
    let server = start(NotifyFlags::parse(b"KEA").unwrap());
    let mut subscriber = connect(&server);
    subscribe(&mut subscriber, b"psubscribe", b"__keyevent@0__:*");
    let mut writer = connect(&server);

    send(
        &mut writer,
        &[b"ZADD", b"a", b"1", b"m", b"3", b"n"],
        b":2\r\n",
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:zadd",
            b"a"
        ])
    );
    send(&mut writer, &[b"ZADD", b"b", b"2", b"m"], b":1\r\n");
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:zadd",
            b"b"
        ])
    );

    send(
        &mut writer,
        &[b"ZUNIONSTORE", b"d", b"2", b"a", b"b"],
        b":2\r\n",
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:zunionstore",
            b"d"
        ])
    );
    send(
        &mut writer,
        &[b"ZINTERSTORE", b"i", b"2", b"a", b"b"],
        b":1\r\n",
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:zinterstore",
            b"i"
        ])
    );
    send(
        &mut writer,
        &[b"ZDIFFSTORE", b"f", b"2", b"a", b"b"],
        b":1\r\n",
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:zdiffstore",
            b"f"
        ])
    );
    send(
        &mut writer,
        &[b"ZDIFFSTORE", b"d", b"2", b"a", b"a"],
        b":0\r\n",
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:del",
            b"d"
        ])
    );
    server.shutdown().unwrap();
}

#[test]
fn zmpop_emits_direction_specific_pop_events() {
    let server = start(NotifyFlags::parse(b"KEA").unwrap());
    let mut subscriber = connect(&server);
    subscribe(&mut subscriber, b"psubscribe", b"__keyevent@0__:*");
    let mut writer = connect(&server);

    send(&mut writer, &[b"ZADD", b"z", b"1", b"a"], b":1\r\n");
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:zadd",
            b"z"
        ])
    );
    send(
        &mut writer,
        &[b"ZMPOP", b"1", b"z", b"MIN"],
        b"*2\r\n$1\r\nz\r\n*1\r\n*2\r\n$1\r\na\r\n$1\r\n1\r\n",
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:zpopmin",
            b"z"
        ])
    );
    send(&mut writer, &[b"ZADD", b"z", b"1", b"a"], b":1\r\n");
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:zadd",
            b"z"
        ])
    );
    send(
        &mut writer,
        &[b"ZMPOP", b"1", b"z", b"MAX"],
        b"*2\r\n$1\r\nz\r\n*1\r\n*2\r\n$1\r\na\r\n$1\r\n1\r\n",
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:zpopmax",
            b"z"
        ])
    );
    server.shutdown().unwrap();
}
#[test]
fn bzmpop_emits_direction_specific_pop_events() {
    let server = start(NotifyFlags::parse(b"KEA").unwrap());
    let mut subscriber = connect(&server);
    subscribe(&mut subscriber, b"psubscribe", b"__keyevent@0__:*");
    let mut writer = connect(&server);

    send(
        &mut writer,
        &[b"ZADD", b"n", b"1", b"a", b"2", b"b"],
        b":2\r\n",
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:zadd",
            b"n"
        ])
    );
    send(
        &mut writer,
        &[b"BZMPOP", b"0", b"1", b"n", b"MAX"],
        b"*2\r\n$1\r\nn\r\n*1\r\n*2\r\n$1\r\nb\r\n$1\r\n2\r\n",
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:zpopmax",
            b"n"
        ])
    );
    send(
        &mut writer,
        &[b"BZMPOP", b"0", b"1", b"n", b"MIN"],
        b"*2\r\n$1\r\nn\r\n*1\r\n*2\r\n$1\r\na\r\n$1\r\n1\r\n",
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:zpopmin",
            b"n"
        ])
    );
    server.shutdown().unwrap();
}

#[test]
fn sort_store_emits_sortstore_and_deletion_events() {
    let server = start(NotifyFlags::parse(b"KEA").unwrap());
    let mut subscriber = connect(&server);
    subscribe(&mut subscriber, b"psubscribe", b"__keyevent@0__:*");
    let mut writer = connect(&server);

    send(&mut writer, &[b"RPUSH", b"l", b"2", b"1"], b":2\r\n");
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:rpush",
            b"l"
        ])
    );
    send(&mut writer, &[b"SORT", b"l", b"STORE", b"out"], b":2\r\n");
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:sortstore",
            b"out"
        ])
    );
    send(
        &mut writer,
        &[b"SORT", b"missing", b"STORE", b"out"],
        b":0\r\n",
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:del",
            b"out"
        ])
    );
    server.shutdown().unwrap();
}

#[test]
fn lmpop_emits_multipop_event() {
    let server = start(NotifyFlags::parse(b"KEA").unwrap());
    let mut subscriber = connect(&server);
    subscribe(&mut subscriber, b"psubscribe", b"__keyevent@0__:*");
    let mut writer = connect(&server);

    send(&mut writer, &[b"RPUSH", b"l", b"a", b"b"], b":2\r\n");
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:rpush",
            b"l"
        ])
    );
    send(
        &mut writer,
        &[b"LMPOP", b"1", b"l", b"LEFT"],
        b"*2\r\n$1\r\nl\r\n*1\r\n$1\r\na\r\n",
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:lmpop",
            b"l"
        ])
    );
    server.shutdown().unwrap();
}

#[test]
fn blmpop_emits_lmpop_event() {
    let server = start(NotifyFlags::parse(b"KEA").unwrap());
    let mut subscriber = connect(&server);
    subscribe(&mut subscriber, b"psubscribe", b"__keyevent@0__:*");
    let mut writer = connect(&server);

    send(&mut writer, &[b"RPUSH", b"l", b"a"], b":1\r\n");
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:rpush",
            b"l"
        ])
    );
    send(
        &mut writer,
        &[b"BLMPOP", b"0", b"1", b"l", b"LEFT"],
        b"*2\r\n$1\r\nl\r\n*1\r\n$1\r\na\r\n",
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:lmpop",
            b"l"
        ])
    );
    server.shutdown().unwrap();
}
