#![forbid(unsafe_code)]
#![cfg(target_os = "linux")]

//! Real storage-fault evidence for the append-only journal (gap D4).
//!
//! These drills use a kernel-enforced, size-bounded tmpfs as the journal
//! directory. Every failure below is produced by the real filesystem: writes
//! return `ENOSPC`, a large write is truncated by the kernel (a real short
//! write), and a rewrite temporary file runs out of space. Nothing is mocked:
//! there is no fault-injection hook, no patched journal and no simulated error
//! value.
//!
//! The tests only run when `RILLCACHE_MOUNT_FAULTS=1` is set, because they need
//! a mountable tmpfs (root, or passwordless `sudo -n mount`). The required
//! `Tests and offline CI` job runs them with that variable set; a failure to
//! mount is a test failure, never a silent skip.
//!
//! Coverage boundaries:
//!
//! * `appendfsync always` append: write error, acknowledged-prefix recovery,
//!   latch and recovery after the operator frees space.
//! * short write: a kernel-truncated journal record is discarded on restart.
//! * rewrite: temporary-file `ENOSPC` keeps the original journal and latches
//!   the rewrite status; a retry after freeing space succeeds.
//! * startup: an unwritable data directory fails closed.
//!
//! Not covered here (and not claimed): device-level `EIO`, power loss, block
//! rotation and filesystem corruption. Those need real device fault injection
//! (`dm-error`, hardware or a reviewed FUSE harness) and remain open in D4.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, Command, Stdio};
use std::time::{Duration, Instant};

use rillcache::protocol::encode_request;

const ENABLE: &str = "RILLCACHE_MOUNT_FAULTS";

fn enabled() -> bool {
    std::env::var(ENABLE).is_ok_and(|value| value == "1")
}

fn skip(label: &str) -> bool {
    if enabled() {
        return false;
    }
    eprintln!("{label}: skipped; set {ENABLE}=1 with mount privileges to run real storage faults");
    true
}

fn run(argv: &[&str]) -> std::process::Output {
    Command::new(argv[0])
        .args(&argv[1..])
        .output()
        .unwrap_or_else(|error| panic!("cannot run {argv:?}: {error}"))
}

/// A size-bounded tmpfs owned by the test.
struct FaultFs {
    dir: PathBuf,
    mounted: bool,
}

