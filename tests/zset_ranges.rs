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

fn seed(stream: &mut TcpStream, key: &[u8]) {
    command(
        stream,
        &[b"ZADD", key, b"1", b"a", b"2", b"b", b"3", b"c", b"4", b"d"],
        b":4\r\n",
    );
}

fn seed_lex(stream: &mut TcpStream, key: &[u8]) {
    command(
        stream,
        &[b"ZADD", key, b"0", b"a", b"0", b"b", b"0", b"c", b"0", b"d"],
        b":4\r\n",
    );
}

#[test]
fn sorted_set_ranges_selection_and_store_work_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers, Config::default().max_data_bytes);
        let mut stream = connect(&server);
        seed(&mut stream, b"z");
        seed_lex(&mut stream, b"lex");

        command(
            &mut stream,
            &[b"ZRANGE", b"z", b"0", b"-1"],
            b"*4\r\n$1\r\na\r\n$1\r\nb\r\n$1\r\nc\r\n$1\r\nd\r\n",
        );
        command(
            &mut stream,
            &[b"ZRANGE", b"z", b"1", b"2"],
            b"*2\r\n$1\r\nb\r\n$1\r\nc\r\n",
        );
        command(
            &mut stream,
            &[b"ZRANGE", b"z", b"0", b"-1", b"REV"],
            b"*4\r\n$1\r\nd\r\n$1\r\nc\r\n$1\r\nb\r\n$1\r\na\r\n",
        );
        command(
            &mut stream,
            &[b"ZRANGE", b"z", b"2", b"4", b"BYSCORE"],
            b"*3\r\n$1\r\nb\r\n$1\r\nc\r\n$1\r\nd\r\n",
        );
        command(
            &mut stream,
            &[b"ZRANGE", b"z", b"4", b"2", b"BYSCORE", b"REV"],
            b"*3\r\n$1\r\nd\r\n$1\r\nc\r\n$1\r\nb\r\n",
        );
        command(
            &mut stream,
            &[
                b"ZRANGE", b"z", b"2", b"4", b"BYSCORE", b"LIMIT", b"1", b"2",
            ],
            b"*2\r\n$1\r\nc\r\n$1\r\nd\r\n",
        );
        command(
            &mut stream,
            &[b"ZRANGE", b"z", b"2", b"2", b"BYSCORE", b"WITHSCORES"],
            b"*2\r\n$1\r\nb\r\n$1\r\n2\r\n",
        );
        command(
            &mut stream,
            &[b"ZRANGE", b"lex", b"[b", b"[c", b"BYLEX"],
            b"*2\r\n$1\r\nb\r\n$1\r\nc\r\n",
        );
        command(
            &mut stream,
            &[b"ZRANGE", b"lex", b"[c", b"[b", b"BYLEX", b"REV"],
            b"*2\r\n$1\r\nc\r\n$1\r\nb\r\n",
        );
        command(
            &mut stream,
            &[
                b"ZRANGE", b"lex", b"(b", b"+", b"BYLEX", b"LIMIT", b"0", b"2",
            ],
            b"*2\r\n$1\r\nc\r\n$1\r\nd\r\n",
        );

        command(
            &mut stream,
            &[b"ZLEXCOUNT", b"lex", b"[b", b"[c"],
            b":2\r\n",
        );
        command(&mut stream, &[b"ZLEXCOUNT", b"lex", b"-", b"+"], b":4\r\n");
        command(
            &mut stream,
            &[b"ZLEXCOUNT", b"lex", b"(b", b"[c"],
            b":1\r\n",
        );

        command(
            &mut stream,
            &[b"ZRANGESTORE", b"out", b"z", b"0", b"-1"],
            b":4\r\n",
        );
        command(&mut stream, &[b"TYPE", b"out"], b"+zset\r\n");
        command(
            &mut stream,
            &[b"ZRANGE", b"out", b"0", b"-1", b"WITHSCORES"],
            b"*8\r\n$1\r\na\r\n$1\r\n1\r\n$1\r\nb\r\n$1\r\n2\r\n$1\r\nc\r\n$1\r\n3\r\n$1\r\nd\r\n$1\r\n4\r\n",
        );
        command(
            &mut stream,
            &[
                b"ZRANGESTORE",
                b"out",
                b"z",
                b"2",
                b"4",
                b"BYSCORE",
                b"LIMIT",
                b"1",
                b"1",
            ],
            b":1\r\n",
        );
        command(
            &mut stream,
            &[b"ZRANGE", b"out", b"0", b"-1"],
            b"*1\r\n$1\r\nc\r\n",
        );
        command(&mut stream, &[b"SET", b"out", b"v"], b"+OK\r\n");
        command(
            &mut stream,
            &[b"ZRANGESTORE", b"out", b"z", b"100", b"200", b"BYSCORE"],
            b":0\r\n",
        );
        command(&mut stream, &[b"EXISTS", b"out"], b":0\r\n");
        command(
            &mut stream,
            &[b"ZRANGESTORE", b"out", b"z", b"0", b"-1"],
            b":4\r\n",
        );
        command(&mut stream, &[b"PEXPIRE", b"out", b"100000"], b":1\r\n");
        command(
            &mut stream,
            &[b"ZRANGESTORE", b"out", b"z", b"0", b"1"],
            b":2\r\n",
        );
        let ttl = reply(&mut stream, &[b"PTTL", b"out"]);
        let ttl = std::str::from_utf8(&ttl[1..ttl.len() - 2])
            .unwrap()
            .parse::<i64>()
            .unwrap();
        assert_eq!(ttl, -1);
        command(
            &mut stream,
            &[b"ZRANGESTORE", b"self", b"self", b"0", b"-1"],
            b":0\r\n",
        );
        seed(&mut stream, b"self");
        command(
            &mut stream,
            &[b"ZRANGESTORE", b"self", b"self", b"0", b"1"],
            b":2\r\n",
        );
        command(&mut stream, &[b"ZCARD", b"self"], b":2\r\n");

        command(
            &mut stream,
            &[b"ZREMRANGEBYLEX", b"lex", b"[b", b"[c"],
            b":2\r\n",
        );
        command(
            &mut stream,
            &[b"ZRANGE", b"lex", b"0", b"-1"],
            b"*2\r\n$1\r\na\r\n$1\r\nd\r\n",
        );
        command(&mut stream, &[b"PEXPIRE", b"lex", b"100000"], b":1\r\n");
        command(
            &mut stream,
            &[b"ZREMRANGEBYLEX", b"lex", b"[a", b"[a"],
            b":1\r\n",
        );
        let ttl = reply(&mut stream, &[b"PTTL", b"lex"]);
        let ttl = std::str::from_utf8(&ttl[1..ttl.len() - 2])
            .unwrap()
            .parse::<i64>()
            .unwrap();
        assert!(ttl > 0);

        let random = reply(&mut stream, &[b"ZRANDMEMBER", b"z"]);
        assert!(random.starts_with(b"$1\r\n"));
        let random = reply(&mut stream, &[b"ZRANDMEMBER", b"z", b"10"]);
        assert!(random.starts_with(b"*4\r\n"));
        let random = reply(&mut stream, &[b"ZRANDMEMBER", b"z", b"10", b"WITHSCORES"]);
        assert!(random.starts_with(b"*8\r\n"));
        let random = reply(&mut stream, &[b"ZRANDMEMBER", b"z", b"2"]);
        assert!(random.starts_with(b"*2\r\n"));
        let random = reply(&mut stream, &[b"ZRANDMEMBER", b"z", b"-6"]);
        assert!(random.starts_with(b"*6\r\n"));
        command(&mut stream, &[b"ZRANDMEMBER", b"z", b"0"], b"*0\r\n");
        command(&mut stream, &[b"ZRANDMEMBER", b"missing"], b"$-1\r\n");
        command(&mut stream, &[b"ZRANDMEMBER", b"missing", b"2"], b"*0\r\n");

        command(
            &mut stream,
            &[b"ZRANGE", b"z", b"0", b"-1", b"LIMIT", b"0", b"1"],
            b"-ERR syntax error, LIMIT is only supported in combination with either BYSCORE or BYLEX\r\n",
        );
        command(
            &mut stream,
            &[b"ZRANGE", b"lex", b"-", b"+", b"BYLEX", b"WITHSCORES"],
            b"-ERR syntax error, WITHSCORES not supported in combination with BYLEX\r\n",
        );
        command(
            &mut stream,
            &[b"ZRANGE", b"z", b"0", b"-1", b"BYSCORE", b"BYLEX"],
            b"-ERR syntax error\r\n",
        );
        command(
            &mut stream,
            &[b"ZRANGE", b"z", b"bad", b"1", b"BYSCORE"],
            b"-ERR min or max is not a float\r\n",
        );
        command(
            &mut stream,
            &[b"ZRANGE", b"lex", b"b", b"[c", b"BYLEX"],
            b"-ERR min or max not valid string range item\r\n",
        );
        command(
            &mut stream,
            &[b"ZLEXCOUNT", b"lex", b"b", b"+", b"BYLEX"],
            b"-ERR wrong number of arguments for 'zlexcount' command\r\n",
        );
        command(
            &mut stream,
            &[b"ZRANDMEMBER", b"z", b"bad"],
            b"-ERR value is not an integer or out of range\r\n",
        );
        command(
            &mut stream,
            &[b"ZRANDMEMBER", b"z", b"2", b"BOGUS"],
            b"-ERR syntax error\r\n",
        );

        command(&mut stream, &[b"RPUSH", b"list", b"x"], b":1\r\n");
        command(
            &mut stream,
            &[b"ZRANGE", b"list", b"0", b"-1"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );
        command(
            &mut stream,
            &[b"ZLEXCOUNT", b"list", b"-", b"+"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );
        command(
            &mut stream,
            &[b"ZREMRANGEBYLEX", b"list", b"-", b"+"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );
        command(
            &mut stream,
            &[b"ZRANDMEMBER", b"list"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );
        command(&mut stream, &[b"SET", b"dest", b"v"], b"+OK\r\n");
        command(
            &mut stream,
            &[b"ZRANGESTORE", b"dest", b"list", b"0", b"-1"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );
        command(&mut stream, &[b"GET", b"dest"], b"$1\r\nv\r\n");

        command(&mut stream, &[b"WATCH", b"lex"], b"+OK\r\n");
        let mut other = connect(&server);
        command(
            &mut other,
            &[b"ZREMRANGEBYLEX", b"lex", b"-", b"+"],
            b":1\r\n",
        );
        command(&mut stream, &[b"MULTI"], b"+OK\r\n");
        command(&mut stream, &[b"ZCARD", b"lex"], b"+QUEUED\r\n");
        command(&mut stream, &[b"EXEC"], b"*-1\r\n");
        command(&mut stream, &[b"EXISTS", b"lex"], b":0\r\n");

        server.shutdown().unwrap();

        let small = start(workers, 4);
        let mut limited = connect(&small);
        command(&mut limited, &[b"ZADD", b"s", b"0", b"a"], b":1\r\n");
        command(
            &mut limited,
            &[b"ZRANGESTORE", b"dest", b"s", b"0", b"-1"],
            b"-OOM configured payload limit exceeded\r\n",
        );
        command(&mut limited, &[b"EXISTS", b"dest"], b":0\r\n");
        small.shutdown().unwrap();
    }
}
