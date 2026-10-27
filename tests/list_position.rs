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

#[test]
fn list_position_and_multipop_work_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = connect(&server);
        command(
            &mut stream,
            &[b"RPUSH", b"l", b"a", b"b", b"a", b"c", b"a"],
            b":5\r\n",
        );
        command(&mut stream, &[b"LPOS", b"l", b"a"], b":0\r\n");
        command(
            &mut stream,
            &[b"LPOS", b"l", b"a", b"RANK", b"2"],
            b":2\r\n",
        );
        command(
            &mut stream,
            &[b"LPOS", b"l", b"a", b"RANK", b"3"],
            b":4\r\n",
        );
        command(
            &mut stream,
            &[b"LPOS", b"l", b"a", b"RANK", b"4"],
            b"$-1\r\n",
        );
        command(
            &mut stream,
            &[b"LPOS", b"l", b"a", b"RANK", b"-1"],
            b":4\r\n",
        );
        command(
            &mut stream,
            &[b"LPOS", b"l", b"a", b"RANK", b"-2"],
            b":2\r\n",
        );
        command(
            &mut stream,
            &[b"LPOS", b"l", b"a", b"COUNT", b"0"],
            b"*3\r\n:0\r\n:2\r\n:4\r\n",
        );
        command(
            &mut stream,
            &[b"LPOS", b"l", b"a", b"COUNT", b"2"],
            b"*2\r\n:0\r\n:2\r\n",
        );
        command(
            &mut stream,
            &[b"LPOS", b"l", b"a", b"RANK", b"2", b"COUNT", b"0"],
            b"*2\r\n:2\r\n:4\r\n",
        );
        command(
            &mut stream,
            &[b"LPOS", b"l", b"a", b"RANK", b"-1", b"COUNT", b"1"],
            b"*1\r\n:4\r\n",
        );
        command(
            &mut stream,
            &[b"LPOS", b"l", b"a", b"MAXLEN", b"3", b"COUNT", b"0"],
            b"*2\r\n:0\r\n:2\r\n",
        );
        command(&mut stream, &[b"LPOS", b"l", b"missing"], b"$-1\r\n");
        command(
            &mut stream,
            &[b"LPOS", b"l", b"missing", b"COUNT", b"0"],
            b"*0\r\n",
        );
        command(&mut stream, &[b"LPOS", b"missing", b"a"], b"$-1\r\n");
        command(
            &mut stream,
            &[b"LPOS", b"missing", b"a", b"COUNT", b"0"],
            b"*0\r\n",
        );
        command(
            &mut stream,
            &[b"LPOS", b"l", b"a", b"RANK", b"0"],
            b"-ERR RANK can't be zero: use 1 to start from the first match, 2 from the second ... or use negative to start from the end of the list\r\n",
        );
        command(
            &mut stream,
            &[b"LPOS", b"l", b"a", b"COUNT", b"-1"],
            b"-ERR COUNT can't be negative\r\n",
        );
        command(
            &mut stream,
            &[b"LPOS", b"l", b"a", b"MAXLEN", b"-1"],
            b"-ERR MAXLEN can't be negative\r\n",
        );
        command(
            &mut stream,
            &[b"LPOS", b"l", b"a", b"BOGUS", b"1"],
            b"-ERR syntax error\r\n",
        );
        command(&mut stream, &[b"SET", b"str", b"v"], b"+OK\r\n");
        command(
            &mut stream,
            &[b"LPOS", b"str", b"a"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );

        command(&mut stream, &[b"RPUSH", b"l1", b"a", b"b", b"c"], b":3\r\n");
        command(&mut stream, &[b"RPUSH", b"l2", b"d", b"e"], b":2\r\n");
        command(
            &mut stream,
            &[b"LMPOP", b"2", b"missing", b"l1", b"LEFT"],
            b"*2\r\n$2\r\nl1\r\n*1\r\n$1\r\na\r\n",
        );
        command(
            &mut stream,
            &[b"LMPOP", b"2", b"l1", b"l2", b"LEFT", b"COUNT", b"2"],
            b"*2\r\n$2\r\nl1\r\n*2\r\n$1\r\nb\r\n$1\r\nc\r\n",
        );
        command(&mut stream, &[b"EXISTS", b"l1"], b":0\r\n");
        command(
            &mut stream,
            &[b"LMPOP", b"2", b"l1", b"l2", b"RIGHT", b"COUNT", b"5"],
            b"*2\r\n$2\r\nl2\r\n*2\r\n$1\r\ne\r\n$1\r\nd\r\n",
        );
        command(&mut stream, &[b"EXISTS", b"l2"], b":0\r\n");
        command(
            &mut stream,
            &[b"LMPOP", b"2", b"l1", b"l2", b"LEFT"],
            b"*-1\r\n",
        );
        command(&mut stream, &[b"RPUSH", b"pt", b"a", b"b"], b":2\r\n");
        command(&mut stream, &[b"PEXPIRE", b"pt", b"100000"], b":1\r\n");
        command(
            &mut stream,
            &[b"LMPOP", b"1", b"pt", b"LEFT"],
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
            &[b"LMPOP", b"0", b"pt", b"LEFT"],
            b"-ERR numkeys should be greater than 0\r\n",
        );
        command(
            &mut stream,
            &[b"LMPOP", b"1", b"pt", b"BAD"],
            b"-ERR syntax error\r\n",
        );
        command(
            &mut stream,
            &[b"LMPOP", b"1", b"pt", b"LEFT", b"COUNT", b"0"],
            b"-ERR count should be greater than 0\r\n",
        );
        command(
            &mut stream,
            &[b"LMPOP", b"1", b"pt", b"LEFT", b"BOGUS"],
            b"-ERR syntax error\r\n",
        );
        command(&mut stream, &[b"RPUSH", b"list", b"x"], b":1\r\n");
        command(&mut stream, &[b"SET", b"wrong", b"v"], b"+OK\r\n");
        command(
            &mut stream,
            &[b"LMPOP", b"2", b"wrong", b"list", b"LEFT"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );
        command(
            &mut stream,
            &[b"LMPOP", b"2", b"list", b"wrong", b"LEFT"],
            b"*2\r\n$4\r\nlist\r\n*1\r\n$1\r\nx\r\n",
        );

        command(&mut stream, &[b"RPUSH", b"w", b"a"], b":1\r\n");
        command(&mut stream, &[b"WATCH", b"w"], b"+OK\r\n");
        let mut other = connect(&server);
        command(
            &mut other,
            &[b"LMPOP", b"1", b"w", b"LEFT"],
            b"*2\r\n$1\r\nw\r\n*1\r\n$1\r\na\r\n",
        );
        command(&mut stream, &[b"MULTI"], b"+OK\r\n");
        command(&mut stream, &[b"LLEN", b"w"], b"+QUEUED\r\n");
        command(&mut stream, &[b"EXEC"], b"*-1\r\n");

        command(&mut stream, &[b"MULTI"], b"+OK\r\n");
        command(&mut stream, &[b"LPOS", b"l", b"a"], b"+QUEUED\r\n");
        command(&mut stream, &[b"EXEC"], b"*1\r\n:0\r\n");

        server.shutdown().unwrap();
    }
}
