#![forbid(unsafe_code)]

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

fn send(stream: &mut TcpStream, args: &[&[u8]]) {
    let mut request = Vec::new();
    encode_request(args, &mut request);
    stream.write_all(&request).unwrap();
}

fn expect(stream: &mut TcpStream, expected: &[u8]) {
    let mut actual = vec![0; expected.len()];
    stream.read_exact(&mut actual).unwrap();
    assert_eq!(actual, expected);
}

fn line(stream: &mut TcpStream) -> Vec<u8> {
    let mut bytes = Vec::new();
    loop {
        let mut byte = [0];
        stream.read_exact(&mut byte).unwrap();
        bytes.push(byte[0]);
        assert!(bytes.len() <= 128, "RESP line exceeded test bound");
        if bytes.ends_with(b"\r\n") {
            return bytes;
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ReplyValue {
    Bulk(Vec<u8>),
    Integer(i64),
}

fn reply_value(stream: &mut TcpStream) -> ReplyValue {
    let header = line(stream);
    if header.starts_with(b":") {
        return ReplyValue::Integer(
            std::str::from_utf8(&header[1..header.len() - 2])
                .unwrap()
                .parse()
                .unwrap(),
        );
    }
    assert!(
        header.starts_with(b"$"),
        "expected bulk string, got {header:?}"
    );
    let size: usize = std::str::from_utf8(&header[1..header.len() - 2])
        .unwrap()
        .parse()
        .unwrap();
    assert!(size <= 4096, "test bulk reply exceeded bound");
    let mut value = vec![0; size + 2];
    stream.read_exact(&mut value).unwrap();
    assert_eq!(&value[size..], b"\r\n");
    value.truncate(size);
    ReplyValue::Bulk(value)
}

fn bulk(stream: &mut TcpStream) -> Vec<u8> {
    match reply_value(stream) {
        ReplyValue::Bulk(value) => value,
        other => panic!("expected bulk string, got {other:?}"),
    }
}

fn hello_map(stream: &mut TcpStream, prefix: &[u8]) -> Vec<(Vec<u8>, ReplyValue)> {
    let header = line(stream);
    assert!(
        header.starts_with(prefix),
        "unexpected HELLO response {header:?}"
    );
    let count: usize = std::str::from_utf8(&header[1..header.len() - 2])
        .unwrap()
        .parse()
        .unwrap();
    let pairs = if header[0] == b'%' { count } else { count / 2 };
    let mut fields = Vec::with_capacity(pairs);
    for _ in 0..pairs {
        fields.push((bulk(stream), reply_value(stream)));
    }
    fields
}

#[test]
fn hello_protocol_switches_pipeline_replies_and_binary_payloads() {
    let server = start();
    let mut stream = connect(&server);
    send(&mut stream, &[b"HELLO", b"3"]);
    let fields = hello_map(&mut stream, b"%5");
    assert_eq!(
        fields[0],
        (b"server".to_vec(), ReplyValue::Bulk(b"rillcache".to_vec()))
    );
    assert_eq!(fields[1].0, b"version");
    assert!(matches!(fields[1].1, ReplyValue::Bulk(_)));
    assert_eq!(fields[2], (b"proto".to_vec(), ReplyValue::Integer(3)));
    assert!(matches!(&fields[3], (key, ReplyValue::Integer(id)) if key == b"id" && *id > 0));
    send(&mut stream, &[b"CLIENT", b"GETNAME"]);
    expect(&mut stream, b"_\r\n");

    let mut switch_pipeline = Vec::new();
    encode_request(&[b"HELLO", b"2"], &mut switch_pipeline);
    encode_request(&[b"GET", b"missing"], &mut switch_pipeline);
    stream.write_all(&switch_pipeline).unwrap();
    let resp2_hello = hello_map(&mut stream, b"*10");
    assert_eq!(resp2_hello[2], (b"proto".to_vec(), ReplyValue::Integer(2)));
    assert!(matches!(&resp2_hello[3], (key, ReplyValue::Integer(id)) if key == b"id" && *id > 0));
    expect(&mut stream, b"$-1\r\n");
    send(&mut stream, &[b"HELLO", b"3"]);
    hello_map(&mut stream, b"%5");

    let payload = b"contains $-1\r\n bytes\0\xff";
    let mut pipeline = Vec::new();
    encode_request(&[b"GET", b"missing"], &mut pipeline);
    encode_request(&[b"SET", b"binary", payload], &mut pipeline);
    encode_request(&[b"GET", b"binary"], &mut pipeline);
    stream.write_all(&pipeline).unwrap();
    expect(&mut stream, b"_\r\n+OK\r\n");
    let mut expected = Vec::new();
    rillcache::protocol::encode_bulk(Some(payload), &mut expected);
    expect(&mut stream, &expected);

    server.shutdown().unwrap();
}

#[test]
fn connection_metadata_is_isolated_validated_and_select_is_honest() {
    let server = start();
    let mut first = connect(&server);
    let mut second = connect(&server);
    send(&mut first, &[b"CLIENT", b"ID"]);
    let first_id = line(&mut first);
    send(&mut second, &[b"CLIENT", b"ID"]);
    assert_ne!(first_id, line(&mut second));

    send(&mut first, &[b"CLIENT", b"SETNAME", b"worker-a"]);
    expect(&mut first, b"+OK\r\n");
    send(
        &mut first,
        &[b"CLIENT", b"SETINFO", b"LIB-NAME", b"test-lib"],
    );
    expect(&mut first, b"+OK\r\n");
    send(&mut first, &[b"CLIENT", b"SETINFO", b"LIB-VER", b"1.0"]);
    expect(&mut first, b"+OK\r\n");
    send(&mut first, &[b"CLIENT", b"SETNAME", b"not printable\n"]);
    expect(&mut first, b"-ERR invalid client name\r\n");
    send(&mut first, &[b"CLIENT", b"GETNAME"]);
    assert_eq!(bulk(&mut first), b"worker-a");
    send(&mut first, &[b"CLIENT", b"INFO"]);
    let info = bulk(&mut first);
    assert!(info.windows(13).any(|w| w == b"name=worker-a"));
    assert!(info.windows(17).any(|w| w == b"lib-name=test-lib"));

    send(&mut second, &[b"CLIENT", b"GETNAME"]);
    expect(&mut second, b"$-1\r\n");
    send(&mut first, &[b"SELECT", b"0"]);
    expect(&mut first, b"+OK\r\n");
    send(&mut first, &[b"SELECT", b"1"]);
    expect(&mut first, b"-ERR DB index is out of range\r\n");
    send(&mut first, &[b"SELECT", b"nonsense"]);
    expect(&mut first, b"-ERR invalid DB index\r\n");
    server.shutdown().unwrap();
}

#[test]
fn invalid_handshakes_auth_and_quit_are_ordered_and_atomic() {
    let server = start();
    let mut stream = connect(&server);
    send(&mut stream, &[b"HELLO", b"3", b"SETNAME", b"bad name"]);
    expect(&mut stream, b"-ERR syntax error\r\n");
    send(&mut stream, &[b"GET", b"none"]);
    expect(&mut stream, b"$-1\r\n"); // Failed HELLO did not switch protocol.
    send(
        &mut stream,
        &[b"HELLO", b"3", b"AUTH", b"default", b"secret"],
    );
    expect(&mut stream, b"-ERR AUTH is not supported\r\n");
    send(&mut stream, &[b"AUTH", b"secret"]);
    expect(&mut stream, b"-ERR AUTH is not supported\r\n");
    send(&mut stream, &[b"HELLO", b"99"]);
    expect(&mut stream, b"-NOPROTO unsupported protocol version\r\n");
    send(&mut stream, &[b"HELLO", b"3"]);
    hello_map(&mut stream, b"%5");

    let mut quit = Vec::new();
    encode_request(&[b"QUIT"], &mut quit);
    encode_request(&[b"SET", b"after-quit", b"no"], &mut quit);
    stream.write_all(&quit).unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    assert_eq!(response, b"+OK\r\n");
    let mut other = connect(&server);
    send(&mut other, &[b"GET", b"after-quit"]);
    expect(&mut other, b"$-1\r\n");
    server.shutdown().unwrap();
}

#[test]
fn shutdown_joins_clients_in_each_configured_mode() {
    let server = start();
    let mut idle = connect(&server);
    send(&mut idle, &[b"HELLO", b"3"]);
    hello_map(&mut idle, b"%5");
    server.shutdown().unwrap();
    assert_eq!(idle.read(&mut [0]).unwrap(), 0);
}
