#![forbid(unsafe_code)]

//! Signed audit-line integration coverage.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
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

fn temp_name(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "rillcache-audit-hmac-{label}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ))
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

#[test]
fn signed_file_lines_carry_a_bounded_hmac() {
    let acl = temp_name("acl");
    let audit = temp_name("audit");
    let key = temp_name("key");
    std::fs::write(
        &acl,
        b"user default on nopass ~* &* +@all\n\
user reader on >reader-secret allkeys allchannels -@all +get\n",
    )
    .unwrap();
    std::fs::write(&key, b"0123456789abcdef0123456789abcdef\n").unwrap();
    let server = start(Config {
        aclfile: Some(acl.clone()),
        audit_log: Some(audit.clone()),
        audit_hmac_key_file: Some(key.clone()),
        ..Config::default()
    });
    let mut reader = connect(&server);
    send(
        &mut reader,
        &[b"AUTH", b"reader", b"reader-secret"],
        b"+OK\r\n",
    );
    send(
        &mut reader,
        &[b"SET", b"secret-key", b"v"],
        b"-NOPERM this user has no permissions to run the 'set' command\r\n",
    );
    let contents = std::fs::read_to_string(&audit).unwrap();
    let fields: Vec<_> = contents.trim_end().split(' ').collect();
    assert_eq!(fields.len(), 6, "{contents}");
    assert_eq!(fields[1], "command");
    assert_eq!(fields[2], "toplevel");
    assert_eq!(fields[3], "726561646572");
    assert_eq!(fields[4], "736574");
    assert_eq!(fields[5].len(), 64, "{contents}");
    assert!(fields[5].chars().all(|ch| ch.is_ascii_hexdigit()));
    assert!(!contents.contains("reader-secret"));
    assert!(!contents.contains("secret-key"));
    server.shutdown().unwrap();

    for path in [&acl, &audit, &key] {
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn short_or_missing_hmac_keys_fail_startup() {
    let short = temp_name("short-key");
    std::fs::write(&short, b"too-short").unwrap();
    let short_config = Config {
        audit_hmac_key_file: Some(short.clone()),
        ..Config::default()
    };
    assert!(Server::bind(short_config).is_err());
    let _ = std::fs::remove_file(&short);

    let missing = Config {
        audit_hmac_key_file: Some(temp_name("missing-key")),
        ..Config::default()
    };
    assert!(Server::bind(missing).is_err());
}
