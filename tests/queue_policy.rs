#![forbid(unsafe_code)]

//! Slow-client and queue policy (O6).
//!
//! Output is bounded per connection: a subscriber that does not read is dropped
//! once its 1,024-frame queue fills, the drop is counted in
//! `INFO stats pubsub_queue_full_drops`, and neither the publisher nor a healthy
//! subscriber is blocked. Large multi-key replies are refused at the documented
//! 2 MiB MGET bound instead of growing without limit.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

fn start(workers: Option<usize>) -> ServerHandle {
    Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        worker_threads: workers,
        ..Config::default()
    })
    .unwrap()
    .spawn()
    .unwrap()
}

fn connect(server: &ServerHandle) -> TcpStream {
    let stream = TcpStream::connect(server.local_addr()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    stream.set_nodelay(true).unwrap();
    stream
}

fn read_frame(stream: &mut TcpStream, depth: usize) -> Vec<u8> {
    assert!(depth <= 8);
    let mut output = Vec::new();
    let mut prefix = [0u8; 1];
    stream.read_exact(&mut prefix).unwrap();
    output.push(prefix[0]);
    let mut line = Vec::new();
    loop {
        let mut byte = [0u8; 1];
        stream.read_exact(&mut byte).unwrap();
        if byte[0] == b'\n' {
            assert_eq!(line.pop(), Some(b'\r'));
            break;
        }
        line.push(byte[0]);
    }
    output.extend_from_slice(&line);
    output.extend_from_slice(b"\r\n");
    match prefix[0] {
        b'+' | b'-' | b':' => {}
        b'$' => {
            let length: i64 = std::str::from_utf8(&line).unwrap().parse().unwrap();
            if length >= 0 {
                let mut body = vec![0; length as usize + 2];
                stream.read_exact(&mut body).unwrap();
                output.extend_from_slice(&body);
            }
        }
        b'*' => {
            let count: i64 = std::str::from_utf8(&line).unwrap().parse().unwrap();
            for _ in 0..count.max(0) {
                output.extend_from_slice(&read_frame(stream, depth + 1));
            }
        }
        other => panic!("unexpected RESP prefix {other:?}"),
    }
    output
}

fn reply(stream: &mut TcpStream, args: &[&[u8]]) -> Vec<u8> {
    let mut request = Vec::new();
    encode_request(args, &mut request);
    stream.write_all(&request).unwrap();
    read_frame(stream, 0)
}

fn command(stream: &mut TcpStream, args: &[&[u8]], expected: &[u8]) {
    assert_eq!(reply(stream, args), expected, "{args:?}");
}

fn drops(control: &mut TcpStream) -> u64 {
    let stats = reply(control, &[b"INFO", b"stats"]);
    let text = String::from_utf8_lossy(&stats);
    text.lines()
        .find_map(|line| line.strip_prefix("pubsub_queue_full_drops:"))
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0)
}

#[test]
fn slow_subscribers_are_bounded_and_counted_without_blocking_others() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut slow = connect(&server);
        command(
            &mut slow,
            &[b"SUBSCRIBE", b"flood"],
            b"*3\r\n$9\r\nsubscribe\r\n$5\r\nflood\r\n:1\r\n",
        );
        let mut healthy = connect(&server);
        command(
            &mut healthy,
            &[b"SUBSCRIBE", b"live"],
            b"*3\r\n$9\r\nsubscribe\r\n$4\r\nlive\r\n:1\r\n",
        );
        let mut publisher = connect(&server);
        let mut control = connect(&server);

        // The slow subscriber never reads. Publishing more than its 1,024-frame
        // queue can hold must drop it and count the drop instead of blocking the
        // publisher or growing without bound.
        let payload = vec![b'q'; 8 * 1024];
        let mut batch = Vec::new();
        for _ in 0..2000 {
            encode_request(&[b"PUBLISH", b"flood", &payload], &mut batch);
        }
        publisher.write_all(&batch).unwrap();
        for _ in 0..2000 {
            let reply = read_frame(&mut publisher, 0);
            assert!(reply.starts_with(b":"), "{reply:?}");
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while drops(&mut control) == 0 {
            assert!(
                Instant::now() < deadline,
                "the full subscriber queue was never reported as dropped"
            );
            std::thread::sleep(Duration::from_millis(20));
        }

        // A healthy subscriber is unaffected by the slow one.
        command(&mut publisher, &[b"PUBLISH", b"live", b"ok"], b":1\r\n");
        let message = read_frame(&mut healthy, 0);
        assert_eq!(
            message,
            b"*3\r\n$7\r\nmessage\r\n$4\r\nlive\r\n$2\r\nok\r\n"
        );
        server.shutdown().unwrap();
    }
}

#[test]
fn oversized_multi_key_replies_are_refused_at_the_documented_bound() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut client = connect(&server);
        let value = vec![b'v'; 512 * 1024];
        command(&mut client, &[b"SET", b"big", &value], b"+OK\r\n");
        // Eight half-megabyte values exceed the 2 MiB MGET reply bound.
        let keys: Vec<&[u8]> = vec![b"big"; 8];
        let mut arguments: Vec<&[u8]> = vec![b"MGET"];
        arguments.extend(keys);
        command(
            &mut client,
            &arguments,
            b"-ERR MGET reply limit exceeded\r\n",
        );
        command(&mut client, &[b"PING"], b"+PONG\r\n");
        command(&mut client, &[b"STRLEN", b"big"], b":524288\r\n");
        server.shutdown().unwrap();
    }
}
