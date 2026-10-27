#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rillcache::engine::FsyncPolicy;
use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

fn start(workers: Option<usize>, dir: Option<&Path>) -> ServerHandle {
    let (appendonly, appendfsync, dir) = match dir {
        Some(dir) => (true, FsyncPolicy::Always, dir.to_path_buf()),
        None => (false, FsyncPolicy::EverySec, PathBuf::from(".")),
    };
    Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        worker_threads: workers,
        appendonly,
        appendfsync,
        dir,
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

fn bulk_text(reply: &[u8]) -> String {
    assert_eq!(reply.first(), Some(&b'$'), "{reply:?}");
    let header_end = reply
        .windows(2)
        .position(|window| window == b"\r\n")
        .unwrap();
    let length = std::str::from_utf8(&reply[1..header_end])
        .unwrap()
        .parse::<usize>()
        .unwrap();
    std::str::from_utf8(&reply[header_end + 2..header_end + 2 + length])
        .unwrap()
        .to_string()
}

fn field<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    text.lines().find_map(|line| {
        line.strip_prefix(name)?
            .strip_prefix(':')
            .map(|value| value.trim_end_matches('\r'))
    })
}

#[test]
fn persistence_info_reports_disabled_state_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers, None);
        let mut stream = connect(&server);
        let mut request = Vec::new();
        encode_request(&[b"INFO", b"persistence"], &mut request);
        stream.write_all(&request).unwrap();
        let text = bulk_text(&read_frame(&mut stream, 0));
        assert!(text.starts_with("# Persistence\r\n"), "{text}");
        assert_eq!(field(&text, "loading"), Some("0"), "{text}");
        assert_eq!(field(&text, "aof_enabled"), Some("0"), "{text}");
        assert!(field(&text, "aof_current_size").is_none(), "{text}");
        assert!(field(&text, "rdb_last_save_time").is_none(), "{text}");
        server.shutdown().unwrap();
    }
}

#[test]
fn config_reports_startup_journal_mode_and_rejects_startup_only_changes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers, None);
        let mut stream = connect(&server);
        command(
            &mut stream,
            &[b"CONFIG", b"GET", b"appendonly"],
            b"*2\r\n$10\r\nappendonly\r\n$2\r\nno\r\n",
        );
        command(
            &mut stream,
            &[b"CONFIG", b"GET", b"appendfilename"],
            b"*2\r\n$14\r\nappendfilename\r\n$8\r\ntest.aof\r\n",
        );
        command(
            &mut stream,
            &[b"CONFIG", b"GET", b"appendfsync"],
            b"*2\r\n$11\r\nappendfsync\r\n$8\r\neverysec\r\n",
        );
        command(
            &mut stream,
            &[b"CONFIG", b"SET", b"appendfsync", b"ALWAYS"],
            b"+OK\r\n",
        );
        command(
            &mut stream,
            &[b"CONFIG", b"GET", b"appendfsync"],
            b"*2\r\n$11\r\nappendfsync\r\n$6\r\nalways\r\n",
        );
        let reply = {
            let mut request = Vec::new();
            encode_request(&[b"CONFIG", b"SET", b"appendonly", b"yes"], &mut request);
            stream.write_all(&request).unwrap();
            read_frame(&mut stream, 0)
        };
        assert!(
            reply.starts_with(b"-ERR appendonly and appendfilename"),
            "{reply:?}"
        );
        server.shutdown().unwrap();
    }
}

#[test]
fn config_updates_appendfsync_on_a_live_journal_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let dir: PathBuf = std::env::temp_dir().join(format!(
            "rillcache-aof-config-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        {
            let server = start(workers, Some(&dir));
            let mut stream = connect(&server);
            command(
                &mut stream,
                &[b"CONFIG", b"GET", b"appendonly"],
                b"*2\r\n$10\r\nappendonly\r\n$3\r\nyes\r\n",
            );
            command(
                &mut stream,
                &[b"CONFIG", b"GET", b"appendfilename"],
                b"*2\r\n$14\r\nappendfilename\r\n$8\r\ntest.aof\r\n",
            );
            command(
                &mut stream,
                &[b"CONFIG", b"GET", b"appendfsync"],
                b"*2\r\n$11\r\nappendfsync\r\n$6\r\nalways\r\n",
            );
            command(
                &mut stream,
                &[b"CONFIG", b"SET", b"appendfsync", b"no"],
                b"+OK\r\n",
            );
            command(&mut stream, &[b"SET", b"k", b"v"], b"+OK\r\n");
            command(
                &mut stream,
                &[b"CONFIG", b"SET", b"appendfsync", b"always"],
                b"+OK\r\n",
            );
            command(&mut stream, &[b"SET", b"k", b"v2"], b"+OK\r\n");
            command(
                &mut stream,
                &[b"CONFIG", b"GET", b"appendfsync"],
                b"*2\r\n$11\r\nappendfsync\r\n$6\r\nalways\r\n",
            );
            server.shutdown().unwrap();
        }
        let server = start(Some(1), Some(&dir));
        let mut stream = connect(&server);
        command(&mut stream, &[b"GET", b"k"], b"$2\r\nv2\r\n");
        server.shutdown().unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
#[test]
fn persistence_info_reports_live_journal_state_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let dir: PathBuf = std::env::temp_dir().join(format!(
            "rillcache-persistence-info-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        {
            let server = start(workers, Some(&dir));
            let mut stream = connect(&server);
            command(&mut stream, &[b"SET", b"k", b"v"], b"+OK\r\n");

            let mut request = Vec::new();
            encode_request(&[b"INFO", b"persistence"], &mut request);
            stream.write_all(&request).unwrap();
            let text = bulk_text(&read_frame(&mut stream, 0));
            assert_eq!(field(&text, "aof_enabled"), Some("1"), "{text}");
            assert_eq!(field(&text, "aof_rewrite_in_progress"), Some("0"), "{text}");
            assert_eq!(field(&text, "aof_last_write_status"), Some("ok"), "{text}");
            assert_eq!(
                field(&text, "aof_last_bgrewrite_status"),
                Some("ok"),
                "{text}"
            );
            let size = field(&text, "aof_current_size")
                .and_then(|value| value.parse::<u64>().ok())
                .expect("AOF size");
            assert!(size > 0, "{text}");

            command(&mut stream, &[b"BGREWRITEAOF"], b"+OK\r\n");
            let mut request = Vec::new();
            encode_request(&[b"INFO", b"all"], &mut request);
            stream.write_all(&request).unwrap();
            let all = bulk_text(&read_frame(&mut stream, 0));
            assert!(all.contains("# Persistence\r\n"), "{all}");
            assert_eq!(
                field(&all, "aof_last_bgrewrite_status"),
                Some("ok"),
                "{all}"
            );
            assert_eq!(field(&all, "aof_last_write_status"), Some("ok"), "{all}");

            server.shutdown().unwrap();
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
