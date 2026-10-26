#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use rillcache::engine::EvictionPolicy;
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
        max_data_bytes: 11,
        maxmemory_policy: EvictionPolicy::AllKeysRandom,
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

fn send(stream: &mut TcpStream, arguments: &[&[u8]], reply: &[u8]) {
    let mut bytes = Vec::new();
    encode_request(arguments, &mut bytes);
    stream.write_all(&bytes).unwrap();
    let mut actual = vec![0; reply.len()];
    stream.read_exact(&mut actual).unwrap();
    assert_eq!(actual, reply, "command {arguments:?}");
}

const OOM: &[u8] = b"-OOM configured payload limit exceeded\r\n";

fn info(stream: &mut TcpStream, section: &[u8]) -> String {
    let mut bytes = Vec::new();
    encode_request(&[b"INFO", section], &mut bytes);
    stream.write_all(&bytes).unwrap();
    String::from_utf8(bulk(stream)).unwrap()
}

fn bulk(stream: &mut TcpStream) -> Vec<u8> {
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
    body.truncate(size);
    body
}

#[test]
fn allkeys_random_eviction_keeps_the_budget_and_reports_stats() {
    let server = start();
    let mut stream = connect(&server);
    send(&mut stream, &[b"SET", b"a", b"1234"], b"+OK\r\n");
    send(&mut stream, &[b"SET", b"b", b"1234"], b"+OK\r\n");
    send(&mut stream, &[b"SET", b"c", b"v"], b"+OK\r\n");
    send(&mut stream, &[b"DBSIZE"], b":2\r\n");
    send(&mut stream, &[b"GET", b"c"], b"$1\r\nv\r\n");
    let text = info(&mut stream, b"stats");
    assert!(text.contains("evicted_keys:1"), "{text}");
    let memory = info(&mut stream, b"memory");
    assert!(
        memory.contains("maxmemory_policy:allkeys-random"),
        "{memory}"
    );
    send(
        &mut stream,
        &[b"CONFIG", b"GET", b"maxmemory-policy"],
        b"*2\r\n$16\r\nmaxmemory-policy\r\n$14\r\nallkeys-random\r\n",
    );
    send(
        &mut stream,
        &[b"CONFIG", b"SET", b"maxmemory-policy", b"noeviction"],
        b"+OK\r\n",
    );
    send(&mut stream, &[b"SET", b"d", b"1234"], OOM);
    send(&mut stream, &[b"CONFIG", b"RESETSTAT"], b"+OK\r\n");
    send(&mut stream, &[b"PING"], b"+PONG\r\n");
    server.shutdown().unwrap();
}

#[test]
fn volatile_policy_never_evicts_persistent_keys() {
    let server = Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        max_data_bytes: 11,
        maxmemory_policy: EvictionPolicy::VolatileRandom,
        ..Config::default()
    })
    .unwrap()
    .spawn()
    .unwrap();
    let mut stream = connect(&server);
    send(&mut stream, &[b"SET", b"a", b"1234"], b"+OK\r\n");
    send(&mut stream, &[b"SET", b"b", b"1234"], b"+OK\r\n");
    send(&mut stream, &[b"SET", b"c", b"v"], OOM);
    send(&mut stream, &[b"PING"], b"+PONG\r\n");
    server.shutdown().unwrap();
}
