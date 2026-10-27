#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use rillcache::engine::NotifyFlags;
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

fn start_notify() -> ServerHandle {
    Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        notify_keyspace_events: NotifyFlags::parse(b"Kg").unwrap(),
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

fn frame(parts: &[&[u8]]) -> Vec<u8> {
    let mut output = format!("*{}\r\n", parts.len()).into_bytes();
    for part in parts {
        output.extend_from_slice(format!("${}\r\n", part.len()).as_bytes());
        output.extend_from_slice(part);
        output.extend_from_slice(b"\r\n");
    }
    output
}

#[test]
fn unlink_deletes_mixed_types_and_reports_the_removed_count_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = connect(&server);

        command(&mut stream, &[b"SET", b"str", b"v"], b"+OK\r\n");
        command(&mut stream, &[b"RPUSH", b"list", b"a", b"b"], b":2\r\n");
        command(&mut stream, &[b"HSET", b"hash", b"f", b"v"], b":1\r\n");
        command(&mut stream, &[b"ZADD", b"zset", b"1", b"m"], b":1\r\n");
        command(&mut stream, &[b"PEXPIRE", b"str", b"100000"], b":1\r\n");
        command(
            &mut stream,
            &[b"UNLINK", b"str", b"list", b"missing", b"str"],
            b":2\r\n",
        );
        command(
            &mut stream,
            &[b"EXISTS", b"str", b"list", b"hash", b"zset"],
            b":2\r\n",
        );
        command(&mut stream, &[b"UNLINK", b"hash", b"zset"], b":2\r\n");
        command(&mut stream, &[b"DBSIZE"], b":0\r\n");
        command(
            &mut stream,
            &[b"UNLINK"],
            b"-ERR wrong number of arguments for 'unlink' command\r\n",
        );
        command(
            &mut stream,
            &[b"COMMAND", b"GETKEYS", b"UNLINK", b"a", b"b"],
            b"*2\r\n$1\r\na\r\n$1\r\nb\r\n",
        );

        // Expired keys are not counted, matching DEL.
        command(&mut stream, &[b"SET", b"expired", b"v"], b"+OK\r\n");
        command(&mut stream, &[b"PEXPIRE", b"expired", b"1"], b":1\r\n");
        std::thread::sleep(Duration::from_millis(5));
        command(&mut stream, &[b"UNLINK", b"expired"], b":0\r\n");

        command(&mut stream, &[b"SET", b"multi", b"v"], b"+OK\r\n");
        command(&mut stream, &[b"MULTI"], b"+OK\r\n");
        command(&mut stream, &[b"UNLINK", b"multi"], b"+QUEUED\r\n");
        command(&mut stream, &[b"EXEC"], b"*1\r\n:1\r\n");
        command(&mut stream, &[b"EXISTS", b"multi"], b":0\r\n");

        server.shutdown().unwrap();
    }
}

#[test]
fn unlink_emits_del_notifications_only_for_removed_keys() {
    let server = start_notify();
    let mut writer = connect(&server);
    command(&mut writer, &[b"SET", b"k", b"v"], b"+OK\r\n");
    let mut subscriber = connect(&server);
    let mut request = Vec::new();
    encode_request(&[b"SUBSCRIBE", b"__keyspace@0__:k"], &mut request);
    subscriber.write_all(&request).unwrap();
    let confirmation = read_frame(&mut subscriber, 0);
    assert!(confirmation.starts_with(b"*3\r\n"), "{confirmation:?}");

    command(&mut writer, &[b"UNLINK", b"k"], b":1\r\n");
    assert_eq!(
        read_frame(&mut subscriber, 0),
        frame(&[b"message", b"__keyspace@0__:k", b"del"])
    );
    // Missing keys produce no notification; a PING acts as an ordered barrier.
    command(&mut writer, &[b"UNLINK", b"k"], b":0\r\n");
    command(&mut writer, &[b"PING"], b"+PONG\r\n");
    command(&mut writer, &[b"UNLINK", b"k"], b":0\r\n");

    server.shutdown().unwrap();
}
