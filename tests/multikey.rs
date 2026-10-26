#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

fn start(workers: Option<usize>, max_data_bytes: usize) -> ServerHandle {
    Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        worker_threads: workers,
        max_data_bytes,
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

fn send(stream: &mut TcpStream, args: &[&[u8]], expected: &[u8]) {
    let mut request = Vec::new();
    encode_request(args, &mut request);
    stream.write_all(&request).unwrap();
    let mut actual = vec![0; expected.len()];
    stream.read_exact(&mut actual).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn multikey_commands_preserve_binary_duplicates_order_and_atomicity_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers, 16);
        let mut stream = connect(&server);
        send(
            &mut stream,
            &[b"MSET", b"k\0\xff", b"", b"b", b"\xff\0"],
            b"+OK\r\n",
        );
        send(
            &mut stream,
            &[b"MGET", b"b", b"missing", b"k\0\xff", b"b"],
            b"*4\r\n$2\r\n\xff\0\r\n$-1\r\n$0\r\n\r\n$2\r\n\xff\0\r\n",
        );
        send(
            &mut stream,
            &[b"MSETNX", b"fresh", b"first", b"fresh", b"last"],
            b":1\r\n",
        );
        send(
            &mut stream,
            &[b"MGET", b"fresh", b"fresh"],
            b"*2\r\n$4\r\nlast\r\n$4\r\nlast\r\n",
        );
        send(
            &mut stream,
            &[b"MSETNX", b"fresh", b"no", b"untouched", b"x"],
            b":0\r\n",
        );
        send(&mut stream, &[b"GET", b"untouched"], b"$-1\r\n");
        send(
            &mut stream,
            &[b"MSET", b"k\0\xff", b"123456789012345"],
            b"-OOM configured payload limit exceeded\r\n",
        );
        send(&mut stream, &[b"MGET", b"k\0\xff"], b"*1\r\n$0\r\n\r\n");
        send(
            &mut stream,
            &[b"MGET"],
            b"-ERR wrong number of arguments for 'mget' command\r\n",
        );
        send(
            &mut stream,
            &[b"MSET", b"odd"],
            b"-ERR wrong number of arguments for 'mset' command\r\n",
        );
        send(
            &mut stream,
            &[b"MSETNX", b"odd", b"v", b"dangling"],
            b"-ERR wrong number of arguments for 'msetnx' command\r\n",
        );
        server.shutdown().unwrap();
    }
}

#[test]
fn mget_reply_amplification_is_rejected_before_partial_output() {
    let server = start(Some(4), 1024 * 1024 + 8);
    let mut stream = connect(&server);
    let large_value = vec![b'x'; 1024 * 1024];
    send(&mut stream, &[b"SET", b"large", &large_value], b"+OK\r\n");
    send(
        &mut stream,
        &[b"MGET", b"large", b"large"],
        b"-ERR MGET reply limit exceeded\r\n",
    );
    let mut expected = Vec::new();
    rillcache::protocol::encode_bulk(Some(&large_value), &mut expected);
    send(&mut stream, &[b"GET", b"large"], &expected);
    server.shutdown().unwrap();
}

#[test]
fn concurrent_msetnx_over_wire_never_exposes_a_partial_pair() {
    let server = start(Some(4), 1024);
    let ready = std::sync::Barrier::new(8);
    std::thread::scope(|scope| {
        for id in 0..8 {
            let ready = &ready;
            let address = server.local_addr();
            scope.spawn(move || {
                let mut stream = TcpStream::connect(address).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let value = [id as u8];
                let mut command = Vec::new();
                encode_request(
                    &[b"MSETNX", b"left", &value, b"right", &value],
                    &mut command,
                );
                ready.wait();
                stream.write_all(&command).unwrap();
                let mut reply = [0; 4];
                stream.read_exact(&mut reply).unwrap();
                assert!(reply == *b":1\r\n" || reply == *b":0\r\n");
            });
        }
    });
    let mut stream = connect(&server);
    let mut request = Vec::new();
    encode_request(&[b"MGET", b"left", b"right"], &mut request);
    stream.write_all(&request).unwrap();
    let mut reply = [0; 18];
    stream.read_exact(&mut reply).unwrap();
    assert_eq!(&reply[..8], b"*2\r\n$1\r\n");
    assert_eq!(&reply[9..15], b"\r\n$1\r\n");
    assert_eq!(&reply[16..], b"\r\n");
    assert_eq!(
        reply[8], reply[15],
        "both keys must come from the same winner"
    );
    server.shutdown().unwrap();
}
