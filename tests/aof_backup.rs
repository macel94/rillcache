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
fn offline_rollback_recovers_checkpoint_without_overwriting_newer_journal() {
    let source = temp_dir("rollback-source");
    let rollback = temp_dir("rollback-target");
    let archive = source.join("checkpoint.rcb");
    let source_journal = source.join("appendonly.aof");
    let key = b"key\0";
    let before = b"old\xff";

    let server = start(&source);
    let mut stream = connect(&server);
    assert_eq!(call(&mut stream, &[b"SET", key, before]), b"+OK\r\n");
    assert_eq!(call(&mut stream, &[b"PEXPIRE", key, b"600000"]), b":1\r\n");
    let deadline = call(&mut stream, &[b"PEXPIRETIME", key]);
    let expires_at = std::str::from_utf8(&deadline[1..deadline.len() - 2])
        .unwrap()
        .parse::<i64>()
        .unwrap();
    assert!(expires_at > 0);
    server.shutdown().unwrap();

    let created = create(&source, &archive);
    assert!(created.status.success(), "{created:?}");
    let archive_bytes = std::fs::read(&archive).unwrap();
    let verified = command(&["verify", "--input", archive.to_str().unwrap()]);
    assert!(verified.status.success(), "{verified:?}");

    let server = start(&source);
    let mut stream = connect(&server);
    assert_eq!(call(&mut stream, &[b"SET", key, b"new"]), b"+OK\r\n");
    assert_eq!(
        call(&mut stream, &[b"SET", b"new-key", b"new-only"]),
        b"+OK\r\n"
    );
    server.shutdown().unwrap();
    let newer_journal = std::fs::read(&source_journal).unwrap();

    let restore = |target: &Path| {
        command(&[
            "restore",
            "--input",
            archive.to_str().unwrap(),
            "--dir",
            target.to_str().unwrap(),
        ])
    };
    let in_place = restore(&source);
    assert!(
        !in_place.status.success(),
        "rollback must not overwrite a journal"
    );
    assert!(String::from_utf8_lossy(&in_place.stderr).contains("refusing to overwrite"));
    let restored = restore(&rollback);
    assert!(restored.status.success(), "{restored:?}");

    let server = start(&rollback);
    let mut stream = connect(&server);
    assert_eq!(call(&mut stream, &[b"GET", key]), b"$4\r\nold\xff\r\n");
    assert_eq!(call(&mut stream, &[b"GET", b"new-key"]), b"$-1\r\n");
    assert_eq!(call(&mut stream, &[b"DBSIZE"]), b":1\r\n");
    assert_eq!(call(&mut stream, &[b"PEXPIRETIME", key]), deadline);
    let memory = call(&mut stream, &[b"INFO", b"memory"]);
    let memory = std::str::from_utf8(&memory).unwrap();
    assert!(
        memory
            .lines()
            .any(|line| line == "rillcache_payload_bytes:8"),
        "{memory}"
    );
    server.shutdown().unwrap();

    assert_eq!(std::fs::read(&source_journal).unwrap(), newer_journal);
    assert_eq!(std::fs::read(&archive).unwrap(), archive_bytes);
    std::fs::remove_dir_all(source).unwrap();
    std::fs::remove_dir_all(rollback).unwrap();
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
fn verify_can_stage_on_an_explicit_volume_and_cleans_up() {
    let source = temp_dir("verify-source");
    let scratch = temp_dir("verify-scratch");
    let archive = source.join("snapshot.rcb");
    let server = start(&source);
    let mut stream = connect(&server);
    assert_eq!(call(&mut stream, &[b"SET", b"key", b"value"]), b"+OK\r\n");
    server.shutdown().unwrap();
    assert!(create(&source, &archive).status.success());
    let missing = scratch.join("missing");
    let verify = |directory: Option<&Path>| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_rillcache-backup"));
        cmd.args(["verify", "--input"])
            .arg(&archive)
            .env("TMPDIR", &missing);
        if let Some(directory) = directory {
            cmd.arg("--temp-dir").arg(directory);
        }
        cmd.output().unwrap()
    };
    assert!(!verify(None).status.success());
    for invalid in [&missing, &archive] {
        let result = verify(Some(invalid));
        assert!(!result.status.success());
        assert!(
            String::from_utf8_lossy(&result.stderr).contains("verification temporary directory")
        );
    }
    let verified = verify(Some(&scratch));
    assert!(
        verified.status.success(),
        "{}",
        String::from_utf8_lossy(&verified.stderr)
    );
    assert_eq!(std::fs::read_dir(&scratch).unwrap().count(), 0);
    let mut corrupt = std::fs::read(&archive).unwrap();
    corrupt[MAGIC.len() + 8] ^= 1;
    std::fs::write(&archive, corrupt).unwrap();
    let rejected = verify(Some(&scratch));
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("CRC32 does not match"));
    assert_eq!(std::fs::read_dir(&scratch).unwrap().count(), 0);
    std::fs::remove_dir_all(source).unwrap();
    std::fs::remove_dir_all(scratch).unwrap();
}

