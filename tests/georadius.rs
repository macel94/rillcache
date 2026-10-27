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

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn bulk_f64(reply: &[u8]) -> f64 {
    assert_eq!(reply.first(), Some(&b'$'), "{reply:?}");
    let body = &reply[1..reply.len() - 2];
    let start = body.iter().position(|byte| *byte == b'\n').unwrap() + 1;
    std::str::from_utf8(&body[start..])
        .unwrap()
        .parse()
        .unwrap()
}

fn seed(stream: &mut TcpStream) {
    command(
        stream,
        &[
            b"GEOADD",
            b"Sicily",
            b"13.361389",
            b"38.115556",
            b"Palermo",
            b"15.087269",
            b"37.502669",
            b"Catania",
        ],
        b":2\r\n",
    );
}

#[test]
fn legacy_georadius_aliases_match_search_semantics_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = connect(&server);
        seed(&mut stream);

        command(
            &mut stream,
            &[b"GEORADIUS", b"Sicily", b"15", b"37", b"200", b"km"],
            b"*2\r\n$7\r\nPalermo\r\n$7\r\nCatania\r\n",
        );
        command(
            &mut stream,
            &[
                b"GEORADIUS",
                b"Sicily",
                b"15",
                b"37",
                b"200",
                b"km",
                b"COUNT",
                b"1",
                b"WITHDIST",
            ],
            b"*1\r\n*2\r\n$7\r\nCatania\r\n$7\r\n56.4413\r\n",
        );
        command(
            &mut stream,
            &[
                b"GEORADIUS_RO",
                b"Sicily",
                b"15",
                b"37",
                b"200",
                b"km",
                b"ASC",
                b"COUNT",
                b"1",
            ],
            b"*1\r\n$7\r\nCatania\r\n",
        );
        let detailed = reply(
            &mut stream,
            &[
                b"GEORADIUS",
                b"Sicily",
                b"15",
                b"37",
                b"200",
                b"km",
                b"ASC",
                b"WITHCOORD",
                b"WITHDIST",
                b"WITHHASH",
                b"COUNT",
                b"1",
            ],
        );
        assert!(detailed.starts_with(b"*1\r\n*4\r\n$7\r\nCatania\r\n"));
        assert!(contains(&detailed, b"56.4413"));
        assert!(contains(&detailed, b"3479447370796909"));
        assert!(contains(&detailed, b"15.087267458438873"));
        assert!(contains(&detailed, b"37.50266842333162"));

        command(
            &mut stream,
            &[
                b"GEORADIUSBYMEMBER",
                b"Sicily",
                b"Palermo",
                b"200",
                b"km",
                b"ASC",
                b"COUNT",
                b"1",
                b"WITHDIST",
            ],
            b"*1\r\n*2\r\n$7\r\nPalermo\r\n$6\r\n0.0000\r\n",
        );
        command(
            &mut stream,
            &[
                b"GEORADIUSBYMEMBER_RO",
                b"Sicily",
                b"Catania",
                b"200",
                b"km",
                b"ASC",
                b"COUNT",
                b"1",
                b"WITHDIST",
            ],
            b"*1\r\n*2\r\n$7\r\nCatania\r\n$6\r\n0.0000\r\n",
        );

        command(
            &mut stream,
            &[
                b"GEORADIUS",
                b"Sicily",
                b"15",
                b"37",
                b"200",
                b"km",
                b"STORE",
                b"hashes",
            ],
            b":2\r\n",
        );
        command(
            &mut stream,
            &[b"ZRANGE", b"hashes", b"0", b"-1", b"WITHSCORES"],
            b"*4\r\n$7\r\nPalermo\r\n$16\r\n3479099956230698\r\n$7\r\nCatania\r\n$16\r\n3479447370796909\r\n",
        );
        command(&mut stream, &[b"PEXPIRE", b"hashes", b"100000"], b":1\r\n");
        command(
            &mut stream,
            &[
                b"GEORADIUS",
                b"Sicily",
                b"15",
                b"37",
                b"200",
                b"km",
                b"STOREDIST",
                b"dists",
                b"COUNT",
                b"1",
                b"ASC",
            ],
            b":1\r\n",
        );
        let distance = bulk_f64(&reply(&mut stream, &[b"ZSCORE", b"dists", b"Catania"]));
        assert!((distance - 56.4412578701582).abs() < 0.000001);
        command(
            &mut stream,
            &[
                b"GEORADIUS",
                b"Sicily",
                b"15",
                b"37",
                b"200",
                b"km",
                b"STORE",
                b"hashes",
            ],
            b":2\r\n",
        );
        command(&mut stream, &[b"PTTL", b"hashes"], b":-1\r\n");

        command(
            &mut stream,
            &[
                b"GEORADIUS",
                b"Sicily",
                b"15",
                b"37",
                b"200",
                b"km",
                b"STORE",
                b"first",
                b"STOREDIST",
                b"second",
            ],
            b":2\r\n",
        );
        command(&mut stream, &[b"EXISTS", b"first"], b":0\r\n");
        let second = bulk_f64(&reply(&mut stream, &[b"ZSCORE", b"second", b"Catania"]));
        assert!((second - 56.4412578701582).abs() < 0.000001);

        command(
            &mut stream,
            &[b"GEORADIUS", b"missing", b"15", b"37", b"200", b"km"],
            b"*0\r\n",
        );
        command(
            &mut stream,
            &[
                b"GEORADIUS",
                b"missing",
                b"15",
                b"37",
                b"200",
                b"km",
                b"STORE",
                b"out",
            ],
            b":0\r\n",
        );
        command(&mut stream, &[b"EXISTS", b"out"], b":0\r\n");
        command(
            &mut stream,
            &[
                b"GEORADIUSBYMEMBER",
                b"missing",
                b"member",
                b"200",
                b"km",
                b"STORE",
                b"out",
            ],
            b":0\r\n",
        );
        command(
            &mut stream,
            &[b"GEORADIUSBYMEMBER", b"Sicily", b"missing", b"200", b"km"],
            b"-ERR could not decode requested zset member\r\n",
        );

        command(&mut stream, &[b"SET", b"str", b"v"], b"+OK\r\n");
        command(
            &mut stream,
            &[b"GEORADIUS", b"str", b"15", b"37", b"200", b"km"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );
        command(
            &mut stream,
            &[b"GEORADIUSBYMEMBER", b"str", b"member", b"200", b"km"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );

        command(
            &mut stream,
            &[b"GEORADIUS", b"Sicily", b"15", b"37", b"-1", b"km"],
            b"-ERR radius cannot be negative\r\n",
        );
        command(
            &mut stream,
            &[b"GEORADIUS", b"Sicily", b"x", b"37", b"1", b"km"],
            b"-ERR value is not a valid float\r\n",
        );
        command(
            &mut stream,
            &[b"GEORADIUS", b"Sicily", b"15", b"37", b"1", b"parsec"],
            b"-ERR unsupported unit provided. please use M, KM, FT, MI\r\n",
        );
        command(
            &mut stream,
            &[
                b"GEORADIUS",
                b"Sicily",
                b"15",
                b"37",
                b"1",
                b"km",
                b"COUNT",
                b"0",
            ],
            b"-ERR COUNT must be > 0\r\n",
        );
        command(
            &mut stream,
            &[b"GEORADIUS", b"Sicily", b"15", b"37", b"1", b"km", b"ANY"],
            b"-ERR the ANY argument requires COUNT argument\r\n",
        );
        command(
            &mut stream,
            &[
                b"GEORADIUS",
                b"Sicily",
                b"15",
                b"37",
                b"1",
                b"km",
                b"STORE",
                b"d",
                b"WITHDIST",
            ],
            b"-ERR STORE option in GEORADIUS is not compatible with WITHDIST, WITHHASH and WITHCOORD options\r\n",
        );
        command(
            &mut stream,
            &[b"GEORADIUS", b"Sicily", b"15", b"37", b"1", b"km", b"BOGUS"],
            b"-ERR syntax error\r\n",
        );
        command(
            &mut stream,
            &[
                b"GEORADIUS_RO",
                b"Sicily",
                b"15",
                b"37",
                b"1",
                b"km",
                b"STORE",
                b"d",
            ],
            b"-ERR syntax error\r\n",
        );
        command(
            &mut stream,
            &[
                b"GEORADIUSBYMEMBER_RO",
                b"Sicily",
                b"Palermo",
                b"1",
                b"km",
                b"STOREDIST",
                b"d",
            ],
            b"-ERR syntax error\r\n",
        );
        command(
            &mut stream,
            &[b"GEORADIUS", b"Sicily", b"15", b"37", b"1"],
            b"-ERR wrong number of arguments for 'georadius' command\r\n",
        );
        command(
            &mut stream,
            &[b"GEORADIUSBYMEMBER", b"Sicily", b"Palermo"],
            b"-ERR wrong number of arguments for 'georadiusbymember' command\r\n",
        );

        server.shutdown().unwrap();
    }
}

