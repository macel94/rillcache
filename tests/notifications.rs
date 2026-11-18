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
fn geo_read_misses_emit_keymiss_for_source_keys_only() {
    let server = start(NotifyFlags::parse(b"Em").unwrap());
    let mut subscriber = connect(&server);
    subscribe(&mut subscriber, b"psubscribe", b"__keyevent@0__:keymiss");
    let mut writer = connect(&server);

    let expect_miss = |subscriber: &mut TcpStream, key: &[u8]| {
        assert_eq!(
            read_frame(subscriber),
            frame(&[
                b"pmessage",
                b"__keyevent@0__:keymiss",
                b"__keyevent@0__:keymiss",
                key,
            ])
        );
    };

    send(
        &mut writer,
        &[b"GEODIST", b"dist-miss", b"a", b"b"],
        b"$-1\r\n",
    );
    expect_miss(&mut subscriber, b"dist-miss");
    send(
        &mut writer,
        &[b"GEOHASH", b"hash-miss", b"a"],
        b"*1\r\n$-1\r\n",
    );
    expect_miss(&mut subscriber, b"hash-miss");
    send(
        &mut writer,
        &[b"GEOPOS", b"pos-miss", b"a"],
        b"*1\r\n*-1\r\n",
    );
    expect_miss(&mut subscriber, b"pos-miss");
    send(
        &mut writer,
        &[
            b"GEOSEARCH",
            b"search-miss",
            b"FROMLONLAT",
            b"0",
            b"0",
            b"BYRADIUS",
            b"1",
            b"km",
        ],
        b"*0\r\n",
    );
    expect_miss(&mut subscriber, b"search-miss");
    send(
        &mut writer,
        &[
            b"GEOSEARCHSTORE",
            b"store-dest",
            b"store-source-miss",
            b"FROMLONLAT",
            b"0",
            b"0",
            b"BYRADIUS",
            b"1",
            b"km",
        ],
        b":0\r\n",
    );
    expect_miss(&mut subscriber, b"store-source-miss");
    send(
        &mut writer,
        &[b"GEORADIUS", b"radius-miss", b"0", b"0", b"1", b"km"],
        b"*0\r\n",
    );
    expect_miss(&mut subscriber, b"radius-miss");
    send(
        &mut writer,
        &[
            b"GEORADIUSBYMEMBER",
            b"member-radius-miss",
            b"home",
            b"1",
            b"km",
        ],
        b"*0\r\n",
    );
    expect_miss(&mut subscriber, b"member-radius-miss");

    send(&mut writer, &[b"LPUSH", b"wrong-type", b"value"], b":1\r\n");
    send(
        &mut writer,
        &[
            b"GEOSEARCH",
            b"wrong-type",
            b"FROMLONLAT",
            b"0",
            b"0",
            b"BYRADIUS",
            b"1",
            b"km",
        ],
        b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
    );
    send(
        &mut writer,
        &[b"GEOADD", b"existing", b"0", b"0", b"home"],
        b":1\r\n",
    );
    send(
        &mut writer,
        &[b"GEODIST", b"existing", b"home", b"absent-member"],
        b"$-1\r\n",
    );
    send(
        &mut writer,
        &[b"GEOPOS", b"existing", b"absent-member"],
        b"*1\r\n*-1\r\n",
    );
    send(
        &mut writer,
        &[b"GEOHASH", b"existing", b"absent-member"],
        b"*1\r\n$-1\r\n",
    );
    send(
        &mut writer,
        &[b"GEODIST", b"marker", b"a", b"b"],
        b"$-1\r\n",
    );
    expect_miss(&mut subscriber, b"marker");
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

#[test]
fn blocking_xreadgroup_emits_delivery_event() {
    let server = start(NotifyFlags::parse(b"KEA").unwrap());
    let mut subscriber = connect(&server);
    subscribe(&mut subscriber, b"psubscribe", b"__keyevent@0__:*");
    let mut writer = connect(&server);
    let mut blocked = connect(&server);

    send(
        &mut writer,
        &[b"XADD", b"w", b"1-0", b"f", b"v"],
        b"$3\r\n1-0\r\n",
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:xadd",
            b"w"
        ])
    );
    send(
        &mut writer,
        &[b"XGROUP", b"CREATE", b"w", b"g", b"$"],
        b"+OK\r\n",
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:xgroup",
            b"w"
        ])
    );

    send(
        &mut blocked,
        &[
            b"XREADGROUP",
            b"GROUP",
            b"g",
            b"c",
            b"BLOCK",
            b"0",
            b"STREAMS",
            b"w",
            b">",
        ],
        b"",
    );
    std::thread::sleep(std::time::Duration::from_millis(100));
    send(
        &mut writer,
        &[b"XADD", b"w", b"2-0", b"f", b"v2"],
        b"$3\r\n2-0\r\n",
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:xadd",
            b"w"
        ])
    );
    assert_eq!(
        read_frame(&mut blocked),
        b"*1\r\n*2\r\n$1\r\nw\r\n*1\r\n*2\r\n$3\r\n2-0\r\n*2\r\n$1\r\nf\r\n$2\r\nv2\r\n"
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:xreadgroup",
            b"w"
        ])
    );
    server.shutdown().unwrap();
}

