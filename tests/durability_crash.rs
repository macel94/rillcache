#![forbid(unsafe_code)]
#![cfg(target_os = "linux")]

//! Crash-consistency evidence for the append-only journal (gap D4).
//!
//! Unlike the in-process persistence tests, these spawn the real binary so the
//! process can be killed without unwinding, running destructors or flushing
//! buffers. The two contracts asserted here are deliberately narrow and testable:
//!
//! 1. With `appendfsync always`, every write the client saw acknowledged is
//!    present after `SIGKILL` and restart: the reply must not precede the durable
//!    append.
//! 2. With `appendfsync everysec`, the recovered keys form a contiguous prefix of
//!    the write order. A crash may drop un-fsynced writes, but it must never
//!    recover a hole or a later write without the earlier ones.
//!
//! ENOSPC, fsync failure and permission loss are not injectable without either
//! privileges or unsafe syscalls, so they remain open in the readiness checklist.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, Command, Stdio};
use std::time::Duration;

use rillcache::protocol::encode_request;

fn temp_dir(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rillcache-durability-{label}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

struct Server {
    child: Child,
    addr: String,
    stderr: BufReader<ChildStderr>,
}

impl Server {
    fn start(dir: &Path, fsync: &str) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rillcache"))
            .args([
                "--bind",
                "127.0.0.1:0",
                "--appendonly",
                "yes",
                "--appendfsync",
                fsync,
                "--dir",
            ])
            .arg(dir)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stderr = BufReader::new(child.stderr.take().unwrap());
        let mut line = String::new();
        stderr.read_line(&mut line).unwrap();
        assert!(line.contains(" listening on "), "startup: {line}");
        let addr = line
            .split(" listening on ")
            .nth(1)
            .expect("listener address")
            .trim()
            .to_owned();
        Self {
            child,
            addr,
            stderr,
        }
    }

    fn connect(&self) -> TcpStream {
        self.connect_with_timeout(Duration::from_secs(5))
    }

    /// The rewrite-crash test pipelines 100 `appendfsync always` writes per
    /// batch. A slow shared runner can need more than five seconds to fsync
    /// those batches, so that test opts into a longer socket deadline instead
    /// of treating scheduler/disk latency as a protocol failure.
    fn connect_with_timeout(&self, timeout: Duration) -> TcpStream {
        let stream = TcpStream::connect(&self.addr).unwrap();
        stream.set_read_timeout(Some(timeout)).unwrap();
        stream.set_write_timeout(Some(timeout)).unwrap();
        stream.set_nodelay(true).unwrap();
        stream
    }

    /// Kill without unwinding, destructors or a final flush.
    fn kill(mut self) {
        self.child.kill().unwrap();
        let status = self.child.wait().unwrap();
        assert!(
            !status.success(),
            "SIGKILL should not look like a clean exit"
        );
        let mut log = String::new();
        let _ = self.stderr.read_to_string(&mut log);
    }

    fn terminate(mut self) {
        let signal_status = Command::new("/bin/kill")
            .args(["-TERM", &self.child.id().to_string()])
            .status()
            .unwrap();
        assert!(signal_status.success(), "could not send SIGTERM");
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "SIGTERM did not finish server shutdown"
            );
            std::thread::yield_now();
        };
        assert!(status.success(), "SIGTERM exit: {status}");
        let mut log = String::new();
        let _ = self.stderr.read_to_string(&mut log);
        assert!(log.contains("received SIGTERM"), "signal log: {log}");
    }

    fn shutdown(mut self) {
        let mut stream = self.connect();
        stream.write_all(b"*1\r\n$8\r\nSHUTDOWN\r\n").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success(), "shutdown exit: {status}");
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "shutdown did not join"
            );
            std::thread::yield_now();
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn send(stream: &mut TcpStream, arguments: &[&[u8]], reply: &[u8]) {
    let mut bytes = Vec::new();
    encode_request(arguments, &mut bytes);
    stream.write_all(&bytes).unwrap();
    let mut actual = vec![0; reply.len()];
    stream.read_exact(&mut actual).unwrap();
    assert_eq!(actual, reply, "unexpected reply for {arguments:?}");
}

