#![forbid(unsafe_code)]

//! Declared resident-memory bound (O1).
//!
//! `--memory-bound-bytes` declares an RSS bound and
//! `--memory-bound-headroom-bytes` reserves the part of it that must stay free
//! for stacks, allocator metadata, socket buffers and fragmentation. Startup
//! fails closed when the declaration is incomplete, inconsistent, above the
//! readable cgroup limit, or larger than what the measured baseline resident
//! set allows. The derived payload budget is enforced by the existing payload
//! admission path, so a write that would cross the bound is refused with the
//! documented `-OOM` error instead of being accepted and relying on the kernel.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::Command;
use std::time::Duration;

use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

const MIB: u64 = 1024 * 1024;

/// Resident set of the currently running test process, used as the startup
/// baseline proxy for the in-process server.
fn resident_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|line| line.starts_with("VmRSS:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    kib.checked_mul(1024)
}

fn check_config(arguments: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rillcache"))
        .arg("--check-config")
        .args(arguments)
        .output()
        .unwrap()
}

/// `--check-config` intentionally redacts directive values and paths, so the
/// observable contract is the nonzero exit; the precise rule messages are
/// covered by the unit tests in `src/memory_bound.rs`.
fn assert_rejected(output: &std::process::Output) {
    assert!(
        !output.status.success(),
        "check-config unexpectedly accepted the declaration: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn connect(server: &ServerHandle) -> TcpStream {
    let stream = TcpStream::connect(server.local_addr()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
}

fn command(stream: &mut TcpStream, arguments: &[&[u8]]) -> Vec<u8> {
    let mut request = Vec::new();
    encode_request(arguments, &mut request);
    stream.write_all(&request).unwrap();
    read_reply(stream)
}

/// Read one complete RESP reply, including multi-bulk frames.
fn read_reply(stream: &mut TcpStream) -> Vec<u8> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        stream.read_exact(&mut byte).unwrap();
        head.push(byte[0]);
        if head.ends_with(b"\r\n") {
            break;
        }
    }
    match head.first() {
        Some(b'*') | Some(b'>') => {
            let count: usize = String::from_utf8_lossy(&head[1..head.len() - 2])
                .parse()
                .unwrap();
            let mut body = head;
            for _ in 0..count {
                body.extend_from_slice(&read_reply(stream));
            }
            body
        }
        Some(b'$') | Some(b'(') | Some(b'!') => {
            let length: i64 = String::from_utf8_lossy(&head[1..head.len() - 2])
                .parse()
                .unwrap();
            let mut body = head;
            if length > 0 {
                let mut payload = vec![0u8; length as usize + 2];
                stream.read_exact(&mut payload).unwrap();
                body.extend_from_slice(&payload);
            }
            body
        }
        _ => head,
    }
}

fn field(info: &str, name: &str) -> u64 {
    let value = info
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{name}:")))
        .unwrap_or_else(|| panic!("INFO memory did not report {name}:\n{info}"));
    value
        .trim()
        .parse()
        .unwrap_or_else(|_| panic!("{name} is not numeric: {value}"))
}

fn cgroup_limit() -> Option<u64> {
    for path in [
        "/sys/fs/cgroup/memory.max",
        "/sys/fs/cgroup/memory/memory.limit_in_bytes",
    ] {
        let Ok(raw) = std::fs::read_to_string(path) else {
            continue;
        };
        let raw = raw.trim();
        if raw == "max" {
            return None;
        }
        if let Ok(limit) = raw.parse::<u64>() {
            // cgroup v1 reports an enormous value instead of "max".
            if limit < u64::MAX / 2 {
                return Some(limit);
            }
        }
    }
    None
}

#[test]
fn check_config_rejects_an_incomplete_or_inconsistent_declaration() {
    for arguments in [
        vec!["--memory-bound-bytes", "134217728"],
        vec!["--memory-bound-headroom-bytes", "33554432"],
        vec![
            "--memory-bound-bytes",
            "134217728",
            "--memory-bound-headroom-bytes",
            "134217728",
        ],
        vec![
            "--memory-bound-bytes",
            "134217728",
            "--memory-bound-headroom-bytes",
            "0",
        ],
    ] {
        assert_rejected(&check_config(&arguments));
    }
}

