#![forbid(unsafe_code)]

//! The admission and quota boundaries (R5).
//!
//! Admission is global (`max_clients`) plus an optional per-ACL-user
//! `maxconnections` cap; unauthenticated sessions consume global slots and have
//! no separate cap. This test pins those boundaries and their explicit refusals,
//! including release of capacity when a client disconnects.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

fn temp_name(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rillcache-admission-{label}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_file(&path);
    path
}

fn start(acl: Option<&PathBuf>, max_clients: usize) -> ServerHandle {
    Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        aclfile: acl.cloned(),
        requirepass: acl.is_none().then(|| b"global-secret".to_vec()),
        max_clients,
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
fn unauthenticated_sessions_consume_global_slots_and_are_bounded_by_maxclients() {
    let server = start(None, 2);
    let mut first = connect(&server);
    let mut second = connect(&server);
    // Unauthenticated sessions are admitted and counted, and are limited to the
    // authentication-exempt commands.
    command(
        &mut first,
        &[b"GET", b"k"],
        b"-NOAUTH Authentication required.\r\n",
    );
    command(
        &mut second,
        &[b"PING"],
        b"-NOAUTH Authentication required.\r\n",
    );

    // There is no separate unauthenticated cap: the global bound is the only one.
    let mut refused = connect(&server);
    let mut reply = Vec::new();
    refused.read_to_end(&mut reply).unwrap();
    assert_eq!(reply, b"-ERR max number of clients reached\r\n");

    // Releasing an unauthenticated session frees the global slot.
    drop(first);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let mut candidate = connect(&server);
        let mut probe = Vec::new();
        encode_request(&[b"AUTH", b"global-secret"], &mut probe);
        candidate.write_all(&probe).unwrap();
        let answer = read_frame(&mut candidate, 0);
        if answer == b"+OK\r\n" {
            drop(candidate);
            break;
        }
        assert_eq!(answer, b"-ERR max number of clients reached\r\n");
        assert!(
            Instant::now() < deadline,
            "global admission slot was not released"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    server.shutdown().unwrap();
}

#[test]
fn per_user_maxconnections_is_enforced_atomically_and_released() {
    let acl = temp_name("acl");
    std::fs::write(
        &acl,
        b"user default on nopass ~* &* +@all\n\
user capped on >capped-secret ~* &* +@all maxconnections=1\n",
    )
    .unwrap();
    let server = start(Some(&acl), 8);

    let mut first = connect(&server);
    command(
        &mut first,
        &[b"AUTH", b"capped", b"capped-secret"],
        b"+OK\r\n",
    );
    let mut second = connect(&server);
    command(
        &mut second,
        &[b"AUTH", b"capped", b"capped-secret"],
        b"-ERR max connections reached for user\r\n",
    );

    drop(first);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let answer = reply(&mut second, &[b"AUTH", b"capped", b"capped-secret"]);
        if answer == b"+OK\r\n" {
            break;
        }
        assert_eq!(answer, b"-ERR max connections reached for user\r\n");
        assert!(
            Instant::now() < deadline,
            "per-user admission slot was not released"
        );
        std::thread::sleep(Duration::from_millis(10));
    }

    // The supported limits are observable process-wide.
    let info = reply(&mut second, &[b"INFO", b"clients"]);
    assert!(contains(&info, b"maxclients:8"), "{info:?}");
    assert!(contains(&info, b"connected_clients:"), "{info:?}");
    server.shutdown().unwrap();
    let _ = std::fs::remove_file(&acl);
}
