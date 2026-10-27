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

fn wait_until_blocked(stream: &mut TcpStream, expected: usize) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if blocked_count(stream) == expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "blocked count did not reach {expected}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn wait_until_unregistered(stream: &mut TcpStream, id: u64) {
    let id = id.to_string();
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let clients = bulk_text(stream, &[b"CLIENT", b"LIST", b"ID", id.as_bytes()]);
        if clients.is_empty() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "disconnected client remained registered"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn run_all_cases(workers: Option<usize>) {
    let server = start(workers);
    let mut admin = connect(&server);

    // A blocked waiter is visible in both INFO and CLIENT LIST, then disappears
    // from the count as soon as its blocking command is satisfied.
    let mut waking = connect(&server);
    send(&mut waking, &[b"BLPOP", b"wake-queue", b"5"]);
    wait_until_blocked(&mut admin, 1);
    let clients = bulk_text(&mut admin, &[b"CLIENT", b"LIST"]);
    assert!(clients.contains("flags=bN"), "{clients}");
    let all = bulk_text(&mut admin, &[b"INFO", b"all"]);
    assert!(all.contains("blocked_clients:1\r\n"), "{all}");
    command(&mut admin, &[b"LPUSH", b"wake-queue", b"value"], b":1\r\n");
    assert_eq!(
        read_frame(&mut waking, 0),
        b"*2\r\n$10\r\nwake-queue\r\n$5\r\nvalue\r\n"
    );
    wait_until_blocked(&mut admin, 0);

    // Timeout completes the command and clears its client flag/count.
    let mut timing_out = connect(&server);
    send(&mut timing_out, &[b"BLPOP", b"timeout-queue", b"0.05"]);
    wait_until_blocked(&mut admin, 1);
    assert_eq!(read_frame(&mut timing_out, 0), b"*-1\r\n");
    wait_until_blocked(&mut admin, 0);

    // Disconnecting while blocked removes the registry record, so it cannot
    // contribute to INFO even though no command reply can be sent.
    let mut disconnecting = connect(&server);
    send(&mut disconnecting, &[b"BLPOP", b"disconnect-queue", b"5"]);
    wait_until_blocked(&mut admin, 1);
    let id_reply = bulk_text(&mut admin, &[b"CLIENT", b"LIST"]);
    let id = id_reply
        .lines()
        .find(|line| line.contains("flags=bN"))
        .and_then(|line| {
            line.split_whitespace()
                .find_map(|field| field.strip_prefix("id="))
        })
        .unwrap()
        .parse::<u64>()
        .unwrap();
    drop(disconnecting);
    wait_until_unregistered(&mut admin, id);
    wait_until_blocked(&mut admin, 0);

    server.shutdown().unwrap();
}

#[test]
fn blocked_clients_are_visible_and_cleared_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        run_all_cases(workers);
    }
}
