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

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn ttl(stream: &mut TcpStream, key: &[u8]) -> i64 {
    let reply = reply(stream, &[b"PTTL", key]);
    std::str::from_utf8(&reply[1..reply.len() - 2])
        .unwrap()
        .parse()
        .unwrap()
}

#[test]
fn geospatial_commands_are_consistent_over_tcp_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers, Config::default().max_data_bytes);
        let mut stream = connect(&server);

        command(
            &mut stream,
            &[b"GEOADD", b"Sicily", b"181", b"0", b"bad"],
            b"-ERR invalid longitude,latitude pair 181.000000,0.000000\r\n",
        );
        command(
            &mut stream,
            &[b"GEOADD", b"Sicily", b"NX", b"XX", b"0", b"0", b"bad"],
            b"-ERR syntax error\r\n",
        );
        command(
            &mut stream,
            &[b"GEOADD", b"Sicily", b"0", b"0"],
            b"-ERR wrong number of arguments for 'geoadd' command\r\n",
        );
        command(
            &mut stream,
            &[b"GEOPOS", b"missing", b"a"],
            b"*1\r\n*-1\r\n",
        );
        command(
            &mut stream,
            &[b"GEOHASH", b"missing", b"a"],
            b"*1\r\n$-1\r\n",
        );
        command(
            &mut stream,
            &[b"GEODIST", b"missing", b"a", b"b"],
            b"$-1\r\n",
        );

        command(
            &mut stream,
            &[b"GEOADD", b"Sicily", b"13.361389", b"38.115556", b"Palermo"],
            b":1\r\n",
        );
        command(
            &mut stream,
            &[b"GEOADD", b"Sicily", b"15.087269", b"37.502669", b"Catania"],
            b":1\r\n",
        );
        command(&mut stream, &[b"TYPE", b"Sicily"], b"+zset\r\n");
        command(&mut stream, &[b"ZCARD", b"Sicily"], b":2\r\n");
        command(
            &mut stream,
            &[b"GEOADD", b"Sicily", b"NX", b"13.5", b"38.2", b"Palermo"],
            b":0\r\n",
        );
        command(
            &mut stream,
            &[
                b"GEOADD", b"Sicily", b"XX", b"CH", b"13.5", b"38.2", b"Palermo",
            ],
            b":1\r\n",
        );
        command(
            &mut stream,
            &[
                b"GEOADD",
                b"Sicily",
                b"XX",
                b"13.361389",
                b"38.115556",
                b"Palermo",
            ],
            b":0\r\n",
        );

        command(
            &mut stream,
            &[b"GEODIST", b"Sicily", b"Palermo", b"Catania"],
            b"$11\r\n166274.1516\r\n",
        );
        command(
            &mut stream,
            &[b"GEODIST", b"Sicily", b"Palermo", b"Catania", b"km"],
            b"$8\r\n166.2742\r\n",
        );
        command(
            &mut stream,
            &[b"GEODIST", b"Sicily", b"Palermo", b"missing"],
            b"$-1\r\n",
        );
        command(
            &mut stream,
            &[b"GEODIST", b"Sicily", b"Palermo", b"Catania", b"lightyear"],
            b"-ERR unsupported unit provided. please use M, KM, FT, MI\r\n",
        );
        command(
            &mut stream,
            &[b"GEOPOS", b"Sicily", b"Palermo", b"missing"],
            b"*2\r\n*2\r\n$20\r\n13.36138933897018433\r\n$20\r\n38.11555639549629859\r\n*-1\r\n",
        );
        command(
            &mut stream,
            &[b"GEOHASH", b"Sicily", b"Palermo", b"Catania", b"missing"],
            b"*3\r\n$11\r\nsqc8b49rny0\r\n$11\r\nsqdtr74hyu0\r\n$-1\r\n",
        );

        command(
            &mut stream,
            &[
                b"GEOSEARCH",
                b"Sicily",
                b"FROMLONLAT",
                b"15",
                b"37",
                b"BYRADIUS",
                b"200",
                b"km",
                b"ASC",
            ],
            b"*2\r\n$7\r\nCatania\r\n$7\r\nPalermo\r\n",
        );
        let detailed = reply(
            &mut stream,
            &[
                b"GEOSEARCH",
                b"Sicily",
                b"FROMMEMBER",
                b"Palermo",
                b"BYRADIUS",
                b"200",
                b"km",
                b"ASC",
                b"WITHDIST",
                b"WITHHASH",
                b"WITHCOORD",
            ],
        );
        assert!(detailed.starts_with(b"*2\r\n*4\r\n$7\r\nPalermo\r\n"));
        assert!(contains(&detailed, b"0.0000"));
        assert!(contains(&detailed, b"13.36138933897018433"));
        command(
            &mut stream,
            &[
                b"GEOSEARCH",
                b"Sicily",
                b"FROMLONLAT",
                b"15",
                b"37",
                b"BYRADIUS",
                b"200",
                b"km",
                b"COUNT",
                b"1",
            ],
            b"*1\r\n$7\r\nCatania\r\n",
        );
        let any = reply(
            &mut stream,
            &[
                b"GEOSEARCH",
                b"Sicily",
                b"FROMLONLAT",
                b"15",
                b"37",
                b"BYRADIUS",
                b"200",
                b"km",
                b"COUNT",
                b"1",
                b"ANY",
            ],
        );
        assert!(any.starts_with(b"*1\r\n"));
        command(
            &mut stream,
            &[
                b"GEOSEARCH",
                b"Sicily",
                b"FROMMEMBER",
                b"Palermo",
                b"BYBOX",
                b"400",
                b"400",
                b"km",
                b"DESC",
            ],
            b"*2\r\n$7\r\nCatania\r\n$7\r\nPalermo\r\n",
        );

        command(
            &mut stream,
            &[
                b"GEOSEARCHSTORE",
                b"out",
                b"Sicily",
                b"FROMLONLAT",
                b"15",
                b"37",
                b"BYRADIUS",
                b"200",
                b"km",
                b"ASC",
            ],
            b":2\r\n",
        );
        command(&mut stream, &[b"TYPE", b"out"], b"+zset\r\n");
        command(&mut stream, &[b"ZCARD", b"out"], b":2\r\n");
        command(
            &mut stream,
            &[b"GEOPOS", b"out", b"Catania"],
            b"*1\r\n*2\r\n$20\r\n15.08726745843887329\r\n$20\r\n37.50266842333162032\r\n",
        );
        command(
            &mut stream,
            &[
                b"GEOSEARCHSTORE",
                b"dist",
                b"Sicily",
                b"FROMLONLAT",
                b"15",
                b"37",
                b"BYRADIUS",
                b"200",
                b"km",
                b"STOREDIST",
            ],
            b":2\r\n",
        );
        assert!(reply(&mut stream, &[b"ZSCORE", b"dist", b"Catania"]).starts_with(b"$"));
        command(
            &mut stream,
            &[
                b"GEOSEARCHSTORE",
                b"empty",
                b"Sicily",
                b"FROMLONLAT",
                b"0",
                b"0",
                b"BYRADIUS",
                b"1",
                b"km",
            ],
            b":0\r\n",
        );
        command(&mut stream, &[b"EXISTS", b"empty"], b":0\r\n");
        command(&mut stream, &[b"SET", b"empty", b"v"], b"+OK\r\n");
        command(
            &mut stream,
            &[
                b"GEOSEARCHSTORE",
                b"empty",
                b"Sicily",
                b"FROMLONLAT",
                b"0",
                b"0",
                b"BYRADIUS",
                b"1",
                b"km",
            ],
            b":0\r\n",
        );
        command(&mut stream, &[b"EXISTS", b"empty"], b":0\r\n");
        command(
            &mut stream,
            &[b"SET", b"out", b"v", b"PX", b"100000"],
            b"+OK\r\n",
        );
        command(
            &mut stream,
            &[
                b"GEOSEARCHSTORE",
                b"out",
                b"Sicily",
                b"FROMMEMBER",
                b"Palermo",
                b"BYRADIUS",
                b"1",
                b"km",
            ],
            b":1\r\n",
        );
        assert_eq!(ttl(&mut stream, b"out"), -1);

        command(
            &mut stream,
            &[b"GEOADD", b"ttl", b"0", b"0", b"a"],
            b":1\r\n",
        );
        command(&mut stream, &[b"PEXPIRE", b"ttl", b"100000"], b":1\r\n");
        command(
            &mut stream,
            &[b"GEOADD", b"ttl", b"1", b"1", b"b"],
            b":1\r\n",
        );
        assert!(ttl(&mut stream, b"ttl") > 0);

        command(
            &mut stream,
            &[
                b"GEOSEARCH",
                b"Sicily",
                b"BYRADIUS",
                b"1",
                b"km",
                b"ASC",
                b"WITHDIST",
            ],
            b"-ERR exactly one of FROMMEMBER or FROMLONLAT can be specified for GEOSEARCH\r\n",
        );
        command(
            &mut stream,
            &[
                b"GEOSEARCH",
                b"Sicily",
                b"FROMLONLAT",
                b"0",
                b"0",
                b"ASC",
                b"WITHDIST",
            ],
            b"-ERR exactly one of BYRADIUS and BYBOX can be specified for GEOSEARCH\r\n",
        );
        command(
            &mut stream,
            &[
                b"GEOSEARCH",
                b"Sicily",
                b"FROMLONLAT",
                b"0",
                b"0",
                b"BYRADIUS",
                b"-1",
                b"km",
            ],
            b"-ERR radius cannot be negative\r\n",
        );
        command(
            &mut stream,
            &[
                b"GEOSEARCH",
                b"Sicily",
                b"FROMLONLAT",
                b"0",
                b"0",
                b"BYRADIUS",
                b"1",
                b"parsec",
            ],
            b"-ERR unsupported unit provided. please use M, KM, FT, MI\r\n",
        );
        command(
            &mut stream,
            &[
                b"GEOSEARCH",
                b"Sicily",
                b"FROMLONLAT",
                b"0",
                b"0",
                b"BYBOX",
                b"-1",
                b"1",
                b"km",
            ],
            b"-ERR height or width cannot be negative\r\n",
        );
        command(
            &mut stream,
            &[
                b"GEOSEARCH",
                b"Sicily",
                b"FROMLONLAT",
                b"0",
                b"0",
                b"BYRADIUS",
                b"1",
                b"km",
                b"COUNT",
                b"0",
            ],
            b"-ERR COUNT must be > 0\r\n",
        );
        command(
            &mut stream,
            &[
                b"GEOSEARCH",
                b"Sicily",
                b"FROMLONLAT",
                b"0",
                b"0",
                b"BYRADIUS",
                b"1",
                b"km",
                b"ANY",
            ],
            b"-ERR the ANY argument requires COUNT argument\r\n",
        );
        command(
            &mut stream,
            &[
                b"GEOSEARCH",
                b"Sicily",
                b"FROMMEMBER",
                b"missing",
                b"BYRADIUS",
                b"1",
                b"km",
            ],
            b"-ERR could not decode requested zset member\r\n",
        );
        command(
            &mut stream,
            &[
                b"GEOSEARCHSTORE",
                b"out2",
                b"Sicily",
                b"FROMLONLAT",
                b"0",
                b"0",
                b"BYRADIUS",
                b"1",
                b"km",
                b"WITHCOORD",
            ],
            b"-ERR GEOSEARCHSTORE is not compatible with WITHDIST, WITHHASH and WITHCOORD options\r\n",
        );

        command(&mut stream, &[b"RPUSH", b"list", b"x"], b":1\r\n");
        command(
            &mut stream,
            &[b"GEOPOS", b"list", b"a"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );
        command(
            &mut stream,
            &[
                b"GEOSEARCH",
                b"list",
                b"FROMLONLAT",
                b"0",
                b"0",
                b"BYRADIUS",
                b"1",
                b"km",
            ],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );
        command(
            &mut stream,
            &[
                b"GEOSEARCHSTORE",
                b"out3",
                b"list",
                b"FROMLONLAT",
                b"0",
                b"0",
                b"BYRADIUS",
                b"1",
                b"km",
            ],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );
        command(&mut stream, &[b"SET", b"gone", b"v"], b"+OK\r\n");
        command(
            &mut stream,
            &[
                b"GEOSEARCHSTORE",
                b"gone",
                b"missing",
                b"FROMMEMBER",
                b"a",
                b"BYRADIUS",
                b"1",
                b"km",
            ],
            b":0\r\n",
        );
        command(&mut stream, &[b"EXISTS", b"gone"], b":0\r\n");

        command(&mut stream, &[b"WATCH", b"w"], b"+OK\r\n");
        let mut other = connect(&server);
        command(&mut other, &[b"GEOADD", b"w", b"0", b"0", b"a"], b":1\r\n");
        command(&mut stream, &[b"MULTI"], b"+OK\r\n");
        command(
            &mut stream,
            &[b"GEOADD", b"w", b"1", b"1", b"b"],
            b"+QUEUED\r\n",
        );
        command(&mut stream, &[b"EXEC"], b"*-1\r\n");

        command(&mut stream, &[b"MULTI"], b"+OK\r\n");
        command(
            &mut stream,
            &[b"GEOADD", b"q", b"0", b"0", b"a"],
            b"+QUEUED\r\n",
        );
        command(
            &mut stream,
            &[
                b"GEOSEARCH",
                b"q",
                b"FROMLONLAT",
                b"0",
                b"0",
                b"BYRADIUS",
                b"1",
                b"km",
            ],
            b"+QUEUED\r\n",
        );
        command(&mut stream, &[b"EXEC"], b"*2\r\n:1\r\n*1\r\n$1\r\na\r\n");

        server.shutdown().unwrap();

        let small = start(workers, 4);
        let mut limited = connect(&small);
        command(
            &mut limited,
            &[b"GEOADD", b"big", b"0", b"0", b"member"],
            b"-OOM configured payload limit exceeded\r\n",
        );
        command(&mut limited, &[b"EXISTS", b"big"], b":0\r\n");
        small.shutdown().unwrap();
    }
}
