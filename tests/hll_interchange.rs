#![forbid(unsafe_code)]

//! The HyperLogLog representation and interchange boundary (C10).
//!
//! The profile excludes Redis/Valkey HLL payload interchange and documents the
//! private representation. This test pins the observable consequences on the
//! wire: PFADD/PFCOUNT/PFMERGE work with bounded accuracy, PFDEBUG reports the
//! private `exact`→`dense` encodings with 2^14 registers, PFSELFTEST passes,
//! and the payload cannot be exported through DUMP/RESTORE/SAVE.

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
        .set_read_timeout(Some(Duration::from_secs(10)))
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

fn integer(stream: &mut TcpStream, args: &[&[u8]]) -> i64 {
    let reply = reply(stream, args);
    assert_eq!(reply.first(), Some(&b':'), "{args:?} -> {reply:?}");
    std::str::from_utf8(&reply[1..reply.len() - 2])
        .unwrap()
        .parse()
        .unwrap()
}

#[test]
fn private_hll_representation_and_excluded_interchange_are_pinned() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut client = connect(&server);

        // Small cardinalities are stored exactly; the private encoding name is
        // observable only through PFDEBUG, not through a Redis-compatible one.
        for value in [&b"a"[..], b"b", b"c"] {
            reply(&mut client, &[b"PFADD", b"hll", value]);
        }
        assert_eq!(integer(&mut client, &[b"PFCOUNT", b"hll"]), 3);
        assert_eq!(
            reply(&mut client, &[b"PFDEBUG", b"ENCODING", b"hll"]),
            b"$5\r\nexact\r\n"
        );
        assert!(reply(&mut client, &[b"PFDEBUG", b"GETREG", b"hll"]).starts_with(b"*16384\r\n"));
        assert_eq!(reply(&mut client, &[b"PFSELFTEST"]), b"+OK\r\n");

        // Crossing the exact limit switches to the private 2^14-register sketch,
        // whose estimate stays within the documented HLL error bound.
        let distinct = 6000;
        for index in 0..distinct {
            let member = format!("member-{index}");
            reply(&mut client, &[b"PFADD", b"sketch", member.as_bytes()]);
        }
        assert_eq!(
            reply(&mut client, &[b"PFDEBUG", b"ENCODING", b"sketch"]),
            b"$5\r\ndense\r\n"
        );
        let registers = reply(&mut client, &[b"PFDEBUG", b"GETREG", b"sketch"]);
        assert!(registers.starts_with(b"*16384\r\n"));
        let estimate = integer(&mut client, &[b"PFCOUNT", b"sketch"]);
        let error = (estimate - distinct as i64).unsigned_abs();
        assert!(
            error * 100 <= (distinct as u64) * 2,
            "estimate {estimate} is outside 2% of {distinct}"
        );

        // Interchange is excluded: the private payload is not exported or
        // imported through Redis serialization commands.
        let serialization = b"-ERR DUMP, RESTORE and MIGRATE are not supported by this bounded server; use the append-only journal or rillcache-backup\r\n";
        assert_eq!(reply(&mut client, &[b"DUMP", b"sketch"]), serialization);
        assert_eq!(
            reply(&mut client, &[b"RESTORE", b"copy", b"0", b"payload"]),
            serialization
        );
        assert_eq!(
            reply(&mut client, &[b"SAVE"]),
            b"-ERR RDB persistence is not supported by this server\r\n"
        );
        server.shutdown().unwrap();
    }
}
