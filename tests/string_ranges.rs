#![forbid(unsafe_code)]

use rillcache::protocol::encode_request;
use rillcache::{Config, Server};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

fn server() -> rillcache::ServerHandle {
    let workers = match std::env::var("RILLCACHE_TEST_THREADS").as_deref() {
        Ok("1") => Some(1),
        Ok("4") => Some(4),
        _ => None,
    };
    Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        worker_threads: workers,
        max_data_bytes: 256,
        ..Config::default()
    })
    .unwrap()
    .spawn()
    .unwrap()
}
fn command(stream: &mut TcpStream, args: &[&[u8]]) -> Vec<u8> {
    let mut request = Vec::new();
    encode_request(args, &mut request);
    stream.write_all(&request).unwrap();
    let mut header = Vec::new();
    let mut byte = [0];
    loop {
        stream.read_exact(&mut byte).unwrap();
        header.push(byte[0]);
        if header.ends_with(b"\r\n") {
            break;
        }
    }
    if header[0] != b'$' {
        return header;
    }
    let length: isize = std::str::from_utf8(&header[1..header.len() - 2])
        .unwrap()
        .parse()
        .unwrap();
    if length < 0 {
        return header;
    }
    let mut body = vec![0; length as usize + 2];
    stream.read_exact(&mut body).unwrap();
    header.extend(body);
    header
}
fn connect(server: &rillcache::ServerHandle) -> TcpStream {
    let s = TcpStream::connect(server.local_addr()).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    s
}

#[test]
fn binary_empty_ranges_and_inclusive_signed_boundaries() {
    let srv = server();
    let mut c = connect(&srv);
    assert_eq!(command(&mut c, &[b"APPEND", b"k", b"\0\xffabc"]), b":5\r\n");
    assert_eq!(command(&mut c, &[b"STRLEN", b"k"]), b":5\r\n");
    assert_eq!(
        command(&mut c, &[b"GETRANGE", b"k", b"1", b"-2"]),
        b"$3\r\n\xffab\r\n"
    );
    assert_eq!(
        command(&mut c, &[b"GETRANGE", b"k", b"-99", b"99"]),
        b"$5\r\n\0\xffabc\r\n"
    );
    assert_eq!(
        command(
            &mut c,
            &[
                b"GETRANGE",
                b"k",
                b"9223372036854775807",
                b"-9223372036854775808"
            ]
        ),
        b"$0\r\n\r\n"
    );
    assert_eq!(
        command(&mut c, &[b"GETRANGE", b"missing", b"0", b"-1"]),
        b"$0\r\n\r\n"
    );
    assert_eq!(command(&mut c, &[b"APPEND", b"empty", b""]), b":0\r\n");
    assert_eq!(command(&mut c, &[b"STRLEN", b"empty"]), b":0\r\n");
    srv.shutdown().unwrap();
}

#[test]
fn setrange_zero_fill_empty_input_and_errors_are_atomic() {
    let srv = server();
    let mut c = connect(&srv);
    assert_eq!(command(&mut c, &[b"SETRANGE", b"k", b"3", b"x"]), b":4\r\n");
    assert_eq!(command(&mut c, &[b"GET", b"k"]), b"$4\r\n\0\0\0x\r\n");
    assert_eq!(
        command(&mut c, &[b"SETRANGE", b"k", b"9223372036854775808", b""]),
        b"-ERR value is not an integer or out of range\r\n"
    );
    assert_eq!(
        command(&mut c, &[b"SETRANGE", b"k", b"-1", b"x"]),
        b"-ERR offset is out of range\r\n"
    );
    assert_eq!(
        command(&mut c, &[b"SETRANGE", b"k", b"wat", b"x"]),
        b"-ERR value is not an integer or out of range\r\n"
    );
    assert_eq!(command(&mut c, &[b"GET", b"k"]), b"$4\r\n\0\0\0x\r\n");
    assert_eq!(command(&mut c, &[b"DEL", b"k"]), b":1\r\n");
    assert_eq!(
        command(&mut c, &[b"SET", b"budget", &vec![b'a'; 250]]),
        b"+OK\r\n"
    );
    assert_eq!(
        command(&mut c, &[b"APPEND", b"budget", b"x"]),
        b"-OOM configured payload limit exceeded\r\n"
    );
    assert_eq!(command(&mut c, &[b"STRLEN", b"budget"]), b":250\r\n");
    assert_eq!(
        command(&mut c, &[b"SETRANGE", b"no-create", b"2", b""]),
        b":0\r\n"
    );
    assert_eq!(command(&mut c, &[b"GET", b"no-create"]), b"$-1\r\n");
    assert_eq!(
        command(&mut c, &[b"SETRANGE", b"huge", b"536870912", b"x"]),
        b"-ERR string exceeds maximum allowed size (proto-max-bulk-len)\r\n"
    );
    srv.shutdown().unwrap();
}

#[test]
fn large_get_is_data_budget_bounded_and_getrange_reply_is_slice_bounded() {
    let workers = match std::env::var("RILLCACHE_TEST_THREADS").as_deref() {
        Ok("1") => Some(1),
        Ok("4") => Some(4),
        _ => None,
    };
    let srv = Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        worker_threads: workers,
        max_data_bytes: 2 * 1024 * 1024,
        ..Config::default()
    })
    .unwrap()
    .spawn()
    .unwrap();
    let mut c = connect(&srv);
    let value = vec![0xa5; 900 * 1024];
    assert_eq!(command(&mut c, &[b"SET", b"large", &value]).len(), 5);
    let full = command(&mut c, &[b"GET", b"large"]);
    assert_eq!(full.len(), value.len() + b"$921600\r\n\r\n".len());
    assert!(full.starts_with(b"$921600\r\n"));
    assert!(full[9..9 + value.len()].iter().all(|&byte| byte == 0xa5));
    let slice = command(&mut c, &[b"GETRANGE", b"large", b"-8", b"-1"]);
    assert_eq!(slice, b"$8\r\n\xa5\xa5\xa5\xa5\xa5\xa5\xa5\xa5\r\n");
    srv.shutdown().unwrap();
}

#[test]
fn concurrent_appends_are_atomic_and_command_dispatch_falls_through() {
    let srv = server();
    let gate = Arc::new(Barrier::new(9));
    thread::scope(|scope| {
        for _ in 0..8 {
            let gate = gate.clone();
            let addr = srv.local_addr();
            scope.spawn(move || {
                let mut c = TcpStream::connect(addr).unwrap();
                c.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                gate.wait();
                let reply = command(&mut c, &[b"APPEND", b"shared", b"x"]);
                assert!(reply.starts_with(b":"));
            });
        }
        gate.wait();
    });
    let mut c = connect(&srv);
    assert_eq!(command(&mut c, &[b"STRLEN", b"shared"]), b":8\r\n");
    assert_eq!(command(&mut c, &[b"PING"]), b"+PONG\r\n");
    srv.shutdown().unwrap();
}
