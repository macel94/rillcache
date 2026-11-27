#![forbid(unsafe_code)]

//! Security acceptance gate for excluded distributed/scripted features (S10).
//!
//! Scripting, replication, failover and cluster routing are absent. Adding any
//! of them is a security change: each needs an explicit threat model, an
//! authentication/authorization path and negative tests before it can be
//! exposed. This test is the tripwire: while the features are absent, every
//! entry point must fail explicitly and leave data untouched; if one starts
//! succeeding, this test fails and the S10 gate must be satisfied first.

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

#[test]
fn excluded_distributed_and_scripted_features_stay_behind_the_security_gate() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut client = connect(&server);
        command(&mut client, &[b"SET", b"gate", b"value"], b"+OK\r\n");
        command(
            &mut client,
            &[b"ROLE"],
            b"*3\r\n$6\r\nmaster\r\n:0\r\n*0\r\n",
        );

        let absent = b"-ERR unknown command\r\n";
        let unsupported = b"-ERR unsupported subcommand or syntax error\r\n";
        let replication = b"-ERR replication is not supported by this standalone server\r\n";
        let cluster = b"-ERR This instance has cluster support disabled\r\n";
        let modules = b"-ERR MODULE loading is not supported by this server\r\n";

        // Scripting execution: no evaluation path may exist without the S10
        // threat model for untrusted code, CPU/memory bounds and key access.
        for args in [
            &[b"EVAL".as_slice(), b"return 1", b"0"][..],
            &[
                b"EVALSHA",
                b"0000000000000000000000000000000000000000",
                b"0",
            ],
            &[b"EVAL_RO", b"return 1", b"0"],
            &[b"FCALL", b"example", b"0"],
            &[b"FCALL_RO", b"example", b"0"],
        ] {
            command(&mut client, args, absent);
        }
        command(&mut client, &[b"SCRIPT", b"LOAD", b"return 1"], unsupported);
        command(
            &mut client,
            &[b"FUNCTION", b"LOAD", b"#!lua name=example\n"],
            unsupported,
        );

        // Replication/failover: no peer may attach, reconfigure or promote.
        for args in [
            &[b"REPLICAOF".as_slice(), b"127.0.0.1", b"6379"][..],
            &[b"SLAVEOF", b"127.0.0.1", b"6379"],
            &[b"REPLCONF", b"listening-port", b"6379"],
            &[b"PSYNC", b"?", b"-1"],
            &[b"SYNC"],
            &[b"FAILOVER"],
        ] {
            command(&mut client, args, replication);
        }

        // Cluster routing and module loading stay explicit refusals.
        command(&mut client, &[b"CLUSTER", b"SLOTS"], cluster);
        command(&mut client, &[b"CLUSTER", b"NODES"], cluster);
        command(&mut client, &[b"MODULE", b"LOAD", b"x.so"], modules);

        // None of the refused paths touched state.
        command(&mut client, &[b"GET", b"gate"], b"$5\r\nvalue\r\n");
        command(&mut client, &[b"DBSIZE"], b":1\r\n");
        server.shutdown().unwrap();
    }
}