#[test]
fn legacy_georadius_acl_checks_source_and_store_destination() {
    let server = start(None);
    let mut admin = connect(&server);
    seed(&mut admin);
    command(
        &mut admin,
        &[
            b"ACL",
            b"SETUSER",
            b"alice",
            b"on",
            b">pw",
            b"resetkeys",
            b"~geo:*",
            b"allchannels",
            b"+@all",
        ],
        b"+OK\r\n",
    );
    command(
        &mut admin,
        &[
            b"GEOADD",
            b"geo:src",
            b"13.361389",
            b"38.115556",
            b"Palermo",
        ],
        b":1\r\n",
    );
    let mut alice = connect(&server);
    command(&mut alice, &[b"AUTH", b"alice", b"pw"], b"+OK\r\n");
    command(
        &mut alice,
        &[b"GEORADIUS", b"geo:src", b"15", b"37", b"200", b"km"],
        b"*1\r\n$7\r\nPalermo\r\n",
    );
    command(
        &mut alice,
        &[
            b"GEORADIUS",
            b"geo:src",
            b"15",
            b"37",
            b"200",
            b"km",
            b"STORE",
            b"geo:dest",
        ],
        b":1\r\n",
    );
    command(
        &mut alice,
        &[
            b"GEORADIUS",
            b"geo:src",
            b"15",
            b"37",
            b"200",
            b"km",
            b"STORE",
            b"other:dest",
        ],
        b"-NOPERM this user has no permissions to access one of the keys used as arguments\r\n",
    );
    command(
        &mut alice,
        &[b"GEORADIUSBYMEMBER_RO", b"other:src", b"m", b"1", b"km"],
        b"-NOPERM this user has no permissions to access one of the keys used as arguments\r\n",
    );

    server.shutdown().unwrap();
}
