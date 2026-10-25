#![forbid(unsafe_code)]

use std::net::TcpListener;
use std::process::Command;

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
fn healthcheck_is_quiet_on_success_and_fails_for_an_unavailable_endpoint() {
    use rillcache::{Config, Server};
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