#[test]
fn hgetdel_emits_hash_and_deletion_events() {
    let server = start(NotifyFlags::parse(b"KEA").unwrap());
    let mut subscriber = connect(&server);
    subscribe(&mut subscriber, b"psubscribe", b"__keyevent@0__:*");
    let mut writer = connect(&server);

    send(
        &mut writer,
        &[b"HSET", b"h", b"a", b"1", b"b", b"2"],
        b":2\r\n",
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:hset",
            b"h"
        ])
    );
    send(
        &mut writer,
        &[b"HGETDEL", b"h", b"FIELDS", b"1", b"a"],
        b"*1\r\n$1\r\n1\r\n",
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:hdel",
            b"h"
        ])
    );
    send(
        &mut writer,
        &[b"HGETDEL", b"h", b"FIELDS", b"1", b"b"],
        b"*1\r\n$1\r\n2\r\n",
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:hdel",
            b"h"
        ])
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:del",
            b"h"
        ])
    );

    send(&mut writer, &[b"HSET", b"h2", b"a", b"1"], b":1\r\n");
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:hset",
            b"h2"
        ])
    );
    send(&mut writer, &[b"HDEL", b"h2", b"a"], b":1\r\n");
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:hdel",
            b"h2"
        ])
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:del",
            b"h2"
        ])
    );
    server.shutdown().unwrap();
}

#[test]
fn hgetex_due_deadline_emits_expiry_and_empty_hash_deletion_events() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let server = start(NotifyFlags::parse(b"KEA").unwrap());
    let mut subscriber = connect(&server);
    subscribe(&mut subscriber, b"psubscribe", b"__keyevent@0__:*");
    let mut writer = connect(&server);
    send(
        &mut writer,
        &[b"HSET", b"h", b"a", b"1", b"b", b"2"],
        b":2\r\n",
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:hset",
            b"h"
        ])
    );

    let past = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
        .saturating_sub(1_000)
        .to_string();
    send(
        &mut writer,
        &[
            b"HGETEX",
            b"h",
            b"PXAT",
            past.as_bytes(),
            b"FIELDS",
            b"1",
            b"a",
        ],
        b"*1\r\n$1\r\n1\r\n",
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:hexpired",
            b"h"
        ])
    );

    send(
        &mut writer,
        &[
            b"HGETEX",
            b"h",
            b"PXAT",
            past.as_bytes(),
            b"FIELDS",
            b"1",
            b"b",
        ],
        b"*1\r\n$1\r\n2\r\n",
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:hexpired",
            b"h"
        ])
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:del",
            b"h"
        ])
    );
    server.shutdown().unwrap();
}

