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

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[derive(Debug)]
enum Value {
    Int(i64),
    Bulk(Vec<u8>),
    Null,
    Array(Vec<Value>),
}

fn parse_value(bytes: &[u8], index: &mut usize) -> Value {
    let marker = bytes[*index];
    *index += 1;
    let end = bytes[*index..]
        .windows(2)
        .position(|window| window == b"\r\n")
        .unwrap()
        + *index;
    let length = std::str::from_utf8(&bytes[*index..end])
        .unwrap()
        .parse::<i64>()
        .unwrap();
    *index = end + 2;
    match marker {
        b':' => Value::Int(length),
        b'$' if length < 0 => Value::Null,
        b'$' => {
            let length = length as usize;
            let value = bytes[*index..*index + length].to_vec();
            *index += length + 2;
            Value::Bulk(value)
        }
        b'*' if length < 0 => Value::Null,
        b'*' => Value::Array(
            (0..length)
                .map(|_| parse_value(bytes, index))
                .collect::<Vec<_>>(),
        ),
        other => panic!("unexpected parser marker {other:?}"),
    }
}

fn params(frame: &[u8]) -> Vec<Value> {
    let mut index = 0;
    parse_value(frame, &mut index).into_array().expect("array")
}

impl Value {
    fn into_array(self) -> Option<Vec<Value>> {
        match self {
            Value::Array(values) => Some(values),
            _ => None,
        }
    }
    fn id(&self) -> Option<&[u8]> {
        match self {
            Value::Bulk(value) => Some(value),
            _ => None,
        }
    }
    fn int(&self) -> Option<i64> {
        match self {
            Value::Int(value) => Some(*value),
            _ => None,
        }
    }
}

fn delivery_count(frame: &[u8], id: &[u8]) -> i64 {
    params(frame)
        .into_iter()
        .find_map(|entry| {
            let fields = entry.into_array()?;
            (fields.first()?.id()? == id).then(|| fields.get(3)?.int())?
        })
        .unwrap_or_else(|| panic!("missing pending delivery {id:?}"))
}

