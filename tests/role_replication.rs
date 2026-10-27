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

fn command(stream: &mut TcpStream, args: &[&[u8]], expected: &[u8]) {
    let mut request = Vec::new();
    encode_request(args, &mut request);
    stream.write_all(&request).unwrap();
    assert_eq!(read_frame(stream, 0), expected, "{args:?}");
}

fn bulk_text(reply: &[u8]) -> String {
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

#[test]
fn role_and_replication_info_report_truthful_standalone_state_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = connect(&server);

        command(
            &mut stream,
            &[b"ROLE"],
            b"*3\r\n$6\r\nmaster\r\n:0\r\n*0\r\n",
        );
        command(
            &mut stream,
            &[b"ROLE", b"extra"],
            b"-ERR wrong number of arguments for 'role' command\r\n",
        );

        let mut request = Vec::new();
        encode_request(&[b"INFO", b"replication"], &mut request);
        stream.write_all(&request).unwrap();
        let replication = bulk_text(&read_frame(&mut stream, 0));
        assert!(
            replication.starts_with("# Replication\r\n"),
            "{replication}"
        );
        assert!(replication.contains("role:master\r\n"), "{replication}");
        assert!(
            replication.contains("connected_slaves:0\r\n"),
            "{replication}"
        );
        assert!(
            replication.contains("master_failover_state:no-failover\r\n"),
            "{replication}"
        );
        assert!(
            replication.contains("master_repl_offset:0\r\n"),
            "{replication}"
        );
        assert!(!replication.contains("replid"), "{replication}");
        assert!(!replication.contains("slave0"), "{replication}");

        let mut request = Vec::new();
        encode_request(&[b"INFO", b"all"], &mut request);
        stream.write_all(&request).unwrap();
        let all = bulk_text(&read_frame(&mut stream, 0));
        assert!(all.contains("# Replication\r\n"), "{all}");
        assert!(all.contains("# Stats\r\n"), "{all}");

        command(
            &mut stream,
            &[b"INFO", b"bogus"],
            b"-ERR unsupported INFO section\r\n",
        );

        server.shutdown().unwrap();
    }
}
