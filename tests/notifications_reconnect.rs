#![forbid(unsafe_code)]

//! Keyspace notification ordering, duplicate/loss and reconnect behaviour (C8).
//!
//! Delivery is at-most-once and process-local. The supported mutation matrix is
//! pinned in `docs/notifications.md`; these tests bind the parts that are easy
//! to regress and that the readiness criterion names explicitly:
//!
//! 1. One mutation publishes one event per enabled channel class, in command
//!    order, with the documented payload direction (keyspace channel message is
//!    the event name, keyevent channel message is the key).
//! 2. A disconnected subscriber receives nothing it missed: there is no replay,
//!    no queue written on behalf of an absent subscriber, and a reconnected
//!    subscriber sees only events published after it subscribed.
//! 3. The unsupported `FLUSH*` event class stays silent without desynchronising
//!    the connection: the next supported mutation still delivers its event.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use rillcache::engine::NotifyFlags;
use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

fn start() -> ServerHandle {
    let workers = match std::env::var("RILLCACHE_TEST_THREADS").as_deref() {
        Ok("0") | Err(_) => None,
        Ok("1") => Some(1),
        Ok("4") => Some(4),
        Ok(value) => panic!("invalid test worker mode {value}"),
    };
    Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        worker_threads: workers,
        notify_keyspace_events: NotifyFlags::parse(b"KEA").unwrap(),
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

fn send(stream: &mut TcpStream, arguments: &[&[u8]]) {
    let mut request = Vec::new();
    encode_request(arguments, &mut request);
    stream.write_all(&request).unwrap();
}

fn read_line(stream: &mut TcpStream, output: &mut Vec<u8>) -> Vec<u8> {
    let mut line = Vec::new();
    loop {
        let mut byte = [0];
        stream.read_exact(&mut byte).unwrap();
        if byte[0] == b'\n' {
            assert_eq!(line.pop(), Some(b'\r'));
            output.extend_from_slice(&line);
            output.extend_from_slice(b"\r\n");
            return line;
        }
        line.push(byte[0]);
    }
}

fn read_frame(stream: &mut TcpStream, depth: usize) -> Vec<u8> {
    assert!(depth <= 8);
    let mut output = Vec::new();
    let mut prefix = [0];
    stream.read_exact(&mut prefix).unwrap();
    output.push(prefix[0]);
    match prefix[0] {
        b'+' | b'-' | b':' => {
            read_line(stream, &mut output);
        }
        b'$' => {
            let length: i64 = std::str::from_utf8(&read_line(stream, &mut output))
                .unwrap()
                .parse()
                .unwrap();
            if length >= 0 {
                let mut body = vec![0; length as usize + 2];
                stream.read_exact(&mut body).unwrap();
                output.extend_from_slice(&body);
            }
        }
        b'*' => {
            let count: i64 = std::str::from_utf8(&read_line(stream, &mut output))
                .unwrap()
                .parse()
                .unwrap();
            if count >= 0 {
                for _ in 0..count {
                    output.extend_from_slice(&read_frame(stream, depth + 1));
                }
            }
        }
        other => panic!("unexpected RESP prefix {other:?}"),
    }
    output
}

/// Read one pub/sub delivery as `(channel, message)`, skipping the confirmation
/// frame consumed separately by [`subscribe`].
fn message(stream: &mut TcpStream) -> (String, String) {
    let frame = read_frame(stream, 0);
    let text = String::from_utf8_lossy(&frame).into_owned();
    let parts: Vec<&str> = text.split("\r\n").collect();
    match parts.first().copied() {
        // Pattern subscriptions deliver `pmessage` with pattern, channel, payload.
        Some("*3") => {
            assert_eq!(parts[2], "message", "unexpected frame: {text}");
            (parts[4].to_owned(), parts[6].to_owned())
        }
        Some("*4") => {
            assert_eq!(parts[2], "pmessage", "unexpected frame: {text}");
            (parts[6].to_owned(), parts[8].to_owned())
        }
        other => panic!("not a pub/sub message ({other:?}): {text}"),
    }
}

fn subscribe(stream: &mut TcpStream) {
    send(stream, &[b"PSUBSCRIBE", b"__key*@0__:*"]);
    let frame = read_frame(stream, 0);
    let text = String::from_utf8_lossy(&frame).into_owned();
    assert!(text.contains("psubscribe"), "subscribe failed: {text}");
}

fn expect_nothing(stream: &mut TcpStream) {
    stream
        .set_read_timeout(Some(Duration::from_millis(600)))
        .unwrap();
    let mut byte = [0u8; 1];
    let result = stream.read(&mut byte);
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    assert!(
        result.is_err(),
        "expected no notification, received {:?}",
        byte
    );
}

#[test]
fn mutations_publish_one_ordered_event_per_channel() {
    let server = start();
    let mut subscriber = connect(&server);
    let mut client = connect(&server);
    subscribe(&mut subscriber);

    send(&mut client, &[b"SET", b"k", b"one"]);
    assert_eq!(read_frame(&mut client, 0), b"+OK\r\n");
    assert_eq!(
        message(&mut subscriber),
        ("__keyspace@0__:k".into(), "set".into())
    );
    assert_eq!(
        message(&mut subscriber),
        ("__keyevent@0__:set".into(), "k".into())
    );

    send(&mut client, &[b"SET", b"k", b"two"]);
    assert_eq!(read_frame(&mut client, 0), b"+OK\r\n");
    assert_eq!(
        message(&mut subscriber),
        ("__keyspace@0__:k".into(), "set".into())
    );
    assert_eq!(
        message(&mut subscriber),
        ("__keyevent@0__:set".into(), "k".into())
    );

    send(&mut client, &[b"DEL", b"k"]);
    assert_eq!(read_frame(&mut client, 0), b":1\r\n");
    assert_eq!(
        message(&mut subscriber),
        ("__keyspace@0__:k".into(), "del".into())
    );
    assert_eq!(
        message(&mut subscriber),
        ("__keyevent@0__:del".into(), "k".into())
    );

    // A no-op delete publishes nothing.
    send(&mut client, &[b"DEL", b"k"]);
    assert_eq!(read_frame(&mut client, 0), b":0\r\n");
    expect_nothing(&mut subscriber);

    drop(client);
    drop(subscriber);
    server.shutdown().unwrap();
}

#[test]
fn a_reconnected_subscriber_sees_only_events_published_after_it_subscribed() {
    let server = start();
    let mut first = connect(&server);
    let mut client = connect(&server);
    subscribe(&mut first);

    send(&mut client, &[b"SET", b"before", b"v"]);
    assert_eq!(read_frame(&mut client, 0), b"+OK\r\n");
    assert_eq!(
        message(&mut first),
        ("__keyspace@0__:before".into(), "set".into())
    );
    assert_eq!(
        message(&mut first),
        ("__keyevent@0__:set".into(), "before".into())
    );

    // Disconnect, then mutate while nobody is subscribed: the events are lost,
    // not queued for the next subscriber.
    drop(first);
    send(&mut client, &[b"SET", b"missed", b"v"]);
    assert_eq!(read_frame(&mut client, 0), b"+OK\r\n");
    send(&mut client, &[b"DEL", b"missed"]);
    assert_eq!(read_frame(&mut client, 0), b":1\r\n");

    let mut second = connect(&server);
    subscribe(&mut second);
    expect_nothing(&mut second);

    send(&mut client, &[b"SET", b"after", b"v"]);
    assert_eq!(read_frame(&mut client, 0), b"+OK\r\n");
    assert_eq!(
        message(&mut second),
        ("__keyspace@0__:after".into(), "set".into())
    );
    assert_eq!(
        message(&mut second),
        ("__keyevent@0__:set".into(), "after".into())
    );
    expect_nothing(&mut second);

    drop(client);
    drop(second);
    server.shutdown().unwrap();
}

#[test]
fn the_unsupported_flush_family_stays_silent_without_desynchronising() {
    let server = start();
    let mut subscriber = connect(&server);
    let mut client = connect(&server);
    subscribe(&mut subscriber);

    send(&mut client, &[b"SET", b"k", b"v"]);
    assert_eq!(read_frame(&mut client, 0), b"+OK\r\n");
    assert_eq!(
        message(&mut subscriber),
        ("__keyspace@0__:k".into(), "set".into())
    );
    assert_eq!(
        message(&mut subscriber),
        ("__keyevent@0__:set".into(), "k".into())
    );

    // `FLUSH*` event classes are unsupported and documented as such: the command
    // succeeds, publishes nothing, and the next supported mutation still arrives.
    send(&mut client, &[b"FLUSHDB"]);
    assert_eq!(read_frame(&mut client, 0), b"+OK\r\n");
    expect_nothing(&mut subscriber);

    send(&mut client, &[b"SET", b"k", b"v2"]);
    assert_eq!(read_frame(&mut client, 0), b"+OK\r\n");
    assert_eq!(
        message(&mut subscriber),
        ("__keyspace@0__:k".into(), "set".into())
    );
    assert_eq!(
        message(&mut subscriber),
        ("__keyevent@0__:set".into(), "k".into())
    );

    drop(client);
    drop(subscriber);
    server.shutdown().unwrap();
}