#[test]
fn check_config_fails_closed_when_the_headroom_cannot_fit() {
    let Some(resident) = resident_bytes() else {
        return;
    };
    let headroom = 64 * MIB;
    // Only one MiB of room above the headroom: every real process baseline is
    // larger, so no configuration can honour it.
    let bound = headroom + MIB;
    let output = check_config(&[
        "--memory-bound-bytes",
        &bound.to_string(),
        "--memory-bound-headroom-bytes",
        &headroom.to_string(),
    ]);
    assert_rejected(&output);
    let _ = resident;
}

#[test]
fn check_config_rejects_a_bound_above_the_cgroup_limit() {
    let Some(limit) = cgroup_limit() else {
        return;
    };
    let bound = limit + MIB;
    let output = check_config(&[
        "--memory-bound-bytes",
        &bound.to_string(),
        "--memory-bound-headroom-bytes",
        &(MIB).to_string(),
    ]);
    assert_rejected(&output);
}

#[test]
fn check_config_accepts_a_bound_with_payload_room() {
    let Some(resident) = resident_bytes() else {
        return;
    };
    let bound = resident + 128 * MIB;
    let output = check_config(&[
        "--memory-bound-bytes",
        &bound.to_string(),
        "--memory-bound-headroom-bytes",
        &(64 * MIB).to_string(),
    ]);
    assert!(
        output.status.success(),
        "check-config rejected a workable bound: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn info_reports_the_declared_bound_and_the_derived_payload_limit() {
    let Some(resident) = resident_bytes() else {
        return;
    };
    let headroom = 64 * MIB;
    // Leave about eight MiB of derived payload budget above the baseline.
    let bound = resident + headroom + 8 * MIB;
    let mut limits = rillcache::protocol::Limits::default();
    limits.max_frame_bytes = 48 * 1024 * 1024;
    limits.max_bulk_bytes = 32 * 1024 * 1024;
    limits.max_arguments = rillcache::protocol::Limits::argument_cap(limits.max_frame_bytes);
    let server = Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        max_data_bytes: 64 * 1024 * 1024,
        memory_bound_bytes: Some(bound),
        memory_bound_headroom_bytes: Some(headroom),
        limits,
        ..Config::default()
    })
    .unwrap()
    .spawn()
    .unwrap();

    let mut stream = connect(&server);
    let info = String::from_utf8(command(&mut stream, &[b"INFO", b"memory"])).unwrap();
    assert_eq!(field(&info, "rillcache_memory_bound_bytes"), bound);
    assert_eq!(
        field(&info, "rillcache_memory_bound_headroom_bytes"),
        headroom
    );
    let baseline = field(&info, "rillcache_memory_bound_baseline_rss_bytes");
    let payload_limit = field(&info, "rillcache_memory_bound_payload_limit_bytes");
    assert!(
        baseline <= resident + 64 * MIB,
        "unexpected baseline {baseline}"
    );
    assert_eq!(
        payload_limit,
        (bound - headroom - baseline).min(64 * 1024 * 1024),
        "the derived budget must be the bound minus headroom minus baseline"
    );
    assert!(payload_limit > 0);
    assert!(
        info.contains(
            "rillcache_memory_bound_scope:rss_bound_derived_payload_limit_not_hard_rss_cap"
        )
    );
    assert!(info.contains(
        "rillcache_memory_bound_enforcement:payload_admission_refuses_writes_before_the_bound"
    ));
    assert_eq!(
        field(&info, "rillcache_payload_limit_bytes"),
        payload_limit,
        "the reported payload limit must be the enforced one"
    );

    // A write that would cross the derived budget is refused on the payload
    // admission path, not accepted and deferred to the kernel.
    let value = vec![b'x'; 16 * 1024 * 1024];
    let reply = command(&mut stream, &[b"SET", b"big", &value]);
    assert!(
        reply.starts_with(b"-OOM configured payload limit exceeded"),
        "expected a payload-limit refusal, got {}",
        String::from_utf8_lossy(&reply)
    );
    // The refusal leaves the process alive and still serving.
    assert_eq!(command(&mut stream, &[b"PING"]), b"+PONG\r\n".to_vec());
    let _ = server.shutdown();
}

