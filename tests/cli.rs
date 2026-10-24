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
