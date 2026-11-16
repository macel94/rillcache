#![forbid(unsafe_code)]

use std::net::{TcpListener, TcpStream};
use std::process::Command;

// These are functional checks, not a concurrent-load contract. Keep the live
// profiling fixture outside the health probe's actual one-second deadline.
static LIVE_SERVER_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn compiled_binaries_provide_help_and_server_version() {
    for binary in [
        env!("CARGO_BIN_EXE_rillcache"),
        env!("CARGO_BIN_EXE_rillcache-bench"),
    ] {
        let output = Command::new(binary).arg("--help").output().unwrap();
        assert!(output.status.success());
        assert!(String::from_utf8(output.stdout).unwrap().contains("Usage:"));
    }
    let output = Command::new(env!("CARGO_BIN_EXE_rillcache"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("rillcache {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn invalid_cli_arguments_fail_with_usage_status() {
    for binary in [
        env!("CARGO_BIN_EXE_rillcache"),
        env!("CARGO_BIN_EXE_rillcache-bench"),
    ] {
        let output = Command::new(binary)
            .arg("--not-an-option")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains("unknown option")
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn standalone_profiling_validates_all_workloads_and_fails_without_a_target() {
    let _fixture = LIVE_SERVER_TEST.lock().unwrap();
    use std::io::{BufRead, BufReader};
    use std::process::{Child, Stdio};

    struct ServerProcess(Child);
    impl Drop for ServerProcess {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut command = Command::new(env!("CARGO_BIN_EXE_rillcache"));
    command
        .args(["--bind", "127.0.0.1:0"])
        .stderr(Stdio::piped());
    let mode = std::env::var("RILLCACHE_TEST_THREADS").unwrap_or_else(|_| "0".into());
    if mode != "0" {
        command.args(["--threads", &mode]);
    }
    let mut server = ServerProcess(command.spawn().unwrap());
    let mut ready = String::new();
    BufReader::new(server.0.stderr.take().unwrap())
        .read_line(&mut ready)
        .unwrap();
    assert!(ready.contains(" listening on "), "{ready}");
    let addr = ready.split_whitespace().last().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rillcache-bench"))
        .args([
            "--addr",
            addr,
            "--server-pid",
            &server.0.id().to_string(),
            "--profile-idle-ms",
            "20",
            "--requests",
            "19",
            "--clients",
            "4",
            "--pipeline",
            "3",
            "--keyspace",
            "7",
            "--value-size",
            "0",
            "--warmup",
            "0",
            "--runs",
            "1",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = String::from_utf8(output.stdout).unwrap();
    assert_eq!(json.matches("\"requests\":19,").count(), 5);
    assert_eq!(json.matches("\"server_profile\":{").count(), 5);
    assert_eq!(json.matches("\"source\":\"linux-procfs\"").count(), 6);
    assert!(json.contains("\"idle_profile\":{\"source\":\"linux-procfs\""));
    assert!(!json.contains("NaN"));
    assert!(!json.contains("inf"));
    let named_workers = std::fs::read_dir(format!("/proc/{}/task", server.0.id()))
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|entry| std::fs::read_to_string(entry.path().join("comm")).ok())
        .filter(|name| name.trim().starts_with("rillcache-io-"))
        .count();
    assert_eq!(
        named_workers,
        mode.parse::<usize>().unwrap(),
        "actual fixed worker count differs from requested mode"
    );

    let failure = Command::new(env!("CARGO_BIN_EXE_rillcache-bench"))
        .args(["--addr", addr, "--server-pid", "4294967295"])
        .output()
        .unwrap();
    assert_eq!(failure.status.code(), Some(1));
    assert!(failure.stdout.is_empty());
    assert!(!failure.stderr.is_empty());
}

#[test]
fn authenticated_healthcheck_reads_a_bounded_password_file_without_leaking_secrets() {
    let _fixture = LIVE_SERVER_TEST.lock().unwrap();
    use rillcache::{Config, Server};
    use std::time::{SystemTime, UNIX_EPOCH};
    let mode = std::env::var("RILLCACHE_TEST_THREADS").unwrap_or_else(|_| "0".into());
    let secret = b"probe\0\r\n\xff";
    let server = Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        requirepass: Some(secret.to_vec()),
        worker_threads: match mode.as_str() {
            "0" => None,
            _ => Some(mode.parse().unwrap()),
        },
        ..Config::default()
    })
    .unwrap()
    .spawn()
    .unwrap();
    let path = std::env::temp_dir().join(format!(
        "rillcache-healthcheck-password-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let address = server.local_addr().to_string();
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    use std::io::Write;
    file.write_all(&[secret.as_slice(), b"\r\n"].concat())
        .unwrap();
    drop(file);
    let probe = || {
        Command::new(env!("CARGO_BIN_EXE_rillcache"))
            .args(["--healthcheck", &address, "--healthcheck-password-file"])
            .arg(&path)
            .output()
            .unwrap()
    };
    let success = probe();
    assert!(success.status.success(), "{:?}", success.stderr);
    assert!(success.stdout.is_empty());
    assert!(success.stderr.is_empty());
    let unauthenticated = Command::new(env!("CARGO_BIN_EXE_rillcache"))
        .args(["--healthcheck", &address])
        .output()
        .unwrap();
    assert_eq!(unauthenticated.status.code(), Some(1));
    for (content, status) in [
        (b"wrong-probe-secret".to_vec(), 1),
        (Vec::new(), 2),
        (b"\r\n".to_vec(), 2),
        (vec![b'x'; 1025], 2),
    ] {
        std::fs::write(&path, &content).unwrap();
        let failure = probe();
        assert_eq!(failure.status.code(), Some(status));
        assert!(failure.stdout.is_empty());
        assert!(
            !failure
                .stderr
                .windows(secret.len())
                .any(|part| part == secret)
        );
        assert!(
            !failure
                .stderr
                .windows(18)
                .any(|part| part == b"wrong-probe-secret")
        );
    }
    std::fs::remove_file(&path).unwrap();
    assert_eq!(probe().status.code(), Some(2));
    server.shutdown().unwrap();
}

#[test]
fn named_healthcheck_accepts_only_an_enabled_user_with_ping_permission() {
    let _fixture = LIVE_SERVER_TEST.lock().unwrap();
    use rillcache::{Config, Server};
    use std::time::{SystemTime, UNIX_EPOCH};
    let nonce = format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let acl = std::env::temp_dir().join(format!("rillcache-healthcheck-acl-{nonce}"));
    let password = std::env::temp_dir().join(format!("rillcache-healthcheck-secret-{nonce}"));
    std::fs::write(&acl, b"user default on >test-only-admin-secret -@all\nuser probe on >test-only-probe-secret -@all +ping\nuser denied on >test-only-probe-secret -@all\nuser disabled off >test-only-probe-secret -@all +ping\n").unwrap();
    std::fs::write(&password, b"test-only-probe-secret\n").unwrap();
    let mode = std::env::var("RILLCACHE_TEST_THREADS").unwrap_or_else(|_| "0".into());
    let server = Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        aclfile: Some(acl.clone()),
        worker_threads: match mode.as_str() {
            "0" => None,
            _ => Some(mode.parse().unwrap()),
        },
        ..Config::default()
    })
    .unwrap()
    .spawn()
    .unwrap();
    let address = server.local_addr().to_string();
    for user in ["probe", "denied", "disabled", "missing"] {
        for user_first in [false, true] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_rillcache"));
            command.args(["--healthcheck", &address]);
            if user_first {
                command.args(["--healthcheck-user", user]);
            }
            command.arg("--healthcheck-password-file").arg(&password);
            if !user_first {
                command.args(["--healthcheck-user", user]);
            }
            let output = command.output().unwrap();
            assert_eq!(
                output.status.code(),
                Some(if user == "probe" { 0 } else { 1 }),
                "user {user}"
            );
            assert!(output.stdout.is_empty());
            if user == "probe" {
                assert!(output.stderr.is_empty());
            }
            assert!(!String::from_utf8_lossy(&output.stderr).contains("test-only-probe-secret"));
        }
    }
    for extra in [
        vec!["--healthcheck-user", "probe"],
        vec!["--healthcheck-user", ""],
        vec!["--healthcheck-user", "bad user"],
        vec!["--healthcheck-user", "bad|user"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rillcache"))
            .args(["--healthcheck", &address])
            .args(extra)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
    }
    for duplicate in ["--healthcheck-user", "--healthcheck-password-file"] {
        let output = Command::new(env!("CARGO_BIN_EXE_rillcache"))
            .args([
                "--healthcheck",
                &address,
                "--healthcheck-user",
                "probe",
                "--healthcheck-password-file",
            ])
            .arg(&password)
            .args([duplicate, "ignored"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
    }
    server.shutdown().unwrap();
    std::fs::remove_file(acl).unwrap();
    std::fs::remove_file(password).unwrap();
}

#[test]
fn healthcheck_is_quiet_on_success_and_fails_for_an_unavailable_endpoint() {
    let _fixture = LIVE_SERVER_TEST.lock().unwrap();
    use rillcache::{Config, Server};
    use std::io::{Read, Write};
    use std::time::Duration;
    let mode = std::env::var("RILLCACHE_TEST_THREADS").unwrap_or_else(|_| "0".into());
    let server = Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        worker_threads: match mode.as_str() {
            "0" => None,
            _ => Some(mode.parse().unwrap()),
        },
        ..Config::default()
    })
    .unwrap()
    .spawn()
    .unwrap();
    let address = server.local_addr().to_string();
    // spawn() does not promise that the serving thread has started. Establish
    // readiness once, separately from the unchanged CLI deadline; never retry a
    // failed health probe or increase its production timeout.
    {
        let mut ready =
            TcpStream::connect_timeout(&server.local_addr(), Duration::from_secs(3)).unwrap();
        ready
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        ready
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        ready.write_all(b"*1\r\n$4\r\nPING\r\n").unwrap();
        let mut reply = [0; 7];
        ready.read_exact(&mut reply).unwrap();
        assert_eq!(&reply, b"+PONG\r\n");
    }
    let output = Command::new(env!("CARGO_BIN_EXE_rillcache"))
        .args(["--healthcheck", &address])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    server.shutdown().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rillcache"))
        .args(["--healthcheck", &address])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("health check failed"));
}

#[test]
fn bind_failure_is_reported_with_a_nonzero_status() {
    let occupied = TcpListener::bind("127.0.0.1:0").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rillcache"))
        .args(["--bind", &occupied.local_addr().unwrap().to_string()])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty());
}
