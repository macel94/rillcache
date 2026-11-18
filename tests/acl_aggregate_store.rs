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
const NOPERM: &[u8] =
    b"-NOPERM this user has no permissions to access one of the keys used as arguments\r\n";

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
                let mut data = vec![0; size as usize + 2];
                stream.read_exact(&mut data).unwrap();
                assert!(data.ends_with(b"\r\n"));
                output.extend_from_slice(&data);
            }
        }
        b'*' => {
            let count = line(stream, &mut output).parse::<i64>().unwrap();
            if count >= 0 {
                for _ in 0..count {
                    output.extend_from_slice(&frame(stream));
                }
            }
        }
        other => panic!("unexpected RESP prefix {other:?}"),
    }
    output
}

fn command(stream: &mut TcpStream, args: &[&[u8]], expected: &[u8]) {
    let mut encoded = Vec::new();
    encode_request(args, &mut encoded);
    stream.write_all(&encoded).unwrap();
    assert_eq!(frame(stream), expected, "{args:?}");
}

#[test]
fn aggregate_store_acl_checks_all_declared_sources_and_destinations_in_every_mode() {
    for (workers, async_io) in MODES {
        let server = start(workers, async_io);
        let mut admin = connect(&server);
        let source_a = b"src:set:\0a";
        let source_b = b"src:set:\xffb";
        let source_denied = b"private:set";
        let member_binary = b"member\0\xff";

        command(
            &mut admin,
            &[b"SADD", source_a, member_binary, b"shared"],
            b":2\r\n",
        );
        command(
            &mut admin,
            &[b"SADD", source_b, member_binary, b"other"],
            b":2\r\n",
        );
        command(&mut admin, &[b"SADD", source_denied, b"hidden"], b":1\r\n");
        command(
            &mut admin,
            &[b"SET", b"outside:set-dest", b"sentinel"],
            b"+OK\r\n",
        );
        command(&mut admin, &[b"SADD", b"dst:set", b"previous"], b":1\r\n");

        let z_a = b"src:zset:\0a";
        let z_b = b"src:zset:\xffb";
        let z_private = b"private:zset";
        command(
            &mut admin,
            &[b"ZADD", z_a, b"1", member_binary, b"2", b"shared"],
            b":2\r\n",
        );
        command(
            &mut admin,
            &[b"ZADD", z_b, b"3", member_binary, b"4", b"other"],
            b":2\r\n",
        );
        command(
            &mut admin,
            &[b"ZADD", z_private, b"1", b"hidden"],
            b":1\r\n",
        );
        command(
            &mut admin,
            &[b"ZADD", b"dst:zset", b"0", b"previous"],
            b":1\r\n",
        );
        command(
            &mut admin,
            &[b"ZADD", b"outside:zset-dest", b"0", b"sentinel"],
            b":1\r\n",
        );

        command(
            &mut admin,
            &[b"GEOADD", b"src:geo", b"0", b"0", b"origin"],
            b":1\r\n",
        );
        command(
            &mut admin,
            &[b"GEOADD", b"private:geo", b"0", b"0", b"hidden"],
            b":1\r\n",
        );
        command(
            &mut admin,
            &[b"GEOADD", b"dst:geo", b"0", b"0", b"previous"],
            b":1\r\n",
        );
        command(
            &mut admin,
            &[b"GEOADD", b"outside:geo-dest", b"0", b"0", b"sentinel"],
            b":1\r\n",
        );

        command(
            &mut admin,
            &[
                b"ACL",
                b"SETUSER",
                b"aggregator",
                b"on",
                b"nopass",
                b"-@all",
                b"+@all",
                b"%R~src:*",
                b"%W~dst:*",
            ],
            b"+OK\r\n",
        );
        let mut user = connect(&server);
        command(&mut user, &[b"AUTH", b"aggregator", b""], b"+OK\r\n");

        // Set algebra uses its registered destination-plus-source key spec.
        command(
            &mut user,
            &[b"SUNIONSTORE", b"dst:set", source_a, source_b],
            b":3\r\n",
        );
        command(&mut admin, &[b"SCARD", b"dst:set"], b":3\r\n");
        command(&mut user, &[b"SCARD", b"dst:set"], NOPERM);
        command(
            &mut user,
            &[b"SINTERSTORE", b"dst:inter", source_a, source_b],
            b":1\r\n",
        );
        command(
            &mut user,
            &[b"SDIFFSTORE", b"dst:diff", source_a, source_b],
            b":1\r\n",
        );

        // A denied input or output rejects the command before STORE changes data.
        command(
            &mut user,
            &[b"SUNIONSTORE", b"dst:set", source_a, source_denied],
            NOPERM,
        );
        command(&mut admin, &[b"SCARD", b"dst:set"], b":3\r\n");
        command(
            &mut user,
            &[b"SUNIONSTORE", b"outside:set-dest", source_a, source_b],
            NOPERM,
        );
        command(
            &mut admin,
            &[b"GET", b"outside:set-dest"],
            b"$8\r\nsentinel\r\n",
        );
        command(
            &mut admin,
            &[b"SMEMBERS", b"outside:set-dest"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );

        // Counted zset stores take destination, numkeys, then exactly numkeys sources.
        command(
            &mut user,
            &[b"ZUNIONSTORE", b"dst:zset", b"2", z_a, z_b],
            b":3\r\n",
        );
        command(
            &mut user,
            &[b"ZINTERSTORE", b"dst:zinter", b"2", z_a, z_b],
            b":1\r\n",
        );
        command(
            &mut user,
            &[b"ZDIFFSTORE", b"dst:zdiff", b"2", z_a, z_b],
            b":1\r\n",
        );
        command(
            &mut user,
            &[b"ZUNIONSTORE", b"dst:zset", b"2", z_a, z_private],
            NOPERM,
        );
        command(&mut admin, &[b"ZCARD", b"dst:zset"], b":3\r\n");
        command(
            &mut user,
            &[b"ZUNIONSTORE", b"outside:zset-dest", b"2", z_a, z_b],
            NOPERM,
        );
        command(
            &mut admin,
            &[b"ZSCORE", b"outside:zset-dest", b"sentinel"],
            b"$1\r\n0\r\n",
        );

        // Other multi-key STORE specs: range-store (destination/source) and geo-search-store.
        command(
            &mut user,
            &[b"ZRANGESTORE", b"dst:zrange", z_a, b"0", b"-1"],
            b":2\r\n",
        );
        command(
            &mut user,
            &[b"ZRANGESTORE", b"dst:zrange", z_private, b"0", b"-1"],
            NOPERM,
        );
        command(&mut admin, &[b"ZCARD", b"dst:zrange"], b":2\r\n");
        command(
            &mut user,
            &[b"ZRANGESTORE", b"outside:zset-dest", z_a, b"0", b"-1"],
            NOPERM,
        );
        command(
            &mut admin,
            &[b"ZSCORE", b"outside:zset-dest", b"sentinel"],
            b"$1\r\n0\r\n",
        );

        command(
            &mut user,
            &[
                b"GEOSEARCHSTORE",
                b"dst:geo",
                b"src:geo",
                b"FROMLONLAT",
                b"0",
                b"0",
                b"BYRADIUS",
                b"1",
                b"KM",
            ],
            b":1\r\n",
        );
        command(
            &mut user,
            &[
                b"GEOSEARCHSTORE",
                b"dst:geo",
                b"private:geo",
                b"FROMLONLAT",
                b"0",
                b"0",
                b"BYRADIUS",
                b"1",
                b"KM",
            ],
            NOPERM,
        );
        command(&mut admin, &[b"ZCARD", b"dst:geo"], b":1\r\n");
        command(
            &mut user,
            &[
                b"GEOSEARCHSTORE",
                b"outside:geo-dest",
                b"src:geo",
                b"FROMLONLAT",
                b"0",
                b"0",
                b"BYRADIUS",
                b"1",
                b"KM",
            ],
            NOPERM,
        );
        command(&mut admin, &[b"ZCARD", b"outside:geo-dest"], b":1\r\n");

        // ACL DRYRUN shares runtime extraction, including binary source keys.
        command(
            &mut admin,
            &[
                b"ACL",
                b"DRYRUN",
                b"aggregator",
                b"ZUNIONSTORE",
                b"dst:zset",
                b"2",
                z_a,
                z_private,
            ],
            NOPERM,
        );
        server.shutdown().unwrap();
    }
}
