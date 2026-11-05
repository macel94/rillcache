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
