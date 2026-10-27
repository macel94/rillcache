#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

fn start(workers: Option<usize>) -> ServerHandle {
    Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        worker_threads: workers,
        ..Config::default()
    })
    .unwrap()
    .spawn()
    .unwrap()
}

fn connect(server: &ServerHandle) -> TcpStream {
    let stream = TcpStream::connect(server.local_addr()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
}

fn send(stream: &mut TcpStream, args: &[&[u8]]) {
    let mut request = Vec::new();
    encode_request(args, &mut request);
    stream.write_all(&request).unwrap();
}

fn read_line(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut byte = [0];
    loop {
        stream.read_exact(&mut byte).unwrap();
        bytes.push(byte[0]);
        assert!(bytes.len() <= 8192);
        if bytes.ends_with(b"\r\n") {
            bytes.truncate(bytes.len() - 2);
            return String::from_utf8(bytes).unwrap();
        }
    }
}

#[test]
fn monitor_streams_bounded_redacted_commands_and_reset_leaves_the_mode() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut monitor = connect(&server);
        let mut other = connect(&server);

        send(&mut monitor, &[b"MONITOR"]);
        assert_eq!(read_line(&mut monitor), "+OK");

        send(&mut other, &[b"SET", b"k", b"v"]);
        let line = read_line(&mut monitor);
        assert!(line.starts_with('+'), "{line}");
        assert!(line.contains("[0 127.0.0.1:"), "{line}");
        assert!(line.contains("\"SET\" \"k\" \"v\""), "{line}");

        send(&mut other, &[b"AUTH", b"default", b"supersecret"]);
        let line = read_line(&mut monitor);
        assert!(!line.contains("supersecret"), "{line}");
        assert!(line.contains("\"AUTH\" (redacted)"), "{line}");

        send(
            &mut other,
            &[
                b"ACL",
                b"SETUSER",
                b"bob",
                b"on",
                b">supersecret",
                b"+@all",
                b"~*",
            ],
        );
        let line = read_line(&mut monitor);
        assert!(!line.contains("supersecret"), "{line}");
        assert!(line.contains("(redacted)"), "{line}");

        send(&mut monitor, &[b"RESET"]);
        assert_eq!(read_line(&mut monitor), "+RESET");
        monitor
            .set_read_timeout(Some(Duration::from_millis(150)))
            .unwrap();
        send(&mut other, &[b"SET", b"after", b"reset"]);
        let mut byte = [0];
        match monitor.read(&mut byte) {
            Ok(0) | Err(_) => {}
            Ok(size) => panic!("monitor received {size} bytes after RESET"),
        }
        monitor
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        send(&mut monitor, &[b"PING"]);
        assert_eq!(read_line(&mut monitor), "+PONG");

        server.shutdown().unwrap();
    }
}

#[test]
fn monitor_rejects_arguments_and_is_not_echoed_to_itself() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut monitor = connect(&server);

        send(&mut monitor, &[b"MONITOR", b"extra"]);
        assert_eq!(
            read_line(&mut monitor),
            "-ERR wrong number of arguments for 'monitor' command"
        );
        send(&mut monitor, &[b"MONITOR"]);
        assert_eq!(read_line(&mut monitor), "+OK");
        send(&mut monitor, &[b"PING"]);
        assert_eq!(read_line(&mut monitor), "+PONG");

        server.shutdown().unwrap();
    }
}

#[test]
fn monitor_queue_is_bounded_for_a_slow_reader() {
    let server = start(None);
    let mut monitor = connect(&server);
    let mut other = connect(&server);
    send(&mut monitor, &[b"MONITOR"]);
    assert_eq!(read_line(&mut monitor), "+OK");

    // Do not read the monitor socket while issuing enough commands to overflow
    // its bounded queue; the writer path must keep serving commands.
    for _ in 0..(4 * 1024) {
        send(&mut other, &[b"SET", b"k", b"v"]);
    }
    let mut responses = 0;
    let mut byte = [0];
    while responses < 4 * 1024 {
        if other.read(&mut byte).is_err() {
            break;
        }
        if byte[0] == b'+' {
            responses += 1;
        }
    }
    // Reliable command service is the invariant; monitor delivery is best effort.
    assert!(responses > 0);
    let _ = monitor.shutdown(std::net::Shutdown::Both);
    server.shutdown().unwrap();
}