impl FaultFs {
    fn mount(label: &str, size: &str) -> Self {
        assert!(enabled(), "real storage faults require {ENABLE}=1");
        let dir = std::env::temp_dir().join(format!(
            "rillcache-faults-{label}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let options = format!("size={size},mode=0777");
        let dir_text = dir.to_str().unwrap();
        let mut attempt = run(&[
            "mount",
            "-t",
            "tmpfs",
            "-o",
            options.as_str(),
            "tmpfs",
            dir_text,
        ]);
        if !attempt.status.success() {
            // Unprivileged runners use passwordless sudo for mount.
            attempt = run(&[
                "sudo",
                "-n",
                "mount",
                "-t",
                "tmpfs",
                "-o",
                options.as_str(),
                "tmpfs",
                dir_text,
            ]);
        }
        assert!(
            attempt.status.success(),
            "cannot mount the bounded tmpfs ({}); real storage faults cannot be claimed: {}",
            dir.display(),
            String::from_utf8_lossy(&attempt.stderr).trim()
        );
        Self { dir, mounted: true }
    }

    fn resize(&self, size: &str) {
        let options = format!("remount,size={size}");
        let mut attempt = run(&["mount", "-o", &options, self.dir.to_str().unwrap()]);
        if !attempt.status.success() {
            attempt = run(&[
                "sudo",
                "-n",
                "mount",
                "-o",
                &options,
                self.dir.to_str().unwrap(),
            ]);
        }
        assert!(
            attempt.status.success(),
            "cannot remount the tmpfs to {size}: {}",
            String::from_utf8_lossy(&attempt.stderr).trim()
        );
    }

    fn path(&self) -> &Path {
        &self.dir
    }
}

impl Drop for FaultFs {
    fn drop(&mut self) {
        if !self.mounted {
            return;
        }
        self.mounted = false;
        let dir = self.dir.to_str().unwrap().to_owned();
        let attempt = run(&["umount", &dir]);
        if !attempt.status.success() {
            let _ = run(&["sudo", "-n", "umount", &dir]);
        }
        let _ = fs::remove_dir_all(&self.dir);
    }
}

struct Server {
    child: Child,
    addr: String,
    stderr: BufReader<ChildStderr>,
}

impl Server {
    fn start(dir: &Path, fsync: &str) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rillcache"));
        command
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
            .stderr(Stdio::piped());
        let mut child = command.spawn().unwrap();
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
        let stream = TcpStream::connect(&self.addr).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        stream.set_nodelay(true).unwrap();
        stream
    }

    /// Kill without unwinding or a final flush, then reap the zombie so the
    /// journal ownership lock can be reclaimed by the restart.
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
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn call(stream: &mut TcpStream, arguments: &[&[u8]]) -> Vec<u8> {
    let mut request = Vec::new();
    encode_request(arguments, &mut request);
    stream.write_all(&request).unwrap();
    read_reply(stream)
}

fn read_reply(stream: &mut TcpStream) -> Vec<u8> {
    let mut first = [0u8; 1];
    stream.read_exact(&mut first).unwrap();
    let mut header = Vec::new();
    loop {
        let mut byte = [0u8; 1];
        stream.read_exact(&mut byte).unwrap();
        header.push(byte[0]);
        if header.ends_with(b"\r\n") {
            break;
        }
        assert!(header.len() < 8192, "oversized reply header");
    }
    let text = String::from_utf8_lossy(&header[..header.len() - 2]).into_owned();
    match first[0] {
        b'+' | b'-' | b':' => [first.to_vec(), header].concat(),
        b'$' => {
            let length: i64 = text.parse().unwrap();
            let mut payload = Vec::new();
            if length >= 0 {
                payload.resize(usize::try_from(length).unwrap() + 2, 0);
                stream.read_exact(&mut payload).unwrap();
            }
            [first.to_vec(), header, payload].concat()
        }
        b'*' => {
            let count: i64 = text.parse().unwrap();
            let mut out = [first.to_vec(), header].concat();
            for _ in 0..count.max(0) {
                out.extend_from_slice(&read_reply(stream));
            }
            out
        }
        other => panic!("unexpected reply prefix {other:?}"),
    }
}

fn persistence(stream: &mut TcpStream) -> String {
    let reply = call(stream, &[b"INFO", b"persistence"]);
    String::from_utf8_lossy(&reply).into_owned()
}

fn field(info: &str, name: &str) -> String {
    info.lines()
        .find_map(|line| line.strip_prefix(name)?.strip_prefix(':'))
        .unwrap_or_default()
        .trim()
        .to_owned()
}

/// Write 2 KiB values until the journal rejects one, returning the acknowledged
/// keys and the final error reply.
fn fill_until_enospc(stream: &mut TcpStream, prefix: &str, limit: usize) -> (Vec<String>, Vec<u8>) {
    let value = vec![b'x'; 2048];
    let mut acked = Vec::new();
    for index in 0..limit {
        let key = format!("{prefix}{index}");
        let reply = call(stream, &[b"SET", key.as_bytes(), &value]);
        if reply.starts_with(b"+OK") {
            acked.push(key);
            continue;
        }
        assert!(
            reply.starts_with(b"-ERR"),
            "a failed append must reply with an error, got {}",
            String::from_utf8_lossy(&reply)
        );
        return (acked, reply);
    }
    panic!("the bounded filesystem never reported a failed append");
}

#[test]
fn append_enospc_reports_errors_latches_status_and_recovers_the_acked_prefix() {
    if skip("append_enospc") {
        return;
    }
    let fs = FaultFs::mount("append", "512k");
    let data = fs.path().join("data");
    fs::create_dir(&data).unwrap();
    let server = Server::start(&data, "always");
    let mut stream = server.connect();
    let (acked, error) = fill_until_enospc(&mut stream, "acked:", 400);
    assert!(!acked.is_empty(), "some writes must succeed before ENOSPC");
    let text = String::from_utf8_lossy(&error).into_owned();
    assert!(
        text.contains("No space left on device") || text.contains("os error 28"),
        "expected a real ENOSPC error, got {text}"
    );
    let info = persistence(&mut stream);
    assert_eq!("err", field(&info, "aof_last_write_status"));
    // The failed mutation is visible only in memory and was never acknowledged.
    let failed_key = format!("acked:{}", acked.len());
    assert!(
        call(&mut stream, &[b"GET", failed_key.as_bytes()]).starts_with(b"$2048"),
        "the unacknowledged mutation stays in memory until restart"
    );
    // Further mutations fail closed instead of being silently acknowledged.
    let later = call(&mut stream, &[b"SET", b"after", b"v"]);
    assert!(
        later.starts_with(b"-ERR"),
        "{:?}",
        String::from_utf8_lossy(&later)
    );
    assert!(
        String::from_utf8_lossy(&later).contains("failed state"),
        "the journal must latch its failed state: {}",
        String::from_utf8_lossy(&later)
    );
    // Every acknowledged write is readable before the restart.
    for key in &acked {
        assert!(call(&mut stream, &[b"GET", key.as_bytes()]).starts_with(b"$2048"));
    }
    server.kill();

    // Restart on the still-full filesystem recovers exactly the acknowledged prefix.
    let server = Server::start(&data, "always");
    let mut stream = server.connect();
    let size = call(&mut stream, &[b"DBSIZE"]);
    assert_eq!(
        format!(":{}\r\n", acked.len()),
        String::from_utf8_lossy(&size),
        "the recovered keyspace must equal the acknowledged prefix"
    );
    for key in &acked {
        assert!(call(&mut stream, &[b"GET", key.as_bytes()]).starts_with(b"$2048"));
    }
    assert_eq!(
        b"$-1\r\n".to_vec(),
        call(&mut stream, &[b"GET", failed_key.as_bytes()]),
        "an unacknowledged write must not survive the restart"
    );
    // A fresh process starts healthy; the latch is not sticky across restarts.
    assert_eq!(
        "ok",
        field(&persistence(&mut stream), "aof_last_write_status")
    );
    server.kill();

    // After the operator frees space the recovered instance accepts writes again.
    fs.resize("2M");
    let server = Server::start(&data, "always");
    let mut stream = server.connect();
    assert!(call(&mut stream, &[b"SET", b"recovered", b"v"]).starts_with(b"+OK"));
    server.kill();
    let server = Server::start(&data, "always");
    let mut stream = server.connect();
    assert_eq!(
        b"$1\r\nv\r\n".to_vec(),
        call(&mut stream, &[b"GET", b"recovered"])
    );
    assert_eq!(
        format!(":{}\r\n", acked.len() + 1),
        String::from_utf8_lossy(&call(&mut stream, &[b"DBSIZE"]))
    );
    server.kill();
}

#[test]
fn short_write_torn_tail_is_discarded_on_restart() {
    if skip("short_write") {
        return;
    }
    let fs = FaultFs::mount("short", "512k");
    let data = fs.path().join("data");
    fs::create_dir(&data).unwrap();
    let server = Server::start(&data, "always");
    let mut stream = server.connect();
    let (acked, _) = fill_until_enospc(&mut stream, "base:", 400);
    server.kill();

    // Restart with space left for part of one large multi-key write only.
    fs.resize("640k");
    let server = Server::start(&data, "always");
    let mut stream = server.connect();
    let value = vec![b'y'; 96 * 1024];
    let torn = call(
        &mut stream,
        &[b"MSET", b"torn:a", &value, b"torn:b", &value],
    );
    assert!(
        torn.starts_with(b"-ERR"),
        "the straddling multi-key write must fail, got {}",
        String::from_utf8_lossy(&torn)
    );
    assert!(
        String::from_utf8_lossy(&torn).contains("No space left on device"),
        "{}",
        String::from_utf8_lossy(&torn)
    );
    server.kill();

    // The kernel left a partial record; restart must truncate it to the last
    // verified boundary and keep the acknowledged prefix.
    let server = Server::start(&data, "always");
    let mut stream = server.connect();
    assert_eq!(
        format!(":{}\r\n", acked.len()),
        String::from_utf8_lossy(&call(&mut stream, &[b"DBSIZE"]))
    );
    for key in acked.iter().take(8) {
        assert!(call(&mut stream, &[b"GET", key.as_bytes()]).starts_with(b"$2048"));
    }
    assert_eq!(b"$-1\r\n".to_vec(), call(&mut stream, &[b"GET", b"torn:a"]));
    assert_eq!(b"$-1\r\n".to_vec(), call(&mut stream, &[b"GET", b"torn:b"]));
    // The truncated journal accepts new writes once space is available.
    fs.resize("2M");
    assert!(call(&mut stream, &[b"SET", b"after-torn", b"v"]).starts_with(b"+OK"));
    server.kill();
    let server = Server::start(&data, "always");
    let mut stream = server.connect();
    assert_eq!(
        b"$1\r\nv\r\n".to_vec(),
        call(&mut stream, &[b"GET", b"after-torn"])
    );
    assert_eq!(
        format!(":{}\r\n", acked.len() + 1),
        String::from_utf8_lossy(&call(&mut stream, &[b"DBSIZE"]))
    );
    server.kill();
}

fn wait_for_rewrite(stream: &mut TcpStream, expected: &str, deadline: Duration) -> String {
    let until = Instant::now() + deadline;
    loop {
        let info = persistence(stream);
        if field(&info, "aof_last_bgrewrite_status") == expected {
            return info;
        }
        if Instant::now() >= until {
            return info;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn rewrite_enospc_keeps_the_original_journal_and_latches_rewrite_status() {
    if skip("rewrite_enospc") {
        return;
    }
    let fs = FaultFs::mount("rewrite", "1M");
    let data = fs.path().join("data");
    fs::create_dir(&data).unwrap();
    let server = Server::start(&data, "always");
    let mut stream = server.connect();
    let (acked, _) = fill_until_enospc(&mut stream, "r:", 1000);
    assert!(!acked.is_empty());
    let journal = data.join("appendonly.aof");
    let before = fs::metadata(&journal).unwrap().len();

    // The filesystem is full, so the rewrite temporary file cannot be written.
    let requested = call(&mut stream, &[b"BGREWRITEAOF"]);
    assert!(
        requested.starts_with(b"+Background")
            || requested.starts_with(b"+OK")
            || requested.starts_with(b"-ERR"),
        "{}",
        String::from_utf8_lossy(&requested)
    );
    let info = wait_for_rewrite(&mut stream, "err", Duration::from_secs(10));
    assert_eq!(
        "err",
        field(&info, "aof_last_bgrewrite_status"),
        "the rewrite must record its ENOSPC failure: {info}"
    );
    // The live journal and every acknowledged key are untouched.
    assert_eq!(before, fs::metadata(&journal).unwrap().len());
    for key in acked.iter().take(8) {
        assert!(call(&mut stream, &[b"GET", key.as_bytes()]).starts_with(b"$2048"));
    }
    server.kill();

    // Restart on the full filesystem still recovers the acknowledged prefix.
    let server = Server::start(&data, "always");
    let mut stream = server.connect();
    for key in acked.iter().take(8) {
        assert!(call(&mut stream, &[b"GET", key.as_bytes()]).starts_with(b"$2048"));
    }
    server.kill();

    // Freeing space and retrying compacts without losing data.
    fs.resize("2M");
    let server = Server::start(&data, "always");
    let mut stream = server.connect();
    assert!(call(&mut stream, &[b"BGREWRITEAOF"]).starts_with(b"+Background"));
    let info = wait_for_rewrite(&mut stream, "ok", Duration::from_secs(10));
    assert_eq!("ok", field(&info, "aof_last_bgrewrite_status"), "{info}");
    let compacted = fs::metadata(&journal).unwrap().len();
    assert!(
        compacted < before,
        "rewrite must compact {before} -> {compacted}"
    );
    for key in acked.iter().take(8) {
        assert!(call(&mut stream, &[b"GET", key.as_bytes()]).starts_with(b"$2048"));
    }
    assert!(call(&mut stream, &[b"SET", b"serving", b"v"]).starts_with(b"+OK"));
    server.kill();
    let server = Server::start(&data, "always");
    let mut stream = server.connect();
    assert_eq!(
        b"$1\r\nv\r\n".to_vec(),
        call(&mut stream, &[b"GET", b"serving"])
    );
    assert_eq!(
        format!(":{}\r\n", acked.len() + 1),
        String::from_utf8_lossy(&call(&mut stream, &[b"DBSIZE"]))
    );
    server.kill();
}

#[test]
fn startup_with_an_unwritable_data_directory_fails_closed() {
    if skip("startup_permissions") {
        return;
    }
    use std::os::unix::fs::PermissionsExt as _;
    let fs = FaultFs::mount("permissions", "256k");
    let data = fs.path().join("data");
    fs::create_dir(&data).unwrap();
    fs::set_permissions(&data, fs::Permissions::from_mode(0o500)).unwrap();
    // A privileged test process bypasses directory permissions, so the real
    // EACCES fault cannot be produced; report that instead of faking it.
    if fs::write(data.join("probe"), b"x").is_ok() {
        let _ = fs::remove_file(data.join("probe"));
        fs::set_permissions(&data, fs::Permissions::from_mode(0o700)).unwrap();
        eprintln!(
            "startup_permissions: skipped; the test process is privileged and bypasses mode 0500"
        );
        return;
    }
    let output = Command::new(env!("CARGO_BIN_EXE_rillcache"))
        .args(["--bind", "127.0.0.1:0", "--appendonly", "yes", "--dir"])
        .arg(&data)
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "an unwritable data directory must not start"
    );
    let stderr = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
    assert!(
        stderr.contains("permission denied") || stderr.contains("ownership"),
        "unexpected startup error: {stderr}"
    );
    fs::set_permissions(&data, fs::Permissions::from_mode(0o700)).unwrap();
}