/// `GET` a key, returning `Some(value)` for a hit and `None` for a miss.
fn get(stream: &mut TcpStream, key: &str) -> Option<Vec<u8>> {
    send(stream, &[b"GET", key.as_bytes()], b"");
    let mut prefix = [0u8; 1];
    stream.read_exact(&mut prefix).unwrap();
    match prefix[0] {
        b'_' => {
            let mut nul = [0u8; 2];
            stream.read_exact(&mut nul).unwrap();
            assert_eq!(&nul, b"\r\n");
            None
        }
        b'$' => {
            let mut header = Vec::new();
            let mut byte = [0u8; 1];
            loop {
                stream.read_exact(&mut byte).unwrap();
                if byte[0] == b'\r' {
                    let mut newline = [0u8; 1];
                    stream.read_exact(&mut newline).unwrap();
                    break;
                }
                header.push(byte[0]);
            }
            let length: usize = std::str::from_utf8(&header).unwrap().parse().unwrap();
            let mut body = vec![0u8; length + 2];
            stream.read_exact(&mut body).unwrap();
            body.truncate(length);
            Some(body)
        }
        other => panic!("unexpected GET reply prefix: {}", other as char),
    }
}

fn key(index: usize) -> String {
    format!("durable:{index}")
}

fn value(index: usize) -> String {
    format!("value-{index}")
}