#[test]
fn a_bound_never_below_the_configured_payload_budget_caps_it() {
    let Some(resident) = resident_bytes() else {
        return;
    };
    let headroom = 16 * MIB;
    // Generous room: this case only proves that a smaller explicit payload
    // budget survives the bound derivation.
    let bound = resident + headroom + 64 * MIB;
    let server = Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        max_data_bytes: 1024,
        memory_bound_bytes: Some(bound),
        memory_bound_headroom_bytes: Some(headroom),
        ..Config::default()
    })
    .unwrap()
    .spawn()
    .unwrap();
    let mut stream = connect(&server);
    let info = String::from_utf8(command(&mut stream, &[b"INFO", b"memory"])).unwrap();
    assert_eq!(
        field(&info, "rillcache_payload_limit_bytes"),
        1024,
        "an explicit smaller payload budget is retained"
    );
    let _ = server.shutdown();
}

/// A real process must survive a memory-bounded environment with the guard
/// enabled, and a process without the guard must fail loudly rather than lose
/// acknowledged writes silently.
#[cfg(target_os = "linux")]
mod process {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::time::Instant;

    fn temp_dir(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "rillcache-memory-bound-{label}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn free_addr() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().to_string()
    }

    struct ChildServer {
        child: Child,
    }

    fn spawn(
        dir: &Path,
        addr: &str,
        address_space_kib: Option<u64>,
        bound: Option<(u64, u64)>,
    ) -> ChildServer {
        let binary = env!("CARGO_BIN_EXE_rillcache");
        let mut command = match address_space_kib {
            // `ulimit -v` bounds the address space so an allocation failure is
            // reachable without any privileged cgroup setup.
            Some(kib) => {
                let mut command = Command::new("bash");
                command
                    .arg("-c")
                    .arg(format!("ulimit -v {kib}; exec \"$0\" \"$@\""))
                    .arg(binary);
                command
            }
            None => Command::new(binary),
        };
        command.args([
            "--bind",
            addr,
            "--appendonly",
            "yes",
            "--appendfsync",
            "always",
            "--max-data-bytes",
            "2147483648",
            "--max-bulk-bytes",
            "8388608",
            "--max-frame-bytes",
            "12582912",
            "--threads",
            "1",
            "--dir",
        ]);
        command.arg(dir);
        if let Some((bound, headroom)) = bound {
            command
                .arg("--memory-bound-bytes")
                .arg(bound.to_string())
                .arg("--memory-bound-headroom-bytes")
                .arg(headroom.to_string());
        }
        let mut child = command
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut startup = BufReader::new(child.stderr.take().unwrap());
        let mut line = String::new();
        startup.read_line(&mut line).unwrap();
        assert!(line.contains(" listening on "), "startup: {line}");
        ChildServer { child }
    }

    fn connect(addr: &str) -> Option<TcpStream> {
        let stream = TcpStream::connect(addr).ok()?;
        // `--appendfsync always` can be slow on a shared runner.
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .ok()?;
        stream
            .set_write_timeout(Some(Duration::from_secs(30)))
            .ok()?;
        stream.set_nodelay(true).ok()?;
        Some(stream)
    }

