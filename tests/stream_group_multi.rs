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

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn seed(stream: &mut TcpStream, key: &[u8], value: &[u8]) {
    command(
        stream,
        &[b"XADD", key, b"1-0", b"f", value],
        b"$3\r\n1-0\r\n",
    );
    command(
        stream,
        &[b"XADD", key, b"2-0", b"f", value],
        b"$3\r\n2-0\r\n",
    );
    command(stream, &[b"XGROUP", b"CREATE", key, b"g", b"0"], b"+OK\r\n");
}

#[test]
fn multi_stream_group_reads_mix_new_and_history_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = connect(&server);
        seed(&mut stream, b"a", b"a");
        seed(&mut stream, b"b", b"b");

        command(
            &mut stream,
            &[
                b"XREADGROUP", b"GROUP", b"g", b"c", b"COUNT", b"1", b"STREAMS", b"a", b"b",
                b">", b">",
            ],
            b"*2\r\n*2\r\n$1\r\na\r\n*1\r\n*2\r\n$3\r\n1-0\r\n*2\r\n$1\r\nf\r\n$1\r\na\r\n*2\r\n$1\r\nb\r\n*1\r\n*2\r\n$3\r\n1-0\r\n*2\r\n$1\r\nf\r\n$1\r\nb\r\n",
        );
        command(
            &mut stream,
            &[
                b"XREADGROUP", b"GROUP", b"g", b"c", b"STREAMS", b"a", b"b", b"0", b">",
            ],
            b"*2\r\n*2\r\n$1\r\na\r\n*1\r\n*2\r\n$3\r\n1-0\r\n*2\r\n$1\r\nf\r\n$1\r\na\r\n*2\r\n$1\r\nb\r\n*1\r\n*2\r\n$3\r\n2-0\r\n*2\r\n$1\r\nf\r\n$1\r\nb\r\n",
        );
        command(
            &mut stream,
            &[b"XGROUP", b"CREATECONSUMER", b"a", b"g", b"d"],
            b":1\r\n",
        );
        command(
            &mut stream,
            &[b"XGROUP", b"CREATECONSUMER", b"b", b"g", b"d"],
            b":1\r\n",
        );
        command(
            &mut stream,
            &[
                b"XREADGROUP",
                b"GROUP",
                b"g",
                b"d",
                b"BLOCK",
                b"0",
                b"STREAMS",
                b"a",
                b"b",
                b"0",
                b"0",
            ],
            b"*2\r\n*2\r\n$1\r\na\r\n*0\r\n*2\r\n$1\r\nb\r\n*0\r\n",
        );

        command(
            &mut stream,
            &[b"XGROUP", b"CREATE", b"a", b"h", b"0"],
            b"+OK\r\n",
        );
        command(
            &mut stream,
            &[b"XGROUP", b"CREATE", b"b", b"h", b"0"],
            b"+OK\r\n",
        );
        command(
            &mut stream,
            &[
                b"XREADGROUP", b"GROUP", b"h", b"c", b"NOACK", b"STREAMS", b"a", b"b", b">",
                b">",
            ],
            b"*2\r\n*2\r\n$1\r\na\r\n*2\r\n*2\r\n$3\r\n1-0\r\n*2\r\n$1\r\nf\r\n$1\r\na\r\n*2\r\n$3\r\n2-0\r\n*2\r\n$1\r\nf\r\n$1\r\na\r\n*2\r\n$1\r\nb\r\n*2\r\n*2\r\n$3\r\n1-0\r\n*2\r\n$1\r\nf\r\n$1\r\nb\r\n*2\r\n$3\r\n2-0\r\n*2\r\n$1\r\nf\r\n$1\r\nb\r\n",
        );
        command(
            &mut stream,
            &[b"XPENDING", b"a", b"h"],
            b"*4\r\n:0\r\n$-1\r\n$-1\r\n*0\r\n",
        );

        seed(&mut stream, b"c", b"c");
        command(
            &mut stream,
            &[b"XREADGROUP", b"GROUP", b"g", b"x", b"STREAMS", b"c", b">"],
            b"*1\r\n*2\r\n$1\r\nc\r\n*2\r\n*2\r\n$3\r\n1-0\r\n*2\r\n$1\r\nf\r\n$1\r\nc\r\n*2\r\n$3\r\n2-0\r\n*2\r\n$1\r\nf\r\n$1\r\nc\r\n",
        );
        command(
            &mut stream,
            &[
                b"XREADGROUP", b"GROUP", b"g", b"x", b"STREAMS", b"c", b"missing", b">", b">",
            ],
            b"-NOGROUP No such key 'missing' or consumer group 'g' in XREADGROUP with GROUP option\r\n",
        );
        let groups = reply(&mut stream, &[b"XINFO", b"GROUPS", b"c"]);
        assert!(contains(&groups, b"last-delivered-id\r\n$3\r\n2-0"));
        command(
            &mut stream,
            &[b"XGROUP", b"CREATE", b"c", b"h", b"0"],
            b"+OK\r\n",
        );
        command(
            &mut stream,
            &[b"XGROUP", b"CREATE", b"missing", b"h", b"0", b"MKSTREAM"],
            b"+OK\r\n",
        );
        command(&mut stream, &[b"XDEL", b"c", b"1-0"], b":1\r\n");
        command(
            &mut stream,
            &[
                b"XREADGROUP",
                b"GROUP",
                b"h",
                b"d",
                b"STREAMS",
                b"c",
                b"missing",
                b"0",
                b"0",
            ],
            b"*2\r\n*2\r\n$1\r\nc\r\n*0\r\n*2\r\n$7\r\nmissing\r\n*0\r\n",
        );

        server.shutdown().unwrap();
    }
}

