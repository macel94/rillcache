#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

const MODES: [(Option<usize>, bool); 4] = [
    (None, false),
    (None, true),
    (Some(1), false),
    (Some(4), false),
];
const DENIED: &[u8] =
    b"-NOPERM keyspace-wide commands require an all-key ACL grant with the required access\r\n";

fn start(workers: Option<usize>, async_io: bool) -> ServerHandle {
    Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        worker_threads: workers,
        io_mode: if async_io {
            rillcache::server::IoMode::Tokio
        } else {
            rillcache::server::IoMode::Mio
        },
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
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream.set_nodelay(true).unwrap();
    stream
}

fn line(stream: &mut TcpStream, output: &mut Vec<u8>) -> String {
    let mut bytes = Vec::new();
    loop {
        let mut byte = [0];
        stream.read_exact(&mut byte).unwrap();
        if byte[0] == b'\n' {
            assert_eq!(bytes.pop(), Some(b'\r'));
            output.extend_from_slice(&bytes);
            output.extend_from_slice(b"\r\n");
            return String::from_utf8(bytes).unwrap();
        }
        bytes.push(byte[0]);
    }
}

fn frame(stream: &mut TcpStream) -> Vec<u8> {
    let mut output = Vec::new();
    let mut prefix = [0];
    stream.read_exact(&mut prefix).unwrap();
    output.push(prefix[0]);
    match prefix[0] {
        b'+' | b'-' | b':' => {
            line(stream, &mut output);
        }
        b'$' => {
            let size = line(stream, &mut output).parse::<i64>().unwrap();
            if size >= 0 {
                let mut bytes = vec![0; size as usize + 2];
                stream.read_exact(&mut bytes).unwrap();
                output.extend_from_slice(&bytes);
            }
        }
        b'*' => {
            let count = line(stream, &mut output).parse::<i64>().unwrap();
            for _ in 0..count.max(0) {
                output.extend_from_slice(&frame(stream));
            }
        }
        other => panic!("unexpected RESP prefix {other:?}"),
    }
    output
}

fn command(stream: &mut TcpStream, args: &[&[u8]], expected: &[u8]) {
    let mut request = Vec::new();
    encode_request(args, &mut request);
    stream.write_all(&request).unwrap();
    assert_eq!(frame(stream), expected, "{args:?}");
}

#[test]
fn keyspace_wide_acl_operations_fail_closed_without_all_key_access_in_every_mode() {
    for (workers, async_io) in MODES {
        let server = start(workers, async_io);
        let mut admin = connect(&server);
        command(&mut admin, &[b"SET", b"safe:one", b"one"], b"+OK\r\n");
        command(&mut admin, &[b"SET", b"private:two", b"two"], b"+OK\r\n");
        command(
            &mut admin,
            &[
                b"ACL", b"SETUSER", b"scoped", b"on", b"nopass", b"-@all", b"+@all", b"~safe:*",
            ],
            b"+OK\r\n",
        );
        command(
            &mut admin,
            &[
                b"ACL",
                b"SETUSER",
                b"read-all",
                b"on",
                b"nopass",
                b"-@all",
                b"+@all",
                b"%R~*",
            ],
            b"+OK\r\n",
        );
        command(
            &mut admin,
            &[
                b"ACL",
                b"SETUSER",
                b"write-all",
                b"on",
                b"nopass",
                b"-@all",
                b"+@all",
                b"%W~*",
            ],
            b"+OK\r\n",
        );
        command(
            &mut admin,
            &[
                b"ACL",
                b"SETUSER",
                b"selector-scoped",
                b"on",
                b"nopass",
                b"-@all",
                b"+@all",
                b"~safe:*",
                b"(+@all ~private:*)",
            ],
            b"+OK\r\n",
        );
        command(
            &mut admin,
            &[
                b"ACL",
                b"SETUSER",
                b"selector-all",
                b"on",
                b"nopass",
                b"-@all",
                b"(+@all ~*)",
            ],
            b"+OK\r\n",
        );

        let mut scoped = connect(&server);
        command(&mut scoped, &[b"AUTH", b"scoped", b""], b"+OK\r\n");
        for args in [
            &[b"KEYS".as_slice(), b"*"][..],
            &[b"SCAN".as_slice(), b"0"],
            &[b"RANDOMKEY".as_slice()][..],
            &[b"DBSIZE".as_slice()][..],
            &[b"FLUSHDB".as_slice()][..],
            &[b"FLUSHALL".as_slice(), b"ASYNC"],
        ] {
            command(&mut scoped, args, DENIED);
            let mut dryrun = vec![b"ACL".as_slice(), b"DRYRUN", b"scoped", args[0]];
            dryrun.extend_from_slice(&args[1..]);
            command(&mut admin, &dryrun, DENIED);
        }
        // Both reads and writes outside the scoped pattern survive all denials.
        command(&mut admin, &[b"GET", b"safe:one"], b"$3\r\none\r\n");
        command(&mut admin, &[b"GET", b"private:two"], b"$3\r\ntwo\r\n");
        command(&mut admin, &[b"DBSIZE"], b":2\r\n");
        let mut selector_scoped = connect(&server);
        command(
            &mut selector_scoped,
            &[b"AUTH", b"selector-scoped", b""],
            b"+OK\r\n",
        );
        command(&mut selector_scoped, &[b"KEYS", b"*"], DENIED);
        command(
            &mut admin,
            &[b"ACL", b"DRYRUN", b"selector-scoped", b"FLUSHALL"],
            DENIED,
        );
        let mut selector_all = connect(&server);
        command(
            &mut selector_all,
            &[b"AUTH", b"selector-all", b""],
            b"+OK\r\n",
        );
        command(
            &mut selector_all,
            &[b"KEYS", b"*"],
            b"*2\r\n$11\r\nprivate:two\r\n$8\r\nsafe:one\r\n",
        );

        // A read-all grant allows introspection but not destructive flush.
        let mut reader = connect(&server);
        command(&mut reader, &[b"AUTH", b"read-all", b""], b"+OK\r\n");
        command(&mut reader, &[b"DBSIZE"], b":2\r\n");
        command(&mut reader, &[b"FLUSHDB"], DENIED);
        // A write-all grant can flush but cannot enumerate the resulting keyspace.
        let mut writer = connect(&server);
        command(&mut writer, &[b"AUTH", b"write-all", b""], b"+OK\r\n");
        command(&mut writer, &[b"KEYS", b"*"], DENIED);
        command(
            &mut admin,
            &[b"ACL", b"DRYRUN", b"write-all", b"FLUSHDB"],
            b"+OK\r\n",
        );

        // Root/all-key access retains upstream keyspace behavior.
        command(
            &mut admin,
            &[b"KEYS", b"safe:*"],
            b"*1\r\n$8\r\nsafe:one\r\n",
        );
        command(&mut admin, &[b"DBSIZE"], b":2\r\n");
        command(&mut admin, &[b"FLUSHDB", b"SYNC"], b"+OK\r\n");
        command(&mut admin, &[b"DBSIZE"], b":0\r\n");
        server.shutdown().unwrap();
    }
}
