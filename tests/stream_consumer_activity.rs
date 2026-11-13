#![forbid(unsafe_code)]

//! Consumer activity timestamps are persisted through the journal as namespaced
//! ECHO records. These tests live separately so their timed aging does not add
//! concurrent servers to the broader persistence suite.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rillcache::engine::FsyncPolicy;
use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

fn temp_dir(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rillcache-consumer-activity-{label}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn start(dir: &Path) -> ServerHandle {
    let workers = match std::env::var("RILLCACHE_TEST_THREADS").as_deref() {
        Ok("0") | Err(_) => None,
        Ok("1") => Some(1),
        Ok("4") => Some(4),
        Ok(value) => panic!("invalid test worker mode {value}"),
    };
    Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        worker_threads: workers,
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

/// Extract the `inactive` millisecond value for one consumer from XINFO CONSUMERS.
fn inactive_for(raw: &[u8], name: &[u8]) -> i64 {
    let mut at = 0;
    let Resp::Array(consumers) = parse_resp(raw, &mut at) else {
        panic!("XINFO CONSUMERS must be an array")
    };
    for consumer in consumers {
        let Resp::Array(fields) = consumer else {
            panic!("consumer must be an array")
        };
        let (pairs, _) = fields.as_chunks::<2>();
        let mut pairs = pairs.iter();
        let consumer_name = pairs.next().and_then(|pair| match &pair[1] {
            Resp::Bulk(Some(value)) => Some(value.as_slice()),
            _ => None,
        });
        if consumer_name != Some(name) {
            continue;
        }
        for pair in pairs {
            if let (Resp::Bulk(Some(field)), Resp::Int(value)) = (&pair[0], &pair[1])
                && field == b"inactive"
            {
                return *value;
            }
        }
    }
    panic!("consumer {name:?} missing");
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

fn seed_consumers(stream: &mut TcpStream) {
    send(
        stream,
        &[b"XADD", b"s", b"1-0", b"f", b"v"],
        b"$3\r\n1-0\r\n",
    );
    send(stream, &[b"PEXPIRE", b"s", b"600000"], b":1\r\n");
    send(
        stream,
        &[b"XGROUP", b"CREATE", b"s", b"g", b"0"],
        b"+OK\r\n",
    );
    send(
        stream,
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
    let _ = reply(stream);
    send(
        stream,
        &[b"XGROUP", b"CREATECONSUMER", b"s", b"g", b"idle"],
        b":1\r\n",
    );
}

fn assert_aged_and_restored(dir: &Path, before_worker: i64, before_idle: i64) {
    let server = start(dir);
    let mut stream = connect(&server);
    let consumers = call(&mut stream, &[b"XINFO", b"CONSUMERS", b"s", b"g"]);
    let worker = inactive_for(&consumers, b"worker");
    let idle = inactive_for(&consumers, b"idle");
    assert!(
        worker >= before_worker,
        "worker seen_at was reset: before={before_worker} after={worker}"
    );
    assert!(
        idle >= before_idle,
        "idle seen_at was reset: before={before_idle} after={idle}"
    );
    assert!(
        worker < before_worker + 30_000 && idle < before_idle + 30_000,
        "restored timestamps are implausibly far in the past"
    );
    server.shutdown().unwrap();
}

#[test]
fn duplicate_createconsumer_keeps_pending_history_and_does_not_append_to_the_aof() {
    let dir = temp_dir("duplicate-create");
    let history = b"*1\r\n*2\r\n$1\r\ns\r\n*1\r\n*2\r\n$3\r\n1-0\r\n*2\r\n$1\r\nf\r\n$1\r\nv\r\n";
    {
        let server = start(&dir);
        let mut stream = connect(&server);
        seed_consumers(&mut stream);
        let before = std::fs::read(dir.join("test.aof")).unwrap();
        for consumer in [b"worker".as_slice(), b"idle"] {
            send(
                &mut stream,
                &[b"XGROUP", b"CREATECONSUMER", b"s", b"g", consumer],
                b":0\r\n",
            );
        }
        assert_eq!(std::fs::read(dir.join("test.aof")).unwrap(), before);
        send(
            &mut stream,
            &[
                b"XREADGROUP",
                b"GROUP",
                b"g",
                b"worker",
                b"STREAMS",
                b"s",
                b"0",
            ],
            history,
        );
        let consumers = call(&mut stream, &[b"XINFO", b"CONSUMERS", b"s", b"g"]);
        let pending = b"$6\r\nworker\r\n$7\r\npending\r\n:1\r\n";
        assert!(
            consumers
                .windows(pending.len())
                .any(|bytes| bytes == pending)
        );
        server.shutdown().unwrap();
    }
    {
        let server = start(&dir);
        let mut stream = connect(&server);
        send(
            &mut stream,
            &[
                b"XREADGROUP",
                b"GROUP",
                b"g",
                b"worker",
                b"STREAMS",
                b"s",
                b"0",
            ],
            history,
        );
        send(&mut stream, &[b"XACK", b"s", b"g", b"1-0"], b":1\r\n");
        send(
            &mut stream,
            &[
                b"XREADGROUP",
                b"GROUP",
                b"g",
                b"worker",
                b"STREAMS",
                b"s",
                b"0",
            ],
            b"*1\r\n*2\r\n$1\r\ns\r\n*0\r\n",
        );
        server.shutdown().unwrap();
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn consumer_activity_survives_live_journal_restart() {
    let dir = temp_dir("live");
    let (before_worker, before_idle) = {
        let server = start(&dir);
        let mut stream = connect(&server);
        seed_consumers(&mut stream);
        std::thread::sleep(Duration::from_millis(350));
        let consumers = call(&mut stream, &[b"XINFO", b"CONSUMERS", b"s", b"g"]);
        let worker = inactive_for(&consumers, b"worker");
        let idle = inactive_for(&consumers, b"idle");
        assert!(worker >= 200, "worker did not age: {worker} ms");
        assert!(idle >= 200, "idle did not age: {idle} ms");
        server.shutdown().unwrap();
        (worker, idle)
    };
    assert_aged_and_restored(&dir, before_worker, before_idle);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn consumer_activity_survives_rewrite() {
    let dir = temp_dir("rewrite");
    let (before_worker, before_idle) = {
        let server = start(&dir);
        let mut stream = connect(&server);
        seed_consumers(&mut stream);
        send(
            &mut stream,
            &[b"BGREWRITEAOF"],
            b"+Background append only file rewriting started\r\n",
        );
        wait_for_rewrite(&server);
        std::thread::sleep(Duration::from_millis(350));
        let consumers = call(&mut stream, &[b"XINFO", b"CONSUMERS", b"s", b"g"]);
        let worker = inactive_for(&consumers, b"worker");
        let idle = inactive_for(&consumers, b"idle");
        assert!(
            worker >= 200,
            "worker did not age after rewrite: {worker} ms"
        );
        assert!(idle >= 200, "idle did not age after rewrite: {idle} ms");
        server.shutdown().unwrap();
        (worker, idle)
    };
    assert_aged_and_restored(&dir, before_worker, before_idle);
    let _ = std::fs::remove_dir_all(&dir);
}