#[test]
fn hexpire_and_hpersist_emit_only_changed_field_ttls() {
    let server = start(NotifyFlags::parse(b"KEA").unwrap());
    let mut subscriber = connect(&server);
    subscribe(&mut subscriber, b"psubscribe", b"__keyevent@0__:*");
    let mut writer = connect(&server);
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

    send(
        &mut writer,
        &[b"HEXPIRE", b"h", b"10", b"FIELDS", b"1", b"f"],
        b"*1\r\n:1\r\n",
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:hexpire",
            b"h"
        ])
    );

    // A condition that changes nothing must not publish an event. The next
    // frame read is the HPERSIST event, proving nothing was queued in between.
    send(
        &mut writer,
        &[b"HEXPIRE", b"h", b"10", b"NX", b"FIELDS", b"1", b"f"],
        b"*1\r\n:0\r\n",
    );
    send(
        &mut writer,
        &[b"HPERSIST", b"h", b"FIELDS", b"1", b"f"],
        b"*1\r\n:1\r\n",
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:hpersist",
            b"h"
        ])
    );

    // Repeating HPERSIST changes nothing, so the next frame is the HEXPIRE
    // event from the following command.
    send(
        &mut writer,
        &[b"HPERSIST", b"h", b"FIELDS", b"1", b"f"],
        b"*1\r\n:-1\r\n",
    );
    send(
        &mut writer,
        &[b"HEXPIRE", b"h", b"10", b"FIELDS", b"1", b"f"],
        b"*1\r\n:1\r\n",
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:hexpire",
            b"h"
        ])
    );
    server.shutdown().unwrap();
}

#[test]
fn get_and_mget_misses_emit_keymiss_only_for_missing_keys() {
    let server = start(NotifyFlags::parse(b"Em").unwrap());
    let mut subscriber = connect(&server);
    subscribe(&mut subscriber, b"psubscribe", b"__keyevent@0__:*");
    let mut writer = connect(&server);

    send(&mut writer, &[b"GET", b"missing"], b"$-1\r\n");
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:keymiss",
            b"missing"
        ])
    );

    send(&mut writer, &[b"SET", b"k", b"v"], b"+OK\r\n");
    send(&mut writer, &[b"GET", b"k"], b"$1\r\nv\r\n");
    send(
        &mut writer,
        &[b"MGET", b"k", b"missing"],
        b"*2\r\n$1\r\nv\r\n$-1\r\n",
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:keymiss",
            b"missing"
        ])
    );

    // A non-string key exists, so MGET's null is not a lookup miss.
    send(&mut writer, &[b"LPUSH", b"w", b"a"], b":1\r\n");
    send(&mut writer, &[b"MGET", b"w"], b"*1\r\n$-1\r\n");

    for (command, reply) in [
        (vec![b"EXISTS".as_slice(), b"missing"], b":0\r\n".as_slice()),
        (vec![b"TYPE", b"missing"], b"+none\r\n"),
        (vec![b"TTL", b"missing"], b":-2\r\n"),
        (vec![b"PTTL", b"missing"], b":-2\r\n"),
    ] {
        send(&mut writer, &command, reply);
        assert_eq!(
            read_frame(&mut subscriber),
            frame(&[
                b"pmessage",
                b"__keyevent@0__:*",
                b"__keyevent@0__:keymiss",
                b"missing"
            ])
        );
    }
    server.shutdown().unwrap();
}

#[test]
fn touch_read_misses_emit_once_per_missing_operand() {
    let server = start(NotifyFlags::parse(b"Em").unwrap());
    let mut subscriber = connect(&server);
    subscribe(&mut subscriber, b"psubscribe", b"__keyevent@0__:keymiss");
    let mut writer = connect(&server);

    send(&mut writer, &[b"TOUCH", b"missing", b"missing"], b":0\r\n");
    for _ in 0..2 {
        assert_eq!(
            read_frame(&mut subscriber),
            frame(&[
                b"pmessage",
                b"__keyevent@0__:keymiss",
                b"__keyevent@0__:keymiss",
                b"missing"
            ])
        );
    }

    send(&mut writer, &[b"SET", b"present", b"value"], b"+OK\r\n");
    send(
        &mut writer,
        &[b"TOUCH", b"present", b"absent", b"present"],
        b":2\r\n",
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:keymiss",
            b"__keyevent@0__:keymiss",
            b"absent"
        ])
    );

    send(&mut writer, &[b"LPUSH", b"wrong", b"type"], b":1\r\n");
    send(&mut writer, &[b"TOUCH", b"wrong"], b":1\r\n");
    send(&mut writer, &[b"GET", b"after-hit-marker"], b"$-1\r\n");
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:keymiss",
            b"__keyevent@0__:keymiss",
            b"after-hit-marker"
        ])
    );
    server.shutdown().unwrap();
}

