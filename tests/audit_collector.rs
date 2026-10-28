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
fn collector_rejects_short_hmac_key() {
    let output = temp_name("short-key-output");
    let key = temp_name("short-hmac-key");
    std::fs::write(&key, b"short").unwrap();
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
    let _ = std::fs::remove_file(output);
    let _ = std::fs::remove_file(key);
}
