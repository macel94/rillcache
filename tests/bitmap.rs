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
    stream
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

fn command(stream: &mut TcpStream, args: &[&[u8]], expected: &[u8]) {
    let mut request = Vec::new();
    encode_request(args, &mut request);
    stream.write_all(&request).unwrap();
    assert_eq!(read_frame(stream, 0), expected, "{args:?}");
}

#[test]
fn bitmap_algorithms_work_over_tcp_in_all_execution_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = connect(&server);

        command(&mut stream, &[b"SET", b"k", b"\xff\x00\x0f"], b"+OK\r\n");
        command(&mut stream, &[b"BITCOUNT", b"k"], b":12\r\n");
        command(&mut stream, &[b"BITCOUNT", b"k", b"0", b"0"], b":8\r\n");
        command(
            &mut stream,
            &[b"BITCOUNT", b"k", b"0", b"7", b"BIT"],
            b":8\r\n",
        );
        command(&mut stream, &[b"BITPOS", b"k", b"0"], b":8\r\n");
        command(&mut stream, &[b"BITPOS", b"k", b"1", b"0", b"0"], b":0\r\n");
        command(&mut stream, &[b"BITPOS", b"k", b"0", b"0", b"1"], b":8\r\n");

        command(&mut stream, &[b"SET", b"a", b"\xf0\x0f"], b"+OK\r\n");
        command(&mut stream, &[b"SET", b"b", b"\x0f\xf0\xff"], b"+OK\r\n");
        command(
            &mut stream,
            &[b"BITOP", b"AND", b"d", b"a", b"b"],
            b":3\r\n",
        );
        command(&mut stream, &[b"GET", b"d"], b"$3\r\n\0\0\0\r\n");
        command(&mut stream, &[b"BITOP", b"OR", b"d", b"a", b"b"], b":3\r\n");
        command(&mut stream, &[b"GET", b"d"], b"$3\r\n\xff\xff\xff\r\n");
        command(&mut stream, &[b"BITOP", b"NOT", b"d", b"a"], b":2\r\n");
        command(&mut stream, &[b"GET", b"d"], b"$2\r\n\x0f\xf0\r\n");
        command(
            &mut stream,
            &[b"BITOP", b"AND", b"d", b"missing"],
            b":0\r\n",
        );
        command(&mut stream, &[b"EXISTS", b"d"], b":0\r\n");

        command(
            &mut stream,
            &[b"BITCOUNT", b"k", b"0", b"1", b"BOGUS"],
            b"-ERR syntax error\r\n",
        );
        command(
            &mut stream,
            &[b"BITPOS", b"k", b"2"],
            b"-ERR The bit argument must be 1 or 0.\r\n",
        );
        command(
            &mut stream,
            &[b"BITOP", b"NOT", b"d", b"a", b"b"],
            b"-ERR BITOP NOT must be called with a single source key.\r\n",
        );
        command(&mut stream, &[b"RPUSH", b"list", b"x"], b":1\r\n");
        command(
            &mut stream,
            &[b"BITOP", b"OR", b"d", b"a", b"list"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );

        command(&mut stream, &[b"MULTI"], b"+OK\r\n");
        command(&mut stream, &[b"BITCOUNT", b"k"], b"+QUEUED\r\n");
        command(
            &mut stream,
            &[b"BITOP", b"OR", b"out", b"a"],
            b"+QUEUED\r\n",
        );
        command(&mut stream, &[b"EXEC"], b"*2\r\n:12\r\n:2\r\n");
        command(&mut stream, &[b"STRLEN", b"out"], b":2\r\n");

        server.shutdown().unwrap();
    }
}
