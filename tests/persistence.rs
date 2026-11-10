#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rillcache::engine::FsyncPolicy;
use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

fn temp_dir(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rillcache-persistence-{label}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}

/// Drive the cooperative rewrite to completion through the server's own
/// maintenance tick, bounded and without a fixed readiness sleep.
fn wait_for_rewrite(server: &ServerHandle) {
    let mut stream = connect(server);
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
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

/// Read exactly one RESP reply so a test can compare dynamic values without
/// hard-coding lengths. Covers the simple/integer/bulk/array frames used here.
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

/// Minimal RESP reader for assertions that must ignore wall-clock fields such as
/// XPENDING's idle time while still comparing ids, owners and delivery counts.
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
        // Simple strings and errors are not needed by these assertions.
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
    // Extended XPENDING replies with the entry list directly; the summary form is
    // [count, min, max, consumers].
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

fn call(stream: &mut TcpStream, arguments: &[&[u8]]) -> Vec<u8> {
    let mut bytes = Vec::new();
    encode_request(arguments, &mut bytes);
    stream.write_all(&bytes).unwrap();
    reply(stream)
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

fn send(stream: &mut TcpStream, arguments: &[&[u8]], reply: &[u8]) {
    let mut bytes = Vec::new();
    encode_request(arguments, &mut bytes);
    stream.write_all(&bytes).unwrap();
    let mut actual = vec![0; reply.len()];
    stream.read_exact(&mut actual).unwrap();
    assert_eq!(actual, reply);
}

#[test]
fn append_only_log_survives_restart_with_types_ttl_and_flush() {
    let dir = temp_dir("restart");
    {
        let server = start(&dir, true, FsyncPolicy::Always);
        let mut stream = connect(&server);
        send(&mut stream, &[b"SET", b"str", b"value\xff"], b"+OK\r\n");
        send(&mut stream, &[b"RPUSH", b"list", b"one", b"two"], b":2\r\n");
        send(&mut stream, &[b"SADD", b"set", b"a", b"b"], b":2\r\n");
        send(
            &mut stream,
            &[b"HSET", b"hash", b"field", b"value"],
            b":1\r\n",
        );
        send(
            &mut stream,
            &[b"ZADD", b"zset", b"1.5", b"member"],
            b":1\r\n",
        );
        send(
            &mut stream,
            &[b"XADD", b"stream", b"1-0", b"field", b"value"],
            b"$3\r\n1-0\r\n",
        );
        send(
            &mut stream,
            &[b"XGROUP", b"CREATE", b"stream", b"group", b"0"],
            b"+OK\r\n",
        );
        send(&mut stream, &[b"PEXPIRE", b"str", b"3600000"], b":1\r\n");
        server.shutdown().unwrap();
    }
    let file = dir.join("test.aof");
    assert!(std::fs::metadata(&file).unwrap().len() > 0);

    {
        let server = start(&dir, true, FsyncPolicy::Always);
        let mut stream = connect(&server);
        send(&mut stream, &[b"GET", b"str"], b"$6\r\nvalue\xff\r\n");
        send(
            &mut stream,
            &[b"LRANGE", b"list", b"0", b"-1"],
            b"*2\r\n$3\r\none\r\n$3\r\ntwo\r\n",
        );
        send(
            &mut stream,
            &[b"SMEMBERS", b"set"],
            b"*2\r\n$1\r\na\r\n$1\r\nb\r\n",
        );
        send(
            &mut stream,
            &[b"HGET", b"hash", b"field"],
            b"$5\r\nvalue\r\n",
        );
        send(
            &mut stream,
            &[b"ZSCORE", b"zset", b"member"],
            b"$3\r\n1.5\r\n",
        );
        send(&mut stream, &[b"XLEN", b"stream"], b":1\r\n");
        let ttl = {
            let mut request = Vec::new();
            encode_request(&[b"PTTL", b"str"], &mut request);
            stream.write_all(&request).unwrap();
            let mut header = [0u8; 2];
            stream.read_exact(&mut header).unwrap();
            assert_eq!(&header[..1], b":");
            let mut line = Vec::new();
            let mut byte = [0u8; 1];
            loop {
                stream.read_exact(&mut byte).unwrap();
                line.push(byte[0]);
                if line.ends_with(b"\r\n") {
                    break;
                }
            }
            std::str::from_utf8(&line[..line.len() - 2])
                .unwrap()
                .parse::<i64>()
                .unwrap()
        };
        assert!(ttl > 0 && ttl <= 3_600_000, "unexpected TTL {ttl}");
        send(&mut stream, &[b"FLUSHALL"], b"+OK\r\n");
        server.shutdown().unwrap();
    }

    {
        let server = start(&dir, true, FsyncPolicy::Always);
        let mut stream = connect(&server);
        send(&mut stream, &[b"DBSIZE"], b":0\r\n");
        server.shutdown().unwrap();
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_partial_tail_is_recovered_and_corruption_is_visible() {
    let dir = temp_dir("tail");
    {
        let server = start(&dir, true, FsyncPolicy::No);
        let mut stream = connect(&server);
        send(&mut stream, &[b"SET", b"k", b"v"], b"+OK\r\n");
        server.shutdown().unwrap();
    }
    let file = dir.join("test.aof");
    let complete = std::fs::metadata(&file).unwrap().len();
    let mut output = std::fs::OpenOptions::new()
        .append(true)
        .open(&file)
        .unwrap();
    output.write_all(b"*3\r\n$3\r\nSET\r\n$1\r\nx").unwrap();
    drop(output);
    {
        let server = start(&dir, true, FsyncPolicy::No);
        let mut stream = connect(&server);
        send(&mut stream, &[b"GET", b"k"], b"$1\r\nv\r\n");
        send(&mut stream, &[b"SET", b"after", b"tail"], b"+OK\r\n");
        server.shutdown().unwrap();
    }
    assert!(std::fs::metadata(&file).unwrap().len() > complete);
    let _ = std::fs::remove_dir_all(&dir);

    let dir = temp_dir("corrupt");
    {
        let server = start(&dir, true, FsyncPolicy::No);
        let mut stream = connect(&server);
        send(&mut stream, &[b"SET", b"k", b"v"], b"+OK\r\n");
        server.shutdown().unwrap();
    }
    let file = dir.join("test.aof");
    let mut bytes = std::fs::read(&file).unwrap();
    bytes[0] = b'X';
    std::fs::write(&file, bytes).unwrap();
    assert!(
        Server::bind(Config {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            appendonly: true,
            dir: dir.clone(),
            appendfilename: "test.aof".into(),
            ..Config::default()
        })
        .is_err()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bgrewriteaof_compacts_a_live_log_and_restart_keeps_the_state() {
    let dir = temp_dir("rewrite");
    {
        let server = start(&dir, true, FsyncPolicy::Always);
        let mut stream = connect(&server);
        for index in 0..100 {
            let value = format!("value-{index}");
            send(&mut stream, &[b"SET", b"k", value.as_bytes()], b"+OK\r\n");
        }
        let file = dir.join("test.aof");
        let before = std::fs::metadata(&file).unwrap().len();
        send(
            &mut stream,
            &[b"BGREWRITEAOF"],
            b"+Background append only file rewriting started\r\n",
        );
        wait_for_rewrite(&server);
        let after = std::fs::metadata(&file).unwrap().len();
        assert!(after < before);
        server.shutdown().unwrap();
    }
    {
        let server = start(&dir, true, FsyncPolicy::Always);
        let mut stream = connect(&server);
        send(&mut stream, &[b"GET", b"k"], b"$8\r\nvalue-99\r\n");
        send(&mut stream, &[b"DBSIZE"], b":1\r\n");
        server.shutdown().unwrap();
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn journal_is_off_by_default_and_validation_is_explicit() {
    let dir = temp_dir("default-off");
    {
        let server = start(&dir, false, FsyncPolicy::EverySec);
        let mut stream = connect(&server);
        send(&mut stream, &[b"SET", b"k", b"v"], b"+OK\r\n");
        server.shutdown().unwrap();
    }
    assert!(!dir.join("test.aof").exists());
    for config in [
        Config {
            appendonly: true,
            appendfilename: "a/b.aof".into(),
            dir: dir.clone(),
            ..Config::default()
        },
        Config {
            appendonly: true,
            appendfilename: String::new(),
            dir: dir.clone(),
            ..Config::default()
        },
        Config {
            appendonly: true,
            appendfilename: "ok.aof".into(),
            dir: PathBuf::new(),
            ..Config::default()
        },
    ] {
        assert_eq!(
            Server::bind(config).err().unwrap().kind(),
            std::io::ErrorKind::InvalidInput
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
#[test]
fn blocking_sorted_set_pop_persists_remaining_members_and_ttl() {
    let dir = temp_dir("bzmpop");
    {
        let server = start(&dir, true, FsyncPolicy::Always);
        let mut stream = connect(&server);
        send(
            &mut stream,
            &[b"ZADD", b"z", b"1", b"a", b"2", b"b"],
            b":2\r\n",
        );
        send(&mut stream, &[b"PEXPIRE", b"z", b"3600000"], b":1\r\n");
        send(
            &mut stream,
            &[b"BZMPOP", b"0", b"1", b"z", b"MIN"],
            b"*2\r\n$1\r\nz\r\n*1\r\n*2\r\n$1\r\na\r\n$1\r\n1\r\n",
        );
        server.shutdown().unwrap();
    }
    {
        let server = start(&dir, true, FsyncPolicy::Always);
        let mut stream = connect(&server);
        send(
            &mut stream,
            &[b"ZRANGE", b"z", b"0", b"-1", b"WITHSCORES"],
            b"*2\r\n$1\r\nb\r\n$1\r\n2\r\n",
        );
        let mut pttl = [0u8; 7];
        send_raw(&mut stream, &[b"PTTL", b"z"], &mut pttl);
        assert!(pttl[0] == b':' && pttl[1] != b'-');
        server.shutdown().unwrap();
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn list_move_commands_persist_both_keys_and_ttl() {
    let dir = temp_dir("list-move");
    {
        let server = start(&dir, true, FsyncPolicy::Always);
        let mut stream = connect(&server);
        send(&mut stream, &[b"RPUSH", b"src", b"a", b"b"], b":2\r\n");
        send(&mut stream, &[b"PEXPIRE", b"src", b"3600000"], b":1\r\n");
        send(&mut stream, &[b"RPUSH", b"dst", b"z"], b":1\r\n");
        send(&mut stream, &[b"PEXPIRE", b"dst", b"3600000"], b":1\r\n");
        send(
            &mut stream,
            &[b"BLMOVE", b"src", b"dst", b"LEFT", b"RIGHT", b"0"],
            b"$1\r\na\r\n",
        );
        send(
            &mut stream,
            &[b"LMOVE", b"src", b"dst", b"RIGHT", b"LEFT"],
            b"$1\r\nb\r\n",
        );
        send(&mut stream, &[b"EXISTS", b"src"], b":0\r\n");
        server.shutdown().unwrap();
    }
    {
        let server = start(&dir, true, FsyncPolicy::Always);
        let mut stream = connect(&server);
        send(&mut stream, &[b"EXISTS", b"src"], b":0\r\n");
        send(
            &mut stream,
            &[b"LRANGE", b"dst", b"0", b"-1"],
            b"*3\r\n$1\r\nb\r\n$1\r\nz\r\n$1\r\na\r\n",
        );
        let mut pttl = [0u8; 7];
        send_raw(&mut stream, &[b"PTTL", b"dst"], &mut pttl);
        assert!(pttl[0] == b':' && pttl[1] != b'-');
        server.shutdown().unwrap();
    }
    let _ = std::fs::remove_dir_all(&dir);
}

fn send_raw(stream: &mut TcpStream, arguments: &[&[u8]], reply: &mut [u8]) {
    let mut bytes = Vec::new();
    encode_request(arguments, &mut bytes);
    stream.write_all(&bytes).unwrap();
    stream.read_exact(reply).unwrap();
}
#[test]
fn automatic_rewrite_is_triggered_by_maintenance_and_replays() {
    let dir = temp_dir("auto-rewrite");
    let path = dir.join("test.aof");
    {
        let server = start(&dir, true, FsyncPolicy::Always);
        let mut stream = connect(&server);
        send(
            &mut stream,
            &[
                b"CONFIG",
                b"SET",
                b"auto-aof-rewrite-percentage",
                b"1",
                b"auto-aof-rewrite-min-size",
                b"128",
            ],
            b"+OK\r\n",
        );
        for index in 0..400 {
            let value = format!("value-{index}");
            send(&mut stream, &[b"SET", b"key", value.as_bytes()], b"+OK\r\n");
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while std::fs::metadata(&path)
            .map(|metadata| metadata.len() >= 2048)
            .unwrap_or(true)
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(20));
        }
        let size = std::fs::metadata(&path).unwrap().len();
        assert!(size < 2048, "maintenance auto rewrite left {size} bytes");
        send(&mut stream, &[b"GET", b"key"], b"$9\r\nvalue-399\r\n");
        server.shutdown().unwrap();
    }
    {
        let server = start(&dir, true, FsyncPolicy::Always);
        let mut stream = connect(&server);
        send(&mut stream, &[b"GET", b"key"], b"$9\r\nvalue-399\r\n");
        server.shutdown().unwrap();
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn append_after_rewrite_replays_cleanly() {
    let dir = temp_dir("rewrite-append");
    {
        let server = start(&dir, true, FsyncPolicy::Always);
        let mut stream = connect(&server);
        send(&mut stream, &[b"SET", b"a", b"1"], b"+OK\r\n");
        send(
            &mut stream,
            &[b"BGREWRITEAOF"],
            b"+Background append only file rewriting started\r\n",
        );
        wait_for_rewrite(&server);
        // Without seeking to the snapshot end, this write lands at offset zero
        // and overwrites the compacted file.
        send(&mut stream, &[b"SET", b"b", b"2"], b"+OK\r\n");
        send(&mut stream, &[b"GET", b"a"], b"$1\r\n1\r\n");
        send(&mut stream, &[b"GET", b"b"], b"$1\r\n2\r\n");
        server.shutdown().unwrap();
    }
    {
        let server = start(&dir, true, FsyncPolicy::Always);
        let mut stream = connect(&server);
        send(&mut stream, &[b"GET", b"a"], b"$1\r\n1\r\n");
        send(&mut stream, &[b"GET", b"b"], b"$1\r\n2\r\n");
        server.shutdown().unwrap();
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rewrite_with_concurrent_mutations_keeps_the_exact_final_state() {
    let dir = temp_dir("rewrite-concurrent");
    let server = start(&dir, true, FsyncPolicy::EverySec);
    let mut reader = connect(&server);
    for index in 0..1_000 {
        send(
            &mut reader,
            &[b"SET", format!("cold:{index}").as_bytes(), b"cold"],
            b"+OK\r\n",
        );
    }
    let writer_addr = server.local_addr();
    let writer = std::thread::spawn(move || {
        let mut stream = TcpStream::connect(writer_addr).unwrap();
        stream.set_nodelay(true).unwrap();
        for round in 0..300u32 {
            let key = format!("hot:{}", round % 16);
            call(
                &mut stream,
                &[b"SET", key.as_bytes(), format!("value-{round}").as_bytes()],
            );
            call(
                &mut stream,
                &[b"RPUSH", b"shared:list", format!("item-{round}").as_bytes()],
            );
            call(&mut stream, &[b"LTRIM", b"shared:list", b"-50", b"-1"]);
            call(
                &mut stream,
                &[
                    b"HSET",
                    b"shared:hash",
                    format!("f{}", round % 8).as_bytes(),
                    format!("v{round}").as_bytes(),
                ],
            );
            let transient = format!("transient:{}", round % 8);
            call(&mut stream, &[b"SET", transient.as_bytes(), b"live"]);
            if round % 3 == 0 {
                call(&mut stream, &[b"DEL", transient.as_bytes()]);
            }
        }
    });
    send(
        &mut reader,
        &[b"BGREWRITEAOF"],
        b"+Background append only file rewriting started\r\n",
    );
    wait_for_rewrite(&server);
    writer.join().unwrap();

    let mut expected: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    for index in 0..16 {
        let key = format!("hot:{index}");
        expected.push((
            key.clone().into_bytes(),
            call(&mut reader, &[b"GET", key.as_bytes()]),
        ));
    }
    expected.push((
        b"shared:list".to_vec(),
        call(&mut reader, &[b"LRANGE", b"shared:list", b"0", b"-1"]),
    ));
    expected.push((
        b"shared:hash".to_vec(),
        call(&mut reader, &[b"HGETALL", b"shared:hash"]),
    ));
    for index in 0..8 {
        let key = format!("transient:{index}");
        expected.push((
            key.clone().into_bytes(),
            call(&mut reader, &[b"GET", key.as_bytes()]),
        ));
    }
    for index in 0..1_000 {
        let key = format!("cold:{index}");
        expected.push((
            key.clone().into_bytes(),
            call(&mut reader, &[b"GET", key.as_bytes()]),
        ));
    }
    server.shutdown().unwrap();

    // Replay the compacted journal and compare every tracked key exactly.
    let server = start(&dir, true, FsyncPolicy::Always);
    let mut stream = connect(&server);
    for (key, value) in &expected {
        let actual = if key.starts_with(b"shared:list") {
            call(&mut stream, &[b"LRANGE", b"shared:list", b"0", b"-1"])
        } else if key.starts_with(b"shared:hash") {
            call(&mut stream, &[b"HGETALL", b"shared:hash"])
        } else {
            call(&mut stream, &[b"GET", key])
        };
        assert_eq!(&actual, value, "key {key:?} differs after rewrite+restart");
    }
    server.shutdown().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn stream_consumers_and_pending_delivery_state_survive_restart() {
    let dir = temp_dir("stream-pel");
    let (expected_consumers, expected_pending) = {
        let server = start(&dir, true, FsyncPolicy::Always);
        let mut stream = connect(&server);
        send(
            &mut stream,
            &[b"XADD", b"s", b"1-0", b"f", b"a"],
            b"$3\r\n1-0\r\n",
        );
        send(
            &mut stream,
            &[b"XADD", b"s", b"2-0", b"f", b"b"],
            b"$3\r\n2-0\r\n",
        );
        send(
            &mut stream,
            &[b"XADD", b"s", b"3-0", b"f", b"c"],
            b"$3\r\n3-0\r\n",
        );
        send(
            &mut stream,
            &[b"XGROUP", b"CREATE", b"s", b"g", b"0"],
            b"+OK\r\n",
        );
        // First consumer reads two entries, second reads one, then one pending
        // entry is claimed across consumers with an explicit retry count.
        send(
            &mut stream,
            &[
                b"XREADGROUP",
                b"GROUP",
                b"g",
                b"alice",
                b"COUNT",
                b"2",
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
                b"XREADGROUP",
                b"GROUP",
                b"g",
                b"bob",
                b"COUNT",
                b"1",
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
                b"carol",
                b"0",
                b"2-0",
                b"RETRYCOUNT",
                b"9",
                b"JUSTID",
                b"FORCE",
            ],
            b"*1\r\n$3\r\n2-0\r\n",
        );
        // A consumer with no pending entries must also survive.
        send(
            &mut stream,
            &[b"XGROUP", b"CREATECONSUMER", b"s", b"g", b"idle"],
            b":1\r\n",
        );
        let consumers = call(&mut stream, &[b"XINFO", b"CONSUMERS", b"s", b"g"]);
        let pending = call(&mut stream, &[b"XPENDING", b"s", b"g", b"-", b"+", b"10"]);
        server.shutdown().unwrap();
        (consumers, pending)
    };
    let server = start(&dir, true, FsyncPolicy::Always);
    let mut stream = connect(&server);
    let consumers = call(&mut stream, &[b"XINFO", b"CONSUMERS", b"s", b"g"]);
    let pending = call(&mut stream, &[b"XPENDING", b"s", b"g", b"-", b"+", b"10"]);
    // `inactive` and pending idle time are wall-clock dependent; compare the
    // durable fields instead.
    let durable = |raw: &[u8]| {
        let text = String::from_utf8_lossy(raw).into_owned();
        let mut names: Vec<String> = text
            .split("\r\n")
            .filter(|line| {
                line.starts_with("alice")
                    || line.starts_with("bob")
                    || line.starts_with("carol")
                    || line.starts_with("idle")
            })
            .map(str::to_owned)
            .collect();
        names.sort();
        names
    };
    assert_eq!(
        durable(&consumers),
        durable(&expected_consumers),
        "consumers differ"
    );
    assert_eq!(
        pending_triples(&pending),
        pending_triples(&expected_pending),
        "pending ids, owners or delivery counts differ after restart"
    );
    // The replayed PEL still supports acknowledgement and claiming.
    let ack = call(&mut stream, &[b"XACK", b"s", b"g", b"1-0", b"2-0", b"3-0"]);
    assert_eq!(
        ack, b":3\r\n",
        "all replayed pending entries must be ackable"
    );
    let empty = call(&mut stream, &[b"XPENDING", b"s", b"g"]);
    assert_eq!(empty, b"*4\r\n:0\r\n$-1\r\n$-1\r\n*0\r\n");
    server.shutdown().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn journal_checksum_detects_valid_resp_corruption_and_legacy_logs_upgrade_on_append() {
    // New journals include standard RESP ECHO checksum records. Flip a value
    // byte while keeping RESP framing valid: replay must detect it before serving.
    let dir = temp_dir("checksum-corrupt");
    {
        let server = start(&dir, true, FsyncPolicy::Always);
        let mut stream = connect(&server);
        send(&mut stream, &[b"SET", b"k", b"v"], b"+OK\r\n");
        server.shutdown().unwrap();
    }
    let file = dir.join("test.aof");
    let mut bytes = std::fs::read(&file).unwrap();
    assert!(
        bytes
            .windows(27)
            .any(|window| window == b"__RILLCACHE_AOF_CRC32_V1__:")
    );
    let value_at = bytes
        .windows(7)
        .position(|window| window == b"$1\r\nv\r\n")
        .unwrap()
        + 4;
    bytes[value_at] = b"x"[0];
    std::fs::write(&file, bytes).unwrap();
    assert!(
        Server::bind(Config {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            appendonly: true,
            dir: dir.clone(),
            appendfilename: "test.aof".into(),
            ..Config::default()
        })
        .is_err(),
        "valid-RESP data corruption must fail closed on checksum mismatch"
    );
    let _ = std::fs::remove_dir_all(&dir);

    // Old journals remain readable. The first new append starts a checksummed
    // segment after the legacy RESP stream; subsequent restarts verify it.
    let dir = temp_dir("checksum-upgrade");
    let file = dir.join("test.aof");
    let mut legacy = Vec::new();
    encode_request(&[b"SET", b"old", b"value"], &mut legacy);
    std::fs::write(&file, legacy).unwrap();
    {
        let server = start(&dir, true, FsyncPolicy::Always);
        let mut stream = connect(&server);
        send(&mut stream, &[b"GET", b"old"], b"$5\r\nvalue\r\n");
        send(&mut stream, &[b"SET", b"new", b"checked"], b"+OK\r\n");
        server.shutdown().unwrap();
    }
    let upgraded = std::fs::read(&file).unwrap();
    assert!(
        upgraded
            .windows(27)
            .any(|window| window == b"__RILLCACHE_AOF_CRC32_V1__:")
    );
    let record_marker = b"__RILLCACHE_RECORD_CRC32_V1__:";
    assert!(
        upgraded
            .windows(record_marker.len())
            .any(|window| window == record_marker)
    );
    {
        let server = start(&dir, true, FsyncPolicy::Always);
        let mut stream = connect(&server);
        send(&mut stream, &[b"GET", b"old"], b"$5\r\nvalue\r\n");
        send(&mut stream, &[b"GET", b"new"], b"$7\r\nchecked\r\n");
        server.shutdown().unwrap();
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_corrupted_checksum_marker_fails_closed_instead_of_becoming_legacy_echo() {
    let dir = temp_dir("checksum-marker-corrupt");
    {
        let server = start(&dir, true, FsyncPolicy::Always);
        let mut stream = connect(&server);
        send(&mut stream, &[b"SET", b"k", b"v"], b"+OK\r\n");
        server.shutdown().unwrap();
    }
    let path = dir.join("test.aof");
    let mut bytes = std::fs::read(&path).unwrap();
    let marker = b"__RILLCACHE_RECORD_CRC32_V1__:";
    let at = bytes
        .windows(marker.len())
        .position(|window| window == marker)
        .expect("checksum marker");
    // Flip a CRC hex digit without disturbing RESP framing or marker syntax.
    let crc_start = bytes[at..]
        .iter()
        .position(|byte| *byte == b':')
        .map(|offset| at + offset + 1)
        .unwrap();
    bytes[crc_start] = if bytes[crc_start] == b'0' { b'1' } else { b'0' };
    std::fs::write(&path, bytes).unwrap();
    assert!(
        Server::bind(Config {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            appendonly: true,
            dir: dir.clone(),
            appendfilename: "test.aof".into(),
            ..Config::default()
        })
        .is_err(),
        "a complete but corrupted checksum marker must fail closed"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn torn_checksummed_transaction_tail_is_discarded_as_a_whole() {
    let dir = temp_dir("checksum-tail");
    {
        let server = start(&dir, true, FsyncPolicy::Always);
        let mut stream = connect(&server);
        send(&mut stream, &[b"SET", b"first", b"one"], b"+OK\r\n");
        send(&mut stream, &[b"SET", b"second", b"two"], b"+OK\r\n");
        server.shutdown().unwrap();
    }
    let file = dir.join("test.aof");
    let mut bytes = std::fs::read(&file).unwrap();
    let marker = b"__RILLCACHE_RECORD_CRC32_V1__:";
    let positions: Vec<usize> = bytes
        .windows(marker.len())
        .enumerate()
        .filter_map(|(index, window)| (window == marker).then_some(index))
        .collect();
    assert!(
        positions.len() >= 2,
        "expected one checksum per append transaction"
    );
    // Keep the second SET fully written but truncate in its trailing checksum
    // marker: recovery must stop at the preceding complete transaction.
    bytes.truncate(positions[1] + marker.len() / 2);
    std::fs::write(&file, bytes).unwrap();
    {
        let server = start(&dir, true, FsyncPolicy::Always);
        let mut stream = connect(&server);
        send(&mut stream, &[b"GET", b"first"], b"$3\r\none\r\n");
        send(&mut stream, &[b"GET", b"second"], b"$-1\r\n");
        send(&mut stream, &[b"SET", b"after", b"recovery"], b"+OK\r\n");
        server.shutdown().unwrap();
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn empty_stream_group_and_consumer_metadata_survive_restart() {
    let dir = temp_dir("stream-empty-group");
    {
        let server = start(&dir, true, FsyncPolicy::Always);
        let mut stream = connect(&server);
        send(
            &mut stream,
            &[b"XGROUP", b"CREATE", b"s", b"g", b"0", b"MKSTREAM"],
            b"+OK\r\n",
        );
        send(
            &mut stream,
            &[b"XGROUP", b"CREATECONSUMER", b"s", b"g", b"idle"],
            b":1\r\n",
        );
        server.shutdown().unwrap();
    }
    let server = start(&dir, true, FsyncPolicy::Always);
    let mut stream = connect(&server);
    send(&mut stream, &[b"XLEN", b"s"], b":0\r\n");
    let pending = call(&mut stream, &[b"XPENDING", b"s", b"g"]);
    assert!(
        pending.windows(4).any(|window| window == b":0\r\n"),
        "an empty group must have no pending entries: {pending:?}"
    );
    let consumers = call(&mut stream, &[b"XINFO", b"CONSUMERS", b"s", b"g"]);
    assert!(
        consumers.windows(4).any(|window| window == b"idle"),
        "recreated consumer missing: {consumers:?}"
    );
    server.shutdown().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}
