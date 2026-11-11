#![forbid(unsafe_code)]
#![cfg(target_os = "linux")]

//! End-to-end offline backup/restore CLI contract.
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use rillcache::engine::FsyncPolicy;
use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

const MAGIC: &[u8] = b"RILLCACHE-AOF-BACKUP-V1\n";

fn temp_dir(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rillcache-backup-{label}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn config(dir: &Path) -> Config {
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
        appendfilename: "appendonly.aof".into(),
        ..Config::default()
    }
}

fn start(dir: &Path) -> ServerHandle {
    Server::bind(config(dir)).unwrap().spawn().unwrap()
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

fn read_reply(stream: &mut TcpStream) -> Vec<u8> {
    let mut reply = Vec::new();
    let mut byte = [0u8; 1];
    stream.read_exact(&mut byte).unwrap();
    reply.push(byte[0]);
    match byte[0] {
        b'+' | b'-' | b':' => loop {
            stream.read_exact(&mut byte).unwrap();
            reply.push(byte[0]);
            if reply.ends_with(b"\r\n") {
                break;
            }
        },
        b'*' => {
            let mut line = Vec::new();
            loop {
                stream.read_exact(&mut byte).unwrap();
                reply.push(byte[0]);
                if byte[0] == b'\r' {
                    stream.read_exact(&mut byte).unwrap();
                    reply.push(byte[0]);
                    break;
                }
                line.push(byte[0]);
            }
            let count: i64 = std::str::from_utf8(&line).unwrap().parse().unwrap();
            for _ in 0..count.max(0) {
                reply.extend_from_slice(&read_reply(stream));
            }
        }
        b'$' => {
            let mut line = Vec::new();
            loop {
                stream.read_exact(&mut byte).unwrap();
                reply.push(byte[0]);
                if byte[0] == b'\r' {
                    stream.read_exact(&mut byte).unwrap();
                    reply.push(byte[0]);
                    break;
                }
                line.push(byte[0]);
            }
            let len: i64 = std::str::from_utf8(&line).unwrap().parse().unwrap();
            if len >= 0 {
                let mut body = vec![0u8; len as usize];
                stream.read_exact(&mut body).unwrap();
                reply.extend_from_slice(&body);
                stream.read_exact(&mut byte).unwrap();
                reply.push(byte[0]);
                stream.read_exact(&mut byte).unwrap();
                reply.push(byte[0]);
            }
        }
        other => panic!("unexpected RESP reply type {}", other as char),
    }
    reply
}

fn call(stream: &mut TcpStream, args: &[&[u8]]) -> Vec<u8> {
    let mut request = Vec::new();
    encode_request(args, &mut request);
    stream.write_all(&request).unwrap();
    read_reply(stream)
}

fn command(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rillcache-backup"))
        .args(args)
        .output()
        .unwrap()
}

fn create(dir: &Path, output: &Path) -> std::process::Output {
    command(&[
        "create",
        "--dir",
        dir.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
    ])
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

#[test]
fn offline_backup_restore_preserves_strings_ttl_and_stream_pel() {
    let source = temp_dir("source");
    let archive_dir = temp_dir("archive");
    let archive = archive_dir.join("snapshot.rcb");
    {
        let server = start(&source);
        let mut stream = connect(&server);
        assert_eq!(call(&mut stream, &[b"SET", b"key", b"value"]), b"+OK\r\n");
        assert_eq!(
            call(&mut stream, &[b"PEXPIRE", b"key", b"600000"]),
            b":1\r\n"
        );
        for id in [b"1-0".as_slice(), b"2-0", b"3-0"] {
            assert_eq!(
                call(&mut stream, &[b"XADD", b"stream", id, b"f", id]),
                [&b"$3\r\n"[..], id, b"\r\n"].concat()
            );
        }
        assert_eq!(
            call(
                &mut stream,
                &[b"XGROUP", b"CREATE", b"stream", b"group", b"0"]
            ),
            b"+OK\r\n"
        );
        let _ = call(
            &mut stream,
            &[
                b"XREADGROUP",
                b"GROUP",
                b"group",
                b"worker",
                b"STREAMS",
                b"stream",
                b">",
            ],
        );
        assert_eq!(
            call(
                &mut stream,
                &[
                    b"XCLAIM",
                    b"stream",
                    b"group",
                    b"other",
                    b"0",
                    b"2-0",
                    b"RETRYCOUNT",
                    b"7",
                    b"JUSTID",
                    b"FORCE",
                ],
            ),
            b"*1\r\n$3\r\n2-0\r\n"
        );
        server.shutdown().unwrap();
    }

    let made = create(&source, &archive);
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );
    let duplicate = create(&source, &archive);
    assert!(
        !duplicate.status.success(),
        "backup must not overwrite an archive"
    );
    let verified = command(&["verify", "--input", archive.to_str().unwrap()]);
    assert!(
        verified.status.success(),
        "{}",
        String::from_utf8_lossy(&verified.stderr)
    );

    let destination = temp_dir("restored");
    let restored = command(&[
        "restore",
        "--input",
        archive.to_str().unwrap(),
        "--dir",
        destination.to_str().unwrap(),
    ]);
    assert!(
        restored.status.success(),
        "{}",
        String::from_utf8_lossy(&restored.stderr)
    );
    {
        let server = start(&destination);
        let mut stream = connect(&server);
        assert_eq!(call(&mut stream, &[b"GET", b"key"]), b"$5\r\nvalue\r\n");
        let pttl = call(&mut stream, &[b"PTTL", b"key"]);
        assert!(pttl.starts_with(b":") && pttl != b":-1\r\n" && pttl != b":-2\r\n");
        let pending = call(
            &mut stream,
            &[b"XPENDING", b"stream", b"group", b"-", b"+", b"10"],
        );
        assert!(pending.windows(3).any(|w| w == b"1-0"));
        assert!(pending.windows(3).any(|w| w == b"2-0"));
        assert!(pending.windows(3).any(|w| w == b"3-0"));
        server.shutdown().unwrap();
    }
    let second_restore = command(&[
        "restore",
        "--input",
        archive.to_str().unwrap(),
        "--dir",
        destination.to_str().unwrap(),
    ]);
    assert!(
        !second_restore.status.success(),
        "restore must not overwrite an existing journal"
    );
    let _ = std::fs::remove_dir_all(&source);
    let _ = std::fs::remove_dir_all(&archive_dir);
    let _ = std::fs::remove_dir_all(&destination);
}

#[test]
fn empty_journals_can_be_backed_up_verified_and_restored() {
    let source = temp_dir("empty-source");
    let archive_dir = temp_dir("empty-archive");
    let archive = archive_dir.join("empty.rcb");
    let server = start(&source);
    server.shutdown().unwrap();
    assert_eq!(
        std::fs::metadata(source.join("appendonly.aof"))
            .unwrap()
            .len(),
        0
    );
    let made = create(&source, &archive);
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );
    let verified = command(&["verify", "--input", archive.to_str().unwrap()]);
    assert!(
        verified.status.success(),
        "{}",
        String::from_utf8_lossy(&verified.stderr)
    );
    let destination = temp_dir("empty-destination");
    let restored = command(&[
        "restore",
        "--input",
        archive.to_str().unwrap(),
        "--dir",
        destination.to_str().unwrap(),
    ]);
    assert!(
        restored.status.success(),
        "{}",
        String::from_utf8_lossy(&restored.stderr)
    );
    assert_eq!(
        std::fs::metadata(destination.join("appendonly.aof"))
            .unwrap()
            .len(),
        0
    );
    let _ = std::fs::remove_dir_all(&source);
    let _ = std::fs::remove_dir_all(&archive_dir);
    let _ = std::fs::remove_dir_all(&destination);
}

