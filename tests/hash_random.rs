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
fn hash_random_field_and_getdel_work_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = connect(&server);
        command(
            &mut stream,
            &[b"HSET", b"h", b"f1", b"v1", b"f2", b"v2", b"f3", b"v3"],
            b":3\r\n",
        );
        let field = reply(&mut stream, &[b"HRANDFIELD", b"h"]);
        assert!(field.starts_with(b"$2\r\n"));
        let all = reply(&mut stream, &[b"HRANDFIELD", b"h", b"10"]);
        assert!(all.starts_with(b"*3\r\n"));
        for value in [b"f1".as_slice(), b"f2", b"f3"] {
            assert!(contains(&all, value));
        }
        let values = reply(&mut stream, &[b"HRANDFIELD", b"h", b"10", b"WITHVALUES"]);
        assert!(values.starts_with(b"*6\r\n"));
        assert!(contains(&values, b"v1"));
        let two = reply(&mut stream, &[b"HRANDFIELD", b"h", b"2"]);
        assert!(two.starts_with(b"*2\r\n"));
        let repeated = reply(&mut stream, &[b"HRANDFIELD", b"h", b"-5"]);
        assert!(repeated.starts_with(b"*5\r\n"));
        command(&mut stream, &[b"HRANDFIELD", b"h", b"0"], b"*0\r\n");
        command(&mut stream, &[b"HRANDFIELD", b"missing"], b"$-1\r\n");
        command(&mut stream, &[b"HRANDFIELD", b"missing", b"2"], b"*0\r\n");
        command(
            &mut stream,
            &[b"HRANDFIELD", b"h", b"bad"],
            b"-ERR value is not an integer or out of range\r\n",
        );
        command(
            &mut stream,
            &[b"HRANDFIELD", b"h", b"2", b"BOGUS"],
            b"-ERR syntax error\r\n",
        );
        command(&mut stream, &[b"SET", b"str", b"v"], b"+OK\r\n");
        command(
            &mut stream,
            &[b"HRANDFIELD", b"str"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );

        command(
            &mut stream,
            &[b"HGETDEL", b"h", b"FIELDS", b"2", b"f1", b"missing"],
            b"*2\r\n$2\r\nv1\r\n$-1\r\n",
        );
        command(&mut stream, &[b"HGET", b"h", b"f1"], b"$-1\r\n");
        command(
            &mut stream,
            &[b"HGETDEL", b"h", b"FIELDS", b"2", b"f2", b"f3"],
            b"*2\r\n$2\r\nv2\r\n$2\r\nv3\r\n",
        );
        command(&mut stream, &[b"EXISTS", b"h"], b":0\r\n");

        command(
            &mut stream,
            &[b"HSET", b"ttl", b"a", b"1", b"b", b"2"],
            b":2\r\n",
        );
        command(&mut stream, &[b"PEXPIRE", b"ttl", b"100000"], b":1\r\n");
        command(
            &mut stream,
            &[b"HGETDEL", b"ttl", b"FIELDS", b"1", b"a"],
            b"*1\r\n$1\r\n1\r\n",
        );
        let ttl = reply(&mut stream, &[b"PTTL", b"ttl"]);
        let ttl = std::str::from_utf8(&ttl[1..ttl.len() - 2])
            .unwrap()
            .parse::<i64>()
            .unwrap();
        assert!(ttl > 0);
        command(
            &mut stream,
            &[b"HGETDEL", b"missing", b"FIELDS", b"2", b"a", b"b"],
            b"*2\r\n$-1\r\n$-1\r\n",
        );
        command(
            &mut stream,
            &[b"HGETDEL", b"h", b"BAD", b"1", b"a"],
            b"-ERR Mandatory argument FIELDS is missing or not at the right position\r\n",
        );
        command(
            &mut stream,
            &[b"HGETDEL", b"h", b"FIELDS", b"0", b"a"],
            b"-ERR Number of fields must be a positive integer\r\n",
        );
        command(
            &mut stream,
            &[b"HGETDEL", b"h", b"FIELDS", b"2", b"a"],
            b"-ERR The `numfields` parameter must match the number of arguments\r\n",
        );
        command(
            &mut stream,
            &[b"HGETDEL", b"str", b"FIELDS", b"1", b"a"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );

        command(
            &mut stream,
            &[b"HSET", b"w", b"a", b"1", b"b", b"2"],
            b":2\r\n",
        );
        command(&mut stream, &[b"WATCH", b"w"], b"+OK\r\n");
        let mut other = connect(&server);
        command(
            &mut other,
            &[b"HGETDEL", b"w", b"FIELDS", b"1", b"a"],
            b"*1\r\n$1\r\n1\r\n",
        );
        command(&mut stream, &[b"MULTI"], b"+OK\r\n");
        command(&mut stream, &[b"HLEN", b"w"], b"+QUEUED\r\n");
        command(&mut stream, &[b"EXEC"], b"*-1\r\n");

        command(&mut stream, &[b"MULTI"], b"+OK\r\n");
        command(&mut stream, &[b"HRANDFIELD", b"ttl"], b"+QUEUED\r\n");
        let exec = reply(&mut stream, &[b"EXEC"]);
        assert!(exec.starts_with(b"*1\r\n$1\r\nb\r\n"));

        server.shutdown().unwrap();
    }
}
