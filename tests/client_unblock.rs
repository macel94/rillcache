#![forbid(unsafe_code)]

//! CLIENT UNBLOCK and blocked-client wake-up semantics.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

fn start() -> ServerHandle {
    let workers = match std::env::var("RILLCACHE_TEST_THREADS").as_deref() {
        Ok("0") | Err(_) => None,
        Ok("1") => Some(1),
        Ok("4") => Some(4),
        Ok(value) => panic!("invalid test worker mode {value}"),
    };
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
    stream.set_nodelay(true).unwrap();
    stream
}

fn send(stream: &mut TcpStream, arguments: &[&[u8]]) {
    let mut bytes = Vec::new();
    encode_request(arguments, &mut bytes);
    stream.write_all(&bytes).unwrap();
}

fn read_frame(stream: &mut TcpStream, depth: usize) -> Vec<u8> {
    assert!(depth <= 8);
    let mut output = Vec::new();
    let mut prefix = [0u8];
    stream.read_exact(&mut prefix).unwrap();
    output.push(prefix[0]);
    let mut line = Vec::new();
    loop {
        let mut byte = [0u8];
        stream.read_exact(&mut byte).unwrap();
        if byte[0] == b'\n' {
            assert_eq!(line.pop(), Some(b'\r'));
            output.extend_from_slice(&line);
            output.extend_from_slice(b"\r\n");
            break;
        }
        line.push(byte[0]);
    }
    match prefix[0] {
        b'$' | b'=' => {
            let length = std::str::from_utf8(&line).unwrap().parse::<i64>().unwrap();
            if length >= 0 {
                let mut body = vec![0; length as usize + 2];
                stream.read_exact(&mut body).unwrap();
                output.extend_from_slice(&body);
            }
        }
        b'*' | b'~' | b'>' => {
            let count = std::str::from_utf8(&line).unwrap().parse::<i64>().unwrap();
            for _ in 0..count.max(0) {
                output.extend_from_slice(&read_frame(stream, depth + 1));
            }
        }
        b'%' => {
            let count = std::str::from_utf8(&line).unwrap().parse::<i64>().unwrap();
            for _ in 0..count.max(0) * 2 {
                output.extend_from_slice(&read_frame(stream, depth + 1));
            }
        }
        _ => {}
    }
    output
}

fn request(stream: &mut TcpStream, arguments: &[&[u8]]) -> Vec<u8> {
    send(stream, arguments);
    read_frame(stream, 0)
}

fn client_id(stream: &mut TcpStream) -> u64 {
    let reply = request(stream, &[b"CLIENT", b"ID"]);
    std::str::from_utf8(&reply[1..reply.len() - 2])
        .unwrap()
        .trim_start_matches(':')
        .parse()
        .unwrap()
}

fn wait_until_blocked(admin: &mut TcpStream) {
    for _ in 0..100 {
        let info = request(admin, &[b"INFO", b"clients"]);
        if String::from_utf8_lossy(&info).contains("blocked_clients:1") {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("client did not become blocked");
}

#[test]
fn unblock_timeout_and_error_modes_wake_only_the_target() {
    let server = start();
    let mut admin = connect(&server);
    let mut blocked = connect(&server);
    let id = client_id(&mut blocked);
    send(&mut blocked, &[b"BLPOP", b"queue", b"0"]);
    wait_until_blocked(&mut admin);
    let reply = request(
        &mut admin,
        &[b"CLIENT", b"UNBLOCK", id.to_string().as_bytes()],
    );
    assert_eq!(reply, b":1\r\n");
    assert_eq!(read_frame(&mut blocked, 0), b"*-1\r\n");
    assert_eq!(
        request(
            &mut admin,
            &[b"CLIENT", b"UNBLOCK", id.to_string().as_bytes()]
        ),
        b":0\r\n"
    );

    send(&mut blocked, &[b"BLPOP", b"queue2", b"0"]);
    wait_until_blocked(&mut admin);
    let reply = request(
        &mut admin,
        &[b"CLIENT", b"UNBLOCK", id.to_string().as_bytes(), b"ERROR"],
    );
    assert_eq!(reply, b":1\r\n");
    assert_eq!(
        read_frame(&mut blocked, 0),
        b"-UNBLOCKED client unblocked via CLIENT UNBLOCK\r\n"
    );

    assert_eq!(
        request(
            &mut admin,
            &[b"CLIENT", b"UNBLOCK", b"999999999", b"TIMEOUT"]
        ),
        b":0\r\n"
    );
    assert_eq!(
        request(
            &mut admin,
            &[b"CLIENT", b"UNBLOCK", id.to_string().as_bytes(), b"BOGUS"]
        ),
        b"-ERR CLIENT UNBLOCK reason should be TIMEOUT or ERROR\r\n"
    );
    server.shutdown().unwrap();
}

#[test]
fn resp3_timeout_unblock_is_a_null_value() {
    let server = start();
    let mut admin = connect(&server);
    let mut blocked = connect(&server);
    assert_eq!(
        request(&mut blocked, &[b"HELLO", b"3"]).first(),
        Some(&b'%')
    );
    let id = client_id(&mut blocked);
    send(&mut blocked, &[b"BLPOP", b"queue3", b"0"]);
    wait_until_blocked(&mut admin);
    assert_eq!(
        request(
            &mut admin,
            &[b"CLIENT", b"UNBLOCK", id.to_string().as_bytes(), b"TIMEOUT"]
        ),
        b":1\r\n"
    );
    assert_eq!(read_frame(&mut blocked, 0), b"_\r\n");
    server.shutdown().unwrap();
}
