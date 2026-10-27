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

#[test]
fn blocking_xread_returns_immediately_when_data_exists() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = connect(&server);
        command(
            &mut stream,
            &[b"XADD", b"s", b"1-0", b"f", b"v1"],
            b"$3\r\n1-0\r\n",
        );
        command(
            &mut stream,
            &[b"XADD", b"s", b"2-0", b"f", b"v2"],
            b"$3\r\n2-0\r\n",
        );
        command(
            &mut stream,
            &[
                b"XREAD", b"BLOCK", b"0", b"COUNT", b"1", b"STREAMS", b"s", b"0",
            ],
            b"*1\r\n*2\r\n$1\r\ns\r\n*1\r\n*2\r\n$3\r\n1-0\r\n*2\r\n$1\r\nf\r\n$2\r\nv1\r\n",
        );
        command(
            &mut stream,
            &[b"XREAD", b"COUNT", b"1", b"STREAMS", b"s", b"0"],
            b"*1\r\n*2\r\n$1\r\ns\r\n*1\r\n*2\r\n$3\r\n1-0\r\n*2\r\n$1\r\nf\r\n$2\r\nv1\r\n",
        );
        command(
            &mut stream,
            &[b"XREAD", b"BLOCK", b"5", b"STREAMS", b"missing", b"0"],
            b"*-1\r\n",
        );
        command(&mut stream, &[b"SET", b"str", b"v"], b"+OK\r\n");
        command(
            &mut stream,
            &[b"XREAD", b"BLOCK", b"0", b"STREAMS", b"str", b"0"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );
        server.shutdown().unwrap();
    }
}

#[test]
fn blocking_xread_times_out_and_wakes_on_append_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut blocked = connect(&server);
        let mut writer = connect(&server);

        let started = Instant::now();
        command(
            &mut blocked,
            &[b"XREAD", b"BLOCK", b"50", b"STREAMS", b"missing", b"0"],
            b"*-1\r\n",
        );
        assert!(started.elapsed() >= Duration::from_millis(20));
        assert!(started.elapsed() < Duration::from_secs(3));

        send(
            &mut blocked,
            &[b"XREAD", b"BLOCK", b"0", b"STREAMS", b"wake", b"$"],
        );
        std::thread::sleep(Duration::from_millis(100));
        command(
            &mut writer,
            &[b"XADD", b"wake", b"1-0", b"f", b"v"],
            b"$3\r\n1-0\r\n",
        );
        assert_eq!(
            read_frame(&mut blocked, 0),
            b"*1\r\n*2\r\n$4\r\nwake\r\n*1\r\n*2\r\n$3\r\n1-0\r\n*2\r\n$1\r\nf\r\n$1\r\nv\r\n"
        );

        command(
            &mut writer,
            &[b"XADD", b"wake", b"2-0", b"f", b"v2"],
            b"$3\r\n2-0\r\n",
        );
        send(
            &mut blocked,
            &[b"XREAD", b"BLOCK", b"0", b"STREAMS", b"wake", b"$"],
        );
        std::thread::sleep(Duration::from_millis(100));
        command(
            &mut writer,
            &[b"XADD", b"wake", b"3-0", b"f", b"v3"],
            b"$3\r\n3-0\r\n",
        );
        assert_eq!(
            read_frame(&mut blocked, 0),
            b"*1\r\n*2\r\n$4\r\nwake\r\n*1\r\n*2\r\n$3\r\n3-0\r\n*2\r\n$1\r\nf\r\n$2\r\nv3\r\n"
        );

        send(
            &mut blocked,
            &[
                b"XREAD", b"BLOCK", b"0", b"STREAMS", b"first", b"second", b"0", b"$",
            ],
        );
        std::thread::sleep(Duration::from_millis(100));
        command(
            &mut writer,
            &[b"XADD", b"second", b"1-0", b"f", b"v"],
            b"$3\r\n1-0\r\n",
        );
        assert_eq!(
            read_frame(&mut blocked, 0),
            b"*1\r\n*2\r\n$6\r\nsecond\r\n*1\r\n*2\r\n$3\r\n1-0\r\n*2\r\n$1\r\nf\r\n$1\r\nv\r\n"
        );

        server.shutdown().unwrap();
    }
}

#[test]
fn blocking_xread_executes_non_blocking_inside_transactions_and_errors() {
    let server = start(None);
    let mut stream = connect(&server);
    command(
        &mut stream,
        &[b"XADD", b"q", b"1-0", b"f", b"v"],
        b"$3\r\n1-0\r\n",
    );

    command(&mut stream, &[b"MULTI"], b"+OK\r\n");
    command(
        &mut stream,
        &[b"XREAD", b"BLOCK", b"0", b"STREAMS", b"q", b"0"],
        b"+QUEUED\r\n",
    );
    command(
        &mut stream,
        &[b"EXEC"],
        b"*1\r\n*1\r\n*2\r\n$1\r\nq\r\n*1\r\n*2\r\n$3\r\n1-0\r\n*2\r\n$1\r\nf\r\n$1\r\nv\r\n",
    );

    let started = Instant::now();
    command(&mut stream, &[b"MULTI"], b"+OK\r\n");
    command(
        &mut stream,
        &[b"XREAD", b"BLOCK", b"0", b"STREAMS", b"missing", b"0"],
        b"+QUEUED\r\n",
    );
    command(&mut stream, &[b"EXEC"], b"*1\r\n*-1\r\n");
    assert!(started.elapsed() < Duration::from_secs(1));

    command(
        &mut stream,
        &[b"XREAD", b"BLOCK", b"x", b"STREAMS", b"q", b"0"],
        b"-ERR timeout is not an integer or out of range\r\n",
    );
    command(
        &mut stream,
        &[b"XREAD", b"BLOCK", b"-1", b"STREAMS", b"q", b"0"],
        b"-ERR timeout is negative\r\n",
    );
    command(
        &mut stream,
        &[b"XREAD", b"BLOCK", b"1.5", b"STREAMS", b"q", b"0"],
        b"-ERR timeout is not an integer or out of range\r\n",
    );
    command(
        &mut stream,
        &[
            b"XREAD",
            b"BLOCK",
            b"99999999999999999999",
            b"STREAMS",
            b"q",
            b"0",
        ],
        b"-ERR timeout is not an integer or out of range\r\n",
    );
    command(&mut stream, &[b"XREAD", b"BLOCK"], b"-ERR syntax error\r\n");
    command(
        &mut stream,
        &[b"XREAD", b"BLOCK", b"1"],
        b"-ERR syntax error\r\n",
    );
    command(
        &mut stream,
        &[b"XREAD", b"BLOCK", b"1", b"STREAMS", b"q"],
        b"-ERR Unbalanced 'XREAD' STREAMS list\r\n",
    );
    command(
        &mut stream,
        &[b"XREAD", b"BLOCK", b"1", b"STREAMS", b"q", b"bogus"],
        b"-ERR Invalid stream ID specified as stream command argument\r\n",
    );

    server.shutdown().unwrap();
}
