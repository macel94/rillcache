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

fn integer(reply: &[u8]) -> i64 {
    assert_eq!(reply.first(), Some(&b':'), "{reply:?}");
    std::str::from_utf8(&reply[1..reply.len() - 2])
        .unwrap()
        .parse()
        .unwrap()
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

const WRONG_TYPE: &[u8] = b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n";

#[test]
fn hsetex_conditions_duplicates_and_hgetex_reads_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = connect(&server);

        command(
            &mut stream,
            &[b"HSETEX", b"h", b"FIELDS", b"2", b"a", b"1", b"b", b"2"],
            b":1\r\n",
        );
        command(&mut stream, &[b"HGET", b"h", b"a"], b"$1\r\n1\r\n");
        command(&mut stream, &[b"HGET", b"h", b"b"], b"$1\r\n2\r\n");

        // A duplicate field is allowed and the last value wins.
        command(
            &mut stream,
            &[b"HSETEX", b"h", b"FIELDS", b"2", b"dup", b"x", b"dup", b"y"],
            b":1\r\n",
        );
        command(&mut stream, &[b"HGET", b"h", b"dup"], b"$1\r\ny\r\n");

        // FNX fails atomically when any requested field already exists.
        command(
            &mut stream,
            &[
                b"HSETEX", b"h", b"FNX", b"FIELDS", b"2", b"a", b"changed", b"c", b"3",
            ],
            b":0\r\n",
        );
        command(&mut stream, &[b"HGET", b"h", b"a"], b"$1\r\n1\r\n");
        command(&mut stream, &[b"HEXISTS", b"h", b"c"], b":0\r\n");
        command(
            &mut stream,
            &[b"HSETEX", b"h", b"FNX", b"FIELDS", b"1", b"c", b"3"],
            b":1\r\n",
        );

        // FXX fails atomically when any requested field is missing.
        command(
            &mut stream,
            &[
                b"HSETEX", b"h", b"FXX", b"FIELDS", b"2", b"a", b"9", b"missing", b"4",
            ],
            b":0\r\n",
        );
        command(&mut stream, &[b"HGET", b"h", b"a"], b"$1\r\n1\r\n");
        command(
            &mut stream,
            &[
                b"HSETEX", b"h", b"FXX", b"FIELDS", b"2", b"a", b"9", b"c", b"3",
            ],
            b":1\r\n",
        );
        command(&mut stream, &[b"HGET", b"h", b"a"], b"$1\r\n9\r\n");

        // FXX on a missing key does not create it.
        command(
            &mut stream,
            &[b"HSETEX", b"absent", b"FXX", b"FIELDS", b"1", b"a", b"1"],
            b":0\r\n",
        );
        command(&mut stream, &[b"EXISTS", b"absent"], b":0\r\n");

        // HGETEX returns values in request order and null for missing fields.
        command(
            &mut stream,
            &[b"HGETEX", b"h", b"FIELDS", b"2", b"a", b"missing"],
            b"*2\r\n$1\r\n9\r\n$-1\r\n",
        );
        // It does not create a missing key.
        command(
            &mut stream,
            &[b"HGETEX", b"absent", b"FIELDS", b"2", b"a", b"b"],
            b"*2\r\n$-1\r\n$-1\r\n",
        );
        command(&mut stream, &[b"EXISTS", b"absent"], b":0\r\n");

        // Wrong types are rejected without mutation.
        command(&mut stream, &[b"SET", b"s", b"v"], b"+OK\r\n");
        command(
            &mut stream,
            &[b"HSETEX", b"s", b"FIELDS", b"1", b"a", b"1"],
            WRONG_TYPE,
        );
        command(
            &mut stream,
            &[b"HGETEX", b"s", b"FIELDS", b"1", b"a"],
            WRONG_TYPE,
        );

        server.shutdown().unwrap();
    }
}