#[test]
fn xreadgroup_history_reads_owned_pending_entries_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = connect(&server);

        for (id, value) in [
            (b"1-0".as_slice(), b"v1".as_slice()),
            (b"2-0", b"v2"),
            (b"3-0", b"v3"),
        ] {
            command(
                &mut stream,
                &[b"XADD", b"s", id, b"f", value],
                format!("$3\r\n{}\r\n", String::from_utf8_lossy(id)).as_bytes(),
            );
        }
        command(
            &mut stream,
            &[b"XGROUP", b"CREATE", b"s", b"g", b"0"],
            b"+OK\r\n",
        );
        command(
            &mut stream,
            &[b"XREADGROUP", b"GROUP", b"g", b"c", b"COUNT", b"2", b"STREAMS", b"s", b">"],
            b"*1\r\n*2\r\n$1\r\ns\r\n*2\r\n*2\r\n$3\r\n1-0\r\n*2\r\n$1\r\nf\r\n$2\r\nv1\r\n*2\r\n$3\r\n2-0\r\n*2\r\n$1\r\nf\r\n$2\r\nv2\r\n",
        );
        command(
            &mut stream,
            &[b"XREADGROUP", b"GROUP", b"g", b"c", b"STREAMS", b"s", b"0"],
            b"*1\r\n*2\r\n$1\r\ns\r\n*2\r\n*2\r\n$3\r\n1-0\r\n*2\r\n$1\r\nf\r\n$2\r\nv1\r\n*2\r\n$3\r\n2-0\r\n*2\r\n$1\r\nf\r\n$2\r\nv2\r\n",
        );
        command(
            &mut stream,
            &[
                b"XREADGROUP",
                b"GROUP",
                b"g",
                b"c",
                b"STREAMS",
                b"s",
                b"1-0",
            ],
            b"*1\r\n*2\r\n$1\r\ns\r\n*1\r\n*2\r\n$3\r\n2-0\r\n*2\r\n$1\r\nf\r\n$2\r\nv2\r\n",
        );
        command(
            &mut stream,
            &[
                b"XREADGROUP",
                b"GROUP",
                b"g",
                b"c",
                b"COUNT",
                b"1",
                b"STREAMS",
                b"s",
                b"0",
            ],
            b"*1\r\n*2\r\n$1\r\ns\r\n*1\r\n*2\r\n$3\r\n1-0\r\n*2\r\n$1\r\nf\r\n$2\r\nv1\r\n",
        );
        command(
            &mut stream,
            &[
                b"XREADGROUP",
                b"GROUP",
                b"g",
                b"c",
                b"STREAMS",
                b"s",
                b"3-0",
            ],
            b"*1\r\n*2\r\n$1\r\ns\r\n*0\r\n",
        );
        let pending = reply(&mut stream, &[b"XPENDING", b"s", b"g", b"-", b"+", b"10"]);
        assert_eq!(delivery_count(&pending, b"1-0"), 3);
        assert_eq!(delivery_count(&pending, b"2-0"), 3);
        let groups = reply(&mut stream, &[b"XINFO", b"GROUPS", b"s"]);
        assert!(contains(&groups, b"last-delivered-id\r\n$3\r\n2-0"));

        command(
            &mut stream,
            &[b"XREADGROUP", b"GROUP", b"g", b"d", b"STREAMS", b"s", b"0"],
            b"*1\r\n*2\r\n$1\r\ns\r\n*0\r\n",
        );
        let consumers = reply(&mut stream, &[b"XINFO", b"CONSUMERS", b"s", b"g"]);
        assert!(contains(&consumers, b"$1\r\nd\r\n"));
        command(
            &mut stream,
            &[b"XREADGROUP", b"GROUP", b"g", b"c", b"NOACK", b"STREAMS", b"s", b"0"],
            b"*1\r\n*2\r\n$1\r\ns\r\n*2\r\n*2\r\n$3\r\n1-0\r\n*2\r\n$1\r\nf\r\n$2\r\nv1\r\n*2\r\n$3\r\n2-0\r\n*2\r\n$1\r\nf\r\n$2\r\nv2\r\n",
        );
        let pending = reply(&mut stream, &[b"XPENDING", b"s", b"g", b"-", b"+", b"10"]);
        assert_eq!(delivery_count(&pending, b"1-0"), 4);
        assert_eq!(delivery_count(&pending, b"2-0"), 4);

        command(&mut stream, &[b"XDEL", b"s", b"1-0"], b":1\r\n");
        command(
            &mut stream,
            &[b"XREADGROUP", b"GROUP", b"g", b"c", b"STREAMS", b"s", b"0"],
            b"*1\r\n*2\r\n$1\r\ns\r\n*2\r\n*2\r\n$3\r\n1-0\r\n*-1\r\n*2\r\n$3\r\n2-0\r\n*2\r\n$1\r\nf\r\n$2\r\nv2\r\n",
        );

        command(
            &mut stream,
            &[b"XREADGROUP", b"GROUP", b"g", b"c", b"STREAMS", b"s", b"$"],
            b"-ERR The $ ID is meaningless in the context of XREADGROUP: you want to read the history of this consumer by specifying a proper ID, or use the > ID to get new messages. The $ ID would just return an empty result set.\r\n",
        );
        command(
            &mut stream,
            &[
                b"XREADGROUP",
                b"GROUP",
                b"g",
                b"c",
                b"STREAMS",
                b"s",
                b"bogus",
            ],
            b"-ERR Invalid stream ID specified as stream command argument\r\n",
        );
        command(
            &mut stream,
            &[b"XREADGROUP", b"GROUP", b"g", b"c", b"STREAMS", b"missing", b"0"],
            b"-NOGROUP No such key 'missing' or consumer group 'g' in XREADGROUP with GROUP option\r\n",
        );
        command(
            &mut stream,
            &[b"XREADGROUP", b"GROUP", b"absent", b"c", b"STREAMS", b"s", b"0"],
            b"-NOGROUP No such key 's' or consumer group 'absent' in XREADGROUP with GROUP option\r\n",
        );
        command(&mut stream, &[b"SET", b"str", b"v"], b"+OK\r\n");
        command(
            &mut stream,
            &[
                b"XREADGROUP",
                b"GROUP",
                b"g",
                b"c",
                b"STREAMS",
                b"str",
                b"0",
            ],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );

        command(
            &mut stream,
            &[b"XADD", b"w", b"1-0", b"f", b"v"],
            b"$3\r\n1-0\r\n",
        );
        command(
            &mut stream,
            &[b"XGROUP", b"CREATE", b"w", b"g", b"0"],
            b"+OK\r\n",
        );
        command(
            &mut stream,
            &[b"XREADGROUP", b"GROUP", b"g", b"c", b"STREAMS", b"w", b">"],
            b"*1\r\n*2\r\n$1\r\nw\r\n*1\r\n*2\r\n$3\r\n1-0\r\n*2\r\n$1\r\nf\r\n$1\r\nv\r\n",
        );
        command(&mut stream, &[b"WATCH", b"w"], b"+OK\r\n");
        let mut other = connect(&server);
        command(
            &mut other,
            &[b"XREADGROUP", b"GROUP", b"g", b"c", b"STREAMS", b"w", b"0"],
            b"*1\r\n*2\r\n$1\r\nw\r\n*1\r\n*2\r\n$3\r\n1-0\r\n*2\r\n$1\r\nf\r\n$1\r\nv\r\n",
        );
        command(&mut stream, &[b"MULTI"], b"+OK\r\n");
        command(&mut stream, &[b"XLEN", b"w"], b"+QUEUED\r\n");
        command(&mut stream, &[b"EXEC"], b"*-1\r\n");

        server.shutdown().unwrap();
    }
}
