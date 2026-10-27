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

fn bulk_text(stream: &mut TcpStream, args: &[&[u8]]) -> String {
    let mut request = Vec::new();
    encode_request(args, &mut request);
    stream.write_all(&request).unwrap();
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

fn avg_ttl(text: &str) -> u64 {
    text.split("avg_ttl=")
        .nth(1)
        .unwrap()
        .split(',')
        .next()
        .unwrap()
        .trim_end_matches('\r')
        .parse()
        .unwrap()
}

#[test]
fn avg_ttl_reports_the_mean_remaining_ttl_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = connect(&server);

        let empty = bulk_text(&mut stream, &[b"INFO", b"keyspace"]);
        assert_eq!(avg_ttl(&empty), 0, "{empty}");

        command(
            &mut stream,
            &[b"SET", b"a", b"1", b"PX", b"100000"],
            b"+OK\r\n",
        );
        command(
            &mut stream,
            &[b"SET", b"b", b"2", b"PX", b"300000"],
            b"+OK\r\n",
        );
        command(&mut stream, &[b"SET", b"c", b"3"], b"+OK\r\n");
        let text = bulk_text(&mut stream, &[b"INFO", b"keyspace"]);
        let average = avg_ttl(&text);
        assert!(
            (150_000..=250_000).contains(&average),
            "average {average} outside expected mean: {text}"
        );
        assert!(text.contains("keys=3"), "{text}");
        assert!(text.contains("expires=2"), "{text}");

        command(&mut stream, &[b"DEL", b"a", b"b"], b":2\r\n");
        let text = bulk_text(&mut stream, &[b"INFO", b"keyspace"]);
        assert_eq!(avg_ttl(&text), 0, "{text}");
        assert!(text.contains("keys=1"), "{text}");
        assert!(text.contains("expires=0"), "{text}");
        server.shutdown().unwrap();
    }
}
