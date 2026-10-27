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
fn blmpop_immediate_wake_timeout_and_errors_work_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = connect(&server);
        command(&mut stream, &[b"RPUSH", b"l", b"a", b"b", b"c"], b":3\r\n");
        command(
            &mut stream,
            &[b"BLMPOP", b"0", b"1", b"l", b"LEFT", b"COUNT", b"2"],
            b"*2\r\n$1\r\nl\r\n*2\r\n$1\r\na\r\n$1\r\nb\r\n",
        );
        command(
            &mut stream,
            &[b"LMPOP", b"1", b"l", b"RIGHT"],
            b"*2\r\n$1\r\nl\r\n*1\r\n$1\r\nc\r\n",
        );
        command(&mut stream, &[b"EXISTS", b"l"], b":0\r\n");
        command(
            &mut stream,
            &[b"BLMPOP", b"0.05", b"1", b"missing", b"LEFT"],
            b"*-1\r\n",
        );

        let mut writer = connect(&server);
        send(
            &mut stream,
            &[b"BLMPOP", b"0", b"2", b"missing", b"ready", b"LEFT"],
        );
        std::thread::sleep(Duration::from_millis(80));
        command(&mut writer, &[b"RPUSH", b"ready", b"x"], b":1\r\n");
        assert_eq!(
            read_frame(&mut stream, 0),
            b"*2\r\n$5\r\nready\r\n*1\r\n$1\r\nx\r\n"
        );

        command(&mut stream, &[b"RPUSH", b"pt", b"a", b"b"], b":2\r\n");
        command(&mut stream, &[b"PEXPIRE", b"pt", b"100000"], b":1\r\n");
        command(
            &mut stream,
            &[b"BLMPOP", b"0.1", b"1", b"pt", b"LEFT"],
            b"*2\r\n$2\r\npt\r\n*1\r\n$1\r\na\r\n",
        );
        let ttl = reply(&mut stream, &[b"PTTL", b"pt"]);
        let ttl = std::str::from_utf8(&ttl[1..ttl.len() - 2])
            .unwrap()
            .parse::<i64>()
            .unwrap();
        assert!(ttl > 0);

        command(
            &mut stream,
            &[b"BLMPOP", b"-1", b"1", b"pt", b"LEFT"],
            b"-ERR timeout is negative\r\n",
        );
        command(
            &mut stream,
            &[b"BLMPOP", b"bad", b"1", b"pt", b"LEFT"],
            b"-ERR timeout is not a float or out of range\r\n",
        );
        command(
            &mut stream,
            &[b"BLMPOP", b"0", b"0", b"pt", b"LEFT"],
            b"-ERR numkeys should be greater than 0\r\n",
        );
        command(
            &mut stream,
            &[b"BLMPOP", b"0", b"1", b"pt", b"BAD"],
            b"-ERR syntax error\r\n",
        );
        command(
            &mut stream,
            &[b"BLMPOP", b"0", b"1", b"pt", b"LEFT", b"COUNT", b"0"],
            b"-ERR count should be greater than 0\r\n",
        );
        command(
            &mut stream,
            &[b"BLMPOP", b"0", b"1", b"pt", b"LEFT", b"BOGUS"],
            b"-ERR syntax error\r\n",
        );
        command(&mut stream, &[b"SET", b"wrong", b"v"], b"+OK\r\n");
        command(
            &mut stream,
            &[b"BLMPOP", b"0", b"2", b"wrong", b"missing", b"LEFT"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );

        command(&mut stream, &[b"RPUSH", b"multi", b"a"], b":1\r\n");
        command(&mut stream, &[b"MULTI"], b"+OK\r\n");
        command(
            &mut stream,
            &[b"BLMPOP", b"0", b"1", b"multi", b"LEFT"],
            b"+QUEUED\r\n",
        );
        command(
            &mut stream,
            &[b"EXEC"],
            b"*1\r\n*2\r\n$5\r\nmulti\r\n*1\r\n$1\r\na\r\n",
        );

        command(&mut stream, &[b"RPUSH", b"w", b"a"], b":1\r\n");
        command(&mut stream, &[b"WATCH", b"w"], b"+OK\r\n");
        let mut other = connect(&server);
        command(
            &mut other,
            &[b"BLMPOP", b"0", b"1", b"w", b"LEFT"],
            b"*2\r\n$1\r\nw\r\n*1\r\n$1\r\na\r\n",
        );
        command(&mut stream, &[b"MULTI"], b"+OK\r\n");
        command(&mut stream, &[b"LLEN", b"w"], b"+QUEUED\r\n");
        command(&mut stream, &[b"EXEC"], b"*-1\r\n");

        server.shutdown().unwrap();
    }
}
