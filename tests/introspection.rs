#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

#[test]
fn time_and_object_introspection_work_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = connect(&server);
        let time = reply(&mut stream, &[b"TIME"]);
        assert!(time.starts_with(b"*2\r\n$"));
        let text = String::from_utf8_lossy(&time);
        let pieces: Vec<&str> = text.split("\r\n").collect();
        let seconds = pieces[2].parse::<u64>().unwrap();
        let micros = pieces[4].parse::<u32>().unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert!(seconds.abs_diff(now) <= 5);
        assert!(micros < 1_000_000);
        command(
            &mut stream,
            &[b"TIME", b"extra"],
            b"-ERR wrong number of arguments for 'time' command\r\n",
        );

        command(&mut stream, &[b"SET", b"s", b"v"], b"+OK\r\n");
        command(
            &mut stream,
            &[b"OBJECT", b"ENCODING", b"s"],
            b"$3\r\nraw\r\n",
        );
        command(&mut stream, &[b"OBJECT", b"REFCOUNT", b"s"], b":1\r\n");
        command(
            &mut stream,
            &[b"OBJECT", b"ENCODING", b"missing"],
            b"$-1\r\n",
        );
        command(
            &mut stream,
            &[b"OBJECT", b"REFCOUNT", b"missing"],
            b"$-1\r\n",
        );
        command(&mut stream, &[b"RPUSH", b"small-list", b"a"], b":1\r\n");
        command(
            &mut stream,
            &[b"OBJECT", b"ENCODING", b"small-list"],
            b"$8\r\nlistpack\r\n",
        );
        command(
            &mut stream,
            &[b"CONFIG", b"SET", b"list-max-listpack-size", b"1"],
            b"+OK\r\n",
        );
        command(&mut stream, &[b"RPUSH", b"big-list", b"a", b"b"], b":2\r\n");
        command(
            &mut stream,
            &[b"OBJECT", b"ENCODING", b"big-list"],
            b"$9\r\nquicklist\r\n",
        );
        command(
            &mut stream,
            &[b"CONFIG", b"SET", b"list-max-listpack-size", b"-2"],
            b"+OK\r\n",
        );
        command(&mut stream, &[b"SADD", b"ints", b"1", b"2"], b":2\r\n");
        command(
            &mut stream,
            &[b"OBJECT", b"ENCODING", b"ints"],
            b"$6\r\nintset\r\n",
        );
        command(
            &mut stream,
            &[b"CONFIG", b"SET", b"set-max-listpack-entries", b"1"],
            b"+OK\r\n",
        );
        command(&mut stream, &[b"SADD", b"big-set", b"x", b"y"], b":2\r\n");
        command(
            &mut stream,
            &[b"OBJECT", b"ENCODING", b"big-set"],
            b"$9\r\nhashtable\r\n",
        );
        command(
            &mut stream,
            &[b"CONFIG", b"SET", b"set-max-listpack-entries", b"128"],
            b"+OK\r\n",
        );
        command(
            &mut stream,
            &[b"HSET", b"small-hash", b"f", b"v"],
            b":1\r\n",
        );
        command(
            &mut stream,
            &[b"OBJECT", b"ENCODING", b"small-hash"],
            b"$8\r\nlistpack\r\n",
        );
        command(
            &mut stream,
            &[b"ZADD", b"small-zset", b"1", b"a"],
            b":1\r\n",
        );
        command(
            &mut stream,
            &[b"OBJECT", b"ENCODING", b"small-zset"],
            b"$8\r\nlistpack\r\n",
        );
        for index in 0..200 {
            let value = format!("member-{index}");
            command(
                &mut stream,
                &[
                    b"ZADD",
                    b"big-zset",
                    index.to_string().as_bytes(),
                    value.as_bytes(),
                ],
                b":1\r\n",
            );
        }
        command(
            &mut stream,
            &[b"OBJECT", b"ENCODING", b"big-zset"],
            b"$8\r\nskiplist\r\n",
        );
        command(
            &mut stream,
            &[b"XADD", b"stream", b"1-0", b"f", b"v"],
            b"$3\r\n1-0\r\n",
        );
        command(
            &mut stream,
            &[b"OBJECT", b"ENCODING", b"stream"],
            b"$6\r\nstream\r\n",
        );
        command(
            &mut stream,
            &[b"SET", b"expired", b"v", b"PX", b"1"],
            b"+OK\r\n",
        );
        std::thread::sleep(Duration::from_millis(5));
        command(
            &mut stream,
            &[b"OBJECT", b"ENCODING", b"expired"],
            b"$-1\r\n",
        );

        command(
            &mut stream,
            &[b"OBJECT", b"FREQ", b"s"],
            b"-ERR An LFU maxmemory policy is not selected, access frequency not tracked. Please note that when switching between maxmemory policies at runtime LFU and LRU data will take some time to adjust.\r\n",
        );
        command(
            &mut stream,
            &[b"OBJECT", b"IDLETIME", b"s"],
            b"-ERR OBJECT IDLETIME is not supported by this bounded server: read-idle time is not tracked\r\n",
        );
        let help = reply(&mut stream, &[b"OBJECT", b"HELP"]);
        assert!(help.starts_with(b"*"));
        assert!(contains(&help, b"ENCODING <key>"));
        command(
            &mut stream,
            &[b"OBJECT", b"BOGUS", b"s"],
            b"-ERR unknown subcommand or wrong number of arguments for 'BOGUS'. Try OBJECT HELP.\r\n",
        );
        command(
            &mut stream,
            &[b"OBJECT", b"ENCODING"],
            b"-ERR unknown subcommand or wrong number of arguments for 'ENCODING'. Try OBJECT HELP.\r\n",
        );
        command(
            &mut stream,
            &[b"OBJECT"],
            b"-ERR wrong number of arguments for 'object' command\r\n",
        );

        command(&mut stream, &[b"MULTI"], b"+OK\r\n");
        command(&mut stream, &[b"TIME"], b"+QUEUED\r\n");
        command(&mut stream, &[b"OBJECT", b"ENCODING", b"s"], b"+QUEUED\r\n");
        let exec = reply(&mut stream, &[b"EXEC"]);
        assert!(exec.starts_with(b"*2\r\n*2\r\n$"));

        server.shutdown().unwrap();
    }
}
