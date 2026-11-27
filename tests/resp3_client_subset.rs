#![forbid(unsafe_code)]

//! The RESP3 subset used by the pinned client matrix (C6).
//!
//! The supported clients authenticate, round-trip strings and integers, run
//! pipelines and transactions, subscribe to Pub/Sub and reconnect. This test
//! pins the wire types those operations actually see under `HELLO 3`, the
//! documented bounded behaviours (array replies where Redis uses maps, no
//! attributes or client tracking), the explicit errors for excluded features,
//! and fragmentation of the handshake request itself.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

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
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream.set_nodelay(true).unwrap();
    stream
}

fn read_line(stream: &mut TcpStream, output: &mut Vec<u8>) -> String {
    let mut line = Vec::new();
    loop {
        let mut byte = [0u8; 1];
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

fn read_frame(stream: &mut TcpStream, depth: usize) -> Vec<u8> {
    assert!(depth <= 8);
    let mut output = Vec::new();
    let mut prefix = [0u8; 1];
    stream.read_exact(&mut prefix).unwrap();
    output.push(prefix[0]);
    match prefix[0] {
        b'+' | b'-' | b':' | b',' | b'#' | b'_' => {
            read_line(stream, &mut output);
        }
        b'$' | b'=' => {
            let length = read_line(stream, &mut output).parse::<i64>().unwrap();
            if length >= 0 {
                let mut body = vec![0; length as usize + 2];
                stream.read_exact(&mut body).unwrap();
                output.extend_from_slice(&body);
            }
        }
        b'*' | b'~' | b'>' | b'%' => {
            let count = read_line(stream, &mut output).parse::<i64>().unwrap();
            for _ in 0..count.max(0) * if prefix[0] == b'%' { 2 } else { 1 } {
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

#[test]
fn client_matrix_resp3_types_and_excluded_features_are_pinned() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut client = connect(&server);

        // The handshake request is fragmented across writes and must reassemble
        // exactly like an unfragmented one.
        let mut hello = Vec::new();
        encode_request(&[b"HELLO", b"3"], &mut hello);
        for chunk in hello.chunks(3) {
            client.write_all(chunk).unwrap();
        }
        let mut prefix = [0u8; 4];
        client.read_exact(&mut prefix).unwrap();
        assert_eq!(&prefix, b"%5\r\n");
        for _ in 0..10 {
            let frame = read_frame(&mut client, 1);
            assert!(
                frame.starts_with(b"$") || frame.starts_with(b":"),
                "unexpected HELLO value {frame:?}"
            );
        }

        // The matrix round-trip uses simple strings, integers, bulk strings and
        // arrays; those keep their RESP2-compatible types under RESP3.
        command(&mut client, &[b"PING"], b"+PONG\r\n");
        command(&mut client, &[b"SET", b"k", b"v\x00"], b"+OK\r\n");
        command(&mut client, &[b"GET", b"k"], b"$2\r\nv\x00\r\n");
        command(&mut client, &[b"INCR", b"counter"], b":1\r\n");
        command(&mut client, &[b"HSET", b"h", b"f", b"v"], b":1\r\n");
        // Redis returns a map for HGETALL under RESP3; this profile returns the
        // documented bounded array, which the pinned clients accept.
        command(
            &mut client,
            &[b"HGETALL", b"h"],
            b"*2\r\n$1\r\nf\r\n$1\r\nv\r\n",
        );

        // Excluded RESP3 features are explicit, never silently "enabled".
        command(
            &mut client,
            &[b"CLIENT", b"TRACKING", b"on"],
            b"-ERR unsupported CLIENT subcommand or invalid arguments\r\n",
        );
        command(&mut client, &[b"CLIENT", b"GETREDIR"], b":-1\r\n");
        command(
            &mut client,
            &[b"HELLO", b"4"],
            b"-NOPROTO unsupported protocol version\r\n",
        );

        // Pub/Sub uses push frames for the supported subscriber case.
        let mut subscriber = connect(&server);
        let mut hello = Vec::new();
        encode_request(&[b"HELLO", b"3"], &mut hello);
        subscriber.write_all(&hello).unwrap();
        let mut map = [0u8; 4];
        subscriber.read_exact(&mut map).unwrap();
        assert_eq!(&map, b"%5\r\n");
        for _ in 0..10 {
            read_frame(&mut subscriber, 1);
        }
        command(
            &mut subscriber,
            &[b"SUBSCRIBE", b"news"],
            b">3\r\n$9\r\nsubscribe\r\n$4\r\nnews\r\n:1\r\n",
        );
        command(&mut client, &[b"PUBLISH", b"news", b"payload"], b":1\r\n");
        command_expected(
            &mut subscriber,
            b">3\r\n$7\r\nmessage\r\n$4\r\nnews\r\n$7\r\npayload\r\n",
        );
        server.shutdown().unwrap();
    }
}

fn command_expected(stream: &mut TcpStream, expected: &[u8]) {
    let actual = read_frame(stream, 0);
    assert_eq!(actual, expected);
}
