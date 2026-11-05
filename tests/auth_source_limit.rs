#![forbid(unsafe_code)]

//! Per-peer-IP failed-authentication throttle.

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::time::Duration;

use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

fn start(config: Config) -> ServerHandle {
    let workers = match std::env::var("RILLCACHE_TEST_THREADS").as_deref() {
        Ok("0") | Err(_) => None,
        Ok("1") => Some(1),
        Ok("4") => Some(4),
        Ok(value) => panic!("invalid test worker mode {value}"),
    };
    Server::bind(Config {
        bind_addr: "0.0.0.0:0".parse().unwrap(),
        worker_threads: workers,
        ..config
    })
    .unwrap()
    .spawn()
    .unwrap()
}

fn connect(server: &ServerHandle, ip: Ipv4Addr) -> TcpStream {
    let address = SocketAddr::new(IpAddr::V4(ip), server.local_addr().port());
    let stream = TcpStream::connect(address).unwrap();
    // These tests assert reply content, isolation and lockout ordering; they do
    // not assert a wall-clock reply bound. A 3 s read deadline turned normal
    // thread-scheduling delay on an oversubscribed host into a false failure.
    // Keep a bounded deadline for real hangs without requiring prompt scheduling.
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream.set_nodelay(true).unwrap();
    stream
}

fn send(stream: &mut TcpStream, arguments: &[&[u8]], reply: &[u8]) {
    let mut bytes = Vec::new();
    encode_request(arguments, &mut bytes);
    stream.write_all(&bytes).unwrap();
    let mut actual = vec![0; reply.len()];
    stream.read_exact(&mut actual).unwrap();
    assert_eq!(actual, reply);
}

const WRONGPASS: &[u8] = b"-WRONGPASS invalid username-password pair or user is disabled.\r\n";
const THROTTLED: &[u8] = b"-ERR too many authentication attempts, try later\r\n";

/// Failure-window and post-lockout wait with generous scheduling headroom.
///
/// The throttle counts failures inside one window and then locks for that same
/// window. A 100 ms config cannot distinguish "no lockout" from a host that
/// descheduled this test between two round trips: with the machine's load
/// average near 11 on 6 CPUs, one measured run got `+OK` instead of the
/// lockout reply because the counter window had elapsed, and the same test
/// also failed 2/60 times on unmodified protected main. A 10x window keeps the
/// ordering/semantics under test while removing the wall-clock race; the test
/// still proves both the lockout and its expiry.
const SOURCE_WINDOW_MS: u64 = 1_000;
const LOCKOUT_WAIT: Duration = Duration::from_millis(1_500);

#[test]
fn source_budget_locks_one_peer_and_expires_after_the_window() {
    let server = start(Config {
        requirepass: Some(b"secret".to_vec()),
        auth_failure_limit: 0,
        auth_source_limit: 3,
        auth_source_window_ms: SOURCE_WINDOW_MS,
        ..Config::default()
    });
    let mut local = connect(&server, Ipv4Addr::LOCALHOST);
    for _ in 0..3 {
        send(&mut local, &[b"AUTH", b"wrong"], WRONGPASS);
    }
    let mut same_source = connect(&server, Ipv4Addr::LOCALHOST);
    send(&mut same_source, &[b"AUTH", b"secret"], THROTTLED);

    std::thread::sleep(LOCKOUT_WAIT);
    let mut after = connect(&server, Ipv4Addr::LOCALHOST);
    send(&mut after, &[b"AUTH", b"secret"], b"+OK\r\n");
    server.shutdown().unwrap();
}

#[test]
fn zero_disables_the_source_budget() {
    let server = start(Config {
        requirepass: Some(b"secret".to_vec()),
        auth_failure_limit: 0,
        auth_source_limit: 0,
        auth_source_window_ms: 100,
        ..Config::default()
    });
    let mut client = connect(&server, Ipv4Addr::LOCALHOST);
    for _ in 0..10 {
        send(&mut client, &[b"AUTH", b"wrong"], WRONGPASS);
    }
    send(&mut client, &[b"AUTH", b"secret"], b"+OK\r\n");
    server.shutdown().unwrap();
}

#[test]
fn success_resets_the_source_budget() {
    let server = start(Config {
        requirepass: Some(b"secret".to_vec()),
        auth_failure_limit: 0,
        auth_source_limit: 3,
        auth_source_window_ms: 60_000,
        ..Config::default()
    });
    let mut client = connect(&server, Ipv4Addr::LOCALHOST);
    send(&mut client, &[b"AUTH", b"wrong"], WRONGPASS);
    send(&mut client, &[b"AUTH", b"wrong"], WRONGPASS);
    send(&mut client, &[b"AUTH", b"secret"], b"+OK\r\n");
    send(&mut client, &[b"AUTH", b"wrong"], WRONGPASS);
    send(&mut client, &[b"AUTH", b"wrong"], WRONGPASS);
    send(&mut client, &[b"AUTH", b"secret"], b"+OK\r\n");
    server.shutdown().unwrap();
}

#[test]
fn hello_auth_failures_contribute_to_the_source_budget() {
    let server = start(Config {
        requirepass: Some(b"secret".to_vec()),
        auth_failure_limit: 0,
        auth_source_limit: 2,
        auth_source_window_ms: 60_000,
        ..Config::default()
    });
    for _ in 0..2 {
        let mut client = connect(&server, Ipv4Addr::LOCALHOST);
        send(
            &mut client,
            &[b"HELLO", b"2", b"AUTH", b"default", b"wrong"],
            WRONGPASS,
        );
    }
    let mut client = connect(&server, Ipv4Addr::LOCALHOST);
    send(&mut client, &[b"AUTH", b"secret"], THROTTLED);
    server.shutdown().unwrap();
}