#[test]
fn fsync_always_keeps_every_acknowledged_write_across_sigkill() {
    let dir = temp_dir("always");
    let server = Server::start(&dir, "always");
    let mut stream = server.connect();
    send(&mut stream, &[b"PING"], b"+PONG\r\n");
    const WRITES: usize = 200;
    for index in 0..WRITES {
        send(
            &mut stream,
            &[b"SET", key(index).as_bytes(), value(index).as_bytes()],
            b"+OK\r\n",
        );
    }
    drop(stream);
    server.kill();

    let restarted = Server::start(&dir, "always");
    let mut stream = restarted.connect();
    send(&mut stream, &[b"PING"], b"+PONG\r\n");
    for index in 0..WRITES {
        assert_eq!(
            get(&mut stream, &key(index)).as_deref(),
            Some(value(index).as_bytes()),
            "acknowledged write {index} was not durable after SIGKILL",
        );
    }
    // Recovery must accept new writes on top of the replayed prefix.
    send(&mut stream, &[b"SET", b"after:crash", b"ok"], b"+OK\r\n");
    restarted.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sigterm_stops_admission_and_flushes_acknowledged_writes() {
    let dir = temp_dir("sigterm");
    let server = Server::start(&dir, "always");
    let mut stream = server.connect();
    send(
        &mut stream,
        &[b"SET", b"before:term", b"durable"],
        b"+OK\r\n",
    );
    drop(stream);
    server.terminate();

    let restarted = Server::start(&dir, "always");
    let mut stream = restarted.connect();
    assert_eq!(
        get(&mut stream, "before:term").as_deref(),
        Some(b"durable".as_slice()),
        "SIGTERM shutdown lost an acknowledged write",
    );
    restarted.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn everysec_recovers_a_contiguous_prefix_without_holes_after_sigkill() {
    let dir = temp_dir("everysec");
    let server = Server::start(&dir, "everysec");
    let mut stream = server.connect();
    send(&mut stream, &[b"PING"], b"+PONG\r\n");
    const WRITES: usize = 300;
    for index in 0..WRITES {
        send(
            &mut stream,
            &[b"SET", key(index).as_bytes(), value(index).as_bytes()],
            b"+OK\r\n",
        );
    }
    drop(stream);
    server.kill();

    let restarted = Server::start(&dir, "everysec");
    let mut stream = restarted.connect();
    send(&mut stream, &[b"PING"], b"+PONG\r\n");
    let mut recovered = 0usize;
    let mut missing_seen = false;
    for index in 0..WRITES {
        match get(&mut stream, &key(index)) {
            Some(actual) => {
                assert_eq!(actual, value(index).as_bytes(), "wrong value at {index}");
                assert!(
                    !missing_seen,
                    "recovered key {index} after a gap: the journal must replay a prefix",
                );
                recovered += 1;
            }
            None => missing_seen = true,
        }
    }
    assert!(
        recovered > 0,
        "everysec should still persist kernel-buffered writes across SIGKILL",
    );
    restarted.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_kill_during_a_background_rewrite_keeps_the_live_journal_and_all_acked_writes() {
    let dir = temp_dir("rewrite-crash");
    let server = Server::start(&dir, "always");
    let mut stream = server.connect_with_timeout(Duration::from_secs(120));
    send(&mut stream, &[b"PING"], b"+PONG\r\n");
    const WRITES: usize = 20_000;
    let value = [b'v'; 64];
    for chunk in 0..WRITES / 100 {
        let mut bytes = Vec::new();
        for index in chunk * 100..chunk * 100 + 100 {
            let key = key(index);
            rillcache::protocol::encode_request(&[b"SET", key.as_bytes(), &value], &mut bytes);
        }
        stream.write_all(&bytes).unwrap();
        let mut replies = vec![0u8; 5 * 100];
        stream.read_exact(&mut replies).unwrap();
    }
    // Start the cooperative rewrite and confirm it is actually running before the
    // process dies, so the kill lands mid-rewrite rather than before it starts.
    send(
        &mut stream,
        &[b"BGREWRITEAOF"],
        b"+Background append only file rewriting started\r\n",
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        stream
            .write_all(b"*2\r\n$4\r\nINFO\r\n$11\r\npersistence\r\n")
            .unwrap();
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            stream.read_exact(&mut byte).unwrap();
            head.push(byte[0]);
            if head.ends_with(b"\r\n") {
                break;
            }
        }
        let length: usize = std::str::from_utf8(&head[1..head.len() - 2])
            .unwrap()
            .parse()
            .unwrap();
        let mut body = vec![0u8; length + 2];
        stream.read_exact(&mut body).unwrap();
        if String::from_utf8_lossy(&body).contains("aof_rewrite_in_progress:1") {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "rewrite never started"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    drop(stream);
    server.kill();

    // The old journal must still hold every acknowledged write; the temporary
    // rewrite file must not be replayed.
    let restarted = Server::start(&dir, "always");
    let mut stream = restarted.connect_with_timeout(Duration::from_secs(120));
    send(&mut stream, &[b"PING"], b"+PONG\r\n");
    for index in 0..WRITES {
        let mut reply = Vec::new();
        send(&mut stream, &[b"GET", key(index).as_bytes()], b"");
        let mut byte = [0u8; 1];
        stream.read_exact(&mut byte).unwrap();
        assert_eq!(byte[0], b'$', "key {index} missing after crash");
        loop {
            stream.read_exact(&mut byte).unwrap();
            reply.push(byte[0]);
            if reply.ends_with(b"\r\n") {
                break;
            }
        }
        let length: usize = std::str::from_utf8(&reply[..reply.len() - 2])
            .unwrap()
            .parse()
            .unwrap();
        let mut body = vec![0u8; length + 2];
        stream.read_exact(&mut body).unwrap();
        assert_eq!(&body[..length], &value, "key {index} value differs");
    }
    // Recovery must accept a fresh rewrite after the interrupted one.
    send(
        &mut stream,
        &[b"BGREWRITEAOF"],
        b"+Background append only file rewriting started\r\n",
    );
    restarted.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}
