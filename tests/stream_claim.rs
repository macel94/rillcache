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
fn stream_claim_and_introspection_work_over_tcp_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = connect(&server);
        for (id, value) in [
            (b"1".as_slice(), b"a".as_slice()),
            (b"2", b"b"),
            (b"3", b"c"),
        ] {
            assert!(reply(&mut stream, &[b"XADD", b"s", id, b"f", value]).starts_with(b"$"));
        }
        command(
            &mut stream,
            &[b"XGROUP", b"CREATE", b"s", b"g", b"0"],
            b"+OK\r\n",
        );
        let delivered = reply(
            &mut stream,
            &[
                b"XREADGROUP",
                b"GROUP",
                b"g",
                b"alice",
                b"COUNT",
                b"2",
                b"STREAMS",
                b"s",
                b">",
            ],
        );
        assert!(contains(&delivered, b"1-0") && contains(&delivered, b"2-0"));

        command(
            &mut stream,
            &[b"XINFO", b"GROUPS", b"s"],
            b"*1\r\n*12\r\n$4\r\nname\r\n$1\r\ng\r\n$9\r\nconsumers\r\n:1\r\n$7\r\npending\r\n:2\r\n$17\r\nlast-delivered-id\r\n$3\r\n2-0\r\n$12\r\nentries-read\r\n$-1\r\n$3\r\nlag\r\n:1\r\n",
        );
        let consumers = reply(&mut stream, &[b"XINFO", b"CONSUMERS", b"s", b"g"]);
        assert!(
            consumers
                .starts_with(b"*1\r\n*8\r\n$4\r\nname\r\n$5\r\nalice\r\n$7\r\npending\r\n:2\r\n")
        );
        assert!(contains(&consumers, b"$4\r\nidle\r\n"));
        assert!(contains(&consumers, b"$8\r\ninactive\r\n"));

        command(
            &mut stream,
            &[b"XCLAIM", b"s", b"g", b"bob", b"0", b"1-0"],
            b"*1\r\n*2\r\n$3\r\n1-0\r\n*2\r\n$1\r\nf\r\n$1\r\na\r\n",
        );
        command(
            &mut stream,
            &[b"XCLAIM", b"s", b"g", b"bob", b"0", b"2-0", b"JUSTID"],
            b"*1\r\n$3\r\n2-0\r\n",
        );
        command(
            &mut stream,
            &[
                b"XCLAIM",
                b"s",
                b"g",
                b"bob",
                b"0",
                b"3-0",
                b"FORCE",
                b"RETRYCOUNT",
                b"5",
            ],
            b"*1\r\n*2\r\n$3\r\n3-0\r\n*2\r\n$1\r\nf\r\n$1\r\nc\r\n",
        );
        let pending = reply(
            &mut stream,
            &[b"XPENDING", b"s", b"g", b"-", b"+", b"10", b"bob"],
        );
        assert!(pending.starts_with(b"*3\r\n"));
        assert!(contains(&pending, b"3-0"));
        assert!(contains(&pending, b":5\r\n"));
        command(
            &mut stream,
            &[b"XCLAIM", b"s", b"g", b"bob", b"999999", b"1-0"],
            b"*0\r\n",
        );
        command(
            &mut stream,
            &[b"XCLAIM", b"s", b"g", b"bob", b"0", b"3-0", b"TIME", b"1"],
            b"*1\r\n*2\r\n$3\r\n3-0\r\n*2\r\n$1\r\nf\r\n$1\r\nc\r\n",
        );
        assert!(contains(
            &reply(
                &mut stream,
                &[
                    b"XPENDING",
                    b"s",
                    b"g",
                    b"IDLE",
                    b"1000",
                    b"-",
                    b"+",
                    b"10",
                    b"bob"
                ]
            ),
            b"3-0"
        ));
        command(
            &mut stream,
            &[
                b"XCLAIM", b"s", b"g", b"bob", b"0", b"3-0", b"LASTID", b"5-0",
            ],
            b"*1\r\n*2\r\n$3\r\n3-0\r\n*2\r\n$1\r\nf\r\n$1\r\nc\r\n",
        );
        command(
            &mut stream,
            &[b"XINFO", b"GROUPS", b"s"],
            b"*1\r\n*12\r\n$4\r\nname\r\n$1\r\ng\r\n$9\r\nconsumers\r\n:2\r\n$7\r\npending\r\n:3\r\n$17\r\nlast-delivered-id\r\n$3\r\n5-0\r\n$12\r\nentries-read\r\n$-1\r\n$3\r\nlag\r\n:0\r\n",
        );

        let first = reply(
            &mut stream,
            &[
                b"XAUTOCLAIM",
                b"s",
                b"g",
                b"bob",
                b"0",
                b"0-0",
                b"COUNT",
                b"2",
            ],
        );
        assert!(first.starts_with(b"*3\r\n$3\r\n3-0\r\n*2\r\n"));
        command(
            &mut stream,
            &[
                b"XAUTOCLAIM",
                b"s",
                b"g",
                b"bob",
                b"0",
                b"3-0",
                b"COUNT",
                b"10",
                b"JUSTID",
            ],
            b"*3\r\n$3\r\n0-0\r\n*1\r\n$3\r\n3-0\r\n*0\r\n",
        );

        command(&mut stream, &[b"XDEL", b"s", b"1-0"], b":1\r\n");
        let cleaned = reply(
            &mut stream,
            &[
                b"XAUTOCLAIM",
                b"s",
                b"g",
                b"bob",
                b"0",
                b"0-0",
                b"COUNT",
                b"10",
            ],
        );
        assert!(cleaned.starts_with(b"*3\r\n"));
        assert!(contains(&cleaned, b"1-0"));

        command(
            &mut stream,
            &[b"XCLAIM", b"s", b"g", b"bob", b"not-an-integer", b"1-0"],
            b"-ERR Invalid min-idle-time argument for XCLAIM\r\n",
        );
        command(
            &mut stream,
            &[b"XCLAIM", b"s", b"g", b"bob", b"0", b"1-0", b"BOGUS"],
            b"-ERR Unrecognized XCLAIM option 'BOGUS'\r\n",
        );
        command(
            &mut stream,
            &[b"XCLAIM", b"missing", b"g", b"bob", b"0", b"1-0"],
            b"-NOGROUP No such key or consumer group\r\n",
        );
        command(
            &mut stream,
            &[b"XCLAIM", b"s", b"missing-group", b"bob", b"0", b"1-0"],
            b"-NOGROUP No such consumer group\r\n",
        );
        command(
            &mut stream,
            &[b"XAUTOCLAIM", b"s", b"g", b"bob", b"0", b"not-an-id"],
            b"-ERR Invalid stream ID specified as stream command argument\r\n",
        );
        command(
            &mut stream,
            &[
                b"XAUTOCLAIM",
                b"s",
                b"g",
                b"bob",
                b"0",
                b"0-0",
                b"COUNT",
                b"0",
            ],
            b"-ERR COUNT must be > 0\r\n",
        );
        command(
            &mut stream,
            &[b"XAUTOCLAIM", b"s", b"g", b"bob", b"0", b"0-0", b"BOGUS"],
            b"-ERR syntax error\r\n",
        );
        command(
            &mut stream,
            &[b"XAUTOCLAIM", b"missing", b"g", b"bob", b"0", b"0-0"],
            b"-NOGROUP No such key or consumer group\r\n",
        );
        command(
            &mut stream,
            &[b"XINFO", b"GROUPS", b"missing"],
            b"-ERR no such key\r\n",
        );
        command(
            &mut stream,
            &[b"XINFO", b"CONSUMERS", b"s", b"missing-group"],
            b"-NOGROUP No such consumer group\r\n",
        );

        command(&mut stream, &[b"RPUSH", b"list", b"x"], b":1\r\n");
        command(
            &mut stream,
            &[b"XCLAIM", b"list", b"g", b"bob", b"0", b"1-0"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );
        command(
            &mut stream,
            &[b"XINFO", b"GROUPS", b"list"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );

        command(
            &mut stream,
            &[b"XGROUP", b"SETID", b"s", b"g", b"0"],
            b"+OK\r\n",
        );
        let delivered = reply(
            &mut stream,
            &[
                b"XREADGROUP",
                b"GROUP",
                b"g",
                b"bob",
                b"STREAMS",
                b"s",
                b">",
            ],
        );
        assert!(contains(&delivered, b"2-0"));
        command(&mut stream, &[b"WATCH", b"s"], b"+OK\r\n");
        let mut other = connect(&server);
        command(
            &mut other,
            &[b"XCLAIM", b"s", b"g", b"mallory", b"0", b"2-0", b"JUSTID"],
            b"*1\r\n$3\r\n2-0\r\n",
        );
        command(&mut stream, &[b"MULTI"], b"+OK\r\n");
        command(&mut stream, &[b"XLEN", b"s"], b"+QUEUED\r\n");
        command(&mut stream, &[b"EXEC"], b"*-1\r\n");

        server.shutdown().unwrap();
    }
}
