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
