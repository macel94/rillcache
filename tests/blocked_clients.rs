#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

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
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream.set_nodelay(true).unwrap();
    stream
}

fn read_line(stream: &mut TcpStream, output: &mut Vec<u8>) -> String {
    let mut line = Vec::new();
    loop {
        let mut byte = [0];
        stream.read_exact(&mut byte).unwrap();
        if byte[0] == b'\n' {
            assert_eq!(line.pop(), Some(b'\r'));
            output.extend_from_slice(&line);
            output.extend_from_slice(b"\r\n");
            return String::from_utf8(line).unwrap();
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
            let length = read_line(stream, &mut output).parse::<i64>().unwrap();
            if length >= 0 {
                let mut body = vec![0; length as usize + 2];
                stream.read_exact(&mut body).unwrap();
                assert!(body.ends_with(b"\r\n"));
                output.extend_from_slice(&body);
            }
        }
        b'*' => {
            let count = read_line(stream, &mut output).parse::<i64>().unwrap();
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

fn send(stream: &mut TcpStream, args: &[&[u8]]) {
    let mut request = Vec::new();
    encode_request(args, &mut request);
    stream.write_all(&request).unwrap();
}

fn command(stream: &mut TcpStream, args: &[&[u8]], expected: &[u8]) {
    send(stream, args);
    assert_eq!(read_frame(stream, 0), expected, "{args:?}");
}

fn bulk_text(stream: &mut TcpStream, args: &[&[u8]]) -> String {
    send(stream, args);
    let reply = read_frame(stream, 0);
    assert_eq!(reply.first(), Some(&b'$'), "{reply:?}");
    let header_end = reply
        .windows(2)
        .position(|window| window == b"\r\n")
        .unwrap();
    let length = std::str::from_utf8(&reply[1..header_end])
        .unwrap()
        .parse::<usize>()
        .unwrap();
    std::str::from_utf8(&reply[header_end + 2..header_end + 2 + length])
        .unwrap()
        .to_string()
}

fn blocked_count(stream: &mut TcpStream) -> usize {
    let info = bulk_text(stream, &[b"INFO", b"clients"]);
    info.lines()
        .find_map(|line| line.strip_prefix("blocked_clients:"))
        .unwrap()
        .trim_end_matches('\r')
        .parse()
        .unwrap()
}

#[test]
fn blocked_clients_are_visible_while_waiting_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut admin = connect(&server);
        let mut worker = connect(&server);

        send(&mut worker, &[b"BLPOP", b"queue", b"5"]);
        let deadline = Instant::now() + Duration::from_secs(2);
        while blocked_count(&mut admin) != 1 {
            assert!(Instant::now() < deadline, "worker never showed as blocked");
            std::thread::sleep(Duration::from_millis(10));
        }
        let list = bulk_text(&mut admin, &[b"CLIENT", b"LIST"]);
        assert!(list.contains("flags=bN"), "{list}");

        command(&mut admin, &[b"LPUSH", b"queue", b"value"], b":1\r\n");
        assert_eq!(
            read_frame(&mut worker, 0),
            b"*2\r\n$5\r\nqueue\r\n$5\r\nvalue\r\n"
        );

        let deadline = Instant::now() + Duration::from_secs(2);
        while blocked_count(&mut admin) != 0 {
            assert!(Instant::now() < deadline, "worker stayed blocked");
            std::thread::sleep(Duration::from_millis(10));
        }
        let all = bulk_text(&mut admin, &[b"INFO", b"all"]);
        assert!(all.contains("blocked_clients:0\r\n"), "{all}");

        server.shutdown().unwrap();
    }
}
