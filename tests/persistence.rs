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
        send(&mut stream, &[b"BGREWRITEAOF"], b"+OK\r\n");
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