#[test]
fn pfcount_read_misses_emit_keymiss_per_missing_source_lookup() {
    let server = start(NotifyFlags::parse(b"Em").unwrap());
    let mut subscriber = connect(&server);
    subscribe(&mut subscriber, b"psubscribe", b"__keyevent@0__:keymiss");
    let mut writer = connect(&server);

    send(&mut writer, &[b"PFADD", b"seen", b"value"], b":1\r\n");
    send(&mut writer, &[b"PFCOUNT", b"seen"], b":1\r\n");
    send(&mut writer, &[b"LPUSH", b"wrong", b"type"], b":1\r\n");
    send(
        &mut writer,
        &[b"PFCOUNT", b"wrong"],
        b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
    );

    send(&mut writer, &[b"PFCOUNT", b"missing"], b":0\r\n");
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:keymiss",
            b"__keyevent@0__:keymiss",
            b"missing"
        ])
    );

    send(
        &mut writer,
        &[b"PFCOUNT", b"seen", b"missing", b"missing"],
        b":1\r\n",
    );
    for _ in 0..2 {
        assert_eq!(
            read_frame(&mut subscriber),
            frame(&[
                b"pmessage",
                b"__keyevent@0__:keymiss",
                b"__keyevent@0__:keymiss",
                b"missing"
            ])
        );
    }
    server.shutdown().unwrap();
}

#[test]
fn pfmerge_read_misses_emit_destination_then_each_missing_source() {
    let server = start(NotifyFlags::parse(b"Em").unwrap());
    let mut subscriber = connect(&server);
    subscribe(&mut subscriber, b"psubscribe", b"__keyevent@0__:keymiss");
    let mut writer = connect(&server);

    send(
        &mut writer,
        &[b"PFMERGE", b"dest", b"missing", b"missing"],
        b"+OK\r\n",
    );
    for key in [b"dest".as_slice(), b"missing", b"missing"] {
        assert_eq!(
            read_frame(&mut subscriber),
            frame(&[
                b"pmessage",
                b"__keyevent@0__:keymiss",
                b"__keyevent@0__:keymiss",
                key
            ])
        );
    }

    send(
        &mut writer,
        &[b"PFADD", b"existing-dest", b"value"],
        b":1\r\n",
    );
    send(
        &mut writer,
        &[b"PFADD", b"existing-source", b"value"],
        b":1\r\n",
    );
    send(
        &mut writer,
        &[b"PFMERGE", b"existing-dest", b"existing-source"],
        b"+OK\r\n",
    );
    send(&mut writer, &[b"PFMERGE", b"marker"], b"+OK\r\n");
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:keymiss",
            b"__keyevent@0__:keymiss",
            b"marker"
        ])
    );

    send(&mut writer, &[b"LPUSH", b"wrong", b"type"], b":1\r\n");
    send(
        &mut writer,
        &[b"PFMERGE", b"wrong", b"missing"],
        b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
    );
    send(
        &mut writer,
        &[b"PFMERGE", b"error-dest", b"missing-before-error", b"wrong"],
        b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
    );
    for key in [b"error-dest".as_slice(), b"missing-before-error"] {
        assert_eq!(
            read_frame(&mut subscriber),
            frame(&[
                b"pmessage",
                b"__keyevent@0__:keymiss",
                b"__keyevent@0__:keymiss",
                key
            ])
        );
    }
    send(&mut writer, &[b"PFMERGE", b"after-error"], b"+OK\r\n");
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:keymiss",
            b"__keyevent@0__:keymiss",
            b"after-error"
        ])
    );
    server.shutdown().unwrap();
}

