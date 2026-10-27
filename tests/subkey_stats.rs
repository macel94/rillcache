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

fn field<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    text.lines().find_map(|line| {
        line.strip_prefix(name)?
            .strip_prefix(':')
            .map(|value| value.trim_end_matches('\r'))
    })
}

#[test]
fn subkey_expiry_stats_are_truthful_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = connect(&server);

        command(
            &mut stream,
            &[b"HSET", b"h", b"a", b"1", b"b", b"2"],
            b":2\r\n",
        );
        command(
            &mut stream,
            &[b"HPEXPIRE", b"h", b"1", b"FIELDS", b"2", b"a", b"b"],
            b"*2\r\n:1\r\n:1\r\n",
        );
        std::thread::sleep(Duration::from_millis(10));
        command(&mut stream, &[b"HGET", b"h", b"a"], b"$-1\r\n");

        let mut request = Vec::new();
        encode_request(&[b"INFO", b"stats"], &mut request);
        stream.write_all(&request).unwrap();
        let stats = bulk_text(&read_frame(&mut stream, 0));
        assert_eq!(field(&stats, "expired_subkeys"), Some("2"), "{stats}");

        let mut request = Vec::new();
        encode_request(&[b"INFO", b"keyspace"], &mut request);
        stream.write_all(&request).unwrap();
        let keyspace = bulk_text(&read_frame(&mut stream, 0));
        assert!(keyspace.contains("subexpiry=0"), "{keyspace}");

        command(&mut stream, &[b"HSET", b"g", b"f", b"v"], b":1\r\n");
        command(
            &mut stream,
            &[b"HPEXPIRE", b"g", b"100000", b"FIELDS", b"1", b"f"],
            b"*1\r\n:1\r\n",
        );
        let mut request = Vec::new();
        encode_request(&[b"INFO", b"keyspace"], &mut request);
        stream.write_all(&request).unwrap();
        let keyspace = bulk_text(&read_frame(&mut stream, 0));
        assert!(keyspace.contains("subexpiry=1"), "{keyspace}");

        command(&mut stream, &[b"CONFIG", b"RESETSTAT"], b"+OK\r\n");
        let mut request = Vec::new();
        encode_request(&[b"INFO", b"stats"], &mut request);
        stream.write_all(&request).unwrap();
        let stats = bulk_text(&read_frame(&mut stream, 0));
        assert_eq!(field(&stats, "expired_subkeys"), Some("0"), "{stats}");

        let mut request = Vec::new();
        encode_request(&[b"INFO", b"keyspace"], &mut request);
        stream.write_all(&request).unwrap();
        let keyspace = bulk_text(&read_frame(&mut stream, 0));
        assert!(keyspace.contains("subexpiry=1"), "{keyspace}");

        server.shutdown().unwrap();
    }
}
