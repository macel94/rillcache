#![forbid(unsafe_code)]

//! Runtime reload contract for the mutable settings (O5).
//!
//! `CONFIG SET` is the atomic reload path: every pair in one command is
//! validated against a candidate before anything is published, startup-only
//! names are rejected instead of silently ignored, and a failed batch leaves
//! the previous generation in place.

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

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[test]
fn mutable_settings_reload_atomically_and_startup_only_names_fail_closed() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut client = connect(&server);

        let initial = reply(&mut client, &[b"CONFIG", b"GET", b"list-max-listpack-size"]);
        assert!(contains(&initial, b"\r\n-2\r\n"), "{initial:?}");

        // A batch of mutable settings is published together.
        command(
            &mut client,
            &[
                b"CONFIG",
                b"SET",
                b"list-max-listpack-size",
                b"-3",
                b"hash-max-listpack-entries",
                b"0",
            ],
            b"+OK\r\n",
        );
        let applied = reply(
            &mut client,
            &[
                b"CONFIG",
                b"GET",
                b"list-max-listpack-size",
                b"hash-max-listpack-entries",
            ],
        );
        assert!(contains(&applied, b"\r\n-3\r\n"), "{applied:?}");
        assert!(contains(&applied, b"\r\n0\r\n"), "{applied:?}");

        // A startup-only name rejects the whole batch; the previous generation
        // stays in effect, so a failed reload never partially applies.
        let rejected = reply(
            &mut client,
            &[
                b"CONFIG",
                b"SET",
                b"list-max-listpack-size",
                b"-4",
                b"appendfilename",
                b"other.aof",
            ],
        );
        assert!(rejected.starts_with(b"-ERR"), "{rejected:?}");
        let retained = reply(
            &mut client,
            &[
                b"CONFIG",
                b"GET",
                b"list-max-listpack-size",
                b"hash-max-listpack-entries",
            ],
        );
        assert!(contains(&retained, b"\r\n-3\r\n"), "{retained:?}");
        assert!(contains(&retained, b"\r\n0\r\n"), "{retained:?}");

        // An invalid value in a later pair also rejects the earlier pairs.
        let invalid = reply(
            &mut client,
            &[
                b"CONFIG",
                b"SET",
                b"list-max-listpack-size",
                b"-5",
                b"hash-max-listpack-entries",
                b"-1",
            ],
        );
        assert!(invalid.starts_with(b"-ERR"), "{invalid:?}");
        let retained = reply(
            &mut client,
            &[
                b"CONFIG",
                b"GET",
                b"list-max-listpack-size",
                b"hash-max-listpack-entries",
            ],
        );
        assert!(contains(&retained, b"\r\n-3\r\n"), "{retained:?}");
        assert!(contains(&retained, b"\r\n0\r\n"), "{retained:?}");

        // The startup file itself is read-only at runtime.
        let rewrite = reply(&mut client, &[b"CONFIG", b"REWRITE"]);
        assert!(rewrite.starts_with(b"-ERR"), "{rewrite:?}");
        command(&mut client, &[b"PING"], b"+PONG\r\n");
        server.shutdown().unwrap();
    }
}
