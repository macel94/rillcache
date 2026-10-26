#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

fn start(mode: Option<usize>) -> ServerHandle {
    let server = Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        worker_threads: mode,
        ..Config::default()
    })
    .unwrap();
    server.spawn().unwrap()
}

fn request(stream: &mut TcpStream, args: &[&[u8]]) -> Vec<u8> {
    let mut request = Vec::new();
    encode_request(args, &mut request);
    stream.write_all(&request).unwrap();
    let mut reply = Vec::new();
    let mut remaining = 2 * 1024 * 1024;
    read_frame(stream, &mut reply, &mut remaining, 0);
    reply
}

fn read_frame(stream: &mut TcpStream, output: &mut Vec<u8>, remaining: &mut usize, depth: usize) {
    assert!(depth <= 16, "RESP nesting limit exceeded");
    let mut prefix = [0];
    stream.read_exact(&mut prefix).expect("RESP frame prefix");
    push_bounded(output, remaining, &prefix);
    match prefix[0] {
        b'+' | b'-' | b':' => {
            let line = read_line(stream, output, remaining);
            if prefix[0] == b':' {
                line.parse::<i64>().expect("RESP integer");
            }
        }
        b'$' => {
            let length = read_line(stream, output, remaining)
                .parse::<i64>()
                .expect("RESP bulk length");
            if length == -1 {
                return;
            }
            assert!(
                (0..=2 * 1024 * 1024).contains(&length),
                "RESP bulk limit exceeded"
            );
            let mut body = vec![0; length as usize + 2];
            stream.read_exact(&mut body).expect("complete RESP bulk");
            assert!(body.ends_with(b"\r\n"), "RESP bulk terminator");
            push_bounded(output, remaining, &body);
        }
        b'*' => {
            let count = read_line(stream, output, remaining)
                .parse::<i64>()
                .expect("RESP array length");
            if count == -1 {
                return;
            }
            assert!((0..=1024).contains(&count), "RESP array limit exceeded");
            for _ in 0..count {
                read_frame(stream, output, remaining, depth + 1);
            }
        }
        other => panic!("unexpected RESP type {other:?}"),
    }
}

fn read_line(stream: &mut TcpStream, output: &mut Vec<u8>, remaining: &mut usize) -> String {
    let mut line = Vec::new();
    loop {
        let mut byte = [0];
        stream.read_exact(&mut byte).expect("complete RESP line");
        push_bounded(output, remaining, &byte);
        if byte[0] == b'\n' {
            assert_eq!(line.pop(), Some(b'\r'), "RESP line terminator");
            return String::from_utf8(line).expect("ASCII RESP line");
        }
        assert!(line.len() < 128, "RESP line limit exceeded");
        line.push(byte[0]);
    }
}

