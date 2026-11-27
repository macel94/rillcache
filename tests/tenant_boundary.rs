#![forbid(unsafe_code)]

//! The tenant-isolation boundary (S7).
//!
//! The profile excludes multi-tenant deployment. This test pins the observable
//! facts behind that exclusion: ACL key patterns deny other tenants' data, but
//! keyspace-wide commands need an all-key grant, Pub/Sub namespace introspection
//! is shared, and memory/connection budgets and the process lifetime are global.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::Duration;

use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

fn temp_name(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rillcache-tenant-{label}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_file(&path);
    path
}

fn start(acl: &PathBuf) -> ServerHandle {
    Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        aclfile: Some(acl.clone()),
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
fn acl_key_patterns_do_not_isolate_tenants_and_introspection_is_shared() {
    let acl = temp_name("acl");
    std::fs::write(
        &acl,
        b"user default on nopass ~* &* +@all\n\
user tenant_a on >a-secret ~a:* &* +@all\n\
user tenant_b on >b-secret ~b:* &* +@all\n\
user spy on >spy-secret ~* &* +@pubsub\n",
    )
    .unwrap();
    let server = start(&acl);

    let mut a = connect(&server);
    command(&mut a, &[b"AUTH", b"tenant_a", b"a-secret"], b"+OK\r\n");
    command(&mut a, &[b"SET", b"a:secret", b"value"], b"+OK\r\n");

    let mut b = connect(&server);
    command(&mut b, &[b"AUTH", b"tenant_b", b"b-secret"], b"+OK\r\n");
    // Key patterns are command authorization, not tenancy: they deny direct
    // access to another tenant's keys but do not partition the keyspace.
    command(
        &mut b,
        &[b"GET", b"a:secret"],
        b"-NOPERM this user has no permissions to access one of the keys used as arguments\r\n",
    );
    command(
        &mut b,
        &[b"DBSIZE"],
        b"-NOPERM keyspace-wide commands require an all-key ACL grant with the required access\r\n",
    );
    command(
        &mut b,
        &[b"FLUSHDB", b"SYNC"],
        b"-NOPERM keyspace-wide commands require an all-key ACL grant with the required access\r\n",
    );
    // The data is still there for its owner: patterns did not isolate storage.
    command(&mut a, &[b"GET", b"a:secret"], b"$5\r\nvalue\r\n");

    // Pub/Sub is one process-wide namespace. A user granted only the pubsub
    // category can enumerate another tenant's channels.
    let mut subscriber = connect(&server);
    command(
        &mut subscriber,
        &[b"AUTH", b"tenant_a", b"a-secret"],
        b"+OK\r\n",
    );
    command(
        &mut subscriber,
        &[b"SUBSCRIBE", b"tenant-a-channel"],
        b"*3\r\n$9\r\nsubscribe\r\n$16\r\ntenant-a-channel\r\n:1\r\n",
    );
    let mut spy = connect(&server);
    command(&mut spy, &[b"AUTH", b"spy", b"spy-secret"], b"+OK\r\n");
    let channels = reply(&mut spy, &[b"PUBSUB", b"CHANNELS"]);
    assert!(
        contains(&channels, b"tenant-a-channel"),
        "cross-tenant channel names were not visible: {channels:?}"
    );

    // Limits are process-wide, so one tenant can consume another's capacity.
    let info = reply(&mut b, &[b"INFO", b"clients"]);
    assert!(contains(&info, b"maxclients:128"), "{info:?}");
    server.shutdown().unwrap();
    let _ = std::fs::remove_file(&acl);
}