#[test]
fn multi_stream_group_blocking_waits_for_any_new_stream() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut blocked = connect(&server);
        let mut writer = connect(&server);
        command(
            &mut writer,
            &[b"XGROUP", b"CREATE", b"x", b"g", b"$", b"MKSTREAM"],
            b"+OK\r\n",
        );
        command(
            &mut writer,
            &[b"XGROUP", b"CREATE", b"y", b"g", b"$", b"MKSTREAM"],
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
                b"x",
                b"y",
                b">",
                b">",
            ],
        );
        std::thread::sleep(Duration::from_millis(100));
        command(
            &mut writer,
            &[b"XADD", b"y", b"1-0", b"f", b"v"],
            b"$3\r\n1-0\r\n",
        );
        assert_eq!(
            read_frame(&mut blocked, 0),
            b"*1\r\n*2\r\n$1\r\ny\r\n*1\r\n*2\r\n$3\r\n1-0\r\n*2\r\n$1\r\nf\r\n$1\r\nv\r\n"
        );
        let groups = reply(&mut writer, &[b"XINFO", b"GROUPS", b"y"]);
        assert!(contains(&groups, b"last-delivered-id\r\n$3\r\n1-0"));

        let started = Instant::now();
        command(
            &mut writer,
            &[
                b"XREADGROUP",
                b"GROUP",
                b"g",
                b"c",
                b"BLOCK",
                b"30",
                b"STREAMS",
                b"x",
                b"y",
                b">",
                b">",
            ],
            b"*-1\r\n",
        );
        assert!(started.elapsed() >= Duration::from_millis(10));
        assert!(started.elapsed() < Duration::from_secs(3));

        seed(&mut writer, b"h", b"h");
        command(
            &mut writer,
            &[b"XREADGROUP", b"GROUP", b"g", b"c", b"STREAMS", b"h", b">"],
            b"*1\r\n*2\r\n$1\r\nh\r\n*2\r\n*2\r\n$3\r\n1-0\r\n*2\r\n$1\r\nf\r\n$1\r\nh\r\n*2\r\n$3\r\n2-0\r\n*2\r\n$1\r\nf\r\n$1\r\nh\r\n",
        );
        let started = Instant::now();
        command(
            &mut blocked,
            &[
                b"XREADGROUP", b"GROUP", b"g", b"c", b"BLOCK", b"0", b"STREAMS", b"h", b"y",
                b"0", b">",
            ],
            b"*1\r\n*2\r\n$1\r\nh\r\n*2\r\n*2\r\n$3\r\n1-0\r\n*2\r\n$1\r\nf\r\n$1\r\nh\r\n*2\r\n$3\r\n2-0\r\n*2\r\n$1\r\nf\r\n$1\r\nh\r\n",
        );
        assert!(started.elapsed() < Duration::from_secs(1));

        server.shutdown().unwrap();
    }
}

#[test]
fn multi_stream_group_multi_and_errors() {
    let server = start(None);
    let mut stream = connect(&server);
    seed(&mut stream, b"m1", b"m");
    seed(&mut stream, b"m2", b"n");
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
            b"m1",
            b"m2",
            b">",
            b">",
        ],
        b"+QUEUED\r\n",
    );
    command(
        &mut stream,
        &[b"EXEC"],
        b"*1\r\n*2\r\n*2\r\n$2\r\nm1\r\n*2\r\n*2\r\n$3\r\n1-0\r\n*2\r\n$1\r\nf\r\n$1\r\nm\r\n*2\r\n$3\r\n2-0\r\n*2\r\n$1\r\nf\r\n$1\r\nm\r\n*2\r\n$2\r\nm2\r\n*2\r\n*2\r\n$3\r\n1-0\r\n*2\r\n$1\r\nf\r\n$1\r\nn\r\n*2\r\n$3\r\n2-0\r\n*2\r\n$1\r\nf\r\n$1\r\nn\r\n",
    );
    command(
        &mut stream,
        &[b"XREADGROUP", b"GROUP", b"g", b"c", b"STREAMS", b"m1", b"m2", b"$", b">"],
        b"-ERR The $ ID is meaningless in the context of XREADGROUP: you want to read the history of this consumer by specifying a proper ID, or use the > ID to get new messages. The $ ID would just return an empty result set.\r\n",
    );
    command(
        &mut stream,
        &[
            b"XREADGROUP",
            b"GROUP",
            b"g",
            b"c",
            b"STREAMS",
            b"m1",
            b"m2",
            b">",
        ],
        b"-ERR Unbalanced 'XREADGROUP' STREAMS list\r\n",
    );
    server.shutdown().unwrap();
}