#[test]
fn bitop_read_misses_emit_once_per_missing_source_occurrence() {
    let server = start(NotifyFlags::parse(b"Em").unwrap());
    let mut subscriber = connect(&server);
    subscribe(&mut subscriber, b"psubscribe", b"__keyevent@0__:keymiss");
    let mut writer = connect(&server);

    send(
        &mut writer,
        &[b"BITOP", b"OR", b"dest", b"missing", b"missing"],
        b":0\r\n",
    );
    for _ in 0..2 {
        assert_eq!(
            read_frame(&mut subscriber),
            frame(&[
                b"pmessage",
                b"__keyevent@0__:keymiss",
                b"__keyevent@0__:keymiss",
                b"missing"
            ])
        );
    }

    send(&mut writer, &[b"SET", b"present", b"\x01"], b"+OK\r\n");
    send(
        &mut writer,
        &[b"BITOP", b"XOR", b"result", b"present", b"absent"],
        b":1\r\n",
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:keymiss",
            b"__keyevent@0__:keymiss",
            b"absent"
        ])
    );

    send(&mut writer, &[b"LPUSH", b"wrong", b"type"], b":1\r\n");
    send(
        &mut writer,
        &[b"BITOP", b"OR", b"wrong-dest", b"wrong"],
        b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
    );
    send(&mut writer, &[b"GET", b"wrong-type-marker"], b"$-1\r\n");
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:keymiss",
            b"__keyevent@0__:keymiss",
            b"wrong-type-marker"
        ])
    );

    send(
        &mut writer,
        &[
            b"BITOP",
            b"OR",
            b"error-dest",
            b"miss-before-error",
            b"wrong",
        ],
        b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:keymiss",
            b"__keyevent@0__:keymiss",
            b"miss-before-error"
        ])
    );
    send(&mut writer, &[b"GET", b"after-error-marker"], b"$-1\r\n");
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:keymiss",
            b"__keyevent@0__:keymiss",
            b"after-error-marker"
        ])
    );

    server.shutdown().unwrap();
}

#[test]
fn new_key_events_fire_once_per_created_key() {
    let server = start(NotifyFlags::parse(b"En").unwrap());
    let mut subscriber = connect(&server);
    subscribe(&mut subscriber, b"psubscribe", b"__keyevent@0__:*");
    let mut writer = connect(&server);

    send(&mut writer, &[b"SET", b"k", b"v"], b"+OK\r\n");
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:new",
            b"k"
        ])
    );
    send(&mut writer, &[b"SET", b"k", b"v2"], b"+OK\r\n");
    send(&mut writer, &[b"HSET", b"h", b"f", b"v"], b":1\r\n");
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:new",
            b"h"
        ])
    );
    send(
        &mut writer,
        &[b"XADD", b"s", b"1-0", b"f", b"v"],
        b"$3\r\n1-0\r\n",
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:*",
            b"__keyevent@0__:new",
            b"s"
        ])
    );
    send(
        &mut writer,
        &[b"XADD", b"s", b"2-0", b"f", b"v"],
        b"$3\r\n2-0\r\n",
    );
    server.shutdown().unwrap();
}

