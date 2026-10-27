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
            read_line_value(stream, &mut output);
        }
        b'$' => {
            let length = read_line_value(stream, &mut output).parse::<i64>().unwrap();
            if length >= 0 {
                let mut body = vec![0; length as usize + 2];
                stream.read_exact(&mut body).unwrap();
                assert!(body.ends_with(b"\r\n"));
                output.extend_from_slice(&body);
            }
        }
        b'*' => {
            let count = read_line_value(stream, &mut output).parse::<i64>().unwrap();
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

fn read_line_value(stream: &mut TcpStream, output: &mut Vec<u8>) -> String {
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
fn collection_mutations_work_over_tcp_in_all_execution_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = connect(&server);

        command(&mut stream, &[b"HSET", b"h", b"f", b"1"], b":1\r\n");
        command(&mut stream, &[b"HSETNX", b"h", b"f", b"2"], b":0\r\n");
        command(&mut stream, &[b"HSETNX", b"h", b"g", b"2"], b":1\r\n");
        command(&mut stream, &[b"HINCRBY", b"h", b"f", b"4"], b":5\r\n");
        command(
            &mut stream,
            &[b"HINCRBYFLOAT", b"h", b"f", b"0.5"],
            b"$3\r\n5.5\r\n",
        );
        command(
            &mut stream,
            &[b"HMGET", b"h", b"f", b"missing"],
            b"*2\r\n$3\r\n5.5\r\n$-1\r\n",
        );
        command(&mut stream, &[b"HEXISTS", b"h", b"g"], b":1\r\n");
        command(&mut stream, &[b"HSTRLEN", b"h", b"f"], b":3\r\n");
        command(&mut stream, &[b"HDEL", b"h", b"g"], b":1\r\n");
        command(&mut stream, &[b"HLEN", b"h"], b":1\r\n");

        command(&mut stream, &[b"RPUSH", b"l", b"a", b"b", b"a"], b":3\r\n");
        command(&mut stream, &[b"LINDEX", b"l", b"-1"], b"$1\r\na\r\n");
        command(
            &mut stream,
            &[b"LINSERT", b"l", b"BEFORE", b"b", b"x"],
            b":4\r\n",
        );
        command(&mut stream, &[b"LSET", b"l", b"0", b"z"], b"+OK\r\n");
        command(&mut stream, &[b"LREM", b"l", b"1", b"a"], b":1\r\n");
        command(&mut stream, &[b"LTRIM", b"l", b"1", b"-1"], b"+OK\r\n");
        command(
            &mut stream,
            &[b"LRANGE", b"l", b"0", b"-1"],
            b"*2\r\n$1\r\nx\r\n$1\r\nb\r\n",
        );
        command(&mut stream, &[b"LPUSHX", b"missing", b"x"], b":0\r\n");
        command(&mut stream, &[b"RPUSHX", b"l", b"tail"], b":3\r\n");

        command(&mut stream, &[b"SADD", b"s1", b"a", b"b", b"c"], b":3\r\n");
        command(&mut stream, &[b"SADD", b"s2", b"b", b"c", b"d"], b":3\r\n");
        command(
            &mut stream,
            &[b"SMISMEMBER", b"s1", b"a", b"z"],
            b"*2\r\n:1\r\n:0\r\n",
        );
        command(
            &mut stream,
            &[b"SINTER", b"s1", b"s2"],
            b"*2\r\n$1\r\nb\r\n$1\r\nc\r\n",
        );
        command(
            &mut stream,
            &[b"SUNION", b"s1", b"s2"],
            b"*4\r\n$1\r\na\r\n$1\r\nb\r\n$1\r\nc\r\n$1\r\nd\r\n",
        );
        command(&mut stream, &[b"SDIFF", b"s1", b"s2"], b"*1\r\n$1\r\na\r\n");
        command(&mut stream, &[b"SINTERCARD", b"2", b"s1", b"s2"], b":2\r\n");
        command(&mut stream, &[b"SMOVE", b"s1", b"s2", b"a"], b":1\r\n");
        command(&mut stream, &[b"SREM", b"s2", b"d"], b":1\r\n");
        command(
            &mut stream,
            &[b"SINTERSTORE", b"si", b"s1", b"s2"],
            b":2\r\n",
        );
        command(
            &mut stream,
            &[b"SUNIONSTORE", b"su", b"s1", b"s2"],
            b":3\r\n",
        );
        command(
            &mut stream,
            &[b"SDIFFSTORE", b"sd", b"s1", b"s2"],
            b":0\r\n",
        );

        command(
            &mut stream,
            &[b"ZADD", b"z", b"1", b"a", b"2", b"b"],
            b":2\r\n",
        );
        command(&mut stream, &[b"ZADD", b"z", b"NX", b"3", b"a"], b":0\r\n");
        command(
            &mut stream,
            &[b"ZADD", b"z", b"XX", b"CH", b"3", b"a"],
            b":1\r\n",
        );
        command(
            &mut stream,
            &[b"ZADD", b"z", b"INCR", b"2", b"a"],
            b"$1\r\n5\r\n",
        );
        command(
            &mut stream,
            &[b"ZINCRBY", b"z", b"0.5", b"c"],
            b"$3\r\n0.5\r\n",
        );
        command(&mut stream, &[b"ZRANK", b"z", b"c"], b":0\r\n");
        command(&mut stream, &[b"ZREVRANK", b"z", b"a"], b":0\r\n");
        command(&mut stream, &[b"ZCOUNT", b"z", b"(0.5", b"+inf"], b":2\r\n");
        command(
            &mut stream,
            &[b"ZRANGEBYSCORE", b"z", b"-inf", b"+inf"],
            b"*3\r\n$1\r\nc\r\n$1\r\nb\r\n$1\r\na\r\n",
        );
        command(
            &mut stream,
            &[b"ZREVRANGE", b"z", b"0", b"-1"],
            b"*3\r\n$1\r\na\r\n$1\r\nb\r\n$1\r\nc\r\n",
        );
        command(&mut stream, &[b"ZREM", b"z", b"c"], b":1\r\n");
        command(
            &mut stream,
            &[b"ZREMRANGEBYRANK", b"z", b"0", b"0"],
            b":1\r\n",
        );
        command(
            &mut stream,
            &[b"ZREMRANGEBYSCORE", b"z", b"5", b"5"],
            b":1\r\n",
        );
        command(&mut stream, &[b"EXISTS", b"z"], b":0\r\n");

        command(&mut stream, &[b"SET", b"wrong", b"x"], b"+OK\r\n");
        command(
            &mut stream,
            &[b"HINCRBY", b"wrong", b"f", b"1"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );
        command(
            &mut stream,
            &[b"LINDEX", b"wrong", b"0"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );
        command(
            &mut stream,
            &[b"SINTER", b"wrong"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );
        command(
            &mut stream,
            &[b"ZCOUNT", b"wrong", b"-inf", b"+inf"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );

        command(&mut stream, &[b"MULTI"], b"+OK\r\n");
        command(&mut stream, &[b"HSET", b"tx", b"f", b"v"], b"+QUEUED\r\n");
        command(&mut stream, &[b"ZADD", b"txz", b"1", b"m"], b"+QUEUED\r\n");
        command(&mut stream, &[b"EXEC"], b"*2\r\n:1\r\n:1\r\n");
        command(&mut stream, &[b"HGET", b"tx", b"f"], b"$1\r\nv\r\n");
        command(&mut stream, &[b"ZSCORE", b"txz", b"m"], b"$1\r\n1\r\n");

        server.shutdown().unwrap();
    }
}

#[test]
fn legacy_hash_score_and_lex_commands_work_over_tcp_in_all_execution_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = connect(&server);

        command(
            &mut stream,
            &[b"HMSET", b"h", b"a", b"1", b"b", b"2"],
            b"+OK\r\n",
        );
        command(&mut stream, &[b"HGET", b"h", b"a"], b"$1\r\n1\r\n");
        command(&mut stream, &[b"HSET", b"h", b"c", b"3"], b":1\r\n");
        command(
            &mut stream,
            &[b"HMSET", b"h", b"a"],
            b"-ERR wrong number of arguments for 'hmset' command\r\n",
        );

        command(
            &mut stream,
            &[b"ZADD", b"z", b"1", b"a", b"2.5", b"b"],
            b":2\r\n",
        );
        command(
            &mut stream,
            &[b"ZMSCORE", b"z", b"a", b"missing", b"b"],
            b"*3\r\n$1\r\n1\r\n$-1\r\n$3\r\n2.5\r\n",
        );
        command(
            &mut stream,
            &[b"ZMSCORE", b"missing", b"a", b"b"],
            b"*2\r\n$-1\r\n$-1\r\n",
        );
        command(
            &mut stream,
            &[b"ZMSCORE", b"z"],
            b"-ERR wrong number of arguments for 'zmscore' command\r\n",
        );

        command(
            &mut stream,
            &[b"ZADD", b"lex", b"0", b"a", b"0", b"b", b"0", b"c"],
            b":3\r\n",
        );
        command(
            &mut stream,
            &[b"ZRANGEBYLEX", b"lex", b"-", b"+"],
            b"*3\r\n$1\r\na\r\n$1\r\nb\r\n$1\r\nc\r\n",
        );
        command(
            &mut stream,
            &[b"ZREVRANGEBYLEX", b"lex", b"[c", b"[a"],
            b"*3\r\n$1\r\nc\r\n$1\r\nb\r\n$1\r\na\r\n",
        );
        command(
            &mut stream,
            &[b"ZRANGEBYLEX", b"lex", b"-", b"+", b"LIMIT", b"1", b"1"],
            b"*1\r\n$1\r\nb\r\n",
        );

        command(&mut stream, &[b"RPUSH", b"wrong", b"x"], b":1\r\n");
        command(
            &mut stream,
            &[b"HMSET", b"wrong", b"a", b"1"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );
        command(
            &mut stream,
            &[b"ZMSCORE", b"wrong", b"a"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );
        command(
            &mut stream,
            &[b"ZRANGEBYLEX", b"wrong", b"-", b"+"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );

        server.shutdown().unwrap();
    }
}
