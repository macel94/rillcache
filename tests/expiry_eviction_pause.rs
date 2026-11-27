#![forbid(unsafe_code)]

//! Expiry, eviction and pause interaction matrix (O7).
//!
//! The supported matrix is deliberately narrow and deterministic:
//!
//! 1. `CLIENT PAUSE ... WRITE` holds mutations but not reads. A key that
//!    expires during the pause is already gone when a read arrives, so a paused
//!    server never serves a dead value and never replays it afterwards.
//! 2. A blocked `BLPOP` is independent of expiry and eviction on other keys: the
//!    blocked client keeps its position, receives the pushed element in push
//!    order, and an expired key read by another client answers nil without
//!    touching the blocked reply.
//! 3. Eviction under `allkeys-random` keeps the payload budget while another
//!    client is blocked, counts the block in `INFO stats`, and still answers the
//!    blocked command with its documented nil timeout reply.
//!
//! Ordering guarantees are per connection: replies are emitted in request order,
//! including replies released when a pause ends. Cross-client ordering is not
//! claimed beyond the blocked-list push order below.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use rillcache::engine::EvictionPolicy;
use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

fn worker_mode() -> Option<usize> {
    match std::env::var("RILLCACHE_TEST_THREADS").as_deref() {
        Ok("0") | Err(_) => None,
        Ok("1") => Some(1),
        Ok("4") => Some(4),
        Ok(value) => panic!("invalid test worker mode {value}"),
    }
}

fn start(overrides: impl FnOnce(&mut Config)) -> ServerHandle {
    let mut config = Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        worker_threads: worker_mode(),
        ..Config::default()
    };
    overrides(&mut config);
    Server::bind(config).unwrap().spawn().unwrap()
}

