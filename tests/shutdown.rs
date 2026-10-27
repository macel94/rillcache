#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rillcache::engine::FsyncPolicy;
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

fn start_persistent(dir: &Path) -> ServerHandle {
    Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        appendonly: true,
        appendfsync: FsyncPolicy::EverySec,
        dir: dir.to_path_buf(),
        appendfilename: "shutdown.aof".into(),
        ..Config::default()
    })
    .unwrap()
    .spawn()
    .unwrap()
}

fn temp_dir(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rillcache-shutdown-{label}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn connect(server: &ServerHandle) -> TcpStream {
    let stream = TcpStream::connect(server.local_addr()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream.set_nodelay(true).unwrap();
    stream
}

fn read_line(stream: &mut TcpStream, output: &mut Vec<u8>) -> Vec<u8> {
    let mut line = Vec::new();
    loop {
        let mut byte = [0];
        stream.read_exact(&mut byte).unwrap();
        if byte[0] == b'\n' {
            assert_eq!(line.pop(), Some(b'\r'));
            output.extend_from_slice(&line);
            output.extend_from_slice(b"\r\n");
            return line;
        }
        line.push(byte[0]);
    }
}

fn read_frame(stream: &mut TcpStream, depth: usize) -> Vec<u8> {
    assert!(depth <= 8);
    let mut output = Vec::new();
    let mut prefix = [0];
    stream.read_exact(&mut prefix).unwrap();
    output.push(prefix[0]);
    match prefix[0] {
        b'+' | b'-' | b':' => {
            read_line(stream, &mut output);
        }
        b'$' => {
            let length: i64 = std::str::from_utf8(&read_line(stream, &mut output))
                .unwrap()
                .parse()
                .unwrap();
            if length >= 0 {
                let mut body = vec![0; length as usize + 2];
                stream.read_exact(&mut body).unwrap();
                output.extend_from_slice(&body);
            }
        }
        b'*' => {
            let count: i64 = std::str::from_utf8(&read_line(stream, &mut output))
                .unwrap()
                .parse()
                .unwrap();
            if count >= 0 {
                for _ in 0..count {
                    output.extend_from_slice(&read_frame(stream, depth + 1));
                }
            }
        }
        other => panic!("unexpected RESP prefix {other:?}"),
    }
    output
}

fn send(stream: &mut TcpStream, arguments: &[&[u8]]) {
    let mut request = Vec::new();
    encode_request(arguments, &mut request);
    stream.write_all(&request).unwrap();
}

fn command(stream: &mut TcpStream, arguments: &[&[u8]], expected: &[u8]) {
    send(stream, arguments);
    assert_eq!(read_frame(stream, 0), expected, "{arguments:?}");
}

fn expect_closed(stream: &mut TcpStream) {
    let mut byte = [0];
    match stream.read(&mut byte) {
        Ok(0) => {}
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::BrokenPipe
            ) => {}
        other => panic!("expected the SHUTDOWN connection to close, got {other:?}"),
    }
}

#[test]
fn shutdown_closes_the_caller_and_stops_the_server_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = connect(&server);
        command(&mut stream, &[b"PING"], b"+PONG\r\n");
        send(&mut stream, &[b"SHUTDOWN"]);
        expect_closed(&mut stream);
        server.shutdown().unwrap();
    }
}

#[test]
fn shutdown_modifier_validation_and_transaction_rejection_keep_the_server_alive() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = connect(&server);

        command(
            &mut stream,
            &[b"SHUTDOWN", b"BOGUS"],
            b"-ERR syntax error\r\n",
        );
        command(
            &mut stream,
            &[b"SHUTDOWN", b"NOSAVE", b"SAVE"],
            b"-ERR syntax error\r\n",
        );
        command(
            &mut stream,
            &[b"SHUTDOWN", b"ABORT"],
            b"-ERR No shutdown in progress.\r\n",
        );
        command(
            &mut stream,
            &[b"SHUTDOWN", b"ABORT", b"NOSAVE"],
            b"-ERR syntax error\r\n",
        );
        command(&mut stream, &[b"PING"], b"+PONG\r\n");

        command(&mut stream, &[b"MULTI"], b"+OK\r\n");
        command(
            &mut stream,
            &[b"SHUTDOWN"],
            b"-ERR SHUTDOWN is not allowed in transactions\r\n",
        );
        command(
            &mut stream,
            &[b"EXEC"],
            b"-EXECABORT Transaction discarded because of previous errors.\r\n",
        );
        command(&mut stream, &[b"PING"], b"+PONG\r\n");

        // Accepted modifiers terminate immediately (NOW/FORCE are no-ops here).
        for modifier in [b"NOSAVE".as_slice(), b"SAVE", b"NOW", b"FORCE"] {
            let server = start(workers);
            let mut stream = connect(&server);
            send(&mut stream, &[b"SHUTDOWN", modifier]);
            expect_closed(&mut stream);
            server.shutdown().unwrap();
        }

        server.shutdown().unwrap();
    }
}

#[test]
fn shutdown_flushes_the_journal_before_stopping() {
    let dir = temp_dir("flush");
    {
        let server = start_persistent(&dir);
        let mut stream = connect(&server);
        command(&mut stream, &[b"SET", b"durable", b"value"], b"+OK\r\n");
        send(&mut stream, &[b"SHUTDOWN", b"SAVE"]);
        expect_closed(&mut stream);
        server.shutdown().unwrap();
    }
    {
        let server = start_persistent(&dir);
        let mut stream = connect(&server);
        command(&mut stream, &[b"GET", b"durable"], b"$5\r\nvalue\r\n");
        send(&mut stream, &[b"SHUTDOWN"]);
        expect_closed(&mut stream);
        server.shutdown().unwrap();
    }
    let _ = std::fs::remove_dir_all(&dir);
}
