#![forbid(unsafe_code)]

//! Deleted/trimmed stream PEL durability. These tests live outside the broad
//! persistence suite so their extra concurrent servers do not lengthen that
//! suite's cooperative-rewrite waits.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rillcache::engine::FsyncPolicy;
use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

fn temp_dir(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rillcache-pel-tombstone-{label}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn start(dir: &Path, appendonly: bool, fsync: FsyncPolicy) -> ServerHandle {
    let workers = match std::env::var("RILLCACHE_TEST_THREADS").as_deref() {
        Ok("0") | Err(_) => None,
        Ok("1") => Some(1),
        Ok("4") => Some(4),
        Ok(value) => panic!("invalid test worker mode {value}"),
    };
    Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        worker_threads: workers,
        appendonly,
        appendfsync: fsync,
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
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream.set_nodelay(true).unwrap();
    stream
}

fn reply(stream: &mut TcpStream) -> Vec<u8> {
    let mut head = Vec::new();
    loop {
        let mut byte = [0u8; 1];
        stream.read_exact(&mut byte).unwrap();
        head.push(byte[0]);
        if head.len() >= 2 && head.ends_with(b"\r\n") {
            break;
        }
    }
    let mut out = head.clone();
    match head[0] {
        b'$' => {
            let length: i64 = std::str::from_utf8(&head[1..head.len() - 2])
                .unwrap()
                .parse()
                .unwrap();
            if length >= 0 {
                let mut body = vec![0u8; length as usize + 2];
                stream.read_exact(&mut body).unwrap();
                out.extend_from_slice(&body);
            }
        }
        b'*' => {
            let count: i64 = std::str::from_utf8(&head[1..head.len() - 2])
                .unwrap()
                .parse()
                .unwrap();
            for _ in 0..count {
                out.extend_from_slice(&reply(stream));
            }
        }
        _ => {}
    }
    out
}

fn call(stream: &mut TcpStream, arguments: &[&[u8]]) -> Vec<u8> {
    let mut bytes = Vec::new();
    encode_request(arguments, &mut bytes);
    stream.write_all(&bytes).unwrap();
    reply(stream)
}

fn send(stream: &mut TcpStream, arguments: &[&[u8]], expected: &[u8]) {
    let mut bytes = Vec::new();
    encode_request(arguments, &mut bytes);
    stream.write_all(&bytes).unwrap();
    let mut actual = vec![0; expected.len()];
    stream.read_exact(&mut actual).unwrap();
    assert_eq!(actual, expected);
}

enum Resp {
    Int(i64),
    Bulk(Option<Vec<u8>>),
    Array(Vec<Resp>),
}

fn parse_resp(raw: &[u8], at: &mut usize) -> Resp {
    let kind = raw[*at];
    *at += 1;
    let end = raw[*at..]
        .windows(2)
        .position(|window| window == b"\r\n")
        .map(|offset| *at + offset)
        .expect("RESP line terminator");
    let line = &raw[*at..end];
    *at = end + 2;
    match kind {
        b'+' | b'-' => Resp::Int(0),
        b':' => Resp::Int(std::str::from_utf8(line).unwrap().parse().unwrap()),
        b'$' => {
            let length: i64 = std::str::from_utf8(line).unwrap().parse().unwrap();
            if length < 0 {
                Resp::Bulk(None)
            } else {
                let body = raw[*at..*at + length as usize].to_vec();
                *at += length as usize + 2;
                Resp::Bulk(Some(body))
            }
        }
        b'*' => {
            let count: i64 = std::str::from_utf8(line).unwrap().parse().unwrap();
            let mut items = Vec::new();
            for _ in 0..count {
                items.push(parse_resp(raw, at));
            }
            Resp::Array(items)
        }
        other => panic!("unexpected RESP type {}", other as char),
    }
}

fn pending_triples(raw: &[u8]) -> Vec<(Vec<u8>, Vec<u8>, i64)> {
    let mut at = 0;
    let Resp::Array(items) = parse_resp(raw, &mut at) else {
        panic!("XPENDING must be an array")
    };
    let entries = match items.first() {
        Some(Resp::Array(_)) => &items,
        _ => match items.get(3) {
            Some(Resp::Array(entries)) => entries,
            _ => return Vec::new(),
        },
    };
    entries
        .iter()
        .map(|entry| {
            let Resp::Array(fields) = entry else {
                panic!("XPENDING entry must be an array")
            };
            let Resp::Bulk(Some(id)) = &fields[0] else {
                panic!("id")
            };
            let Resp::Bulk(Some(consumer)) = &fields[1] else {
                panic!("consumer")
            };
            let Resp::Int(deliveries) = fields[3] else {
                panic!("deliveries")
            };
            (id.clone(), consumer.clone(), deliveries)
        })
        .collect()
}

fn wait_for_rewrite(server: &ServerHandle) {
    let mut stream = connect(server);
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let info = call(&mut stream, &[b"INFO", b"persistence"]);
        let text = String::from_utf8_lossy(&info);
        if text.contains("aof_rewrite_in_progress:0") {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "rewrite did not finish: {text}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn deleted_stream_pending_ids_survive_replay_as_pel_tombstones() {
    let dir = temp_dir("stream-pel-deleted");
    {
        let server = start(&dir, true, FsyncPolicy::Always);
        let mut stream = connect(&server);
        for id in [b"1-0".as_slice(), b"2-0", b"3-0"] {
            send(&mut stream, &[b"XADD", b"s", id, b"f", b"v"], b"");
            let _ = reply(&mut stream);
        }
        send(
            &mut stream,
            &[b"XGROUP", b"CREATE", b"s", b"g", b"0"],
            b"+OK\r\n",
        );
        send(
            &mut stream,
            &[
                b"XREADGROUP",
                b"GROUP",
                b"g",
                b"worker",
                b"STREAMS",
                b"s",
                b">",
            ],
            b"",
        );
        let _ = reply(&mut stream);
        send(
            &mut stream,
            &[
                b"XCLAIM",
                b"s",
                b"g",
                b"other",
                b"0",
                b"2-0",
                b"RETRYCOUNT",
                b"7",
                b"JUSTID",
                b"FORCE",
            ],
            b"*1\r\n$3\r\n2-0\r\n",
        );
        send(&mut stream, &[b"XDEL", b"s", b"2-0", b"3-0"], b":2\r\n");
        send(
            &mut stream,
            &[b"BGREWRITEAOF"],
            b"+Background append only file rewriting started\r\n",
        );
        wait_for_rewrite(&server);
        server.shutdown().unwrap();
    }
    let server = start(&dir, true, FsyncPolicy::Always);
    let mut stream = connect(&server);
    let pending = call(&mut stream, &[b"XPENDING", b"s", b"g", b"-", b"+", b"10"]);
    let triples = pending_triples(&pending);
    assert_eq!(triples.len(), 3, "deleted pending ids must survive replay");
    assert_eq!(triples[0], (b"1-0".to_vec(), b"worker".to_vec(), 1));
    assert_eq!(triples[1], (b"2-0".to_vec(), b"other".to_vec(), 7));
    assert_eq!(triples[2], (b"3-0".to_vec(), b"worker".to_vec(), 1));
    // Placeholder entries are gone, but every PEL tombstone remains ackable.
    send(&mut stream, &[b"XLEN", b"s"], b":1\r\n");
    send(&mut stream, &[b"XRANGE", b"s", b"2-0", b"3-0"], b"*0\r\n");
    send(
        &mut stream,
        &[b"XACK", b"s", b"g", b"2-0", b"3-0"],
        b":2\r\n",
    );
    let pending = call(&mut stream, &[b"XPENDING", b"s", b"g", b"-", b"+", b"10"]);
    let triples = pending_triples(&pending);
    assert_eq!(triples.len(), 1);
    assert_eq!(triples[0].0, b"1-0");
    server.shutdown().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn trimmed_stream_pending_ids_survive_replay_without_reintroducing_payloads() {
    let dir = temp_dir("stream-pel-trimmed");
    {
        let server = start(&dir, true, FsyncPolicy::Always);
        let mut stream = connect(&server);
        for id in [b"1-0".as_slice(), b"2-0", b"3-0", b"4-0", b"5-0"] {
            send(&mut stream, &[b"XADD", b"s", id, b"f", b"v"], b"");
            let _ = reply(&mut stream);
        }
        send(
            &mut stream,
            &[b"XGROUP", b"CREATE", b"s", b"g", b"0"],
            b"+OK\r\n",
        );
        send(
            &mut stream,
            &[
                b"XREADGROUP",
                b"GROUP",
                b"g",
                b"worker",
                b"STREAMS",
                b"s",
                b">",
            ],
            b"",
        );
        let _ = reply(&mut stream);
        send(&mut stream, &[b"XTRIM", b"s", b"MAXLEN", b"3"], b":2\r\n");
        server.shutdown().unwrap();
    }
    let server = start(&dir, true, FsyncPolicy::Always);
    let mut stream = connect(&server);
    send(&mut stream, &[b"XLEN", b"s"], b":3\r\n");
    let pending = call(&mut stream, &[b"XPENDING", b"s", b"g", b"-", b"+", b"10"]);
    let triples = pending_triples(&pending);
    assert_eq!(triples.len(), 5);
    assert_eq!(triples[0].0, b"1-0");
    assert_eq!(triples[4].0, b"5-0");
    send(&mut stream, &[b"XRANGE", b"s", b"1-0", b"2-0"], b"*0\r\n");
    send(
        &mut stream,
        &[b"XACK", b"s", b"g", b"1-0", b"2-0"],
        b":2\r\n",
    );
    let pending = call(&mut stream, &[b"XPENDING", b"s", b"g", b"-", b"+", b"10"]);
    assert_eq!(pending_triples(&pending).len(), 3);
    server.shutdown().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_all_deleted_stream_keeps_its_pel_tombstones_and_empty_length() {
    let dir = temp_dir("all-deleted");
    {
        let server = start(&dir, true, FsyncPolicy::Always);
        let mut stream = connect(&server);
        for id in [b"1-0".as_slice(), b"2-0"] {
            send(&mut stream, &[b"XADD", b"s", id, b"f", b"v"], b"");
            let _ = reply(&mut stream);
        }
        send(
            &mut stream,
            &[b"XGROUP", b"CREATE", b"s", b"g", b"0"],
            b"+OK\r\n",
        );
        send(
            &mut stream,
            &[
                b"XREADGROUP",
                b"GROUP",
                b"g",
                b"worker",
                b"STREAMS",
                b"s",
                b">",
            ],
            b"",
        );
        let _ = reply(&mut stream);
        send(&mut stream, &[b"XDEL", b"s", b"1-0", b"2-0"], b":2\r\n");
        server.shutdown().unwrap();
    }
    let server = start(&dir, true, FsyncPolicy::Always);
    let mut stream = connect(&server);
    send(&mut stream, &[b"XLEN", b"s"], b":0\r\n");
    send(&mut stream, &[b"XRANGE", b"s", b"-", b"+"], b"*0\r\n");
    let pending = call(&mut stream, &[b"XPENDING", b"s", b"g", b"-", b"+", b"10"]);
    let triples = pending_triples(&pending);
    assert_eq!(triples.len(), 2);
    assert_eq!(triples[0].0, b"1-0");
    assert_eq!(triples[1].0, b"2-0");
    send(
        &mut stream,
        &[b"XACK", b"s", b"g", b"1-0", b"2-0"],
        b":2\r\n",
    );
    let pending = call(&mut stream, &[b"XPENDING", b"s", b"g"]);
    assert!(
        pending.starts_with(b"*4\r\n:0\r\n"),
        "empty PEL summary: {pending:?}"
    );
    server.shutdown().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}