    fn read_status(stream: &mut TcpStream) -> Option<Vec<u8>> {
        let mut reply = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            stream.read_exact(&mut byte).ok()?;
            reply.push(byte[0]);
            if reply.ends_with(b"\r\n") {
                return Some(reply);
            }
        }
    }

    fn wait_for_exit(child: &mut Child) -> bool {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                return !status.success();
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    #[test]
    fn an_unbounded_process_reports_allocation_failure_without_silent_loss() {
        let dir = temp_dir("unbounded");
        let addr = free_addr();
        let mut server = spawn(&dir, &addr, Some(384 * 1024), None);
        let mut stream = connect(&addr).expect("connect");
        let value = vec![b'v'; 4 * 1024 * 1024];
        let mut acknowledged: Vec<Vec<u8>> = Vec::new();
        for index in 0..200u32 {
            let key = format!("k{index}").into_bytes();
            let mut request = Vec::new();
            encode_request(&[b"SET", &key, &value], &mut request);
            if stream.write_all(&request).is_err() {
                break;
            }
            match read_status(&mut stream) {
                None => break,
                Some(reply) if reply == b"+OK\r\n" => acknowledged.push(key),
                Some(reply) if reply.starts_with(b"-OOM") => {
                    panic!("the child hit the payload budget instead of the address-space limit")
                }
                Some(other) => panic!("unexpected reply: {}", String::from_utf8_lossy(&other)),
            }
        }
        drop(stream);
        assert!(
            wait_for_exit(&mut server.child),
            "an address-space-bound process must die from allocation failure, not exit cleanly"
        );

        // Either the journal replays every acknowledged write, or startup fails
        // closed on the torn tail; it must never start with a hole.
        let restart_addr = free_addr();
        let mut restart_command = Command::new(env!("CARGO_BIN_EXE_rillcache"));
        restart_command
            .args(["--bind", &restart_addr, "--appendonly", "yes", "--dir"])
            .arg(&dir)
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut child = restart_command.spawn().unwrap();
        let mut stdout_stderr = BufReader::new(child.stderr.take().unwrap());
        let mut line = String::new();
        stdout_stderr.read_line(&mut line).unwrap();
        if !line.contains(" listening on ") {
            let diagnostic = line.to_lowercase();
            assert!(
                diagnostic.contains("journal") || diagnostic.contains("aof"),
                "startup neither recovered nor named the journal: {line}"
            );
            let _ = child.kill();
            let _ = child.wait();
            return;
        }
        let mut stream = connect(&restart_addr).expect("connect after restart");
        for key in &acknowledged {
            let reply = command(&mut stream, &[b"GET", key]);
            assert!(
                reply.starts_with(b"$") && !reply.starts_with(b"$-1"),
                "acknowledged write {} was lost: {}",
                String::from_utf8_lossy(key),
                String::from_utf8_lossy(&reply)
            );
        }
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_bounded_process_refuses_writes_and_survives_the_address_space_limit() {
        let dir = temp_dir("bounded");
        let addr = free_addr();
        // A small derived budget keeps the drill short while still proving
        // that the refusal comes from the declared bound.
        let bound = 128 * 1024 * 1024;
        let headroom = 96 * 1024 * 1024;
        let mut server = spawn(&dir, &addr, Some(1024 * 1024), Some((bound, headroom)));
        let mut stream = connect(&addr).expect("connect");
        let value = vec![b'b'; 4 * 1024 * 1024];
        let mut refused = false;
        for index in 0..64u32 {
            let key = format!("b{index}").into_bytes();
            let reply = command(&mut stream, &[b"SET", &key, &value]);
            if reply.starts_with(b"-OOM") {
                refused = true;
                break;
            }
            assert_eq!(
                reply,
                b"+OK\r\n",
                "unexpected reply: {}",
                String::from_utf8_lossy(&reply)
            );
        }
        assert!(
            refused,
            "the declared bound must refuse writes before the address-space limit"
        );
        assert_eq!(command(&mut stream, &[b"PING"]), b"+PONG\r\n".to_vec());
        assert!(
            server.child.try_wait().unwrap().is_none(),
            "the bounded process must still be running"
        );
        drop(stream);
        let _ = server.child.kill();
        let _ = server.child.wait();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
