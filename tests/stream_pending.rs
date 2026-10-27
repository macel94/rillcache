#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

fn start() -> ServerHandle {
    let workers = match std::env::var("RILLCACHE_TEST_THREADS").as_deref() {
        Ok("0") | Err(_) => None,
        Ok("1") => Some(1),
        Ok("4") => Some(4),
        Ok(value) => panic!("invalid test worker mode {value}"),
    };
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
    let mut byte = [0];
    loop {
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

fn read_frame(stream: &mut TcpStream) -> Vec<u8> {
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
                output.extend_from_slice(&body);
            }
        }
        b'*' => {
            let count = read_line(stream, &mut output).parse::<i64>().unwrap();
            for _ in 0..count.max(0) {
                output.extend_from_slice(&read_frame(stream));
            }
        }
        other => panic!("unexpected prefix {other:?}"),
    }
    output
}

fn send(stream: &mut TcpStream, args: &[&[u8]]) -> Vec<u8> {
    let mut frame = Vec::new();
    encode_request(args, &mut frame);
    stream.write_all(&frame).unwrap();
    read_frame(stream)
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[test]
fn pending_ack_range_and_read_cover_consumer_group_lifecycle() {
    let server = start();
    let mut stream = connect(&server);
    for (id, value) in [
        (b"1".as_slice(), b"a".as_slice()),
        (b"2", b"b"),
        (b"3", b"c"),
    ] {
        assert!(send(&mut stream, &[b"XADD", b"s", id, b"f", value]).starts_with(b"$"));
    }
    assert_eq!(
        send(&mut stream, &[b"XGROUP", b"CREATE", b"s", b"g", b"0"]),
        b"+OK\r\n"
    );
    let read = send(
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
    assert!(contains(&read, b"1-0") && contains(&read, b"2-0"));

    assert_eq!(
        send(&mut stream, &[b"XPENDING", b"s", b"g"]),
        b"*4\r\n:2\r\n$3\r\n1-0\r\n$3\r\n2-0\r\n*1\r\n*2\r\n$5\r\nalice\r\n:2\r\n"
    );
    let pending = send(&mut stream, &[b"XPENDING", b"s", b"g", b"-", b"+", b"10"]);
    assert!(pending.starts_with(b"*2\r\n"));
    assert!(contains(&pending, b"alice"));
    assert_eq!(
        send(
            &mut stream,
            &[
                b"XPENDING",
                b"s",
                b"g",
                b"IDLE",
                b"100000",
                b"-",
                b"+",
                b"10"
            ]
        ),
        b"*0\r\n"
    );
    assert_eq!(send(&mut stream, &[b"XACK", b"s", b"g", b"1-0"]), b":1\r\n");
    assert_eq!(
        send(&mut stream, &[b"XPENDING", b"s", b"g"]),
        b"*4\r\n:1\r\n$3\r\n2-0\r\n$3\r\n2-0\r\n*1\r\n*2\r\n$5\r\nalice\r\n:1\r\n"
    );
    assert_eq!(send(&mut stream, &[b"XACK", b"s", b"g", b"9-0"]), b":0\r\n");

    let range = send(&mut stream, &[b"XRANGE", b"s", b"-", b"+"]);
    assert!(range.starts_with(b"*3\r\n"));
    let limited = send(&mut stream, &[b"XRANGE", b"s", b"-", b"+", b"COUNT", b"2"]);
    assert!(limited.starts_with(b"*2\r\n"));
    let reverse = send(
        &mut stream,
        &[b"XREVRANGE", b"s", b"+", b"-", b"COUNT", b"2"],
    );
    assert!(reverse.starts_with(b"*2\r\n"));
    assert!(contains(&reverse, b"3-0"));
    let pos3 = reverse
        .windows(3)
        .position(|window| window == b"3-0")
        .unwrap();
    let pos2 = reverse
        .windows(3)
        .position(|window| window == b"2-0")
        .unwrap();
    assert!(pos3 < pos2);

    let read = send(
        &mut stream,
        &[b"XREAD", b"COUNT", b"10", b"STREAMS", b"s", b"1-0"],
    );
    assert!(contains(&read, b"2-0") && contains(&read, b"3-0"));
    assert_eq!(
        send(&mut stream, &[b"XREAD", b"STREAMS", b"s", b"$"]),
        b"*-1\r\n"
    );
    assert!(send(&mut stream, &[b"XADD", b"s", b"4", b"f", b"d"]).starts_with(b"$"));
    assert_eq!(
        send(&mut stream, &[b"XREAD", b"STREAMS", b"s", b"$"]),
        b"*-1\r\n"
    );
    let read = send(&mut stream, &[b"XREAD", b"STREAMS", b"s", b"3-0"]);
    assert!(contains(&read, b"4-0"));
    assert_eq!(
        send(
            &mut stream,
            &[b"XREAD", b"BLOCK", b"5", b"STREAMS", b"s", b"$"]
        ),
        b"*-1\r\n"
    );

    assert_eq!(
        send(
            &mut stream,
            &[b"XGROUP", b"DELCONSUMER", b"s", b"g", b"alice"]
        ),
        b":1\r\n"
    );
    assert_eq!(
        send(
            &mut stream,
            &[b"XGROUP", b"CREATECONSUMER", b"s", b"g", b"bob"]
        ),
        b":1\r\n"
    );
    assert_eq!(
        send(
            &mut stream,
            &[b"XGROUP", b"CREATECONSUMER", b"s", b"g", b"bob"]
        ),
        b":0\r\n"
    );
    assert_eq!(
        send(&mut stream, &[b"XGROUP", b"DESTROY", b"s", b"g"]),
        b":1\r\n"
    );
    assert_eq!(
        send(&mut stream, &[b"XGROUP", b"DESTROY", b"s", b"g"]),
        b":0\r\n"
    );
    assert_eq!(
        send(&mut stream, &[b"XPENDING", b"s", b"g"]),
        b"-NOGROUP No such consumer group\r\n"
    );
    assert_eq!(
        send(&mut stream, &[b"XRANGE", b"s", b"bad", b"+"]),
        b"-ERR Invalid stream ID specified as stream command argument\r\n"
    );
    assert_eq!(
        send(&mut stream, &[b"XPENDING", b"missing", b"g"]),
        b"-NOGROUP No such key or consumer group\r\n"
    );
    assert_eq!(send(&mut stream, &[b"SET", b"string", b"x"]), b"+OK\r\n");
    assert_eq!(
        send(&mut stream, &[b"XRANGE", b"string", b"-", b"+"]),
        b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n"
    );
    server.shutdown().unwrap();
}