#[test]
fn hsetex_and_hgetex_ttl_options_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = connect(&server);

        // Default HSETEX clears existing field TTLs on written fields.
        command(
            &mut stream,
            &[
                b"HSETEX", b"h", b"PX", b"100000", b"FIELDS", b"2", b"a", b"1", b"b", b"2",
            ],
            b":1\r\n",
        );
        let ttl = ints(&reply(
            &mut stream,
            &[b"HPTTL", b"h", b"FIELDS", b"2", b"a", b"b"],
        ));
        assert!(ttl.iter().all(|value| *value > 90_000), "{ttl:?}");
        command(
            &mut stream,
            &[b"HSETEX", b"h", b"FIELDS", b"1", b"a", b"3"],
            b":1\r\n",
        );
        command(
            &mut stream,
            &[b"HPTTL", b"h", b"FIELDS", b"1", b"a"],
            b"*1\r\n:-1\r\n",
        );
        let untouched = ints(&reply(
            &mut stream,
            &[b"HPTTL", b"h", b"FIELDS", b"1", b"b"],
        ));
        assert!(untouched[0] > 0, "{untouched:?}");

        // KEEPTTL preserves the existing field deadline.
        command(&mut stream, &[b"EXPIRE", b"h", b"100"], b":1\r\n");
        command(
            &mut stream,
            &[b"HSETEX", b"h", b"EX", b"1000", b"FIELDS", b"1", b"a", b"4"],
            b":1\r\n",
        );
        command(
            &mut stream,
            &[b"HSETEX", b"h", b"KEEPTTL", b"FIELDS", b"1", b"a", b"5"],
            b":1\r\n",
        );
        let ttl = ints(&reply(
            &mut stream,
            &[b"HPTTL", b"h", b"FIELDS", b"1", b"a"],
        ));
        assert!(ttl[0] > 0, "{ttl:?}");
        // The hash key's own TTL is not discarded by HSETEX.
        let key_ttl = integer(&reply(&mut stream, &[b"TTL", b"h"]));
        assert!(key_ttl > 0, "{key_ttl}");

        // A condition may follow the expiry clause.
        command(
            &mut stream,
            &[
                b"HSETEX", b"h", b"PX", b"5000", b"FXX", b"FIELDS", b"1", b"a", b"6",
            ],
            b":1\r\n",
        );
        let ttl = ints(&reply(
            &mut stream,
            &[b"HPTTL", b"h", b"FIELDS", b"1", b"a"],
        ));
        assert!(ttl[0] > 0 && ttl[0] <= 5_000, "{ttl:?}");

        // HGETEX leaves TTLs unchanged when no option is supplied.
        command(
            &mut stream,
            &[b"HSET", b"g", b"x", b"1", b"y", b"2"],
            b":2\r\n",
        );
        command(
            &mut stream,
            &[b"HGETEX", b"g", b"FIELDS", b"2", b"x", b"y"],
            b"*2\r\n$1\r\n1\r\n$1\r\n2\r\n",
        );
        command(
            &mut stream,
            &[b"HPTTL", b"g", b"FIELDS", b"2", b"x", b"y"],
            b"*2\r\n:-1\r\n:-1\r\n",
        );

        // HGETEX sets and removes per-field TTLs.
        command(
            &mut stream,
            &[b"HGETEX", b"g", b"EX", b"1000", b"FIELDS", b"1", b"x"],
            b"*1\r\n$1\r\n1\r\n",
        );
        let ttl = ints(&reply(
            &mut stream,
            &[b"HPTTL", b"g", b"FIELDS", b"1", b"x"],
        ));
        assert!(ttl[0] > 990_000 && ttl[0] <= 1_000_000, "{ttl:?}");
        command(
            &mut stream,
            &[
                b"HGETEX", b"g", b"PERSIST", b"FIELDS", b"2", b"x", b"missing",
            ],
            b"*2\r\n$1\r\n1\r\n$-1\r\n",
        );
        command(
            &mut stream,
            &[b"HPTTL", b"g", b"FIELDS", b"1", b"x"],
            b"*1\r\n:-1\r\n",
        );

        // An absolute deadline in the past expires the fields lazily.
        command(
            &mut stream,
            &[
                b"HSETEX", b"past", b"PXAT", b"1", b"FIELDS", b"1", b"only", b"v",
            ],
            b":1\r\n",
        );
        command(
            &mut stream,
            &[b"HGETEX", b"past", b"FIELDS", b"1", b"only"],
            b"*1\r\n$-1\r\n",
        );
        command(&mut stream, &[b"EXISTS", b"past"], b":0\r\n");

        server.shutdown().unwrap();
    }
}