#[test]
fn only_conditional_delex_reports_a_missing_key() {
    let server = start(NotifyFlags::parse(b"Em").unwrap());
    let mut subscriber = connect(&server);
    subscribe(&mut subscriber, b"psubscribe", b"__keyevent@0__:keymiss");
    let mut writer = connect(&server);
    let event = |key: &[u8]| {
        frame(&[
            b"pmessage",
            b"__keyevent@0__:keymiss",
            b"__keyevent@0__:keymiss",
            key,
        ])
    };
    let key = b"delex\0\xff";
    send(&mut writer, &[b"DELEX", key, b"IFEQ", b"value"], b":0\r\n");
    assert_eq!(read_frame(&mut subscriber), event(key));
    send(&mut writer, &[b"DELEX", key], b":0\r\n");
    send(&mut writer, &[b"DELIFEQ", key, b"value"], b":0\r\n");
    send(&mut writer, &[b"SET", key, b"value"], b"+OK\r\n");
    send(&mut writer, &[b"DELEX", key, b"IFEQ", b"other"], b":0\r\n");
    send(&mut writer, &[b"DELEX", key, b"IFEQ", b"value"], b":1\r\n");
    send(&mut writer, &[b"SADD", b"set", b"m"], b":1\r\n");
    send(
        &mut writer,
        &[b"DELEX", b"set", b"IFEQ", b"value"],
        b"-ERR Key should be of string type if conditions are specified\r\n",
    );
    send(&mut writer, &[b"GET", b"marker"], b"$-1\r\n");
    assert_eq!(read_frame(&mut subscriber), event(b"marker"));
    server.shutdown().unwrap();
}

#[test]
fn zrangestore_reports_only_missing_source_lookups() {
    let server = start(NotifyFlags::parse(b"Em").unwrap());
    let mut subscriber = connect(&server);
    subscribe(&mut subscriber, b"psubscribe", b"__keyevent@0__:keymiss");
    let mut writer = connect(&server);
    let expect_miss = |subscriber: &mut _, key: &[u8]| {
        assert_eq!(
            read_frame(subscriber),
            frame(&[
                b"pmessage",
                b"__keyevent@0__:keymiss",
                b"__keyevent@0__:keymiss",
                key
            ]),
        );
    };
    let source = b"source\0\xff";
    send(&mut writer, &[b"SET", b"dest", b"old"], b"+OK\r\n");
    send(
        &mut writer,
        &[b"ZRANGESTORE", b"dest", source, b"0", b"-1"],
        b":0\r\n",
    );
    expect_miss(&mut subscriber, source);
    send(&mut writer, &[b"TYPE", b"dest"], b"+none\r\n");
    expect_miss(&mut subscriber, b"dest");
    send(
        &mut writer,
        &[b"ZRANGESTORE", b"absent-dest", source, b"0", b"-1"],
        b":0\r\n",
    );
    expect_miss(&mut subscriber, source);
    send(&mut writer, &[b"ZADD", source, b"1", b"member"], b":1\r\n");
    send(
        &mut writer,
        &[b"ZRANGESTORE", b"dest", source, b"0", b"-1"],
        b":1\r\n",
    );
    send(
        &mut writer,
        &[b"ZRANGESTORE", b"other", source, b"1", b"2"],
        b":0\r\n",
    );
    send(&mut writer, &[b"SET", source, b"not-a-set"], b"+OK\r\n");
    send(
        &mut writer,
        &[b"ZRANGESTORE", b"dest", source, b"0", b"-1"],
        b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
    );
    send(&mut writer, &[b"ZCARD", b"dest"], b":1\r\n");
    send(&mut writer, &[b"GET", b"marker"], b"$-1\r\n");
    expect_miss(&mut subscriber, b"marker");
    server.shutdown().unwrap();
}

#[test]
fn getdel_reports_only_a_missing_key_as_keymiss() {
    let server = start(NotifyFlags::parse(b"Emg").unwrap());
    let mut subscriber = connect(&server);
    subscribe(&mut subscriber, b"psubscribe", b"__keyevent@0__:*");
    let mut writer = connect(&server);
    let expect = |subscriber: &mut _, channel: &[u8], key: &[u8]| {
        assert_eq!(
            read_frame(subscriber),
            frame(&[b"pmessage", b"__keyevent@0__:*", channel, key])
        );
    };
    let key = b"key\0\xff";
    send(&mut writer, &[b"GETDEL", key], b"$-1\r\n");
    expect(&mut subscriber, b"__keyevent@0__:keymiss", key);
    send(&mut writer, &[b"SET", key, b"v"], b"+OK\r\n");
    send(&mut writer, &[b"GETDEL", key], b"$1\r\nv\r\n");
    expect(&mut subscriber, b"__keyevent@0__:del", key);
    send(&mut writer, &[b"SADD", key, b"m"], b":1\r\n");
    send(
        &mut writer,
        &[b"GETDEL", key],
        b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
    );
    send(&mut writer, &[b"SISMEMBER", key, b"m"], b":1\r\n");
    send(&mut writer, &[b"GET", b"marker"], b"$-1\r\n");
    expect(&mut subscriber, b"__keyevent@0__:keymiss", b"marker");
    server.shutdown().unwrap();
}