#[test]
fn backup_requires_quiescent_source_and_corrupt_archives_never_restore() {
    let source = temp_dir("live");
    let archive_dir = temp_dir("bad-archive");
    let archive = archive_dir.join("snapshot.rcb");
    let server = start(&source);
    let mut live_stream = connect(&server);
    assert_eq!(
        call(&mut live_stream, &[b"SET", b"live", b"value"]),
        b"+OK\r\n"
    );
    let busy = create(&source, &archive);
    assert!(!busy.status.success(), "live server owns the AOF lock");
    assert!(!archive.exists(), "failed backup does not publish output");
    server.shutdown().unwrap();
    let created = create(&source, &archive);
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );

    let mut bytes = std::fs::read(&archive).unwrap();
    bytes[MAGIC.len() + 8 + 4 + 3] ^= 0x01;
    std::fs::write(&archive, &bytes).unwrap();
    let verify = command(&["verify", "--input", archive.to_str().unwrap()]);
    assert!(
        !verify.status.success(),
        "corrupt archive must fail verification"
    );
    let destination = temp_dir("bad-restore");
    let restore = command(&[
        "restore",
        "--input",
        archive.to_str().unwrap(),
        "--dir",
        destination.to_str().unwrap(),
    ]);
    assert!(
        !restore.status.success(),
        "corrupt archive must not restore"
    );
    assert!(!destination.join("appendonly.aof").exists());
    let _ = std::fs::remove_dir_all(&source);
    let _ = std::fs::remove_dir_all(&archive_dir);
    let _ = std::fs::remove_dir_all(&destination);
}

