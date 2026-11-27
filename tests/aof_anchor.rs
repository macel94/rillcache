#![forbid(unsafe_code)]
#![cfg(target_os = "linux")]

//! End-to-end rollback/truncation anchor contract for authenticated journals.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use rillcache::engine::FsyncPolicy;
use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

const KEY: &[u8] = b"0123456789abcdef";

fn temp_dir(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rillcache-anchor-{label}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn write_key(path: &Path, key: &[u8]) {
    std::fs::write(path, key).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
}

fn config(dir: &Path, key_file: &Path, anchor: Option<&Path>) -> Config {
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
        aof_anchor_file: anchor.map(Path::to_path_buf),
        ..Config::default()
    }
}

fn start(dir: &Path, key_file: &Path, anchor: Option<&Path>) -> ServerHandle {
    Server::bind(config(dir, key_file, anchor))
        .unwrap()
        .spawn()
        .unwrap()
}

fn connect(server: &ServerHandle) -> TcpStream {
    let stream = TcpStream::connect(server.local_addr()).unwrap();
    stream.set_nodelay(true).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
}

fn send(stream: &mut TcpStream, arguments: &[&[u8]], expected: &[u8]) {
    let mut request = Vec::new();
    encode_request(arguments, &mut request);
    stream.write_all(&request).unwrap();
    let mut actual = vec![0; expected.len()];
    stream.read_exact(&mut actual).unwrap();
    assert_eq!(actual, expected);
}

fn command(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rillcache-backup"))
        .args(args)
        .output()
        .unwrap()
}

fn create_anchor(dir: &Path, key_file: &Path, output: &Path) -> std::process::Output {
    command(&[
        "anchor",
        "--dir",
        dir.to_str().unwrap(),
        "--aof-hmac-key-file",
        key_file.to_str().unwrap(),
        "--filename",
        "test.aof",
        "--output",
        output.to_str().unwrap(),
    ])
}

#[test]
fn anchored_start_accepts_history_and_rejects_rollback_and_truncation() {
    let dir = temp_dir("rollback");
    let keys = temp_dir("keys");
    let key_file = keys.join("journal.key");
    write_key(&key_file, KEY);
    let anchor = keys.join("journal.anchor");

    {
        let server = start(&dir, &key_file, None);
        let mut stream = connect(&server);
        send(&mut stream, &[b"SET", b"one", b"1"], b"+OK\r\n");
        send(&mut stream, &[b"SET", b"two", b"2"], b"+OK\r\n");
        server.shutdown().unwrap();
    }
    let anchored_state = std::fs::read(dir.join("test.aof")).unwrap();

    // The offline tool writes the anchor from the verified journal.
    let created = create_anchor(&dir, &key_file, &anchor);
    assert!(
        created.status.success(),
        "anchor creation failed: {}",
        String::from_utf8_lossy(&created.stderr)
    );
    let encoded = std::fs::read(&anchor).unwrap();
    assert!(encoded.starts_with(b"rillcache-aof-anchor-v1\n"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            0o600,
            std::fs::metadata(&anchor).unwrap().permissions().mode() & 0o777
        );
    }
    // Replacing a pinned anchor is an explicit operator action, not a side effect.
    let repeated = create_anchor(&dir, &key_file, &anchor);
    assert!(!repeated.status.success());
    assert!(
        String::from_utf8_lossy(&repeated.stderr).contains("exists")
            || String::from_utf8_lossy(&repeated.stderr).contains("anchor")
    );

    // An anchored start replays the committed history.
    {
        let server = start(&dir, &key_file, Some(&anchor));
        let mut stream = connect(&server);
        send(&mut stream, &[b"GET", b"one"], b"$1\r\n1\r\n");
        send(&mut stream, &[b"GET", b"two"], b"$1\r\n2\r\n");
        // Growth beyond the anchored boundary is expected and still allowed.
        send(&mut stream, &[b"SET", b"three", b"3"], b"+OK\r\n");
        server.shutdown().unwrap();
    }
    {
        let server = start(&dir, &key_file, Some(&anchor));
        let mut stream = connect(&server);
        send(&mut stream, &[b"GET", b"three"], b"$1\r\n3\r\n");
        server.shutdown().unwrap();
    }

    // A journal rolled back below the anchored boundary fails closed.
    std::fs::write(dir.join("test.aof"), &anchored_state).unwrap();
    let short = anchored_state.len().saturating_sub(1);
    std::fs::write(dir.join("test.aof"), &anchored_state[..short]).unwrap();
    let error = Server::bind(config(&dir, &key_file, Some(&anchor)))
        .err()
        .expect("truncated journal must fail closed");
    assert!(
        error.to_string().contains("rollback anchor"),
        "unexpected error: {error}"
    );

    // Minting an anchor needs the journal key: a foreign key cannot verify the
    // journal, so the offline tool fails closed instead of sealing a wrong state.
    std::fs::write(dir.join("test.aof"), &anchored_state).unwrap();
    let foreign = keys.join("foreign.key");
    write_key(&foreign, b"fedcba9876543210");
    let foreign_anchor = keys.join("foreign.anchor");
    let foreign_attempt = create_anchor(&dir, &foreign, &foreign_anchor);
    assert!(!foreign_attempt.status.success());
    assert!(!foreign_anchor.exists());
    assert!(
        String::from_utf8_lossy(&foreign_attempt.stderr).contains("anchor")
            || String::from_utf8_lossy(&foreign_attempt.stderr).contains("authentication")
    );

    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(keys);
}

#[test]
fn anchor_configuration_fails_closed_without_authentication() {
    let dir = temp_dir("config");
    let keys = temp_dir("config-keys");
    let key_file = keys.join("journal.key");
    write_key(&key_file, KEY);
    let anchor = keys.join("journal.anchor");
    std::fs::write(
        &anchor,
        b"rillcache-aof-anchor-v1\nlength 0\ntag 00\nmac 00\n",
    )
    .unwrap();

    // Reject a configured anchor without authentication.
    let error = Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        appendonly: true,
        dir: dir.clone(),
        appendfilename: "test.aof".into(),
        aof_anchor_file: Some(anchor.clone()),
        ..Config::default()
    })
    .err()
    .expect("an anchor without a key is not verifiable");
    assert!(
        error.to_string().contains("aof_anchor_file") || error.to_string().contains("anchor"),
        "unexpected error: {error}"
    );

    // Reject an anchor when the journal is disabled.
    let error = Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        dir: dir.clone(),
        appendfilename: "test.aof".into(),
        aof_hmac_key_file: Some(key_file.clone()),
        aof_anchor_file: Some(anchor.clone()),
        ..Config::default()
    })
    .err()
    .expect("an anchor without a journal is meaningless");
    assert!(error.to_string().contains("aof_anchor_file") || error.to_string().contains("anchor"));

    // A malformed anchor file fails closed rather than being ignored.
    let malformed = keys.join("malformed.anchor");
    std::fs::write(&malformed, b"not an anchor").unwrap();
    let error = Server::bind(config(&dir, &key_file, Some(&malformed)))
        .err()
        .expect("a malformed anchor must not be ignored");
    assert!(
        error.to_string().contains("anchor"),
        "unexpected error: {error}"
    );

    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(keys);
}
