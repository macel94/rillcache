#![forbid(unsafe_code)]

//! Integration coverage for the bounded UDP audit collector.

use std::io::{BufRead, BufReader};
use std::net::{SocketAddr, UdpSocket};
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn temp_name(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "rillcache-collector-test-{label}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ))
}

#[test]
fn collector_appends_two_datagrams_and_exits() {
    let output = temp_name("output");
    let mut child = Command::new(env!("CARGO_BIN_EXE_rillcache-audit-collector"))
        .args([
            "--bind",
            "127.0.0.1:0",
            "--output",
            output.to_str().unwrap(),
            "--max-datagrams",
            "2",
            "--print-address",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let address: SocketAddr = line
        .trim()
        .strip_prefix("listening on ")
        .expect("collector should print its bound address")
        .parse()
        .unwrap();
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket.send_to(b"first line", address).unwrap();
    socket.send_to(b"second line\n", address).unwrap();
    let status = child.wait().unwrap();
    assert!(status.success(), "{status:?}");
    let contents = std::fs::read_to_string(&output).unwrap();
    assert_eq!(contents, "first line\nsecond line\n");
    let _ = std::fs::remove_file(output);
}

#[cfg(target_os = "linux")]
#[test]
fn collector_output_failure_reports_counts_without_echoing_the_datagram() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rillcache-audit-collector"))
        .args([
            "--bind",
            "127.0.0.1:0",
            "--output",
            "/dev/full",
            "--print-address",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let address: SocketAddr = line
        .trim()
        .strip_prefix("listening on ")
        .expect("collector address")
        .parse()
        .unwrap();
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket.send_to(b"private-audit-fixture", address).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while child.try_wait().unwrap().is_none() {
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("collector did not exit after an output failure");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let result = child.wait_with_output().unwrap();
    assert!(!result.status.success());
    let error = String::from_utf8_lossy(&result.stderr);
    assert!(
        error.lines().any(|line| line
            == "audit collector accepted 0, rejected 0, rejected_hmac 0, output_failures 1"),
        "{error}"
    );
    assert!(!error.contains("private-audit-fixture"), "{error}");
}

#[test]
fn collector_rejects_invalid_output_configuration() {
    let result = Command::new(env!("CARGO_BIN_EXE_rillcache-audit-collector"))
        .args(["--output", ""])
        .output()
        .unwrap();
    assert!(!result.status.success());
}

#[test]
fn collector_with_hmac_key_rejects_tampered_and_accepts_signed_line() {
    let output = temp_name("hmac-output");
    let key = temp_name("hmac-key");
    std::fs::write(&key, b"0123456789abcdef\n").unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_rillcache-audit-collector"))
        .args([
            "--bind",
            "127.0.0.1:0",
            "--output",
            output.to_str().unwrap(),
            "--max-datagrams",
            "1",
            "--hmac-key-file",
            key.to_str().unwrap(),
            "--print-address",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let address: SocketAddr = line
        .trim()
        .strip_prefix("listening on ")
        .expect("collector should print its bound address")
        .parse()
        .unwrap();
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    let tampered =
        b"1700000000000 command toplevel 616c696365 736575 9a6efad980470f46df31f9fd5790964a48b74191182cc31ea7897dabebc8f02e";
    let valid = b"1700000000000 command toplevel 616c696365 736574 9a6efad980470f46df31f9fd5790964a48b74191182cc31ea7897dabebc8f02e";
    socket.send_to(tampered, address).unwrap();
    socket.send_to(valid, address).unwrap();
    let status = child.wait().unwrap();
    assert!(status.success(), "{status:?}");
    let contents = std::fs::read_to_string(&output).unwrap();
    assert_eq!(
        contents,
        "1700000000000 command toplevel 616c696365 736574 9a6efad980470f46df31f9fd5790964a48b74191182cc31ea7897dabebc8f02e\n"
    );
    let _ = std::fs::remove_file(output);
    let _ = std::fs::remove_file(key);
}

#[test]
fn collector_rejects_short_and_oversized_hmac_keys_before_opening_output() {
    let output = temp_name("invalid-key-output");
    let key = temp_name("invalid-hmac-key");
    for invalid in [vec![b'x'; 15], vec![b'x'; 1027]] {
        std::fs::write(&key, invalid).unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_rillcache-audit-collector"))
            .args([
                "--bind",
                "127.0.0.1:0",
                "--output",
                output.to_str().unwrap(),
                "--hmac-key-file",
                key.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(
            String::from_utf8_lossy(&result.stderr)
                .contains("audit HMAC key must be 16..=1024 bytes")
        );
        assert!(!output.exists());
    }
    std::fs::remove_file(key).unwrap();
}
