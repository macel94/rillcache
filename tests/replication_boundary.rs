#![forbid(unsafe_code)]

//! A1 exclusion evidence: the standalone replication boundary, end to end.
//!
//! The declared profile excludes replication, so the boundary that has to hold is
//! that no replication link can form and no replica state can be faked:
//!
//! * every replication entry point (`REPLICAOF`, `SLAVEOF`, `PSYNC`, `SYNC`,
//!   `REPLCONF`, `FAILOVER`) fails with the explicit unsupported error and leaves
//!   the connection usable;
//! * `ROLE`/`INFO replication` stay master with zero connected replicas and a
//!   zero stream offset, before and after local writes;
//! * the declared `SLAVEOF NO ONE` no-op does not change any state;
//! * a second *rillcache* instance told to replicate from the first stays an
//!   independent master with its own data, so no silent partial link, snapshot
//!   transfer or split-brain state can appear;
//! * `WAIT` reports zero acknowledged replicas and `WAITAOF` acknowledges only
//!   local journal state.
//!
//! A pinned native Redis/Valkey replica cannot attach either; that two-end
//! evidence runs in the hosted `Replication exclusion` job because the pinned
//! reference binaries are built there.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

const REPLICATION_DISABLED: &[u8] =
    b"-ERR replication is not supported by this standalone server\r\n";

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
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream.set_nodelay(true).unwrap();
    stream
}

fn read_line(stream: &mut TcpStream, output: &mut Vec<u8>) -> String {
    let mut line = Vec::new();
    loop {
        let mut byte = [0];
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
                let mut body = vec![0; usize::try_from(length).unwrap() + 2];
                stream.read_exact(&mut body).unwrap();
                assert!(body.ends_with(b"\r\n"));
                output.extend_from_slice(&body);
            }
        }
        b'*' => {
            let count = read_line(stream, &mut output).parse::<i64>().unwrap();
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

fn command(stream: &mut TcpStream, args: &[&[u8]], expected: &[u8]) {
    let mut request = Vec::new();
    encode_request(args, &mut request);
    stream.write_all(&request).unwrap();
    assert_eq!(read_frame(stream, 0), expected, "{args:?}");
}

fn bulk_text(reply: &[u8]) -> String {
    assert_eq!(reply.first(), Some(&b'$'), "{reply:?}");
    let header_end = reply
        .windows(2)
        .position(|window| window == b"\r\n")
        .unwrap();
    let length = std::str::from_utf8(&reply[1..header_end])
        .unwrap()
        .parse::<usize>()
        .unwrap();
    std::str::from_utf8(&reply[header_end + 2..header_end + 2 + length])
        .unwrap()
        .to_string()
}

fn replication_info(stream: &mut TcpStream) -> String {
    let mut request = Vec::new();
    encode_request(&[b"INFO", b"replication"], &mut request);
    stream.write_all(&request).unwrap();
    bulk_text(&read_frame(stream, 0))
}

fn assert_standalone(stream: &mut TcpStream, offset: &'static str) {
    command(
        stream,
        &[b"ROLE"],
        format!("*3\r\n$6\r\nmaster\r\n:{offset}\r\n*0\r\n").as_bytes(),
    );
    let info = replication_info(stream);
    assert!(info.contains("role:master\r\n"), "{info}");
    assert!(info.contains("connected_slaves:0\r\n"), "{info}");
    assert!(
        info.contains(&format!("master_repl_offset:{offset}\r\n")),
        "{info}"
    );
    assert!(!info.contains("slave0"), "{info}");
    assert!(!info.contains("master_link_status"), "{info}");
}

#[test]
fn replication_entry_points_fail_closed_and_keep_the_standalone_state() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = connect(&server);
        assert_standalone(&mut stream, "0");

        // A local write must not create a replication stream or offset.
        command(&mut stream, &[b"SET", b"local", b"value"], b"+OK\r\n");
        assert_standalone(&mut stream, "0");

        for args in [
            &[b"REPLICAOF".as_slice(), b"127.0.0.1", b"1"] as &[&[u8]],
            &[b"SLAVEOF", b"127.0.0.1", b"1"],
            &[b"PSYNC", b"?", b"-1"],
            &[b"PSYNC", b"some-replid", b"42"],
            &[b"SYNC"],
            &[b"REPLCONF", b"listening-port", b"6380"],
            &[b"REPLCONF", b"ACK", b"0"],
            &[b"FAILOVER"],
        ] {
            command(&mut stream, args, REPLICATION_DISABLED);
            // A refused reconfiguration must not poison the connection.
            command(&mut stream, &[b"PING"], b"+PONG\r\n");
            command(&mut stream, &[b"GET", b"local"], b"$5\r\nvalue\r\n");
            assert_standalone(&mut stream, "0");
        }

        // The declared no-op form is acknowledged but changes nothing.
        command(&mut stream, &[b"SLAVEOF", b"NO", b"ONE"], b"+OK\r\n");
        assert_standalone(&mut stream, "0");

        // Arity stays the normal protocol error, not a fake replication state.
        command(
            &mut stream,
            &[b"REPLICAOF", b"only-one-argument"],
            b"-ERR wrong number of arguments for 'replicaof' command\r\n",
        );
        command(
            &mut stream,
            &[b"PSYNC", b"?"],
            b"-ERR wrong number of arguments for 'psync' command\r\n",
        );
        command(
            &mut stream,
            &[b"SYNC", b"extra"],
            b"-ERR wrong number of arguments for 'sync' command\r\n",
        );

        // WAIT/WAITAOF cannot acknowledge a replica that does not exist.
        command(&mut stream, &[b"WAIT", b"0", b"0"], b":0\r\n");
        command(&mut stream, &[b"WAIT", b"1", b"10"], b":0\r\n");
        // The AOF is disabled in this configuration, so WAITAOF reports only
        // that no local fsync happened; it never reports replica acknowledgements.
        command(
            &mut stream,
            &[b"WAITAOF", b"0", b"1", b"10"],
            b"*2\r\n:0\r\n:0\r\n",
        );

        server.shutdown().unwrap();
    }
}

