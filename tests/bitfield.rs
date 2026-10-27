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
fn bitfield_commands_work_over_tcp_in_all_execution_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = connect(&server);

        command(&mut stream, &[b"BITFIELD", b"k"], b"*0\r\n");
        command(
            &mut stream,
            &[b"BITFIELD", b"k", b"GET", b"u8", b"0"],
            b"*1\r\n:0\r\n",
        );
        command(
            &mut stream,
            &[
                b"BITFIELD",
                b"k",
                b"SET",
                b"u8",
                b"0",
                b"255",
                b"GET",
                b"u8",
                b"0",
            ],
            b"*2\r\n:0\r\n:255\r\n",
        );
        command(
            &mut stream,
            &[
                b"BITFIELD",
                b"k",
                b"SET",
                b"i8",
                b"0",
                b"-1",
                b"GET",
                b"i8",
                b"0",
                b"GET",
                b"u8",
                b"0",
            ],
            b"*3\r\n:-1\r\n:-1\r\n:255\r\n",
        );
        command(
            &mut stream,
            &[
                b"BITFIELD",
                b"k",
                b"SET",
                b"u8",
                b"#1",
                b"7",
                b"GET",
                b"u8",
                b"8",
            ],
            b"*2\r\n:0\r\n:7\r\n",
        );
        command(
            &mut stream,
            &[
                b"BITFIELD",
                b"k",
                b"SET",
                b"u8",
                b"0",
                b"200",
                b"OVERFLOW",
                b"SAT",
                b"INCRBY",
                b"u8",
                b"0",
                b"100",
                b"GET",
                b"u8",
                b"0",
                b"OVERFLOW",
                b"FAIL",
                b"INCRBY",
                b"u8",
                b"0",
                b"100",
                b"GET",
                b"u8",
                b"0",
            ],
            b"*5\r\n:255\r\n:255\r\n:255\r\n$-1\r\n:255\r\n",
        );
        command(&mut stream, &[b"GET", b"k"], b"$2\r\n\xff\x07\r\n");

        command(
            &mut stream,
            &[b"BITFIELD_RO", b"k", b"GET", b"u8", b"0"],
            b"*1\r\n:255\r\n",
        );
        command(
            &mut stream,
            &[b"BITFIELD_RO", b"k", b"SET", b"u8", b"0", b"1"],
            b"-ERR BITFIELD_RO only supports the GET subcommand\r\n",
        );
        command(
            &mut stream,
            &[b"BITFIELD", b"k", b"GET", b"u64", b"0"],
            b"-ERR Invalid bitfield type. Use something like i16 u8. Note that u64 is not supported but i64 is.\r\n",
        );
        command(
            &mut stream,
            &[b"BITFIELD", b"k", b"GET", b"u8", b"x"],
            b"-ERR bit offset is not an integer or out of range\r\n",
        );
        command(
            &mut stream,
            &[
                b"BITFIELD",
                b"k",
                b"OVERFLOW",
                b"BOGUS",
                b"GET",
                b"u8",
                b"0",
            ],
            b"-ERR Invalid OVERFLOW type specified\r\n",
        );
        command(
            &mut stream,
            &[b"BITFIELD", b"k", b"BOGUS", b"u8", b"0"],
            b"-ERR syntax error\r\n",
        );
        command(&mut stream, &[b"RPUSH", b"list", b"x"], b":1\r\n");
        command(
            &mut stream,
            &[b"BITFIELD", b"list", b"GET", b"u8", b"0"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );

        command(&mut stream, &[b"DEL", b"k"], b":1\r\n");
        command(&mut stream, &[b"MULTI"], b"+OK\r\n");
        command(
            &mut stream,
            &[b"BITFIELD", b"k", b"SET", b"u1", b"0", b"1"],
            b"+QUEUED\r\n",
        );
        command(
            &mut stream,
            &[b"BITFIELD_RO", b"k", b"GET", b"u1", b"0"],
            b"+QUEUED\r\n",
        );
        command(&mut stream, &[b"EXEC"], b"*2\r\n*1\r\n:0\r\n*1\r\n:1\r\n");

        server.shutdown().unwrap();
    }
}