#[test]
fn lcs_reports_both_operand_lookups_before_wrong_type() {
    let server = start(NotifyFlags::parse(b"Em").unwrap());
    let mut subscriber = connect(&server);
    subscribe(&mut subscriber, b"psubscribe", b"__keyevent@0__:keymiss");
    let mut writer = connect(&server);
    let assert_miss = |subscriber: &mut _, key: &[u8]| {
        assert_eq!(
            read_frame(subscriber),
            frame(&[
                b"pmessage",
                b"__keyevent@0__:keymiss",
                b"__keyevent@0__:keymiss",
                key,
            ])
        );
    };
    let missing = b"missing\0\xff";
    send(&mut writer, &[b"SADD", b"set", b"member"], b":1\r\n");
    for keys in [[b"set".as_slice(), missing], [missing, b"set"]] {
        send(
            &mut writer,
            &[b"LCS", keys[0], keys[1]],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );
        assert_miss(&mut subscriber, missing);
        send(&mut writer, &[b"GET", b"marker"], b"$-1\r\n");
        assert_miss(&mut subscriber, b"marker");
    }
    send(&mut writer, &[b"LCS", missing, b"other"], b"$0\r\n\r\n");
    assert_miss(&mut subscriber, missing);
    assert_miss(&mut subscriber, b"other");
    send(&mut writer, &[b"LCS", missing, missing, b"LEN"], b":0\r\n");
    assert_miss(&mut subscriber, missing);
    assert_miss(&mut subscriber, missing);
    send(&mut writer, &[b"GET", b"marker"], b"$-1\r\n");
    assert_miss(&mut subscriber, b"marker");
    server.shutdown().unwrap();
}

#[test]
fn smismember_emits_one_keymiss_for_a_missing_set_only() {
    let server = start(NotifyFlags::parse(b"Em").unwrap());
    let mut subscriber = connect(&server);
    subscribe(&mut subscriber, b"psubscribe", b"__keyevent@0__:keymiss");
    let mut writer = connect(&server);
    let binary_missing = b"missing\0\xff";

    send(
        &mut writer,
        &[b"SMISMEMBER", binary_missing, b"a", b"b"],
        b"*2\r\n:0\r\n:0\r\n",
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:keymiss",
            b"__keyevent@0__:keymiss",
            binary_missing
        ])
    );

    send(&mut writer, &[b"SADD", b"existing", b"member"], b":1\r\n");
    send(
        &mut writer,
        &[b"SMISMEMBER", b"existing", b"member", b"absent-member"],
        b"*2\r\n:1\r\n:0\r\n",
    );
    send(&mut writer, &[b"GET", b"marker-after-hit"], b"$-1\r\n");
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:keymiss",
            b"__keyevent@0__:keymiss",
            b"marker-after-hit"
        ])
    );

    send(&mut writer, &[b"SET", b"wrong", b"type"], b"+OK\r\n");
    send(
        &mut writer,
        &[b"SMISMEMBER", b"wrong", b"member"],
        b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
    );
    send(
        &mut writer,
        &[b"GET", b"marker-after-wrong-type"],
        b"$-1\r\n",
    );
    assert_eq!(
        read_frame(&mut subscriber),
        frame(&[
            b"pmessage",
            b"__keyevent@0__:keymiss",
            b"__keyevent@0__:keymiss",
            b"marker-after-wrong-type"
        ])
    );
    server.shutdown().unwrap();
}
