#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

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

fn send(stream: &mut TcpStream, args: &[&[u8]]) {
    let mut request = Vec::new();
    encode_request(args, &mut request);
    stream.write_all(&request).unwrap();
}

fn command(stream: &mut TcpStream, args: &[&[u8]], expected: &[u8]) {
    send(stream, args);
    assert_eq!(read_frame(stream, 0), expected, "{args:?}");
}

fn reply(stream: &mut TcpStream, args: &[&[u8]]) -> Vec<u8> {
    send(stream, args);
    read_frame(stream, 0)
}

#[test]
fn blocking_xreadgroup_returns_immediately_and_times_out_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = connect(&server);
        command(
            &mut stream,
            &[b"XADD", b"s", b"1-0", b"f", b"v"],
            b"$3\r\n1-0\r\n",
        );
        command(
            &mut stream,
            &[b"XGROUP", b"CREATE", b"s", b"g", b"0"],
            b"+OK\r\n",
        );
        command(
            &mut stream,
            &[
                b"XREADGROUP",
                b"GROUP",
                b"g",
                b"c",
                b"BLOCK",
                b"0",
                b"COUNT",
                b"1",
                b"STREAMS",
                b"s",
                b">",
            ],
            b"*1\r\n*2\r\n$1\r\ns\r\n*1\r\n*2\r\n$3\r\n1-0\r\n*2\r\n$1\r\nf\r\n$1\r\nv\r\n",
        );
        let started = Instant::now();
        command(
            &mut stream,
            &[
                b"XREADGROUP",
                b"GROUP",
                b"g",
                b"c",
                b"BLOCK",
                b"50",
                b"STREAMS",
                b"s",
                b">",
            ],
            b"*-1\r\n",
        );
        assert!(started.elapsed() >= Duration::from_millis(20));
        assert!(started.elapsed() < Duration::from_secs(3));

        command(
            &mut stream,
            &[
                b"XREADGROUP",
                b"GROUP",
                b"g",
                b"c",
                b"BLOCK",
                b"0",
                b"STREAMS",
                b"s",
                b"0",
            ],
            b"*1\r\n*2\r\n$1\r\ns\r\n*1\r\n*2\r\n$3\r\n1-0\r\n*2\r\n$1\r\nf\r\n$1\r\nv\r\n",
        );
        command(
            &mut stream,
            &[
                b"XREADGROUP",
                b"GROUP",
                b"g",
                b"c",
                b"BLOCK",
                b"x",
                b"STREAMS",
                b"s",
                b">",
            ],
            b"-ERR timeout is not an integer or out of range\r\n",
        );
        command(
            &mut stream,
            &[
                b"XREADGROUP",
                b"GROUP",
                b"g",
                b"c",
                b"BLOCK",
                b"-1",
                b"STREAMS",
                b"s",
                b">",
            ],
            b"-ERR timeout is negative\r\n",
        );
        command(
            &mut stream,
            &[
                b"XREADGROUP", b"GROUP", b"g", b"c", b"BLOCK", b"1", b"STREAMS", b"s", b"$",
            ],
            b"-ERR The $ ID is meaningless in the context of XREADGROUP: you want to read the history of this consumer by specifying a proper ID, or use the > ID to get new messages. The $ ID would just return an empty result set.\r\n",
        );
        command(
            &mut stream,
            &[
                b"XREADGROUP", b"GROUP", b"g", b"c", b"BLOCK", b"0", b"STREAMS", b"missing",
                b">",
            ],
            b"-NOGROUP No such key 'missing' or consumer group 'g' in XREADGROUP with GROUP option\r\n",
        );
        command(&mut stream, &[b"SET", b"str", b"v"], b"+OK\r\n");
        command(
            &mut stream,
            &[
                b"XREADGROUP",
                b"GROUP",
                b"g",
                b"c",
                b"BLOCK",
                b"0",
                b"STREAMS",
                b"str",
                b">",
            ],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );
        server.shutdown().unwrap();
    }
}

