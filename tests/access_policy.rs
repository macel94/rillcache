#![forbid(unsafe_code)]

//! Eviction and access-statistics policy (O2).
//!
//! The selected policies are the documented LRU/random/TTL approximations and
//! `noeviction`. LFU and read-idle statistics are not implemented: `OBJECT FREQ`
//! and `OBJECT IDLETIME` stay explicit exclusions rather than returning invented
//! values, and the selected policy is observable through `CONFIG GET` and
//! `INFO memory`.

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
fn access_statistics_are_explicit_exclusions_and_the_policy_is_observable() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut client = connect(&server);
        command(&mut client, &[b"SET", b"key", b"value"], b"+OK\r\n");

        // LFU and read-idle time are not tracked, and the server says so instead
        // of inventing a frequency or idle value.
        command(
            &mut client,
            &[b"OBJECT", b"IDLETIME", b"key"],
            b"-ERR OBJECT IDLETIME is not supported by this bounded server: read-idle time is not tracked\r\n",
        );
        command(
            &mut client,
            &[b"OBJECT", b"FREQ", b"key"],
            b"-ERR An LFU maxmemory policy is not selected, access frequency not tracked. Please note that when switching between maxmemory policies at runtime LFU and LRU data will take some time to adjust.\r\n",
        );

        // The selected policy is readable, mutable at runtime and observable.
        let initial = reply(&mut client, &[b"CONFIG", b"GET", b"maxmemory-policy"]);
        assert!(contains(&initial, b"\r\nnoeviction\r\n"), "{initial:?}");
        command(
            &mut client,
            &[b"CONFIG", b"SET", b"maxmemory-policy", b"allkeys-lru"],
            b"+OK\r\n",
        );
        let selected = reply(&mut client, &[b"CONFIG", b"GET", b"maxmemory-policy"]);
        assert!(contains(&selected, b"\r\nallkeys-lru\r\n"), "{selected:?}");
        let memory = reply(&mut client, &[b"INFO", b"memory"]);
        assert!(
            contains(&memory, b"maxmemory_policy:allkeys-lru"),
            "INFO memory did not expose the policy"
        );

        // LFU is not a selectable policy; the error is explicit.
        command(
            &mut client,
            &[b"CONFIG", b"SET", b"maxmemory-policy", b"allkeys-lfu"],
            b"-ERR invalid maxmemory-policy\r\n",
        );
        let retained = reply(&mut client, &[b"CONFIG", b"GET", b"maxmemory-policy"]);
        assert!(contains(&retained, b"\r\nallkeys-lru\r\n"), "{retained:?}");
        server.shutdown().unwrap();
    }
}
