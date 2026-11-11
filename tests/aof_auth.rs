#![forbid(unsafe_code)]

//! Server-level journal authentication: restart, tampering, wrong keys and the
//! explicit migration path. Kept separate from the broader persistence suite so
//! its additional concurrent servers do not lengthen that suite's rewrite waits.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rillcache::engine::FsyncPolicy;
use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

fn temp_dir(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rillcache-auth-{label}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
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

fn send(stream: &mut TcpStream, arguments: &[&[u8]], expected: &[u8]) {
    let mut bytes = Vec::new();
    encode_request(arguments, &mut bytes);
    stream.write_all(&bytes).unwrap();
    let mut actual = vec![0; expected.len()];
    stream.read_exact(&mut actual).unwrap();
    assert_eq!(actual, expected);
}

fn start_plain(dir: &Path) -> ServerHandle {
    let workers = match std::env::var("RILLCACHE_TEST_THREADS").as_deref() {
        Ok("0") | Err(_) => None,
        Ok("1") => Some(1),
        Ok("4") => Some(4),
        Ok(value) => panic!("invalid test worker mode {value}"),
    };
    Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        worker_threads: workers,
        appendonly: true,
        appendfsync: FsyncPolicy::Always,
        dir: dir.to_path_buf(),
        appendfilename: "test.aof".into(),
        ..Config::default()
    })
    .unwrap()
    .spawn()
    .unwrap()
}

fn auth_config(dir: &Path, key_file: &Path, migrate: bool) -> Config {
    let workers = match std::env::var("RILLCACHE_TEST_THREADS").as_deref() {
        Ok("0") | Err(_) => None,
        Ok("1") => Some(1),
        Ok("4") => Some(4),
        Ok(value) => panic!("invalid test worker mode {value}"),
    };
    Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        worker_threads: workers,
        appendonly: true,
        appendfsync: FsyncPolicy::Always,
        dir: dir.to_path_buf(),
        appendfilename: "test.aof".into(),
        aof_hmac_key_file: Some(key_file.to_path_buf()),
        aof_hmac_migrate: migrate,
        ..Config::default()
    }
}

fn write_auth_key(path: &Path, key: &[u8]) {
    std::fs::write(path, [key, b"\n"].concat()).unwrap();
}

#[test]
fn journal_authentication_survives_restart_and_rejects_tampering_and_wrong_keys() {
    let dir = temp_dir("restart");
    let key_file = temp_dir("key").join("journal.key");
    write_auth_key(&key_file, b"0123456789abcdef");
    {
        let server = Server::bind(auth_config(&dir, &key_file, false))
            .unwrap()
            .spawn()
            .unwrap();
        let mut stream = connect(&server);
        send(&mut stream, &[b"SET", b"k", b"v"], b"+OK\r\n");
        server.shutdown().unwrap();
    }
    {
        let server = Server::bind(auth_config(&dir, &key_file, false))
            .unwrap()
            .spawn()
            .unwrap();
        let mut stream = connect(&server);
        send(&mut stream, &[b"GET", b"k"], b"$1\r\nv\r\n");
        server.shutdown().unwrap();
    }
    let path = dir.join("test.aof");
    let original = std::fs::read(&path).unwrap();
    let marker = b"__RILLCACHE_RECORD_HMAC256_V1__:";
    let at = original
        .windows(marker.len())
        .position(|window| window == marker)
        .expect("authentication marker");
    let mut tampered = original.clone();
    let tag_start = at + marker.len();
    tampered[tag_start] = if tampered[tag_start] == b'0' {
        b'1'
    } else {
        b'0'
    };
    std::fs::write(&path, tampered).unwrap();
    assert!(
        Server::bind(auth_config(&dir, &key_file, false)).is_err(),
        "modified keyed tag must fail closed"
    );
    std::fs::write(&path, &original).unwrap();
    let wrong_key = temp_dir("wrong-key").join("journal.key");
    write_auth_key(&wrong_key, b"fedcba9876543210");
    assert!(
        Server::bind(auth_config(&dir, &wrong_key, false)).is_err(),
        "a different key must fail the anchor"
    );
    assert!(
        Server::bind(Config {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            appendonly: true,
            dir: dir.clone(),
            appendfilename: "test.aof".into(),
            ..Config::default()
        })
        .is_ok(),
        "the RESP journal remains readable when authentication is not requested"
    );
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(key_file.parent().unwrap());
    let _ = std::fs::remove_dir_all(wrong_key.parent().unwrap());
}

#[test]
fn enabling_journal_authentication_requires_explicit_migration_then_stays_enforced() {
    let dir = temp_dir("migration");
    let key_file = temp_dir("migration-key").join("journal.key");
    write_auth_key(&key_file, b"0123456789abcdef");
    {
        let server = start_plain(&dir);
        let mut stream = connect(&server);
        send(&mut stream, &[b"SET", b"before", b"value"], b"+OK\r\n");
        server.shutdown().unwrap();
    }
    let error = Server::bind(auth_config(&dir, &key_file, false))
        .err()
        .expect("implicit sealing must fail");
    assert!(error.to_string().contains("--aof-hmac-migrate"));
    {
        let server = Server::bind(auth_config(&dir, &key_file, true))
            .unwrap()
            .spawn()
            .unwrap();
        let mut stream = connect(&server);
        send(&mut stream, &[b"GET", b"before"], b"$5\r\nvalue\r\n");
        send(&mut stream, &[b"SET", b"after", b"value"], b"+OK\r\n");
        server.shutdown().unwrap();
    }
    {
        let server = Server::bind(auth_config(&dir, &key_file, false))
            .unwrap()
            .spawn()
            .unwrap();
        let mut stream = connect(&server);
        send(&mut stream, &[b"GET", b"before"], b"$5\r\nvalue\r\n");
        send(&mut stream, &[b"GET", b"after"], b"$5\r\nvalue\r\n");
        server.shutdown().unwrap();
    }
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(key_file.parent().unwrap());
}