fn connect(server: &ServerHandle) -> TcpStream {
    let stream = TcpStream::connect(server.local_addr()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream.set_nodelay(true).unwrap();
    stream
}

fn read_line(stream: &mut TcpStream, output: &mut Vec<u8>) -> Vec<u8> {
    let mut line = Vec::new();
    loop {
        let mut byte = [0];
        stream.read_exact(&mut byte).unwrap();
        if byte[0] == b'\n' {
            assert_eq!(line.pop(), Some(b'\r'));
            output.extend_from_slice(&line);
            output.extend_from_slice(b"\r\n");
            return line;
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
            let length: i64 = std::str::from_utf8(&read_line(stream, &mut output))
                .unwrap()
                .parse()
                .unwrap();
            if length >= 0 {
                let mut body = vec![0; length as usize + 2];
                stream.read_exact(&mut body).unwrap();
                output.extend_from_slice(&body);
            }
        }
        b'*' => {
            let count: i64 = std::str::from_utf8(&read_line(stream, &mut output))
                .unwrap()
                .parse()
                .unwrap();
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

fn send(stream: &mut TcpStream, arguments: &[&[u8]]) {
    let mut request = Vec::new();
    encode_request(arguments, &mut request);
    stream.write_all(&request).unwrap();
}

fn command(stream: &mut TcpStream, arguments: &[&[u8]], expected: &[u8]) {
    send(stream, arguments);
    assert_eq!(read_frame(stream, 0), expected, "{arguments:?}");
}

fn info_field(info: &str, name: &str) -> u64 {
    info.lines()
        .find_map(|line| line.strip_prefix(&format!("{name}:")))
        .unwrap_or_else(|| panic!("INFO did not report {name}:\n{info}"))
        .trim()
        .parse()
        .unwrap()
}

fn blocked_clients(info: &str) -> u64 {
    info.lines()
        .find_map(|line| line.strip_prefix("blocked_clients:"))
        .unwrap_or_else(|| panic!("INFO did not report blocked_clients:\n{info}"))
        .trim()
        .parse()
        .unwrap()
}

fn info(stream: &mut TcpStream, section: &[u8]) -> String {
    send(stream, &[b"INFO", section]);
    String::from_utf8_lossy(&read_frame(stream, 0)).into_owned()
}

#[test]
fn expiry_during_a_write_pause_is_never_observed_or_replayed() {
    let server = start(|_| {});
    let mut admin = connect(&server);
    let mut worker = connect(&server);
    command(
        &mut worker,
        &[b"SET", b"k", b"v", b"PX", b"100"],
        b"+OK\r\n",
    );

    command(
        &mut admin,
        &[b"CLIENT", b"PAUSE", b"600", b"WRITE"],
        b"+OK\r\n",
    );
    std::thread::sleep(Duration::from_millis(200));
    // Reads are outside a WRITE pause, and the expired key must answer nil.
    command(&mut worker, &[b"GET", b"k"], b"$-1\r\n");
    command(&mut worker, &[b"EXISTS", b"k"], b":0\r\n");
    command(&mut worker, &[b"TTL", b"k"], b":-2\r\n");

    // A write issued during the pause is answered after it expires, and no
    // stale value is replayed before the new one.
    send(&mut worker, &[b"SET", b"k", b"fresh"]);
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(read_frame(&mut worker, 0), b"+OK\r\n");
    command(&mut worker, &[b"GET", b"k"], b"$5\r\nfresh\r\n");
    server.shutdown().unwrap();
}

#[test]
fn expired_keys_and_blocked_pops_keep_their_position_and_order() {
    let server = start(|_| {});
    let mut admin = connect(&server);
    let mut blocked = connect(&server);
    let mut writer = connect(&server);

    command(
        &mut admin,
        &[b"SET", b"gone", b"v", b"PX", b"80"],
        b"+OK\r\n",
    );
    // The blocked command is issued before the key expires.
    send(&mut blocked, &[b"BLPOP", b"q", b"5"]);
    std::thread::sleep(Duration::from_millis(150));
    command(&mut writer, &[b"GET", b"gone"], b"$-1\r\n");
    command(&mut writer, &[b"EXISTS", b"gone"], b":0\r\n");

    // The blocked client is still blocked and receives the pushed value in push
    // order, not an earlier nil timeout.
    command(
        &mut writer,
        &[b"RPUSH", b"q", b"first", b"second"],
        b":2\r\n",
    );
    assert_eq!(
        read_frame(&mut blocked, 0),
        b"*2\r\n$1\r\nq\r\n$5\r\nfirst\r\n"
    );

    // A second blocked pop receives the remaining element in order.
    send(&mut blocked, &[b"BLPOP", b"q", b"1"]);
    assert_eq!(
        read_frame(&mut blocked, 0),
        b"*2\r\n$1\r\nq\r\n$6\r\nsecond\r\n"
    );

    // With nothing left, the documented timeout reply is a null array.
    command(&mut blocked, &[b"BLPOP", b"q", b"1"], b"*-1\r\n");
    command(&mut blocked, &[b"PING"], b"+PONG\r\n");
    server.shutdown().unwrap();
}

#[test]
fn eviction_keeps_the_budget_while_a_client_is_blocked() {
    let server = start(|config| {
        // 64 bytes of payload budget is enough for a couple of small keys, so
        // later writes must evict instead of failing.
        config.max_data_bytes = 64;
        config.maxmemory_policy = EvictionPolicy::AllKeysRandom;
    });
    let mut blocked = connect(&server);
    let mut writer = connect(&server);

    send(&mut blocked, &[b"BLPOP", b"never", b"2"]);
    std::thread::sleep(Duration::from_millis(100));
    let clients = info(&mut writer, b"clients");
    assert!(
        blocked_clients(&clients) >= 1,
        "the blocked command must be counted:\n{clients}"
    );

    // Fill past the budget; eviction must keep every write accepted.
    for index in 0..12 {
        let key = format!("k{index}").into_bytes();
        let value = vec![b'v'; 12];
        command(&mut writer, &[b"SET", &key, &value], b"+OK\r\n");
    }
    let stats = info(&mut writer, b"stats");
    assert!(
        info_field(&stats, "evicted_keys") >= 1,
        "writes beyond the budget must evict rather than refuse:\n{stats}"
    );

    // The blocked client still receives its documented nil timeout reply and the
    // connection remains ordered.
    assert_eq!(read_frame(&mut blocked, 0), b"*-1\r\n");
    command(&mut blocked, &[b"PING"], b"+PONG\r\n");
    let started = Instant::now();
    command(&mut writer, &[b"PING"], b"+PONG\r\n");
    assert!(started.elapsed() < Duration::from_secs(2));
    server.shutdown().unwrap();
}
