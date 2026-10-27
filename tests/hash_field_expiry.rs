#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rillcache::engine::FsyncPolicy;
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

fn start_persistent(dir: &Path) -> ServerHandle {
    Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        appendonly: true,
        appendfsync: FsyncPolicy::Always,
        dir: dir.to_path_buf(),
        appendfilename: "test.aof".into(),
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

fn ints(reply: &[u8]) -> Vec<i64> {
    assert_eq!(reply.first(), Some(&b'*'), "{reply:?}");
    let mut values = Vec::new();
    let mut index = 0;
    while index < reply.len() {
        if reply[index] == b':' {
            let end = reply[index..]
                .windows(2)
                .position(|window| window == b"\r\n")
                .unwrap()
                + index;
            values.push(
                std::str::from_utf8(&reply[index + 1..end])
                    .unwrap()
                    .parse()
                    .unwrap(),
            );
            index = end + 2;
        } else {
            index += 1;
        }
    }
    values
}

#[test]
fn hash_field_expiry_controls_and_lazy_expiry_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = connect(&server);
        command(
            &mut stream,
            &[b"HSET", b"h", b"a", b"1", b"b", b"2"],
            b":2\r\n",
        );
        command(
            &mut stream,
            &[b"HPEXPIRE", b"h", b"100000", b"FIELDS", b"2", b"a", b"b"],
            b"*2\r\n:1\r\n:1\r\n",
        );
        let remaining = ints(&reply(
            &mut stream,
            &[b"HPTTL", b"h", b"FIELDS", b"2", b"a", b"b"],
        ));
        assert!(remaining.iter().all(|value| *value > 90_000));
        let seconds = ints(&reply(
            &mut stream,
            &[b"HTTL", b"h", b"FIELDS", b"2", b"a", b"b"],
        ));
        assert!(seconds.iter().all(|value| *value > 90 && *value <= 100));
        let absolute = ints(&reply(
            &mut stream,
            &[b"HPEXPIRETIME", b"h", b"FIELDS", b"2", b"a", b"b"],
        ));
        assert!(absolute.iter().all(|value| *value > 1_000_000_000_000));

        command(
            &mut stream,
            &[b"HPEXPIRE", b"h", b"200000", b"NX", b"FIELDS", b"1", b"a"],
            b"*1\r\n:0\r\n",
        );
        command(
            &mut stream,
            &[b"HPEXPIRE", b"h", b"200000", b"XX", b"FIELDS", b"1", b"a"],
            b"*1\r\n:1\r\n",
        );
        command(
            &mut stream,
            &[b"HPEXPIRE", b"h", b"300000", b"GT", b"FIELDS", b"1", b"a"],
            b"*1\r\n:1\r\n",
        );
        command(
            &mut stream,
            &[b"HPEXPIRE", b"h", b"250000", b"GT", b"FIELDS", b"1", b"a"],
            b"*1\r\n:0\r\n",
        );
        command(
            &mut stream,
            &[b"HPEXPIRE", b"h", b"250000", b"LT", b"FIELDS", b"1", b"a"],
            b"*1\r\n:1\r\n",
        );
        command(
            &mut stream,
            &[b"HPEXPIRE", b"h", b"260000", b"LT", b"FIELDS", b"1", b"a"],
            b"*1\r\n:0\r\n",
        );
        command(
            &mut stream,
            &[b"HPERSIST", b"h", b"FIELDS", b"1", b"a"],
            b"*1\r\n:1\r\n",
        );
        command(
            &mut stream,
            &[b"HPERSIST", b"h", b"FIELDS", b"1", b"a"],
            b"*1\r\n:-1\r\n",
        );
        command(
            &mut stream,
            &[b"HPTTL", b"h", b"FIELDS", b"1", b"a"],
            b"*1\r\n:-1\r\n",
        );

        command(
            &mut stream,
            &[b"HPEXPIRE", b"h", b"100000", b"FIELDS", b"1", b"a"],
            b"*1\r\n:1\r\n",
        );
        command(&mut stream, &[b"HSET", b"h", b"a", b"9"], b":0\r\n");
        command(
            &mut stream,
            &[b"HPTTL", b"h", b"FIELDS", b"1", b"a"],
            b"*1\r\n:-1\r\n",
        );
        command(&mut stream, &[b"HINCRBY", b"h", b"b", b"1"], b":3\r\n");
        let preserved = ints(&reply(
            &mut stream,
            &[b"HPTTL", b"h", b"FIELDS", b"1", b"b"],
        ));
        assert!(preserved[0] > 90_000);

        command(
            &mut stream,
            &[b"HPEXPIRE", b"h", b"0", b"FIELDS", b"1", b"b"],
            b"*1\r\n:1\r\n",
        );
        command(&mut stream, &[b"HGET", b"h", b"b"], b"$-1\r\n");
        command(&mut stream, &[b"HLEN", b"h"], b":1\r\n");
        command(
            &mut stream,
            &[b"HPTTL", b"h", b"FIELDS", b"2", b"a", b"b"],
            b"*2\r\n:-1\r\n:-2\r\n",
        );
        command(&mut stream, &[b"HDEL", b"h", b"a"], b":1\r\n");
        command(&mut stream, &[b"EXISTS", b"h"], b":0\r\n");

        command(
            &mut stream,
            &[b"HPEXPIRE", b"missing", b"100", b"FIELDS", b"2", b"x", b"y"],
            b"*2\r\n:-2\r\n:-2\r\n",
        );
        command(
            &mut stream,
            &[b"HPTTL", b"missing", b"FIELDS", b"2", b"x", b"y"],
            b"*2\r\n:-2\r\n:-2\r\n",
        );
        command(
            &mut stream,
            &[b"HPERSIST", b"missing", b"FIELDS", b"1", b"x"],
            b"*1\r\n:-2\r\n",
        );

        command(&mut stream, &[b"SET", b"str", b"v"], b"+OK\r\n");
        command(
            &mut stream,
            &[b"HPEXPIRE", b"str", b"100", b"FIELDS", b"1", b"x"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );
        command(
            &mut stream,
            &[b"HPEXPIRE", b"h", b"-1", b"FIELDS", b"1", b"x"],
            b"-ERR invalid expire time, must be >= 0\r\n",
        );
        command(
            &mut stream,
            &[b"HPEXPIRE", b"h", b"x", b"FIELDS", b"1", b"x"],
            b"-ERR value is not an integer or out of range\r\n",
        );
        command(
            &mut stream,
            &[b"HPEXPIRE", b"h", b"100", b"BOGUS", b"FIELDS", b"1", b"x"],
            b"-ERR Mandatory argument FIELDS is missing or not at the right position\r\n",
        );
        command(
            &mut stream,
            &[b"HPEXPIRE", b"h", b"100", b"FIELDS", b"2", b"x"],
            b"-ERR The `numfields` parameter must match the number of arguments\r\n",
        );
        command(
            &mut stream,
            &[b"HPEXPIRE", b"h", b"100", b"FIELDS", b"x", b"a"],
            b"-ERR Parameter `numFields` should be greater than 0\r\n",
        );
        command(
            &mut stream,
            &[b"HPTTL", b"h", b"FIELDS", b"x", b"a"],
            b"-ERR Number of fields must be a positive integer\r\n",
        );
        command(
            &mut stream,
            &[b"HPTTL", b"h", b"FIELDS", b"2", b"a"],
            b"-ERR The `numfields` parameter must match the number of arguments\r\n",
        );
        command(
            &mut stream,
            &[b"HPERSIST", b"h", b"FIELDS", b"x", b"a"],
            b"-ERR Number of fields must be a positive integer\r\n",
        );

        server.shutdown().unwrap();
    }
}

#[test]
fn hash_field_expiry_state_survives_journal_restart() {
    let dir: PathBuf = std::env::temp_dir().join(format!(
        "rillcache-hash-ttl-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let deadline = 4_102_444_800_000i64;
    {
        let server = start_persistent(&dir);
        let mut stream = connect(&server);
        command(&mut stream, &[b"HSET", b"h", b"f", b"v"], b":1\r\n");
        let deadline_text = deadline.to_string();
        command(
            &mut stream,
            &[
                b"HPEXPIREAT",
                b"h",
                deadline_text.as_bytes(),
                b"FIELDS",
                b"1",
                b"f",
            ],
            b"*1\r\n:1\r\n",
        );
        server.shutdown().unwrap();
    }
    {
        let server = start_persistent(&dir);
        let mut stream = connect(&server);
        command(&mut stream, &[b"HGET", b"h", b"f"], b"$1\r\nv\r\n");
        let ttl = ints(&reply(
            &mut stream,
            &[b"HPEXPIRETIME", b"h", b"FIELDS", b"1", b"f"],
        ));
        assert_eq!(ttl, vec![deadline]);
        server.shutdown().unwrap();
    }
    let _ = std::fs::remove_dir_all(&dir);
}
