#![forbid(unsafe_code)]
#![cfg(target_os = "linux")]

//! A2 exclusion evidence: the standalone failover boundary.
//!
//! The declared profile excludes Sentinel and automatic failover. This drill
//! proves the operational consequences against real processes:
//!
//! * every `SENTINEL` form fails closed on the first call, so a Sentinel-
//!   compatible control plane cannot monitor, elect, promote or fence;
//! * `FAILOVER` and the host/port `REPLICAOF`/`SLAVEOF` promotion sources return
//!   the explicit unsupported error, while `REPLICAOF NO ONE`/`SLAVEOF NO ONE`
//!   stay the declared no-op promotion with no state change;
//! * `INFO replication` keeps `master_failover_state:no-failover` and never
//!   invents sentinel/replica rows;
//! * with two independent instances, killing the primary loses its address and
//!   its data, and the surviving instance does **not** take over: it stays a
//!   master over its own keyspace only. Recovery is the documented verified
//!   restore, not an election.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use rillcache::protocol::encode_request;

const REPLICATION_DISABLED: &[u8] =
    b"-ERR replication is not supported by this standalone server\r\n";
const UNKNOWN_COMMAND: &[u8] = b"-ERR unknown command\r\n";

struct ProcessServer {
    child: Child,
    port: u16,
}

impl ProcessServer {
    fn start(label: &str) -> Self {
        let directory =
            std::env::temp_dir().join(format!("rillcache-failover-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rillcache"))
            .args(["--bind", "127.0.0.1:0", "--dir"])
            .arg(&directory)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stderr = BufReader::new(child.stderr.take().unwrap());
        let mut line = String::new();
        stderr.read_line(&mut line).unwrap();
        let port = line
            .split(" listening on 127.0.0.1:")
            .nth(1)
            .expect("listener port")
            .trim()
            .parse()
            .expect("port number");
        Self { child, port }
    }

    fn connect(&self) -> TcpStream {
        let stream = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream.set_nodelay(true).unwrap();
        stream
    }

    /// Model node loss: kill without unwinding or a final flush, then reap.
    fn crash(mut self) {
        self.child.kill().unwrap();
        self.child.wait().unwrap();
    }
}

impl Drop for ProcessServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
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

