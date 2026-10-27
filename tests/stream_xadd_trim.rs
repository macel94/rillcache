#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

fn start(workers: Option<usize>, max_data_bytes: usize) -> ServerHandle {
    Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        worker_threads: workers,
        max_data_bytes,
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

#[test]
fn xadd_trim_and_nomkstream_work_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers, Config::default().max_data_bytes);
        let mut stream = connect(&server);

        command(
            &mut stream,
            &[b"XADD", b"s", b"MAXLEN", b"2", b"1-0", b"f", b"a"],
            b"$3\r\n1-0\r\n",
        );
        command(
            &mut stream,
            &[b"XADD", b"s", b"MAXLEN", b"2", b"2-0", b"f", b"b"],
            b"$3\r\n2-0\r\n",
        );
        command(
            &mut stream,
            &[b"XADD", b"s", b"MAXLEN", b"2", b"3-0", b"f", b"c"],
            b"$3\r\n3-0\r\n",
        );
        command(&mut stream, &[b"XLEN", b"s"], b":2\r\n");
        command(
            &mut stream,
            &[b"XRANGE", b"s", b"-", b"+"],
            b"*2\r\n*2\r\n$3\r\n2-0\r\n*2\r\n$1\r\nf\r\n$1\r\nb\r\n*2\r\n$3\r\n3-0\r\n*2\r\n$1\r\nf\r\n$1\r\nc\r\n",
        );

        command(
            &mut stream,
            &[
                b"XADD", b"s", b"MAXLEN", b"~", b"0", b"LIMIT", b"1", b"4-0", b"f", b"d",
            ],
            b"$3\r\n4-0\r\n",
        );
        command(&mut stream, &[b"XLEN", b"s"], b":2\r\n");
        command(
            &mut stream,
            &[b"XRANGE", b"s", b"-", b"+"],
            b"*2\r\n*2\r\n$3\r\n3-0\r\n*2\r\n$1\r\nf\r\n$1\r\nc\r\n*2\r\n$3\r\n4-0\r\n*2\r\n$1\r\nf\r\n$1\r\nd\r\n",
        );
        command(
            &mut stream,
            &[b"XADD", b"s", b"MINID", b"4-0", b"5-0", b"f", b"e"],
            b"$3\r\n5-0\r\n",
        );
        command(&mut stream, &[b"XLEN", b"s"], b":2\r\n");
        command(
            &mut stream,
            &[b"XADD", b"s", b"MINID", b"~", b"5-0", b"6-0", b"f", b"f"],
            b"$3\r\n6-0\r\n",
        );
        command(
            &mut stream,
            &[b"XRANGE", b"s", b"-", b"+"],
            b"*2\r\n*2\r\n$3\r\n5-0\r\n*2\r\n$1\r\nf\r\n$1\r\ne\r\n*2\r\n$3\r\n6-0\r\n*2\r\n$1\r\nf\r\n$1\r\nf\r\n",
        );

        command(
            &mut stream,
            &[b"XADD", b"missing", b"NOMKSTREAM", b"1-0", b"f", b"a"],
            b"$-1\r\n",
        );
        command(&mut stream, &[b"EXISTS", b"missing"], b":0\r\n");
        command(
            &mut stream,
            &[b"XADD", b"s", b"NOMKSTREAM", b"7-0", b"f", b"g"],
            b"$3\r\n7-0\r\n",
        );

        command(&mut stream, &[b"PEXPIRE", b"s", b"100000"], b":1\r\n");
        command(
            &mut stream,
            &[b"XADD", b"s", b"MAXLEN", b"1", b"8-0", b"f", b"h"],
            b"$3\r\n8-0\r\n",
        );
        let ttl = reply(&mut stream, &[b"PTTL", b"s"]);
        let ttl = std::str::from_utf8(&ttl[1..ttl.len() - 2])
            .unwrap()
            .parse::<i64>()
            .unwrap();
        assert!(ttl > 0);
        command(&mut stream, &[b"XLEN", b"s"], b":1\r\n");

        command(
            &mut stream,
            &[b"XADD", b"zero", b"MAXLEN", b"0", b"1-0", b"f", b"a"],
            b"$3\r\n1-0\r\n",
        );
        command(&mut stream, &[b"XLEN", b"zero"], b":0\r\n");
        command(&mut stream, &[b"EXISTS", b"zero"], b":1\r\n");

        command(
            &mut stream,
            &[b"XADD", b"s", b"MAXLEN", b"-1", b"9-0", b"f", b"i"],
            b"-ERR The MAXLEN argument must be >= 0.\r\n",
        );
        command(
            &mut stream,
            &[
                b"XADD", b"s", b"MAXLEN", b"1", b"MINID", b"2-0", b"9-0", b"f", b"i",
            ],
            b"-ERR syntax error, MAXLEN and MINID options at the same time are not compatible\r\n",
        );
        command(
            &mut stream,
            &[b"XADD", b"s", b"MINID", b"bad", b"9-0", b"f", b"i"],
            b"-ERR Invalid stream ID specified as stream command argument\r\n",
        );
        command(
            &mut stream,
            &[
                b"XADD", b"s", b"MAXLEN", b"1", b"LIMIT", b"1", b"9-0", b"f", b"i",
            ],
            b"-ERR syntax error, LIMIT cannot be used without the special ~ option\r\n",
        );
        command(
            &mut stream,
            &[b"XADD", b"s", b"LIMIT", b"1", b"9-0", b"f", b"i"],
            b"-ERR syntax error, LIMIT cannot be used without specifying a trimming strategy\r\n",
        );
        command(
            &mut stream,
            &[b"XADD", b"s", b"MAXLEN", b"1", b"9-0", b"f"],
            b"-ERR wrong number of arguments for 'xadd' command\r\n",
        );
        command(
            &mut stream,
            &[b"XADD", b"s", b"MAXLEN", b"1", b"bad", b"f", b"i"],
            b"-ERR Invalid stream ID specified as stream command argument\r\n",
        );

        command(&mut stream, &[b"RPUSH", b"list", b"x"], b":1\r\n");
        command(
            &mut stream,
            &[b"XADD", b"list", b"MAXLEN", b"1", b"9-0", b"f", b"i"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );

        command(&mut stream, &[b"MULTI"], b"+OK\r\n");
        command(
            &mut stream,
            &[b"XADD", b"q", b"MAXLEN", b"1", b"1-0", b"f", b"a"],
            b"+QUEUED\r\n",
        );
        command(&mut stream, &[b"EXEC"], b"*1\r\n$3\r\n1-0\r\n");

        command(
            &mut stream,
            &[b"XADD", b"w", b"1-0", b"f", b"a"],
            b"$3\r\n1-0\r\n",
        );
        command(&mut stream, &[b"WATCH", b"w"], b"+OK\r\n");
        let mut other = connect(&server);
        command(
            &mut other,
            &[b"XADD", b"w", b"MAXLEN", b"1", b"2-0", b"f", b"b"],
            b"$3\r\n2-0\r\n",
        );
        command(&mut stream, &[b"MULTI"], b"+OK\r\n");
        command(&mut stream, &[b"XLEN", b"w"], b"+QUEUED\r\n");
        command(&mut stream, &[b"EXEC"], b"*-1\r\n");

        server.shutdown().unwrap();

        let small = start(workers, 4);
        let mut limited = connect(&small);
        command(
            &mut limited,
            &[b"XADD", b"big", b"1-0", b"f", b"long-value"],
            b"-OOM configured payload limit exceeded\r\n",
        );
        command(&mut limited, &[b"EXISTS", b"big"], b":0\r\n");
        small.shutdown().unwrap();
    }
}
