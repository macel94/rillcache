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
        .set_read_timeout(Some(Duration::from_secs(3)))
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

fn info(stream: &mut TcpStream, section: &[u8]) -> Vec<u8> {
    send(stream, &[b"INFO", section]);
    read_frame(stream, 0)
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[test]
fn info_errorstats_counts_codes_and_resets_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = connect(&server);
        let empty = info(&mut stream, b"errorstats");
        assert!(contains(&empty, b"# Errorstats\r\n"));
        assert!(!contains(&empty, b"errorstat_"));

        send(&mut stream, &[b"BOGUS"]);
        assert!(read_frame(&mut stream, 0).starts_with(b"-ERR"));
        command(&mut stream, &[b"SET", b"k", b"v"], b"+OK\r\n");
        command(
            &mut stream,
            &[b"LPUSH", b"k", b"x"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );
        send(&mut stream, &[b"GET"]);
        assert!(read_frame(&mut stream, 0).starts_with(b"-ERR"));

        let stats = info(&mut stream, b"errorstats");
        assert!(contains(&stats, b"errorstat_ERR:2\r\n"), "{stats:?}");
        assert!(contains(&stats, b"errorstat_WRONGTYPE:1\r\n"), "{stats:?}");
        let all = info(&mut stream, b"all");
        assert!(contains(&all, b"# Errorstats\r\n"), "{all:?}");
        assert!(contains(&all, b"errorstat_ERR:2\r\n"), "{all:?}");

        command(&mut stream, &[b"CONFIG", b"RESETSTAT"], b"+OK\r\n");
        let cleared = info(&mut stream, b"errorstats");
        assert!(contains(&cleared, b"# Errorstats\r\n"));
        assert!(!contains(&cleared, b"errorstat_"), "{cleared:?}");

        server.shutdown().unwrap();
    }
}

#[test]
fn plain_errors_are_counted_with_optional_collectors_disabled_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = connect(&server);
        command(
            &mut stream,
            &[b"CONFIG", b"SET", b"slowlog-log-slower-than", b"-1"],
            b"+OK\r\n",
        );
        command(
            &mut stream,
            &[b"CONFIG", b"SET", b"latency-monitor-threshold", b"0"],
            b"+OK\r\n",
        );
        command(&mut stream, &[b"LPUSH", b"list", b"x"], b":1\r\n");
        for (args, prefix) in [
            (
                &[b"PING".as_slice(), b"one", b"two"][..],
                b"-ERR".as_slice(),
            ),
            (&[b"GET".as_slice(), b"list"][..], b"-WRONGTYPE".as_slice()),
            (&[b"SET".as_slice(), b"key"][..], b"-ERR".as_slice()),
            (&[b"ECHO".as_slice()][..], b"-ERR".as_slice()),
        ] {
            send(&mut stream, args);
            assert!(read_frame(&mut stream, 0).starts_with(prefix));
        }
        let stats = info(&mut stream, b"errorstats");
        assert!(contains(&stats, b"errorstat_ERR:3\r\n"), "{stats:?}");
        assert!(contains(&stats, b"errorstat_WRONGTYPE:1\r\n"), "{stats:?}");
        command(&mut stream, &[b"CONFIG", b"RESETSTAT"], b"+OK\r\n");
        assert!(!contains(&info(&mut stream, b"errorstats"), b"errorstat_"));
        send(&mut stream, &[b"PING", b"one", b"two"]);
        assert!(read_frame(&mut stream, 0).starts_with(b"-ERR"));
        assert!(contains(
            &info(&mut stream, b"errorstats"),
            b"errorstat_ERR:1\r\n"
        ));
        server.shutdown().unwrap();
    }
}