fn push_bounded(output: &mut Vec<u8>, remaining: &mut usize, bytes: &[u8]) {
    *remaining = remaining
        .checked_sub(bytes.len())
        .expect("RESP reply limit exceeded");
    output.extend_from_slice(bytes);
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

#[test]
fn command_discovery_has_sorted_truthful_resp2_metadata() {
    for mode in [None, Some(1), Some(4)] {
        let server = start(mode);
        let mut client = connect(&server);
        assert_eq!(request(&mut client, &[b"COMMAND", b"COUNT"]), b":78\r\n");
        let all_info = request(&mut client, &[b"COMMAND", b"INFO"]);
        assert!(all_info.starts_with(b"*78\r\n*6\r\n$6\r\nappend\r\n"));
        assert_eq!(
            request(&mut client, &[b"COMMAND", b"INFO", b"gEt", b"does-not-exist", b"DEL"]),
            b"*3\r\n*6\r\n$3\r\nget\r\n:2\r\n*2\r\n+readonly\r\n+fast\r\n:1\r\n:1\r\n:1\r\n*-1\r\n*6\r\n$3\r\ndel\r\n:-2\r\n*2\r\n+write\r\n+fast\r\n:1\r\n:-1\r\n:1\r\n"
        );
        let expected_names: &[&[u8]] = &[
            b"append",
            b"command",
            b"config",
            b"copy",
            b"dbsize",
            b"decr",
            b"decrby",
            b"del",
            b"echo",
            b"exists",
            b"expire",
            b"expireat",
            b"expiretime",
            b"flushall",
            b"flushdb",
            b"function",
            b"get",
            b"getbit",
            b"getdel",
            b"getex",
            b"getrange",
            b"getset",
            b"hget",
            b"hgetall",
            b"hlen",
            b"hset",
            b"incr",
            b"incrby",
            b"incrbyfloat",
            b"info",
            b"keys",
            b"lcs",
            b"llen",
            b"lpop",
            b"lpush",
            b"lrange",
            b"memory",
            b"mget",
            b"mset",
            b"msetnx",
            b"persist",
            b"pexpire",
            b"pexpireat",
            b"pexpiretime",
            b"ping",
            b"psetex",
            b"pttl",
            b"quit",
            b"randomkey",
            b"rename",
            b"renamenx",
            b"rpop",
            b"rpush",
            b"sadd",
            b"scard",
            b"script",
            b"set",
            b"setbit",
            b"setex",
            b"setnx",
            b"setrange",
            b"sismember",
            b"smembers",
            b"strlen",
            b"substr",
            b"touch",
            b"ttl",
            b"type",
            b"xadd",
            b"xdel",
            b"xgroup",
            b"xinfo",
            b"xlen",
            b"xreadgroup",
            b"zadd",
            b"zcard",
            b"zrange",
            b"zscore",
        ];
        let mut expected_list = Vec::new();
        write!(&mut expected_list, "*{}\r\n", expected_names.len()).unwrap();
        for name in expected_names {
            write!(&mut expected_list, "${}\r\n", name.len()).unwrap();
            expected_list.extend_from_slice(name);
            expected_list.extend_from_slice(b"\r\n");
        }
        assert_eq!(request(&mut client, &[b"COMMAND", b"LIST"]), expected_list);
        let info = request(&mut client, &[b"INFO"]);
        let text = std::str::from_utf8(&info).unwrap();
        assert_eq!(info.first(), Some(&b'$'));
        assert!(text.contains("rillcache_version:0.1.0"));
        assert!(text.contains("rillcache_payload_scope:key_and_value_bytes_not_RSS"));
        assert!(text.contains("db0:keys=0,expires=0"));
        assert!(!text.contains("redis_version:"));
        assert!(!text.contains("used_memory_rss:"));
        server.shutdown().unwrap();
    }
}

#[test]
fn info_reports_payload_and_live_keyspace_and_rejects_unknown_sections() {
    for mode in [None, Some(1), Some(4)] {
        let server = start(mode);
        let mut client = connect(&server);
        assert_eq!(request(&mut client, &[b"SET", b"k", b"abc"]), b"+OK\r\n");
        let memory = request(&mut client, &[b"INFO", b"MEMORY"]);
        assert!(
            std::str::from_utf8(&memory)
                .unwrap()
                .contains("rillcache_payload_bytes:4\r\n")
        );
        let keyspace = request(&mut client, &[b"INFO", b"KEYSPACE"]);
        assert!(
            std::str::from_utf8(&keyspace)
                .unwrap()
                .contains("db0:keys=1,expires=0\r\n")
        );
        assert_eq!(
            request(&mut client, &[b"INFO", b"anything\0SECRET"]),
            b"-ERR unsupported INFO section\r\n"
        );
        assert_eq!(
            request(&mut client, &[b"INFO", b"SERVER", b"MEMORY"]),
            b"-ERR wrong number of arguments for 'info' command\r\n"
        );
        server.shutdown().unwrap();
    }
}

#[test]
fn unsupported_command_options_are_rejected_without_reflecting_input() {
    for mode in [None, Some(1), Some(4)] {
        let server = start(mode);
        let mut client = connect(&server);
        assert_eq!(
            request(
                &mut client,
                &[
                    b"COMMAND",
                    b"LIST",
                    b"FILTERBY",
                    b"MODULE",
                    b"secret\0bytes"
                ]
            ),
            b"-ERR unsupported COMMAND LIST options\r\n"
        );
        assert_eq!(
            request(&mut client, &[b"COMMAND", b"UNKNOWN", b"secret\0bytes"]),
            b"-ERR unsupported COMMAND subcommand\r\n"
        );
        assert_eq!(
            request(&mut client, &[b"COMMAND", b"COUNT", b"secret"]),
            b"-ERR wrong number of arguments for 'command|count' command\r\n"
        );
        assert_eq!(request(&mut client, &[b"PING"]), b"+PONG\r\n");
        server.shutdown().unwrap();
    }
}
