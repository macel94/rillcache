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