#[test]
fn verify_temp_dir_rejects_duplicate_empty_missing_and_other_actions() {
    for (args, error) in [
        (
            &[
                "verify",
                "--input",
                "unused",
                "--temp-dir",
                "a",
                "--temp-dir",
                "b",
            ][..],
            "only once",
        ),
        (
            &["verify", "--input", "unused", "--temp-dir", ""][..],
            "nonempty path",
        ),
        (
            &["verify", "--input", "unused", "--temp-dir"][..],
            "requires a value",
        ),
        (
            &[
                "create",
                "--dir",
                ".",
                "--output",
                "unused",
                "--temp-dir",
                ".",
            ][..],
            "only valid with verify",
        ),
        (
            &[
                "restore",
                "--input",
                "unused",
                "--dir",
                ".",
                "--temp-dir",
                ".",
            ][..],
            "only valid with verify",
        ),
    ] {
        let rejected = command(args);
        assert!(!rejected.status.success());
        assert!(String::from_utf8_lossy(&rejected.stderr).contains(error));
    }
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
fn backup_refuses_to_truncate_a_torn_source_journal() {
    for integrity in ["legacy", "crc", "hmac"] {
        let source = temp_dir(integrity);
        let output = temp_dir(&format!("{integrity}-output"));
        let archive = output.join("snapshot.rcb");
        let journal = source.join("appendonly.aof");
        let key_file =
            (integrity == "hmac").then(|| dirs_key(&source, "journal.key", b"0123456789abcdef\n"));
        let start = || {
            Server::bind(Config {
                aof_hmac_key_file: key_file.clone(),
                ..config(&source)
            })
            .unwrap()
            .spawn()
            .unwrap()
        };
        let prefix = if integrity == "legacy" {
            let mut prefix = Vec::new();
            encode_request(&[b"SET", b"first", b"one"], &mut prefix);
            let mut complete = prefix.clone();
            encode_request(&[b"SET", b"second", b"two"], &mut complete);
            std::fs::write(&journal, complete).unwrap();
            prefix
        } else {
            let server = start();
            let mut stream = connect(&server);
            assert_eq!(call(&mut stream, &[b"SET", b"first", b"one"]), b"+OK\r\n");
            let prefix = std::fs::read(&journal).unwrap();
            assert_eq!(call(&mut stream, &[b"SET", b"second", b"two"]), b"+OK\r\n");
            drop(stream);
            server.shutdown().unwrap();
            prefix
        };
        let mut torn = std::fs::read(&journal).unwrap();
        assert!(torn.starts_with(&prefix));
        assert!(torn.len() > prefix.len() + 1);
        torn.pop();
        std::fs::write(&journal, &torn).unwrap();

        let mut args = vec![
            "create",
            "--dir",
            source.to_str().unwrap(),
            "--output",
            archive.to_str().unwrap(),
        ];
        if let Some(key_file) = &key_file {
            args.extend(["--aof-hmac-key-file", key_file.to_str().unwrap()]);
        }
        let result = command(&args);
        let error = String::from_utf8_lossy(&result.stderr);
        assert!(
            !result.status.success(),
            "{integrity}: published a recovered prefix"
        );
        assert!(
            error.contains("backup will not truncate it"),
            "{integrity}: {error}"
        );
        assert!(result.stdout.is_empty());
        assert_eq!(
            std::fs::read(&journal).unwrap(),
            torn,
            "{integrity}: source changed"
        );
        assert_eq!(output.read_dir().unwrap().count(), 0);
        assert!(!source.join("appendonly.aof.lock").exists());

        // Startup still recovers the prefix, and the same create can then succeed.
        let server = start();
        assert_eq!(std::fs::read(&journal).unwrap(), prefix);
        let mut stream = connect(&server);
        assert_eq!(call(&mut stream, &[b"GET", b"first"]), b"$3\r\none\r\n");
        assert_eq!(call(&mut stream, &[b"EXISTS", b"second"]), b":0\r\n");
        drop(stream);
        server.shutdown().unwrap();
        let result = command(&args);
        assert!(
            result.status.success(),
            "{integrity}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(std::fs::read(&journal).unwrap(), prefix);
        std::fs::remove_dir_all(source).unwrap();
        std::fs::remove_dir_all(output).unwrap();
    }
}

#[test]
fn verify_and_restore_reject_a_torn_journal_even_with_a_valid_archive_crc() {
    let source = temp_dir("torn-journal-source");
    let scratch = temp_dir("torn-journal-scratch");
    let destination = temp_dir("torn-journal-destination");
    let archive = source.join("snapshot.rcb");
    let server = start(&source);
    let mut stream = connect(&server);
    assert_eq!(call(&mut stream, &[b"SET", b"first", b"one"]), b"+OK\r\n");
    assert_eq!(call(&mut stream, &[b"SET", b"second", b"two"]), b"+OK\r\n");
    server.shutdown().unwrap();
    assert!(create(&source, &archive).status.success());

    let mut bytes = std::fs::read(&archive).unwrap();
    let payload_start = MAGIC.len() + 8 + 4;
    assert!(bytes.len() > payload_start + 1);
    bytes.truncate(bytes.len() - 1); // Tear the final inner checksum frame.
    let length = (bytes.len() - payload_start) as u64;
    let crc = crc32(&bytes[payload_start..]);
    bytes[MAGIC.len()..MAGIC.len() + 8].copy_from_slice(&length.to_le_bytes());
    bytes[MAGIC.len() + 8..payload_start].copy_from_slice(&crc.to_le_bytes());
    std::fs::write(&archive, &bytes).unwrap();

    for (action, option, directory) in [
        ("verify", "--temp-dir", &scratch),
        ("restore", "--dir", &destination),
    ] {
        let result = command(&[
            action,
            "--input",
            archive.to_str().unwrap(),
            option,
            directory.to_str().unwrap(),
        ]);
        assert!(
            !result.status.success(),
            "{action} accepted a recovered prefix"
        );
        assert!(result.stdout.is_empty());
        let error = String::from_utf8_lossy(&result.stderr);
        assert!(error.contains("changed length during replay"), "{error}");
        assert_eq!(std::fs::read_dir(directory).unwrap().count(), 0);
    }
    assert_eq!(std::fs::read(&archive).unwrap(), bytes);
    for directory in [source, scratch, destination] {
        std::fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
fn insufficient_replay_payload_budget_rejects_archive_and_allows_retry() {
    let source = temp_dir("payload-budget-source");
    let scratch = temp_dir("payload-budget-scratch");
    let destination = temp_dir("payload-budget-destination");
    let archive = scratch.join("checkpoint.rcb");
    let stage = scratch.join("verify");
    std::fs::create_dir(&stage).unwrap();
    let entries: [(&[u8], [u8; 512]); 2] = [(b"first", [0; 512]), (b"other", [0xff; 512])];
    {
        let server = start(&source);
        let mut stream = connect(&server);
        for &(key, ref value) in &entries {
            assert_eq!(call(&mut stream, &[b"SET", key, value]), b"+OK\r\n");
        }
        server.shutdown().unwrap();
    }
    let created = create(&source, &archive);
    assert!(created.status.success(), "{created:?}");
    let original = std::fs::read(&archive).unwrap();
    let input = archive.to_str().unwrap();
    let staging = stage.to_str().unwrap();
    let target = destination.to_str().unwrap();

    // Each write fits individually; cumulative logical payload exceeds 800 B.
    // Fail after a verified prefix has already populated the staged store.
    for (action, option, directory) in [
        ("verify", "--temp-dir", staging),
        ("restore", "--dir", target),
    ] {
        let rejected = command(&[
            action,
            "--input",
            input,
            option,
            directory,
            "--max-data-bytes",
            "800",
        ]);
        assert!(
            !rejected.status.success(),
            "{action} accepted too small a budget"
        );
        let error = String::from_utf8_lossy(&rejected.stderr);
        assert!(
            error.contains(
                "backup journal failed replay validation: AOF command failed during replay"
            ),
            "{error}"
        );
        assert!(std::fs::read_dir(&stage).unwrap().next().is_none());
        assert!(std::fs::read_dir(&destination).unwrap().next().is_none());
        assert_eq!(std::fs::read(&archive).unwrap(), original);
    }

    for (action, option, directory) in [
        ("verify", "--temp-dir", staging),
        ("restore", "--dir", target),
    ] {
        let accepted = command(&[
            action,
            "--input",
            input,
            option,
            directory,
            "--max-data-bytes",
            "2048",
        ]);
        assert!(accepted.status.success(), "{accepted:?}");
    }
    assert!(std::fs::read_dir(&stage).unwrap().next().is_none());
    let server = Server::bind(Config {
        max_data_bytes: 2048,
        ..config(&destination)
    })
    .unwrap()
    .spawn()
    .unwrap();
    let mut stream = connect(&server);
    assert_eq!(call(&mut stream, &[b"DBSIZE"]), b":2\r\n");
    for &(key, ref value) in &entries {
        assert_eq!(
            call(&mut stream, &[b"GET", key]),
            [&b"$512\r\n"[..], value, b"\r\n"].concat()
        );
    }
    server.shutdown().unwrap();
    assert_eq!(std::fs::read(&archive).unwrap(), original);

    for path in [source, scratch, destination] {
        std::fs::remove_dir_all(path).unwrap();
    }
}

#[test]
fn authenticated_restore_rejects_the_wrong_key_and_can_retry_in_place() {
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
    let original_archive = std::fs::read(&archive).unwrap();
    let wrong_restore = command(&[
        "restore",
        "--input",
        archive.to_str().unwrap(),
        "--dir",
        destination.to_str().unwrap(),
        "--aof-hmac-key-file",
        wrong_key.to_str().unwrap(),
    ]);
    assert!(!wrong_restore.status.success());
    assert!(wrong_restore.stdout.is_empty());
    let error = String::from_utf8_lossy(&wrong_restore.stderr);
    assert!(
        error.contains("AOF authentication anchor does not match"),
        "{error}"
    );
    assert!(
        destination.read_dir().unwrap().next().is_none(),
        "wrong-key restore must leave no published journal, staging file or lock"
    );
    assert_eq!(std::fs::read(&archive).unwrap(), original_archive);

    // No manual cleanup or a new destination should be needed for the retry.
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
    assert_eq!(std::fs::read(&archive).unwrap(), original_archive);
    let _ = std::fs::remove_dir_all(&archive_dir);
    let _ = std::fs::remove_dir_all(&destination);
    let _ = std::fs::remove_dir_all(&key_dir);
}

fn dirs_key(directory: &Path, name: &str, contents: &[u8]) -> PathBuf {
    let path = directory.join(name);
    std::fs::write(&path, contents).unwrap();
    path
}
