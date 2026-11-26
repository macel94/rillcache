#![forbid(unsafe_code)]
#![cfg(target_os = "linux")]

//! End-to-end evidence for the supported migration path (D1).
//!
//! Rillcache deliberately does not implement RDB or `DUMP`/`RESTORE`/`MIGRATE`
//! interchange. The supported alternative is a bounded offline DB 0 string
//! import followed by a verified backup, as documented in `docs/persistence.md`.
//! This test runs that documented sequence with the real tools and then boots a
//! server on the produced journal, so the migration claim is bound to behaviour
//! rather than prose.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rillcache::engine::FsyncPolicy;
use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

fn unique_dir() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rillcache-migration-{}-{}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn python() -> String {
    std::env::var("PYTHON").unwrap_or_else(|_| "python3".to_string())
}

fn import(source: &Path, journal: &Path) -> Output {
    Command::new(python())
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/scripts/import_string_journal.py"
        ))
        .arg(source)
        .arg(journal)
        .output()
        .unwrap()
}

fn backup(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rillcache-backup"))
        .args(arguments)
        .output()
        .unwrap()
}

fn frame(output: &mut Vec<u8>, arguments: &[&[u8]]) {
    output.extend_from_slice(format!("*{}\r\n", arguments.len()).as_bytes());
    for argument in arguments {
        output.extend_from_slice(format!("${}\r\n", argument.len()).as_bytes());
        output.extend_from_slice(argument);
        output.extend_from_slice(b"\r\n");
    }
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

fn command(stream: &mut TcpStream, arguments: &[&[u8]], expected: &[u8]) {
    let mut request = Vec::new();
    encode_request(arguments, &mut request);
    stream.write_all(&request).unwrap();
    let mut reply = vec![0; expected.len()];
    stream.read_exact(&mut reply).unwrap();
    assert_eq!(reply, expected, "{arguments:?}");
}

#[test]
fn documented_import_verify_and_start_path_preserves_strings_and_expiry() {
    let directory = unique_dir();
    let journal = directory.join("appendonly.aof");
    let archive = directory.join("import.rcb");
    let source = directory.join("snapshot.resp");
    let expires_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
        + 600_000;

    let expires_at = expires_at.to_string();
    let mut snapshot = Vec::new();
    frame(&mut snapshot, &[b"SELECT", b"0"]);
    frame(&mut snapshot, &[b"SET", b"binary\0key", b"value\xff"]);
    frame(
        &mut snapshot,
        &[b"SET", b"expiring", b"soon", b"PXAT", expires_at.as_bytes()],
    );
    std::fs::write(&source, &snapshot).unwrap();

    let imported = import(&source, &journal);
    assert!(
        imported.status.success(),
        "import failed: {}",
        String::from_utf8_lossy(&imported.stderr)
    );
    assert!(journal.is_file(), "import did not publish a journal");

    let created = backup(&[
        "create",
        "--dir",
        directory.to_str().unwrap(),
        "--output",
        archive.to_str().unwrap(),
    ]);
    assert!(
        created.status.success(),
        "backup create failed: {}",
        String::from_utf8_lossy(&created.stderr)
    );
    let verified = backup(&["verify", "--input", archive.to_str().unwrap()]);
    assert!(
        verified.status.success(),
        "backup verify failed: {}",
        String::from_utf8_lossy(&verified.stderr)
    );

    let server = Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        appendonly: true,
        appendfsync: FsyncPolicy::Always,
        dir: directory.clone(),
        appendfilename: "appendonly.aof".into(),
        ..Config::default()
    })
    .unwrap()
    .spawn()
    .unwrap();
    let mut client = connect(&server);
    command(
        &mut client,
        &[b"GET", b"binary\0key"],
        b"$6\r\nvalue\xff\r\n",
    );
    command(&mut client, &[b"GET", b"expiring"], b"$4\r\nsoon\r\n");
    command(&mut client, &[b"DBSIZE"], b":2\r\n");
    let mut request = Vec::new();
    encode_request(&[b"PTTL", b"expiring"], &mut request);
    client.write_all(&request).unwrap();
    let mut header = [0u8; 1];
    client.read_exact(&mut header).unwrap();
    assert_eq!(header, [b':']);
    let mut line = Vec::new();
    loop {
        let mut byte = [0u8; 1];
        client.read_exact(&mut byte).unwrap();
        if byte[0] == b'\n' {
            break;
        }
        line.push(byte[0]);
    }
    let ttl: i64 = std::str::from_utf8(&line[..line.len() - 1])
        .unwrap()
        .parse()
        .unwrap();
    assert!(ttl > 0 && ttl <= 600_000, "unexpected TTL {ttl}");
    server.shutdown().unwrap();
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn unsupported_records_and_existing_targets_fail_closed() {
    let directory = unique_dir();
    let journal = directory.join("appendonly.aof");
    let source = directory.join("unsupported.resp");

    let mut snapshot = Vec::new();
    frame(&mut snapshot, &[b"HSET", b"hash", b"field", b"value"]);
    std::fs::write(&source, &snapshot).unwrap();
    let rejected = import(&source, &journal);
    assert!(!rejected.status.success(), "non-SET record was accepted");
    assert!(!journal.exists(), "failed import published a journal");

    let mut valid = Vec::new();
    frame(&mut valid, &[b"SET", b"key", b"value"]);
    std::fs::write(&source, &valid).unwrap();
    std::fs::write(&journal, b"occupied").unwrap();
    let refused = import(&source, &journal);
    assert!(
        !refused.status.success(),
        "existing journal was overwritten"
    );
    assert_eq!(std::fs::read(&journal).unwrap(), b"occupied");
    let _ = std::fs::remove_dir_all(&directory);
}
