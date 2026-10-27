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

fn command(stream: &mut TcpStream, args: &[&[u8]], expected: &[u8]) {
    let mut request = Vec::new();
    encode_request(args, &mut request);
    stream.write_all(&request).unwrap();
    assert_eq!(read_frame(stream, 0), expected, "{args:?}");
}

fn reply(stream: &mut TcpStream, args: &[&[u8]]) -> Vec<u8> {
    let mut request = Vec::new();
    encode_request(args, &mut request);
    stream.write_all(&request).unwrap();
    read_frame(stream, 0)
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[test]
fn stream_trimming_works_over_tcp_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = connect(&server);
        for id in 1..=6 {
            let id = format!("{id}-0");
            assert!(
                reply(&mut stream, &[b"XADD", b"s", id.as_bytes(), b"f", b"v"]).starts_with(b"$")
            );
        }
        command(&mut stream, &[b"XTRIM", b"s", b"MAXLEN", b"3"], b":3\r\n");
        command(&mut stream, &[b"XLEN", b"s"], b":3\r\n");
        let range = reply(&mut stream, &[b"XRANGE", b"s", b"-", b"+"]);
        assert!(range.starts_with(b"*3\r\n"));
        assert!(!contains(&range, b"1-0"));
        assert!(contains(&range, b"4-0") && contains(&range, b"6-0"));
        command(&mut stream, &[b"XTRIM", b"s", b"MINID", b"6-0"], b":2\r\n");
        command(&mut stream, &[b"XLEN", b"s"], b":1\r\n");
        for id in 7..=10 {
            let id = format!("{id}-0");
            assert!(
                reply(&mut stream, &[b"XADD", b"s", id.as_bytes(), b"f", b"v"]).starts_with(b"$")
            );
        }
        command(
            &mut stream,
            &[b"XTRIM", b"s", b"MAXLEN", b"=", b"3"],
            b":2\r\n",
        );
        command(&mut stream, &[b"XLEN", b"s"], b":3\r\n");
        for id in 11..=12 {
            let id = format!("{id}-0");
            assert!(
                reply(&mut stream, &[b"XADD", b"s", id.as_bytes(), b"f", b"v"]).starts_with(b"$")
            );
        }
        command(
            &mut stream,
            &[b"XTRIM", b"s", b"MAXLEN", b"~", b"0", b"LIMIT", b"2"],
            b":2\r\n",
        );
        command(&mut stream, &[b"XLEN", b"s"], b":3\r\n");
        command(
            &mut stream,
            &[b"XTRIM", b"s", b"MAXLEN", b"~", b"0"],
            b":3\r\n",
        );
        command(&mut stream, &[b"XLEN", b"s"], b":0\r\n");
        command(
            &mut stream,
            &[b"XADD", b"s", b"13-0", b"f", b"v"],
            b"$4\r\n13-0\r\n",
        );
        command(
            &mut stream,
            &[b"XTRIM", b"s", b"MINID", b"~", b"99-0"],
            b":1\r\n",
        );
        command(&mut stream, &[b"XLEN", b"s"], b":0\r\n");

        command(
            &mut stream,
            &[b"XTRIM", b"missing", b"MAXLEN", b"0"],
            b":0\r\n",
        );
        command(
            &mut stream,
            &[b"XTRIM", b"s", b"MAXLEN", b"-1"],
            b"-ERR The MAXLEN argument must be >= 0.\r\n",
        );
        command(
            &mut stream,
            &[b"XTRIM", b"s", b"MAXLEN", b"1", b"MINID", b"2-0"],
            b"-ERR syntax error, MAXLEN and MINID options at the same time are not compatible\r\n",
        );
        command(
            &mut stream,
            &[b"XTRIM", b"s", b"MINID", b"bad"],
            b"-ERR Invalid stream ID specified as stream command argument\r\n",
        );
        command(
            &mut stream,
            &[b"XTRIM", b"s", b"LIMIT", b"1"],
            b"-ERR syntax error, LIMIT cannot be used without specifying a trimming strategy\r\n",
        );
        command(
            &mut stream,
            &[b"XTRIM", b"s", b"MAXLEN", b"1", b"LIMIT", b"1"],
            b"-ERR syntax error, LIMIT cannot be used without the special ~ option\r\n",
        );
        command(
            &mut stream,
            &[b"XTRIM", b"s", b"MAXLEN", b"1", b"LIMIT", b"-1"],
            b"-ERR The LIMIT argument must be >= 0.\r\n",
        );
        command(
            &mut stream,
            &[b"XTRIM", b"s", b"BOGUS", b"1"],
            b"-ERR syntax error\r\n",
        );
        command(
            &mut stream,
            &[b"XTRIM", b"s", b"BOGUS"],
            b"-ERR wrong number of arguments for 'xtrim' command\r\n",
        );

        command(
            &mut stream,
            &[b"XADD", b"ttl", b"20-0", b"f", b"v"],
            b"$4\r\n20-0\r\n",
        );
        command(&mut stream, &[b"PEXPIRE", b"ttl", b"100000"], b":1\r\n");
        command(&mut stream, &[b"XTRIM", b"ttl", b"MAXLEN", b"0"], b":1\r\n");
        let ttl = reply(&mut stream, &[b"PTTL", b"ttl"]);
        let ttl = std::str::from_utf8(&ttl[1..ttl.len() - 2])
            .unwrap()
            .parse::<i64>()
            .unwrap();
        assert!(ttl > 0);

        command(
            &mut stream,
            &[b"XADD", b"pending", b"30-0", b"f", b"v"],
            b"$4\r\n30-0\r\n",
        );
        command(
            &mut stream,
            &[b"XGROUP", b"CREATE", b"pending", b"g", b"0"],
            b"+OK\r\n",
        );
        assert!(
            reply(
                &mut stream,
                &[
                    b"XREADGROUP",
                    b"GROUP",
                    b"g",
                    b"c",
                    b"STREAMS",
                    b"pending",
                    b">"
                ]
            )
            .starts_with(b"*1\r\n")
        );
        command(
            &mut stream,
            &[b"XTRIM", b"pending", b"MAXLEN", b"0"],
            b":1\r\n",
        );
        command(
            &mut stream,
            &[b"XPENDING", b"pending", b"g"],
            b"*4\r\n:1\r\n$4\r\n30-0\r\n$4\r\n30-0\r\n*1\r\n*2\r\n$1\r\nc\r\n:1\r\n",
        );

        command(
            &mut stream,
            &[b"XADD", b"w", b"40-0", b"f", b"v"],
            b"$4\r\n40-0\r\n",
        );
        command(
            &mut stream,
            &[b"XADD", b"w", b"41-0", b"f", b"v"],
            b"$4\r\n41-0\r\n",
        );
        command(&mut stream, &[b"WATCH", b"w"], b"+OK\r\n");
        let mut other = connect(&server);
        command(&mut other, &[b"XTRIM", b"w", b"MAXLEN", b"1"], b":1\r\n");
        command(&mut stream, &[b"MULTI"], b"+OK\r\n");
        command(&mut stream, &[b"XLEN", b"w"], b"+QUEUED\r\n");
        command(&mut stream, &[b"EXEC"], b"*-1\r\n");

        command(&mut stream, &[b"RPUSH", b"list", b"x"], b":1\r\n");
        command(
            &mut stream,
            &[b"XTRIM", b"list", b"MAXLEN", b"0"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );

        server.shutdown().unwrap();
    }
}