#[test]
fn an_inner_journal_corruption_is_rejected_even_with_a_valid_outer_archive_crc() {
    let source = temp_dir("inner-source");
    let archive_dir = temp_dir("inner-archive");
    let archive = archive_dir.join("snapshot.rcb");
    {
        let server = start(&source);
        let mut stream = connect(&server);
        assert_eq!(call(&mut stream, &[b"SET", b"k", b"v"]), b"+OK\r\n");
        server.shutdown().unwrap();
    }
    assert!(create(&source, &archive).status.success());
    let mut bytes = std::fs::read(&archive).unwrap();
    let payload_start = MAGIC.len() + 8 + 4;
    let value_at = bytes[payload_start..]
        .windows(7)
        .position(|w| w == b"$1\r\nv\r\n")
        .unwrap()
        + payload_start
        + 4;
    bytes[value_at] ^= 1;
    let payload_len = bytes.len() - payload_start;
    let payload_crc = crc32(&bytes[payload_start..]);
    bytes[MAGIC.len()..MAGIC.len() + 8].copy_from_slice(&(payload_len as u64).to_le_bytes());
    bytes[MAGIC.len() + 8..payload_start].copy_from_slice(&payload_crc.to_le_bytes());
    std::fs::write(&archive, bytes).unwrap();

    let destination = temp_dir("inner-destination");
    let restore = command(&[
        "restore",
        "--input",
        archive.to_str().unwrap(),
        "--dir",
        destination.to_str().unwrap(),
    ]);
    assert!(
        !restore.status.success(),
        "inner AOF checksum must be verified at restore"
    );
    assert!(!destination.join("appendonly.aof").exists());
    let _ = std::fs::remove_dir_all(&source);
    let _ = std::fs::remove_dir_all(&archive_dir);
    let _ = std::fs::remove_dir_all(&destination);
}

#[test]
fn authenticated_journals_back_up_and_restore_only_with_the_matching_key() {
    let source = temp_dir("auth-source");
    let archive_dir = temp_dir("auth-archive");
    let archive = archive_dir.join("snapshot.rcb");
    let key_dir = temp_dir("auth-key");
    let key_file = key_dir.join("journal.key");
    std::fs::write(&key_file, b"0123456789abcdef\n").unwrap();
    let wrong_key = dirs_key(&key_dir, "wrong.key", b"fedcba9876543210\n");
    {
        let server = Server::bind(Config {
            aof_hmac_key_file: Some(key_file.clone()),
            ..config(&source)
        })
        .unwrap()
        .spawn()
        .unwrap();
        let mut stream = connect(&server);
        assert_eq!(call(&mut stream, &[b"SET", b"k", b"v"]), b"+OK\r\n");
        server.shutdown().unwrap();
    }
    let made = command(&[
        "create",
        "--dir",
        source.to_str().unwrap(),
        "--output",
        archive.to_str().unwrap(),
        "--aof-hmac-key-file",
        key_file.to_str().unwrap(),
    ]);
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );
    let verified = command(&[
        "verify",
        "--input",
        archive.to_str().unwrap(),
        "--aof-hmac-key-file",
        key_file.to_str().unwrap(),
    ]);
    assert!(
        verified.status.success(),
        "{}",
        String::from_utf8_lossy(&verified.stderr)
    );
    let wrong = command(&[
        "verify",
        "--input",
        archive.to_str().unwrap(),
        "--aof-hmac-key-file",
        wrong_key.to_str().unwrap(),
    ]);
    assert!(
        !wrong.status.success(),
        "a different key must fail archive replay"
    );

    let destination = temp_dir("auth-destination");
    let restored = command(&[
        "restore",
        "--input",
        archive.to_str().unwrap(),
        "--dir",
        destination.to_str().unwrap(),
        "--aof-hmac-key-file",
        key_file.to_str().unwrap(),
    ]);
    assert!(
        restored.status.success(),
        "{}",
        String::from_utf8_lossy(&restored.stderr)
    );
    {
        let server = Server::bind(Config {
            aof_hmac_key_file: Some(key_file.clone()),
            ..config(&destination)
        })
        .unwrap()
        .spawn()
        .unwrap();
        let mut stream = connect(&server);
        assert_eq!(call(&mut stream, &[b"GET", b"k"]), b"$1\r\nv\r\n");
        server.shutdown().unwrap();
    }
    let _ = std::fs::remove_dir_all(&source);
    let _ = std::fs::remove_dir_all(&archive_dir);
    let _ = std::fs::remove_dir_all(&destination);
    let _ = std::fs::remove_dir_all(&key_dir);
}

