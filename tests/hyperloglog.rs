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
fn hyperloglog_commands_work_over_tcp_in_all_execution_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = connect(&server);

        command(&mut stream, &[b"PFADD", b"hll"], b":1\r\n");
        command(&mut stream, &[b"TYPE", b"hll"], b"+string\r\n");
        command(&mut stream, &[b"PFCOUNT", b"hll"], b":0\r\n");
        command(
            &mut stream,
            &[b"PFADD", b"hll", b"a", b"b", b"c", b""],
            b":1\r\n",
        );
        command(&mut stream, &[b"PFADD", b"hll", b"a", b"b"], b":0\r\n");
        command(&mut stream, &[b"PFCOUNT", b"hll"], b":4\r\n");
        command(&mut stream, &[b"PFADD", b"other", b"c", b"d"], b":1\r\n");
        command(
            &mut stream,
            &[b"PFCOUNT", b"hll", b"other", b"missing"],
            b":5\r\n",
        );
        command(
            &mut stream,
            &[b"PFMERGE", b"union", b"hll", b"other"],
            b"+OK\r\n",
        );
        command(&mut stream, &[b"PFCOUNT", b"union"], b":5\r\n");
        command(&mut stream, &[b"PEXPIRE", b"union", b"100000"], b":1\r\n");
        command(&mut stream, &[b"PFMERGE", b"union"], b"+OK\r\n");
        command(&mut stream, &[b"PFCOUNT", b"union"], b":5\r\n");
        let mut request = Vec::new();
        encode_request(&[b"PTTL", b"union"], &mut request);
        stream.write_all(&request).unwrap();
        let ttl = read_frame(&mut stream, 0);
        assert_ne!(ttl, b":-2\r\n");

        command(&mut stream, &[b"SET", b"bad", b"not-hll"], b"+OK\r\n");
        command(
            &mut stream,
            &[b"PFADD", b"bad", b"x"],
            b"-WRONGTYPE Key is not a valid HyperLogLog string value.\r\n",
        );
        command(
            &mut stream,
            &[b"PFCOUNT", b"bad"],
            b"-WRONGTYPE Key is not a valid HyperLogLog string value.\r\n",
        );
        command(
            &mut stream,
            &[b"PFMERGE", b"bad", b"hll"],
            b"-WRONGTYPE Key is not a valid HyperLogLog string value.\r\n",
        );
        command(&mut stream, &[b"RPUSH", b"list", b"x"], b":1\r\n");
        command(
            &mut stream,
            &[b"PFADD", b"list", b"x"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );
        command(
            &mut stream,
            &[b"PFCOUNT", b"list"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );
        command(
            &mut stream,
            &[b"PFMERGE", b"out", b"list"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );

        command(&mut stream, &[b"MULTI"], b"+OK\r\n");
        command(&mut stream, &[b"PFADD", b"tx", b"x"], b"+QUEUED\r\n");
        command(&mut stream, &[b"PFCOUNT", b"tx"], b"+QUEUED\r\n");
        command(&mut stream, &[b"EXEC"], b"*2\r\n:1\r\n:1\r\n");

        server.shutdown().unwrap();
    }
}
#[test]
fn dense_hyperloglog_promotion_keeps_a_bounded_estimate() {
    let server = start(None);
    let mut stream = connect(&server);
    for index in 0..5000 {
        let value = index.to_string();
        let mut request = Vec::new();
        encode_request(&[b"PFADD", b"dense", value.as_bytes()], &mut request);
        stream.write_all(&request).unwrap();
        let reply = read_frame(&mut stream, 0);
        assert!(reply.starts_with(b":"), "{reply:?}");
    }
    let mut request = Vec::new();
    encode_request(&[b"PFCOUNT", b"dense"], &mut request);
    stream.write_all(&request).unwrap();
    let reply = read_frame(&mut stream, 0);
    let count: u64 = std::str::from_utf8(&reply[1..reply.len() - 2])
        .unwrap()
        .parse()
        .unwrap();
    assert!((4500..=5500).contains(&count), "dense count {count}");
    server.shutdown().unwrap();
}
