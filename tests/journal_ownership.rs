#![forbid(unsafe_code)]
#![cfg(target_os = "linux")]

//! Exclusive append-only journal ownership (readiness gap D8).
//!
//! Two processes pointed at the same journal would interleave appends and corrupt
//! history. Startup claims `<journal>.lock`; a stale lock left by a crash is
//! reclaimed only when the recorded process is gone or its PID has been reused.
//! This is a local-filesystem advisory lock: shared filesystems are unsupported
//! and documented as such.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rillcache::engine::FsyncPolicy;
use rillcache::{Config, Server};

fn temp_dir(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rillcache-ownership-{label}-{}-{:?}",
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
        appendfilename: "owner.aof".into(),
        ..Config::default()
    }
}

fn start(dir: &Path) -> rillcache::ServerHandle {
    Server::bind(config(dir)).unwrap().spawn().unwrap()
}

fn lock_path(dir: &Path) -> PathBuf {
    dir.join("owner.aof.lock")
}

fn send(stream: &mut TcpStream, arguments: &[&[u8]], reply: &[u8]) {
    let mut bytes = Vec::new();
    rillcache::protocol::encode_request(arguments, &mut bytes);
    stream.write_all(&bytes).unwrap();
    let mut actual = vec![0; reply.len()];
    stream.read_exact(&mut actual).unwrap();
    assert_eq!(actual, reply);
}

#[test]
fn a_second_process_cannot_own_the_same_journal() {
    let dir = temp_dir("exclusive");
    let first = start(&dir);
    let mut stream = TcpStream::connect(first.local_addr()).unwrap();
    stream.set_nodelay(true).unwrap();
    send(&mut stream, &[b"SET", b"k", b"v"], b"+OK\r\n");

    let error = Server::bind(config(&dir))
        .err()
        .expect("second owner must fail");
    let message = error.to_string();
    assert!(
        message.contains("already owned by process") && message.contains("--dir"),
        "unexpected ownership error: {message}"
    );

    // The first server keeps working and still owns the journal.
    send(&mut stream, &[b"GET", b"k"], b"$1\r\nv\r\n");
    first.shutdown().unwrap();
    assert!(
        !lock_path(&dir).exists(),
        "clean shutdown releases the lock"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_stale_lock_from_a_dead_process_is_reclaimed() {
    let dir = temp_dir("stale");
    // PID 4_000_000 is above the default pid_max on this platform and cannot be
    // live; the recorded start time is arbitrary because liveness fails first.
    std::fs::write(lock_path(&dir), "4000000 12345\n").unwrap();
    let server = start(&dir);
    let mut stream = TcpStream::connect(server.local_addr()).unwrap();
    stream.set_nodelay(true).unwrap();
    send(&mut stream, &[b"SET", b"after", b"reclaim"], b"+OK\r\n");
    send(&mut stream, &[b"GET", b"after"], b"$7\r\nreclaim\r\n");
    assert!(
        lock_path(&dir).exists(),
        "the new owner writes its own lock"
    );
    server.shutdown().unwrap();
    assert!(!lock_path(&dir).exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_lock_naming_a_live_process_blocks_startup_even_after_restart() {
    let dir = temp_dir("live-pid");
    let stat = std::fs::read_to_string(format!("/proc/{}/stat", std::process::id())).unwrap();
    let start: u64 = stat
        .rsplit_once(") ")
        .unwrap()
        .1
        .split_whitespace()
        .nth(19)
        .unwrap()
        .parse()
        .unwrap();
    std::fs::write(lock_path(&dir), format!("{} {start}\n", std::process::id())).unwrap();
    let error = Server::bind(config(&dir))
        .err()
        .expect("live owner blocks startup");
    assert!(error.to_string().contains("already owned by process"));
    // The test process's own lock file is left untouched.
    assert!(lock_path(&dir).exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn journal_ownership_is_independent_of_a_clean_restart_and_appendonly_off() {
    let dir = temp_dir("restart");
    {
        let server = start(&dir);
        let mut stream = TcpStream::connect(server.local_addr()).unwrap();
        stream.set_nodelay(true).unwrap();
        send(&mut stream, &[b"SET", b"persisted", b"yes"], b"+OK\r\n");
        server.shutdown().unwrap();
    }
    assert!(!lock_path(&dir).exists());
    {
        let server = start(&dir);
        let mut stream = TcpStream::connect(server.local_addr()).unwrap();
        stream.set_nodelay(true).unwrap();
        send(&mut stream, &[b"GET", b"persisted"], b"$3\r\nyes\r\n");
        server.shutdown().unwrap();
    }

    let plain = temp_dir("no-aof");
    let server = Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        dir: plain.clone(),
        ..Config::default()
    })
    .unwrap()
    .spawn()
    .unwrap();
    std::thread::sleep(Duration::from_millis(20));
    assert!(!plain.join("appendonly.aof.lock").exists());
    server.shutdown().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&plain);
}
