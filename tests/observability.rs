#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

fn start(config: Config) -> ServerHandle {
    let workers = match std::env::var("RILLCACHE_TEST_THREADS").as_deref() {
        Ok("0") | Err(_) => None,
        Ok("1") => Some(1),
        Ok("4") => Some(4),
        Ok(value) => panic!("invalid test worker mode {value}"),
    };
    Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        worker_threads: workers,
        ..config
    })
    .unwrap()
    .spawn()
    .unwrap()
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

fn send(stream: &mut TcpStream, arguments: &[&[u8]]) {
    let mut bytes = Vec::new();
    encode_request(arguments, &mut bytes);
    stream.write_all(&bytes).unwrap();
}

fn expect(stream: &mut TcpStream, expected: &[u8]) {
    let mut actual = vec![0; expected.len()];
    stream.read_exact(&mut actual).unwrap();
    assert_eq!(actual, expected);
}

fn info_stats(stream: &mut TcpStream) -> String {
    send(stream, &[b"INFO", b"stats"]);
    let mut header = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        stream.read_exact(&mut byte).unwrap();
        header.push(byte[0]);
        if header.ends_with(b"\r\n") {
            break;
        }
    }
    assert_eq!(header.first(), Some(&b'$'));
    let size: usize = std::str::from_utf8(&header[1..header.len() - 2])
        .unwrap()
        .parse()
        .unwrap();
    let mut body = vec![0; size + 2];
    stream.read_exact(&mut body).unwrap();
    assert_eq!(&body[size..], b"\r\n");
    body.truncate(size);
    String::from_utf8(body).unwrap()
}

fn field<'a>(text: &'a str, name: &str) -> &'a str {
    text.lines()
        .find_map(|line| line.strip_prefix(&format!("{name}:")))
        .unwrap_or_else(|| panic!("missing {name} in {text}"))
}

#[test]
fn info_stats_counts_commands_connections_and_resets() {
    let server = start(Config::default());
    let mut first = connect(&server);
    let mut second = connect(&server);
    send(&mut first, &[b"PING"]);
    expect(&mut first, b"+PONG\r\n");
    send(&mut first, &[b"SET", b"k", b"v"]);
    expect(&mut first, b"+OK\r\n");
    send(&mut second, &[b"GET", b"k"]);
    expect(&mut second, b"$1\r\nv\r\n");

    let stats = info_stats(&mut first);
    assert!(
        field(&stats, "total_connections_received")
            .parse::<u64>()
            .unwrap()
            >= 2
    );
    assert!(
        field(&stats, "total_commands_processed")
            .parse::<u64>()
            .unwrap()
            >= 4
    );
    assert_eq!(field(&stats, "rejected_connections"), "0");
    assert_eq!(field(&stats, "evicted_keys"), "0");
    assert_eq!(field(&stats, "pubsub_channels"), "0");
    assert_eq!(field(&stats, "pubsub_patterns"), "0");

    send(&mut first, &[b"CONFIG", b"RESETSTAT"]);
    expect(&mut first, b"+OK\r\n");
    let stats = info_stats(&mut first);
    assert_eq!(field(&stats, "total_connections_received"), "0");
    // INFO itself is counted before producing the snapshot.
    assert_eq!(field(&stats, "total_commands_processed"), "1");
    assert_eq!(field(&stats, "expired_keys"), "0");

    send(&mut first, &[b"SET", b"expire", b"v", b"PX", b"20"]);
    expect(&mut first, b"+OK\r\n");
    std::thread::sleep(Duration::from_millis(60));
    send(&mut first, &[b"GET", b"expire"]);
    expect(&mut first, b"$-1\r\n");
    let stats = info_stats(&mut first);
    assert!(field(&stats, "expired_keys").parse::<u64>().unwrap() >= 1);
    server.shutdown().unwrap();
}

#[test]
fn rejected_connections_are_visible_in_stats() {
    let server = start(Config {
        max_clients: 1,
        ..Config::default()
    });
    let mut first = connect(&server);
    send(&mut first, &[b"PING"]);
    expect(&mut first, b"+PONG\r\n");
    let mut rejected = connect(&server);
    let mut reply = Vec::new();
    rejected.read_to_end(&mut reply).unwrap();
    assert_eq!(reply, b"-ERR max number of clients reached\r\n");
    let stats = info_stats(&mut first);
    assert_eq!(field(&stats, "rejected_connections"), "1");
    server.shutdown().unwrap();
}