fn dirs_key(directory: &Path, name: &str, contents: &[u8]) -> PathBuf {
    let path = directory.join(name);
    std::fs::write(&path, contents).unwrap();
    path
}

#[test]
fn authenticated_archives_accept_a_rotation_keyring() {
    let source = temp_dir("keyring-source");
    let archive_dir = temp_dir("keyring-archive");
    let archive = archive_dir.join("snapshot.rcb");
    let key_dir = temp_dir("keyring-keys");
    let old = b"0123456789abcdef";
    let new = b"fedcba9876543210";
    let keyring = |name: &str, keys: &[&[u8]]| {
        let path = key_dir.join(name);
        let text = keys
            .iter()
            .map(|key| {
                let mut line = key
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                line.push('\n');
                line
            })
            .collect::<String>();
        std::fs::write(&path, text).unwrap();
        path
    };
    let old_ring = keyring("old.keyring", &[old]);
    let rotated_ring = keyring("rotated.keyring", &[new, old]);
    let new_only_ring = keyring("new.keyring", &[new]);
    {
        let server = Server::bind(Config {
            aof_hmac_keyring_file: Some(old_ring.clone()),
            ..config(&source)
        })
        .unwrap()
        .spawn()
        .unwrap();
        let mut stream = connect(&server);
        assert_eq!(call(&mut stream, &[b"SET", b"k", b"v"]), b"+OK\r\n");
        server.shutdown().unwrap();
    }
    let both = command(&[
        "create",
        "--dir",
        source.to_str().unwrap(),
        "--output",
        archive.to_str().unwrap(),
        "--aof-hmac-key-file",
        old_ring.to_str().unwrap(),
        "--aof-hmac-keyring-file",
        old_ring.to_str().unwrap(),
    ]);
    assert!(
        !both.status.success(),
        "key options must be mutually exclusive"
    );
    let made = command(&[
        "create",
        "--dir",
        source.to_str().unwrap(),
        "--output",
        archive.to_str().unwrap(),
        "--aof-hmac-keyring-file",
        old_ring.to_str().unwrap(),
    ]);
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );
    let verified = command(&[
        "verify",
        "--input",
        archive.to_str().unwrap(),
        "--aof-hmac-keyring-file",
        rotated_ring.to_str().unwrap(),
    ]);
    assert!(
        verified.status.success(),
        "{}",
        String::from_utf8_lossy(&verified.stderr)
    );
    let new_only = command(&[
        "verify",
        "--input",
        archive.to_str().unwrap(),
        "--aof-hmac-keyring-file",
        new_only_ring.to_str().unwrap(),
    ]);
    assert!(
        !new_only.status.success(),
        "the new key alone cannot verify the old archive"
    );

    let destination = temp_dir("keyring-destination");
    let restored = command(&[
        "restore",
        "--input",
        archive.to_str().unwrap(),
        "--dir",
        destination.to_str().unwrap(),
        "--aof-hmac-keyring-file",
        rotated_ring.to_str().unwrap(),
    ]);
    assert!(
        restored.status.success(),
        "{}",
        String::from_utf8_lossy(&restored.stderr)
    );
    {
        let server = Server::bind(Config {
            aof_hmac_keyring_file: Some(rotated_ring.clone()),
            ..config(&destination)
        })
        .unwrap()
        .spawn()
        .unwrap();
        let mut stream = connect(&server);
        assert_eq!(call(&mut stream, &[b"GET", b"k"]), b"$1\r\nv\r\n");
        server.shutdown().unwrap();
    }
    let _ = std::fs::remove_dir_all(&source);
    let _ = std::fs::remove_dir_all(&archive_dir);
    let _ = std::fs::remove_dir_all(&destination);
    let _ = std::fs::remove_dir_all(&key_dir);
}