fn command(stream: &mut TcpStream, args: &[&[u8]]) -> Vec<u8> {
    let mut request = Vec::new();
    encode_request(args, &mut request);
    stream.write_all(&request).unwrap();
    read_frame(stream, 0)
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

fn assert_standalone(stream: &mut TcpStream) {
    assert_eq!(
        command(stream, &[b"ROLE"]),
        b"*3\r\n$6\r\nmaster\r\n:0\r\n*0\r\n"
    );
    let info = bulk_text(&command(stream, &[b"INFO", b"replication"]));
    assert!(info.contains("role:master\r\n"), "{info}");
    assert!(info.contains("connected_slaves:0\r\n"), "{info}");
    assert!(
        info.contains("master_failover_state:no-failover\r\n"),
        "{info}"
    );
    assert!(info.contains("master_repl_offset:0\r\n"), "{info}");
    for absent in ["sentinel", "slave0", "master_link_status", "master_replid"] {
        assert!(!info.contains(absent), "{info}");
    }
}

#[test]
fn sentinel_and_promotion_entry_points_fail_closed() {
    let server = ProcessServer::start("entry-points");
    let mut stream = server.connect();
    assert_standalone(&mut stream);
    assert_eq!(
        command(&mut stream, &[b"SET", b"local", b"value"]),
        b"+OK\r\n"
    );
    assert_standalone(&mut stream);

    // A Sentinel-compatible control plane cannot do anything with this server:
    // every command form fails closed and the connection stays usable.
    for probe in [
        &[b"SENTINEL".as_slice()] as &[&[u8]],
        &[b"SENTINEL", b"masters"],
        &[b"SENTINEL", b"master", b"mymaster"],
        &[b"SENTINEL", b"get-master-addr-by-name", b"mymaster"],
        &[b"SENTINEL", b"sentinels", b"mymaster"],
        &[b"SENTINEL", b"ckquorum", b"mymaster"],
        &[b"SENTINEL", b"failover", b"mymaster"],
        &[
            b"SENTINEL",
            b"monitor",
            b"mymaster",
            b"127.0.0.1",
            b"1",
            b"1",
        ],
    ] {
        assert_eq!(command(&mut stream, probe), UNKNOWN_COMMAND, "{probe:?}");
        assert_eq!(command(&mut stream, &[b"PING"]), b"+PONG\r\n");
        assert_standalone(&mut stream);
    }

    // Operator-requested failover and replica promotion sources are refused.
    for probe in [
        &[b"FAILOVER".as_slice()] as &[&[u8]],
        &[b"FAILOVER", b"TO", b"127.0.0.1", b"1"],
        &[b"REPLICAOF", b"127.0.0.1", b"1"],
        &[b"SLAVEOF", b"127.0.0.1", b"1"],
    ] {
        assert_eq!(
            command(&mut stream, probe),
            REPLICATION_DISABLED,
            "{probe:?}"
        );
        assert_standalone(&mut stream);
    }

    // The declared no-op promotion form is acknowledged and changes nothing.
    let noops: [&[&[u8]]; 2] = [&[b"REPLICAOF", b"NO", b"ONE"], &[b"SLAVEOF", b"NO", b"ONE"]];
    for noop in noops {
        assert_eq!(command(&mut stream, noop), b"+OK\r\n", "{noop:?}");
        assert_standalone(&mut stream);
    }
    assert_eq!(
        command(&mut stream, &[b"GET", b"local"]),
        b"$5\r\nvalue\r\n"
    );
}

#[test]
fn a_dead_primary_is_not_promoted_and_its_address_stays_gone() {
    let primary = ProcessServer::start("primary");
    let survivor = ProcessServer::start("survivor");
    let mut primary_stream = primary.connect();
    let mut survivor_stream = survivor.connect();
    assert_eq!(
        command(&mut primary_stream, &[b"SET", b"primary-only", b"one"]),
        b"+OK\r\n"
    );
    assert_eq!(
        command(&mut survivor_stream, &[b"SET", b"survivor-only", b"two"]),
        b"+OK\r\n"
    );

    // The survivor cannot be told to take over the primary's role.
    let primary_port = primary.port.to_string();
    assert_eq!(
        command(
            &mut survivor_stream,
            &[b"REPLICAOF", b"127.0.0.1", primary_port.as_bytes()]
        ),
        REPLICATION_DISABLED
    );
    assert_eq!(
        command(
            &mut survivor_stream,
            &[b"SLAVEOF", b"127.0.0.1", primary_port.as_bytes()]
        ),
        REPLICATION_DISABLED
    );

    // Node loss: the primary disappears. Its address must not be answered by
    // anything else, and the survivor must not adopt its data or role.
    primary.crash();
    let refused = TcpStream::connect(("127.0.0.1", primary_port.parse::<u16>().unwrap()));
    assert!(
        match refused {
            Err(_) => true,
            Ok(stream) => {
                let _ = stream.shutdown(std::net::Shutdown::Both);
                false
            }
        },
        "the dead primary address must not be served by another process"
    );
    assert_standalone(&mut survivor_stream);
    assert_eq!(
        command(&mut survivor_stream, &[b"GET", b"primary-only"]),
        b"$-1\r\n"
    );
    assert_eq!(
        command(&mut survivor_stream, &[b"GET", b"survivor-only"]),
        b"$3\r\ntwo\r\n"
    );
    // A client of the dead primary sees no failover target: it was never a
    // replica, so there is nothing to promote and no rediscovery protocol.
    assert_eq!(command(&mut survivor_stream, &[b"DBSIZE"]), b":1\r\n");
}
