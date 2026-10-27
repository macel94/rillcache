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
fn sort_and_sort_ro_work_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers, Config::default().max_data_bytes);
        let mut stream = connect(&server);
        command(
            &mut stream,
            &[b"RPUSH", b"list", b"3", b"1", b"2"],
            b":3\r\n",
        );
        command(
            &mut stream,
            &[b"SORT", b"list"],
            b"*3\r\n$1\r\n1\r\n$1\r\n2\r\n$1\r\n3\r\n",
        );
        command(
            &mut stream,
            &[b"SORT", b"list", b"DESC", b"LIMIT", b"0", b"2"],
            b"*2\r\n$1\r\n3\r\n$1\r\n2\r\n",
        );
        command(
            &mut stream,
            &[b"SORT_RO", b"list", b"ASC"],
            b"*3\r\n$1\r\n1\r\n$1\r\n2\r\n$1\r\n3\r\n",
        );
        command(&mut stream, &[b"SADD", b"set", b"b", b"a", b"c"], b":3\r\n");
        command(
            &mut stream,
            &[b"SORT", b"set", b"ALPHA"],
            b"*3\r\n$1\r\na\r\n$1\r\nb\r\n$1\r\nc\r\n",
        );
        command(
            &mut stream,
            &[b"ZADD", b"zset", b"9", b"b", b"1", b"a", b"5", b"c"],
            b":3\r\n",
        );
        command(
            &mut stream,
            &[b"SORT_RO", b"zset", b"ALPHA", b"DESC"],
            b"*3\r\n$1\r\nc\r\n$1\r\nb\r\n$1\r\na\r\n",
        );
        command(&mut stream, &[b"SORT", b"missing"], b"*0\r\n");
        command(
            &mut stream,
            &[b"SORT", b"list", b"STORE", b"out"],
            b":3\r\n",
        );
        command(
            &mut stream,
            &[b"LRANGE", b"out", b"0", b"-1"],
            b"*3\r\n$1\r\n1\r\n$1\r\n2\r\n$1\r\n3\r\n",
        );
        command(&mut stream, &[b"PEXPIRE", b"out", b"100000"], b":1\r\n");
        command(
            &mut stream,
            &[b"SORT", b"list", b"STORE", b"out"],
            b":3\r\n",
        );
        let ttl = reply(&mut stream, &[b"PTTL", b"out"]);
        let ttl = std::str::from_utf8(&ttl[1..ttl.len() - 2])
            .unwrap()
            .parse::<i64>()
            .unwrap();
        assert_eq!(ttl, -1);
        command(&mut stream, &[b"RPUSH", b"self", b"2", b"1"], b":2\r\n");
        command(
            &mut stream,
            &[b"SORT", b"self", b"STORE", b"self"],
            b":2\r\n",
        );
        command(
            &mut stream,
            &[b"LRANGE", b"self", b"0", b"-1"],
            b"*2\r\n$1\r\n1\r\n$1\r\n2\r\n",
        );
        command(&mut stream, &[b"SET", b"out", b"v"], b"+OK\r\n");
        command(
            &mut stream,
            &[b"SORT", b"missing", b"STORE", b"out"],
            b":0\r\n",
        );
        command(&mut stream, &[b"EXISTS", b"out"], b":0\r\n");

        command(&mut stream, &[b"RPUSH", b"bad", b"x", b"1"], b":2\r\n");
        command(
            &mut stream,
            &[b"SORT", b"bad"],
            b"-ERR One or more scores can't be converted into double\r\n",
        );
        command(
            &mut stream,
            &[b"SORT", b"bad", b"ALPHA"],
            b"*2\r\n$1\r\n1\r\n$1\r\nx\r\n",
        );
        command(&mut stream, &[b"RPUSH", b"nums", b"2", b"1"], b":2\r\n");
        command(&mut stream, &[b"SET", b"w_1", b"10"], b"+OK\r\n");
        command(&mut stream, &[b"SET", b"w_2", b"5"], b"+OK\r\n");
        command(
            &mut stream,
            &[b"SORT", b"nums", b"BY", b"w_*"],
            b"*2\r\n$1\r\n2\r\n$1\r\n1\r\n",
        );
        command(
            &mut stream,
            &[
                b"SORT", b"nums", b"BY", b"w_*", b"GET", b"#", b"GET", b"w_*",
            ],
            b"*4\r\n$1\r\n2\r\n$1\r\n5\r\n$1\r\n1\r\n$2\r\n10\r\n",
        );
        command(
            &mut stream,
            &[b"SORT", b"nums", b"BY", b"missing_*"],
            b"*2\r\n$1\r\n1\r\n$1\r\n2\r\n",
        );
        command(&mut stream, &[b"HSET", b"h_1", b"w", b"30"], b":1\r\n");
        command(&mut stream, &[b"HSET", b"h_2", b"w", b"10"], b":1\r\n");
        command(
            &mut stream,
            &[b"SORT", b"nums", b"BY", b"h_*->w"],
            b"*2\r\n$1\r\n2\r\n$1\r\n1\r\n",
        );
        command(
            &mut stream,
            &[b"SORT", b"nums", b"BY", b"nosort"],
            b"*2\r\n$1\r\n2\r\n$1\r\n1\r\n",
        );
        command(
            &mut stream,
            &[b"SORT", b"nums", b"BY", b"nosort", b"DESC"],
            b"*2\r\n$1\r\n1\r\n$1\r\n2\r\n",
        );
        command(
            &mut stream,
            &[b"SORT", b"nums", b"BY", b"w_*", b"ALPHA"],
            b"*2\r\n$1\r\n1\r\n$1\r\n2\r\n",
        );
        command(
            &mut stream,
            &[b"SORT_RO", b"nums", b"GET", b"missing_*"],
            b"*2\r\n$-1\r\n$-1\r\n",
        );
        command(
            &mut stream,
            &[b"SORT", b"nums", b"GET", b"missing_*", b"STORE", b"getout"],
            b":2\r\n",
        );
        command(
            &mut stream,
            &[b"LRANGE", b"getout", b"0", b"-1"],
            b"*2\r\n$0\r\n\r\n$0\r\n\r\n",
        );
        command(&mut stream, &[b"SET", b"bad_w_1", b"notnum"], b"+OK\r\n");
        command(
            &mut stream,
            &[b"SORT", b"nums", b"BY", b"bad_w_*"],
            b"-ERR One or more scores can't be converted into double\r\n",
        );
        command(
            &mut stream,
            &[b"SORT_RO", b"list", b"STORE", b"out"],
            b"-ERR syntax error\r\n",
        );
        command(
            &mut stream,
            &[b"SORT", b"list", b"LIMIT", b"bad", b"1"],
            b"-ERR value is not an integer or out of range\r\n",
        );
        command(
            &mut stream,
            &[b"SORT", b"list", b"LIMIT", b"0"],
            b"-ERR syntax error\r\n",
        );
        command(
            &mut stream,
            &[b"SORT", b"list", b"BOGUS"],
            b"-ERR syntax error\r\n",
        );
        command(&mut stream, &[b"SET", b"str", b"v"], b"+OK\r\n");
        command(
            &mut stream,
            &[b"SORT", b"str"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );
        command(&mut stream, &[b"HSET", b"hash", b"f", b"v"], b":1\r\n");
        command(
            &mut stream,
            &[b"SORT", b"hash"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );

        command(&mut stream, &[b"MULTI"], b"+OK\r\n");
        command(&mut stream, &[b"SORT", b"list"], b"+QUEUED\r\n");
        command(
            &mut stream,
            &[b"EXEC"],
            b"*1\r\n*3\r\n$1\r\n1\r\n$1\r\n2\r\n$1\r\n3\r\n",
        );

        command(&mut stream, &[b"WATCH", b"out"], b"+OK\r\n");
        let mut other = connect(&server);
        command(&mut other, &[b"SORT", b"list", b"STORE", b"out"], b":3\r\n");
        command(&mut stream, &[b"MULTI"], b"+OK\r\n");
        command(&mut stream, &[b"LLEN", b"out"], b"+QUEUED\r\n");
        command(&mut stream, &[b"EXEC"], b"*-1\r\n");

        server.shutdown().unwrap();

        let small = start(workers, 4);
        let mut limited = connect(&small);
        command(&mut limited, &[b"RPUSH", b"l", b"a"], b":1\r\n");
        command(
            &mut limited,
            &[b"SORT", b"l", b"ALPHA", b"STORE", b"dest"],
            b"-OOM configured payload limit exceeded\r\n",
        );
        command(&mut limited, &[b"EXISTS", b"dest"], b":0\r\n");
        small.shutdown().unwrap();
    }
}