#[test]
fn hsetex_and_hgetex_validation_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = connect(&server);
        command(&mut stream, &[b"HSET", b"h", b"a", b"1"], b":1\r\n");

        command(
            &mut stream,
            &[b"HSETEX", b"h", b"FNX", b"FXX", b"FIELDS", b"1", b"a", b"2"],
            b"-ERR Only one of FXX or FNX arguments can be specified\r\n",
        );
        command(
            &mut stream,
            &[
                b"HSETEX", b"h", b"EX", b"10", b"PX", b"10", b"FIELDS", b"1", b"a", b"2",
            ],
            b"-ERR Only one of EX, PX, EXAT, PXAT or KEEPTTL arguments can be specified\r\n",
        );
        command(
            &mut stream,
            &[b"HSETEX", b"h", b"PERSIST", b"FIELDS", b"1", b"a", b"2"],
            b"-ERR syntax error\r\n",
        );
        command(
            &mut stream,
            &[b"HSETEX", b"h", b"FIELDS", b"0", b"a", b"1"],
            b"-ERR Parameter `numFields` should be greater than 0\r\n",
        );
        command(
            &mut stream,
            &[b"HSETEX", b"h", b"FIELDS", b"2", b"a", b"1"],
            b"-ERR The `numfields` parameter must match the number of arguments\r\n",
        );
        command(
            &mut stream,
            &[b"HSETEX", b"h", b"EX", b"-1", b"FIELDS", b"1", b"a", b"2"],
            b"-ERR invalid expire time, must be >= 0\r\n",
        );
        command(
            &mut stream,
            &[b"HSETEX", b"h", b"EX", b"nope", b"FIELDS", b"1", b"a", b"2"],
            b"-ERR value is not an integer or out of range\r\n",
        );
        command(
            &mut stream,
            &[b"HGETEX", b"h", b"KEEPTTL", b"FIELDS", b"1", b"a"],
            b"-ERR syntax error\r\n",
        );
        command(
            &mut stream,
            &[
                b"HGETEX", b"h", b"EX", b"10", b"PERSIST", b"FIELDS", b"1", b"a",
            ],
            b"-ERR Only one of EX, PX, EXAT, PXAT or PERSIST arguments can be specified\r\n",
        );
        command(
            &mut stream,
            &[b"HGETEX", b"h", b"FIELDS", b"3", b"a"],
            b"-ERR The `numfields` parameter must match the number of arguments\r\n",
        );
        // Rejected commands left the hash untouched.
        command(&mut stream, &[b"HGET", b"h", b"a"], b"$1\r\n1\r\n");

        server.shutdown().unwrap();
    }
}

#[test]
fn hash_getex_setex_state_survives_journal_restart() {
    let dir: PathBuf = std::env::temp_dir().join(format!(
        "rillcache-hash-getex-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let deadline = 4_102_444_800_000i64;
    let deadline_text = deadline.to_string();
    {
        let server = start_persistent(&dir);
        let mut stream = connect(&server);
        command(
            &mut stream,
            &[b"HSETEX", b"h", b"FIELDS", b"2", b"a", b"1", b"b", b"2"],
            b":1\r\n",
        );
        command(
            &mut stream,
            &[
                b"HSETEX",
                b"h",
                b"PXAT",
                deadline_text.as_bytes(),
                b"FIELDS",
                b"1",
                b"a",
                b"11",
            ],
            b":1\r\n",
        );
        command(
            &mut stream,
            &[
                b"HGETEX",
                b"h",
                b"PXAT",
                deadline_text.as_bytes(),
                b"FIELDS",
                b"1",
                b"b",
            ],
            b"*1\r\n$1\r\n2\r\n",
        );
        server.shutdown().unwrap();
    }
    {
        let server = start_persistent(&dir);
        let mut stream = connect(&server);
        command(&mut stream, &[b"HGET", b"h", b"a"], b"$2\r\n11\r\n");
        command(&mut stream, &[b"HGET", b"h", b"b"], b"$1\r\n2\r\n");
        let ttl = ints(&reply(
            &mut stream,
            &[b"HPEXPIRETIME", b"h", b"FIELDS", b"2", b"a", b"b"],
        ));
        assert_eq!(ttl, vec![deadline, deadline]);
        server.shutdown().unwrap();
    }
    let _ = std::fs::remove_dir_all(&dir);
}
