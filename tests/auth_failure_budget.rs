#![forbid(unsafe_code)]

//! Per-connection failed-authentication budget.

use std::io::{Read, Write};
use std::net::TcpStream;
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
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        worker_threads: workers,
        ..config
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

fn send(stream: &mut TcpStream, arguments: &[&[u8]], reply: &[u8]) {
    let mut bytes = Vec::new();
    encode_request(arguments, &mut bytes);
    stream.write_all(&bytes).unwrap();
    let mut actual = vec![0; reply.len()];
    stream.read_exact(&mut actual).unwrap();
    assert_eq!(actual, reply);
}

fn assert_closed(stream: &mut TcpStream) {
    let mut byte = [0u8; 1];
    match stream.read(&mut byte) {
        Ok(0) | Err(_) => {}
        Ok(count) => panic!("expected connection close, read {count} byte(s)"),
    }
}

const WRONGPASS: &[u8] = b"-WRONGPASS invalid username-password pair or user is disabled.\r\n";

#[test]
fn failed_auth_budget_closes_only_after_the_configured_attempts() {
    let server = start(Config {
        requirepass: Some(b"secret".to_vec()),
        auth_failure_limit: 3,
        ..Config::default()
    });
    let mut client = connect(&server);
    send(&mut client, &[b"AUTH", b"wrong"], WRONGPASS);
    send(&mut client, &[b"AUTH", b"wrong"], WRONGPASS);
    // A success resets the consecutive counter.
    send(&mut client, &[b"AUTH", b"secret"], b"+OK\r\n");
    send(&mut client, &[b"AUTH", b"wrong"], WRONGPASS);
    send(&mut client, &[b"AUTH", b"wrong"], WRONGPASS);
    send(&mut client, &[b"AUTH", b"wrong"], WRONGPASS);
    assert_closed(&mut client);
    server.shutdown().unwrap();
}

#[test]
fn zero_disables_the_budget_and_other_connections_are_isolated() {
    let limited = start(Config {
        requirepass: Some(b"secret".to_vec()),
        auth_failure_limit: 2,
        ..Config::default()
    });
    let mut first = connect(&limited);
    send(&mut first, &[b"AUTH", b"wrong"], WRONGPASS);
    send(&mut first, &[b"AUTH", b"wrong"], WRONGPASS);
    assert_closed(&mut first);
    let mut second = connect(&limited);
    send(&mut second, &[b"AUTH", b"secret"], b"+OK\r\n");
    send(&mut second, &[b"PING"], b"+PONG\r\n");
    limited.shutdown().unwrap();

    let open = start(Config {
        requirepass: Some(b"secret".to_vec()),
        auth_failure_limit: 0,
        ..Config::default()
    });
    let mut client = connect(&open);
    for _ in 0..8 {
        send(&mut client, &[b"AUTH", b"wrong"], WRONGPASS);
    }
    send(&mut client, &[b"AUTH", b"secret"], b"+OK\r\n");
    send(&mut client, &[b"PING"], b"+PONG\r\n");
    open.shutdown().unwrap();
}

#[test]
fn hello_auth_failures_share_the_same_per_connection_budget() {
    let server = start(Config {
        requirepass: Some(b"secret".to_vec()),
        auth_failure_limit: 2,
        ..Config::default()
    });
    let mut client = connect(&server);
    send(
        &mut client,
        &[b"HELLO", b"2", b"AUTH", b"default", b"wrong"],
        WRONGPASS,
    );
    send(
        &mut client,
        &[b"HELLO", b"2", b"AUTH", b"default", b"wrong"],
        WRONGPASS,
    );
    assert_closed(&mut client);
    server.shutdown().unwrap();
}

#[test]
fn reset_clears_the_failed_auth_counter() {
    let server = start(Config {
        requirepass: Some(b"secret".to_vec()),
        auth_failure_limit: 3,
        ..Config::default()
    });
    let mut client = connect(&server);
    send(&mut client, &[b"AUTH", b"wrong"], WRONGPASS);
    send(&mut client, &[b"AUTH", b"wrong"], WRONGPASS);
    send(&mut client, &[b"RESET"], b"+RESET\r\n");
    send(&mut client, &[b"AUTH", b"wrong"], WRONGPASS);
    send(&mut client, &[b"AUTH", b"wrong"], WRONGPASS);
    send(&mut client, &[b"AUTH", b"secret"], b"+OK\r\n");
    send(&mut client, &[b"PING"], b"+PONG\r\n");
    server.shutdown().unwrap();
}