#[test]
fn a_second_instance_cannot_attach_and_keeps_its_own_data() {
    let primary = start(None);
    let secondary = start(None);
    let mut primary_stream = connect(&primary);
    let mut secondary_stream = connect(&secondary);

    command(
        &mut primary_stream,
        &[b"SET", b"primary-only", b"one"],
        b"+OK\r\n",
    );
    // The secondary is told to replicate from the primary and must refuse.
    let primary_addr = primary.local_addr().to_string();
    let (_, port) = primary_addr.rsplit_once(':').unwrap();
    command(
        &mut secondary_stream,
        &[b"REPLICAOF", b"127.0.0.1", port.as_bytes()],
        REPLICATION_DISABLED,
    );
    command(
        &mut secondary_stream,
        &[b"SLAVEOF", b"127.0.0.1", port.as_bytes()],
        REPLICATION_DISABLED,
    );
    assert_standalone(&mut secondary_stream, "0");

    // Each instance keeps its own keyspace: no snapshot, no apply, no link.
    command(
        &mut secondary_stream,
        &[b"SET", b"secondary-only", b"two"],
        b"+OK\r\n",
    );
    command(
        &mut secondary_stream,
        &[b"GET", b"primary-only"],
        b"$-1\r\n",
    );
    command(
        &mut primary_stream,
        &[b"GET", b"secondary-only"],
        b"$-1\r\n",
    );
    command(
        &mut primary_stream,
        &[b"GET", b"primary-only"],
        b"$3\r\none\r\n",
    );
    command(
        &mut secondary_stream,
        &[b"GET", b"secondary-only"],
        b"$3\r\ntwo\r\n",
    );
    assert_standalone(&mut primary_stream, "0");
    assert_standalone(&mut secondary_stream, "0");

    primary.shutdown().unwrap();
    secondary.shutdown().unwrap();
}
