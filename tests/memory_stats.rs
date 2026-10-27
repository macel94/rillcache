#![forbid(unsafe_code)]

use std::collections::BTreeMap;
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

fn reply(stream: &mut TcpStream, args: &[&[u8]]) -> Vec<u8> {
    let mut request = Vec::new();
    encode_request(args, &mut request);
    stream.write_all(&request).unwrap();
    read_frame(stream, 0)
}

fn command(stream: &mut TcpStream, args: &[&[u8]], expected: &[u8]) {
    let frame = reply(stream, args);
    assert_eq!(frame, expected, "{args:?}");
}

struct Cursor<'a> {
    data: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    fn line(&mut self) -> &'a str {
        let end = self.data[self.position..]
            .windows(2)
            .position(|window| window == b"\r\n")
            .expect("RESP line")
            + self.position;
        let line = std::str::from_utf8(&self.data[self.position..end]).unwrap();
        self.position = end + 2;
        line
    }

    fn value(&mut self) -> String {
        let prefix = self.data[self.position];
        self.position += 1;
        match prefix {
            b'$' => {
                let length = self.line().parse::<i64>().unwrap();
                if length < 0 {
                    return String::new();
                }
                let start = self.position;
                let end = start + length as usize;
                let value = String::from_utf8(self.data[start..end].to_vec()).unwrap();
                self.position = end + 2;
                value
            }
            b':' | b'+' => self.line().to_string(),
            other => panic!("unexpected RESP value prefix {other:?}"),
        }
    }

    fn array(&mut self) -> Vec<String> {
        assert_eq!(self.data[self.position], b'*');
        self.position += 1;
        let count = self.line().parse::<usize>().unwrap();
        (0..count).map(|_| self.value()).collect()
    }
}

fn stats(stream: &mut TcpStream) -> BTreeMap<String, String> {
    let frame = reply(stream, &[b"MEMORY", b"STATS"]);
    let mut cursor = Cursor {
        data: &frame,
        position: 0,
    };
    let values = cursor.array();
    assert_eq!(values.len() % 2, 0, "{values:?}");
    values
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| (pair[0].clone(), pair[1].clone()))
        .collect()
}

fn bulk_text(stream: &mut TcpStream, args: &[&[u8]]) -> String {
    let frame = reply(stream, args);
    let mut cursor = Cursor {
        data: &frame,
        position: 0,
    };
    cursor.value()
}

fn array_text(stream: &mut TcpStream, args: &[&[u8]]) -> Vec<String> {
    let frame = reply(stream, args);
    let mut cursor = Cursor {
        data: &frame,
        position: 0,
    };
    cursor.array()
}

fn contains(haystack: &str, needle: &str) -> bool {
    haystack.contains(needle)
}

#[test]
fn memory_stats_doctor_and_help_are_truthful_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = connect(&server);

        let empty = stats(&mut stream);
        assert_eq!(empty["total.allocated"], "0");
        assert_eq!(empty["peak.allocated"], "0");
        assert_eq!(empty["keys.count"], "0");
        assert_eq!(empty["dataset.bytes"], "0");
        assert_eq!(empty["dataset.percentage"], "0.00");
        assert_eq!(
            empty["rillcache.payload.limit_bytes"],
            (64 * 1024 * 1024).to_string()
        );
        assert_eq!(
            empty["rillcache.payload.scope"],
            "key_and_value_bytes_not_RSS"
        );
        let doctor = bulk_text(&mut stream, &[b"MEMORY", b"DOCTOR"]);
        assert!(contains(&doctor, "payload is empty"), "{doctor}");
        assert!(contains(&doctor, "not RSS"), "{doctor}");
        let help = array_text(&mut stream, &[b"MEMORY", b"HELP"]);
        for item in ["DOCTOR", "STATS", "USAGE <key>", "HELP"] {
            assert!(help.iter().any(|line| line == item), "{help:?}");
        }

        command(
            &mut stream,
            &[b"CONFIG", b"SET", b"maxmemory", b"1000"],
            b"+OK\r\n",
        );
        let value = vec![b'v'; 500];
        command(&mut stream, &[b"SET", b"key", &value], b"+OK\r\n");
        let populated = stats(&mut stream);
        let total = populated["total.allocated"].parse::<u64>().unwrap();
        let peak = populated["peak.allocated"].parse::<u64>().unwrap();
        assert!(total > 0);
        assert_eq!(populated["dataset.bytes"], total.to_string());
        assert_eq!(populated["keys.count"], "1");
        assert_eq!(peak, total);
        assert!(populated["dataset.percentage"].parse::<f64>().unwrap() > 0.0);

        command(&mut stream, &[b"DEL", b"key"], b":1\r\n");
        let removed = stats(&mut stream);
        assert_eq!(removed["total.allocated"], "0");
        assert_eq!(removed["peak.allocated"], peak.to_string());
        assert_eq!(removed["keys.count"], "0");

        command(
            &mut stream,
            &[b"CONFIG", b"SET", b"maxmemory", b"600"],
            b"+OK\r\n",
        );
        let value = vec![b'v'; 550];
        command(&mut stream, &[b"SET", b"big", &value], b"+OK\r\n");
        let doctor = bulk_text(&mut stream, &[b"MEMORY", b"DOCTOR"]);
        assert!(contains(&doctor, "Payload pressure is high"), "{doctor}");
        assert!(contains(&doctor, "not RSS"), "{doctor}");

        command(
            &mut stream,
            &[b"MEMORY", b"STATS", b"extra"],
            b"-ERR wrong number of arguments for 'memory|stats' command\r\n",
        );
        command(
            &mut stream,
            &[b"MEMORY", b"BOGUS"],
            b"-ERR unsupported MEMORY subcommand\r\n",
        );
        server.shutdown().unwrap();
    }
}