#[test]
fn blocking_xreadgroup_wakes_updates_pending_and_respects_noack() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut blocked = connect(&server);
        let mut writer = connect(&server);
        command(
            &mut writer,
            &[b"XADD", b"w", b"1-0", b"f", b"v"],
            b"$3\r\n1-0\r\n",
        );
        command(
            &mut writer,
            &[b"XGROUP", b"CREATE", b"w", b"g", b"$"],
            b"+OK\r\n",
        );
        send(
            &mut blocked,
            &[
                b"XREADGROUP",
                b"GROUP",
                b"g",
                b"c",
                b"BLOCK",
                b"0",
                b"STREAMS",
                b"w",
                b">",
            ],
        );
        std::thread::sleep(Duration::from_millis(100));
        command(
            &mut writer,
            &[b"XADD", b"w", b"2-0", b"f", b"v2"],
            b"$3\r\n2-0\r\n",
        );
        assert_eq!(
            read_frame(&mut blocked, 0),
            b"*1\r\n*2\r\n$1\r\nw\r\n*1\r\n*2\r\n$3\r\n2-0\r\n*2\r\n$1\r\nf\r\n$2\r\nv2\r\n"
        );
        let pending = reply(&mut writer, &[b"XPENDING", b"w", b"g", b"-", b"+", b"10"]);
        assert!(pending.starts_with(b"*1\r\n"));
        assert!(pending.windows(3).any(|window| window == b"2-0"));
        let groups = reply(&mut writer, &[b"XINFO", b"GROUPS", b"w"]);
        assert!(
            groups
                .windows(26)
                .any(|window| window == b"last-delivered-id\r\n$3\r\n2-0")
        );

        command(
            &mut writer,
            &[b"XGROUP", b"CREATE", b"n", b"h", b"$", b"MKSTREAM"],
            b"+OK\r\n",
        );
        send(
            &mut blocked,
            &[
                b"XREADGROUP",
                b"GROUP",
                b"h",
                b"c",
                b"BLOCK",
                b"0",
                b"NOACK",
                b"STREAMS",
                b"n",
                b">",
            ],
        );
        std::thread::sleep(Duration::from_millis(100));
        command(
            &mut writer,
            &[b"XADD", b"n", b"3-0", b"f", b"v3"],
            b"$3\r\n3-0\r\n",
        );
        assert_eq!(
            read_frame(&mut blocked, 0),
            b"*1\r\n*2\r\n$1\r\nn\r\n*1\r\n*2\r\n$3\r\n3-0\r\n*2\r\n$1\r\nf\r\n$2\r\nv3\r\n"
        );
        command(
            &mut writer,
            &[b"XPENDING", b"n", b"h"],
            b"*4\r\n:0\r\n$-1\r\n$-1\r\n*0\r\n",
        );
        let groups = reply(&mut writer, &[b"XINFO", b"GROUPS", b"n"]);
        assert!(
            groups
                .windows(26)
                .any(|window| window == b"last-delivered-id\r\n$3\r\n3-0")
        );

        server.shutdown().unwrap();
    }
}

#[test]
fn blocking_xreadgroup_is_non_blocking_inside_multi() {
    let server = start(None);
    let mut stream = connect(&server);
    command(
        &mut stream,
        &[b"XADD", b"m", b"1-0", b"f", b"v"],
        b"$3\r\n1-0\r\n",
    );
    command(
        &mut stream,
        &[b"XGROUP", b"CREATE", b"m", b"g", b"0"],
        b"+OK\r\n",
    );
    command(&mut stream, &[b"MULTI"], b"+OK\r\n");
    command(
        &mut stream,
        &[
            b"XREADGROUP",
            b"GROUP",
            b"g",
            b"c",
            b"BLOCK",
            b"0",
            b"STREAMS",
            b"m",
            b">",
        ],
        b"+QUEUED\r\n",
    );
    command(
        &mut stream,
        &[b"EXEC"],
        b"*1\r\n*1\r\n*2\r\n$1\r\nm\r\n*1\r\n*2\r\n$3\r\n1-0\r\n*2\r\n$1\r\nf\r\n$1\r\nv\r\n",
    );

    command(
        &mut stream,
        &[b"XGROUP", b"CREATE", b"m", b"g2", b"$"],
        b"+OK\r\n",
    );
    let started = Instant::now();
    command(&mut stream, &[b"MULTI"], b"+OK\r\n");
    command(
        &mut stream,
        &[
            b"XREADGROUP",
            b"GROUP",
            b"g2",
            b"c",
            b"BLOCK",
            b"0",
            b"STREAMS",
            b"m",
            b">",
        ],
        b"+QUEUED\r\n",
    );
    command(&mut stream, &[b"EXEC"], b"*1\r\n*-1\r\n");
    assert!(started.elapsed() < Duration::from_secs(1));

    command(&mut stream, &[b"MULTI"], b"+OK\r\n");
    command(
        &mut stream,
        &[
            b"XREADGROUP",
            b"GROUP",
            b"g",
            b"c",
            b"BLOCK",
            b"0",
            b"STREAMS",
            b"missing",
            b">",
        ],
        b"+QUEUED\r\n",
    );
    command(
        &mut stream,
        &[b"EXEC"],
        b"*1\r\n-NOGROUP No such key 'missing' or consumer group 'g' in XREADGROUP with GROUP option\r\n",
    );

    server.shutdown().unwrap();
}
