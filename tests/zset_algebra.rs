#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

fn start_with_limit(workers: Option<usize>, max_data_bytes: usize) -> ServerHandle {
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

fn start(workers: Option<usize>) -> ServerHandle {
    start_with_limit(workers, Config::default().max_data_bytes)
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

fn seed(stream: &mut TcpStream) {
    command(
        stream,
        &[b"ZADD", b"a", b"1", b"x", b"2", b"y", b"3", b"z"],
        b":3\r\n",
    );
    command(
        stream,
        &[b"ZADD", b"b", b"10", b"x", b"20", b"y", b"30", b"w"],
        b":3\r\n",
    );
}

#[test]
fn sorted_set_algebra_and_store_work_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = connect(&server);
        seed(&mut stream);

        command(
            &mut stream,
            &[b"ZUNION", b"2", b"a", b"b", b"WITHSCORES"],
            b"*8\r\n$1\r\nz\r\n$1\r\n3\r\n$1\r\nx\r\n$2\r\n11\r\n$1\r\ny\r\n$2\r\n22\r\n$1\r\nw\r\n$2\r\n30\r\n",
        );
        command(
            &mut stream,
            &[
                b"ZUNION", b"2", b"a", b"b", b"WEIGHTS", b"2", b"3", b"AGGREGATE", b"MIN",
                b"WITHSCORES",
            ],
            b"*8\r\n$1\r\nx\r\n$1\r\n2\r\n$1\r\ny\r\n$1\r\n4\r\n$1\r\nz\r\n$1\r\n6\r\n$1\r\nw\r\n$2\r\n90\r\n",
        );
        command(
            &mut stream,
            &[
                b"ZUNION", b"2", b"a", b"b", b"WEIGHTS", b"-1", b"1", b"WITHSCORES",
            ],
            b"*8\r\n$1\r\nz\r\n$2\r\n-3\r\n$1\r\nx\r\n$1\r\n9\r\n$1\r\ny\r\n$2\r\n18\r\n$1\r\nw\r\n$2\r\n30\r\n",
        );
        command(
            &mut stream,
            &[b"ZINTER", b"2", b"a", b"b", b"WITHSCORES"],
            b"*4\r\n$1\r\nx\r\n$2\r\n11\r\n$1\r\ny\r\n$2\r\n22\r\n",
        );
        command(
            &mut stream,
            &[
                b"ZINTER",
                b"2",
                b"a",
                b"b",
                b"WEIGHTS",
                b"2",
                b"3",
                b"AGGREGATE",
                b"MAX",
                b"WITHSCORES",
            ],
            b"*4\r\n$1\r\nx\r\n$2\r\n30\r\n$1\r\ny\r\n$2\r\n60\r\n",
        );
        command(
            &mut stream,
            &[b"ZDIFF", b"2", b"a", b"b", b"WITHSCORES"],
            b"*2\r\n$1\r\nz\r\n$1\r\n3\r\n",
        );
        command(
            &mut stream,
            &[b"ZDIFF", b"2", b"b", b"a", b"WITHSCORES"],
            b"*2\r\n$1\r\nw\r\n$2\r\n30\r\n",
        );
        command(&mut stream, &[b"ZINTERCARD", b"2", b"a", b"b"], b":2\r\n");
        command(
            &mut stream,
            &[b"ZINTERCARD", b"2", b"a", b"b", b"LIMIT", b"1"],
            b":1\r\n",
        );
        command(
            &mut stream,
            &[b"ZINTERCARD", b"2", b"a", b"missing"],
            b":0\r\n",
        );
        command(
            &mut stream,
            &[b"ZUNION", b"1", b"missing", b"WITHSCORES"],
            b"*0\r\n",
        );

        command(&mut stream, &[b"ZADD", b"d", b"9", b"old"], b":1\r\n");
        command(
            &mut stream,
            &[b"ZUNIONSTORE", b"d", b"2", b"a", b"b"],
            b":4\r\n",
        );
        command(
            &mut stream,
            &[b"ZRANGE", b"d", b"0", b"-1", b"WITHSCORES"],
            b"*8\r\n$1\r\nz\r\n$1\r\n3\r\n$1\r\nx\r\n$2\r\n11\r\n$1\r\ny\r\n$2\r\n22\r\n$1\r\nw\r\n$2\r\n30\r\n",
        );
        command(&mut stream, &[b"ZADD", b"e", b"0", b"stale"], b":1\r\n");
        command(&mut stream, &[b"PEXPIRE", b"e", b"100000"], b":1\r\n");
        command(
            &mut stream,
            &[
                b"ZUNIONSTORE",
                b"e",
                b"2",
                b"a",
                b"b",
                b"WEIGHTS",
                b"2",
                b"3",
                b"AGGREGATE",
                b"MAX",
            ],
            b":4\r\n",
        );
        command(&mut stream, &[b"PTTL", b"e"], b":-1\r\n");
        command(
            &mut stream,
            &[b"ZRANGE", b"e", b"0", b"-1", b"WITHSCORES"],
            b"*8\r\n$1\r\nz\r\n$1\r\n6\r\n$1\r\nx\r\n$2\r\n30\r\n$1\r\ny\r\n$2\r\n60\r\n$1\r\nw\r\n$2\r\n90\r\n",
        );
        command(
            &mut stream,
            &[b"ZDIFFSTORE", b"d", b"2", b"a", b"b"],
            b":1\r\n",
        );
        command(
            &mut stream,
            &[b"ZRANGE", b"d", b"0", b"-1", b"WITHSCORES"],
            b"*2\r\n$1\r\nz\r\n$1\r\n3\r\n",
        );
        command(
            &mut stream,
            &[b"ZUNIONSTORE", b"empty", b"2", b"missing", b"other"],
            b":0\r\n",
        );
        command(&mut stream, &[b"EXISTS", b"empty"], b":0\r\n");
        command(
            &mut stream,
            &[
                b"ZUNIONSTORE",
                b"a",
                b"2",
                b"a",
                b"b",
                b"WEIGHTS",
                b"1",
                b"1",
            ],
            b":4\r\n",
        );
        command(
            &mut stream,
            &[b"ZRANGE", b"a", b"0", b"-1", b"WITHSCORES"],
            b"*8\r\n$1\r\nz\r\n$1\r\n3\r\n$1\r\nx\r\n$2\r\n11\r\n$1\r\ny\r\n$2\r\n22\r\n$1\r\nw\r\n$2\r\n30\r\n",
        );

        command(&mut stream, &[b"ZADD", b"inf", b"inf", b"m"], b":1\r\n");
        command(
            &mut stream,
            &[b"ZUNION", b"1", b"inf", b"WEIGHTS", b"0", b"WITHSCORES"],
            b"*2\r\n$1\r\nm\r\n$1\r\n0\r\n",
        );
        command(
            &mut stream,
            &[b"ZUNIONSTORE", b"nanstore", b"1", b"inf", b"WEIGHTS", b"0"],
            b":1\r\n",
        );
        command(
            &mut stream,
            &[b"ZRANGE", b"nanstore", b"0", b"-1", b"WITHSCORES"],
            b"*2\r\n$1\r\nm\r\n$1\r\n0\r\n",
        );

        command(
            &mut stream,
            &[b"ZUNION", b"0", b"a"],
            b"-ERR at least 1 input key is needed for 'zunion' command\r\n",
        );
        command(
            &mut stream,
            &[b"ZUNION", b"x", b"a"],
            b"-ERR value is not an integer or out of range\r\n",
        );
        command(
            &mut stream,
            &[b"ZUNION", b"3", b"a", b"b"],
            b"-ERR syntax error\r\n",
        );
        command(
            &mut stream,
            &[b"ZUNION", b"2", b"a", b"b", b"WEIGHTS", b"x", b"1"],
            b"-ERR weight value is not a float\r\n",
        );
        command(
            &mut stream,
            &[b"ZUNION", b"2", b"a", b"b", b"AGGREGATE", b"bogus"],
            b"-ERR syntax error\r\n",
        );
        command(
            &mut stream,
            &[b"ZUNION", b"2", b"a", b"b", b"bogus"],
            b"-ERR syntax error\r\n",
        );
        command(
            &mut stream,
            &[b"ZUNIONSTORE", b"d2", b"2", b"a", b"b", b"WITHSCORES"],
            b"-ERR syntax error\r\n",
        );
        command(
            &mut stream,
            &[b"ZDIFF", b"2", b"a", b"b", b"WEIGHTS", b"1", b"1"],
            b"-ERR syntax error\r\n",
        );
        command(
            &mut stream,
            &[b"ZINTERCARD", b"2", b"a", b"b", b"LIMIT", b"-1"],
            b"-ERR LIMIT can't be negative\r\n",
        );
        command(
            &mut stream,
            &[b"ZINTERCARD", b"2", b"a", b"b", b"LIMIT", b"x"],
            b"-ERR LIMIT can't be negative\r\n",
        );
        command(
            &mut stream,
            &[b"ZINTERCARD", b"0", b"a"],
            b"-ERR at least 1 input key is needed for 'zintercard' command\r\n",
        );

        command(&mut stream, &[b"SET", b"string", b"v"], b"+OK\r\n");
        command(
            &mut stream,
            &[b"ZUNION", b"2", b"a", b"string"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );
        command(&mut stream, &[b"ZADD", b"keep", b"1", b"k"], b":1\r\n");
        command(
            &mut stream,
            &[b"ZUNIONSTORE", b"keep", b"2", b"a", b"string"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );
        command(
            &mut stream,
            &[b"ZRANGE", b"keep", b"0", b"-1", b"WITHSCORES"],
            b"*2\r\n$1\r\nk\r\n$1\r\n1\r\n",
        );

        server.shutdown().unwrap();

        let small = start_with_limit(workers, 7);
        let mut limited = connect(&small);
        command(&mut limited, &[b"ZADD", b"dest", b"0", b"k"], b":1\r\n");
        command(&mut limited, &[b"ZADD", b"s", b"0", b"a"], b":1\r\n");
        command(
            &mut limited,
            &[b"ZUNIONSTORE", b"dest", b"2", b"dest", b"s"],
            b"-OOM configured payload limit exceeded\r\n",
        );
        command(
            &mut limited,
            &[b"ZRANGE", b"dest", b"0", b"-1"],
            b"*1\r\n$1\r\nk\r\n",
        );
        command(
            &mut limited,
            &[b"ZUNIONSTORE", b"newdest", b"1", b"s"],
            b"-OOM configured payload limit exceeded\r\n",
        );
        command(&mut limited, &[b"EXISTS", b"newdest"], b":0\r\n");
        small.shutdown().unwrap();
    }
}

#[test]
fn zset_algebra_enforces_acl_patterns_for_destination_and_counted_sources() {
    let server = start(None);
    let mut admin = connect(&server);
    command(
        &mut admin,
        &[
            b"ACL",
            b"SETUSER",
            b"alice",
            b"on",
            b">pw",
            b"resetkeys",
            b"~a:*",
            b"allchannels",
            b"+@all",
        ],
        b"+OK\r\n",
    );
    let mut alice = connect(&server);
    command(&mut alice, &[b"AUTH", b"alice", b"pw"], b"+OK\r\n");
    command(&mut alice, &[b"ZADD", b"a:one", b"1", b"m"], b":1\r\n");
    command(
        &mut alice,
        &[b"ZADD", b"b:one", b"1", b"m"],
        b"-NOPERM this user has no permissions to access one of the keys used as arguments\r\n",
    );
    command(
        &mut alice,
        &[b"ZUNION", b"2", b"a:one", b"a:two"],
        b"*1\r\n$1\r\nm\r\n",
    );
    command(
        &mut alice,
        &[b"ZUNION", b"2", b"a:one", b"b:one"],
        b"-NOPERM this user has no permissions to access one of the keys used as arguments\r\n",
    );
    command(
        &mut alice,
        &[b"ZUNIONSTORE", b"a:dest", b"2", b"a:one", b"a:two"],
        b":1\r\n",
    );
    command(
        &mut alice,
        &[b"ZUNIONSTORE", b"b:dest", b"2", b"a:one", b"a:two"],
        b"-NOPERM this user has no permissions to access one of the keys used as arguments\r\n",
    );
    command(
        &mut alice,
        &[b"ZUNIONSTORE", b"a:dest", b"2", b"a:one", b"b:one"],
        b"-NOPERM this user has no permissions to access one of the keys used as arguments\r\n",
    );
    command(
        &mut alice,
        &[b"ZINTERCARD", b"2", b"a:one", b"b:one"],
        b"-NOPERM this user has no permissions to access one of the keys used as arguments\r\n",
    );
    command(
        &mut alice,
        &[b"ZINTERCARD", b"2", b"a:one", b"a:two"],
        b":0\r\n",
    );

    server.shutdown().unwrap();
}
