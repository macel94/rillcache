#![forbid(unsafe_code)]

//! Measured rewrite pause budget and byte/keys progress.
//!
//! The accepted pause SLO is recorded in `docs/persistence.md`: a cooperative
//! rewrite must answer `BGREWRITEAOF` inside its key-list materialisation budget,
//! keep serving commands while it snapshots, and report completed keys and bytes
//! until it commits. These measurements cover the start pause and the worst
//! command stall with concurrent writes at two keyspace sizes, so a rewrite that
//! starts blocking again fails here.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rillcache::engine::FsyncPolicy;
use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

/// Accepted start-pause SLO for the measured keyspace sizes (see persistence.md).
const START_PAUSE_BUDGET: Duration = Duration::from_millis(1000);
/// Accepted worst command stall while the snapshot is written.
const CONCURRENT_STALL_BUDGET: Duration = Duration::from_millis(1000);
/// Bounded overall completion for the measured data set.
const COMPLETION_BUDGET: Duration = Duration::from_secs(60);

fn temp_dir(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rillcache-rewrite-pause-{label}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
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
        appendfilename: "rewrite.aof".into(),
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
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream.set_nodelay(true).unwrap();
    stream
}

fn read_frame(stream: &mut TcpStream) -> Vec<u8> {
    let mut prefix = [0];
    stream.read_exact(&mut prefix).unwrap();
    assert!(!prefix[0].is_ascii_digit(), "unexpected frame {prefix:?}");
    let mut line = Vec::new();
    let mut byte = [0];
    loop {
        stream.read_exact(&mut byte).unwrap();
        if byte[0] == b'\n' {
            assert_eq!(line.pop(), Some(b'\r'));
            break;
        }
        line.push(byte[0]);
    }
    let mut frame = vec![prefix[0]];
    frame.extend_from_slice(&line);
    frame.extend_from_slice(b"\r\n");
    if prefix[0] == b'$' {
        let length: i64 = String::from_utf8(line).unwrap().parse().unwrap();
        assert!(length >= 0);
        let mut body = vec![0; length as usize + 2];
        stream.read_exact(&mut body).unwrap();
        frame.extend_from_slice(&body);
    }
    frame
}

fn request(stream: &mut TcpStream, arguments: &[&[u8]]) -> Vec<u8> {
    let mut bytes = Vec::new();
    encode_request(arguments, &mut bytes);
    stream.write_all(&bytes).unwrap();
    read_frame(stream)
}

fn field<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    text.lines()
        .find_map(|line| {
            line.strip_prefix(name)
                .and_then(|rest| rest.strip_prefix(':'))
        })
        .map(|value| value.trim_end_matches('\r'))
}

fn persistence(stream: &mut TcpStream) -> String {
    let frame = request(stream, &[b"INFO", b"persistence"]);
    String::from_utf8(frame).unwrap()
}

fn pipeline_sets(stream: &mut TcpStream, keys: usize) {
    const BATCH: usize = 256;
    for start in (0..keys).step_by(BATCH) {
        let end = (start + BATCH).min(keys);
        let mut request = Vec::new();
        for index in start..end {
            let key = format!("rewrite:key:{index}");
            let value =
                format!("value-{index:08}-0123456789abcdef0123456789abcdef0123456789abcdef");
            encode_request(&[b"SET", key.as_bytes(), value.as_bytes()], &mut request);
        }
        stream.write_all(&request).unwrap();
        for _ in start..end {
            assert_eq!(read_frame(stream), b"+OK\r\n");
        }
    }
}

fn measure(keys: usize) {
    let directory = temp_dir(&format!("keys-{keys}"));
    let server = start(&directory);
    let mut control = connect(&server);
    pipeline_sets(&mut control, keys);
    let mut probe = connect(&server);
    let mut writer = connect(&server);

    let started = Instant::now();
    assert_eq!(
        request(&mut control, &[b"BGREWRITEAOF"]),
        b"+Background append only file rewriting started\r\n"
    );
    let start_pause = started.elapsed();

    let deadline = Instant::now() + COMPLETION_BUDGET;
    let mut worst_stall = Duration::ZERO;
    let mut bytes_seen = 0u64;
    let mut keys_done_peak = 0u64;
    let mut phases = Vec::new();
    let mut writes = 0usize;
    // Keep writing only while the snapshot is being taken: a write during the
    // final commit round would make every round dirty again.
    let mut writing = true;
    loop {
        let probe_started = Instant::now();
        assert_eq!(request(&mut probe, &[b"PING"]), b"+PONG\r\n");
        worst_stall = worst_stall.max(probe_started.elapsed());

        if writing {
            let value = format!("concurrent-{writes}");
            assert_eq!(
                request(
                    &mut writer,
                    &[b"SET", b"rewrite:concurrent", value.as_bytes()]
                ),
                b"+OK\r\n"
            );
            writes += 1;
        }

        let text = persistence(&mut probe);
        let phase = field(&text, "aof_rewrite_phase")
            .unwrap_or("missing")
            .to_string();
        if field(&text, "aof_rewrite_in_progress") == Some("1") {
            if phases.last() != Some(&phase) {
                phases.push(phase.clone());
            }
            let bytes = field(&text, "aof_rewrite_bytes_written")
                .and_then(|value| value.parse::<u64>().ok())
                .expect("byte progress");
            bytes_seen = bytes_seen.max(bytes);
            let done = field(&text, "aof_rewrite_keys_done")
                .and_then(|value| value.parse::<u64>().ok())
                .expect("key progress");
            keys_done_peak = keys_done_peak.max(done);
            if phase != "snapshot" {
                writing = false;
            }
            if Instant::now() > deadline {
                panic!("rewrite did not finish within {COMPLETION_BUDGET:?}: {text}");
            }
            continue;
        }
        assert_eq!(phase, "none", "{text}");
        assert_eq!(
            field(&text, "aof_last_bgrewrite_status"),
            Some("ok"),
            "{text}"
        );
        break;
    }

    println!(
        "keys={keys} start_pause={:?} worst_stall={:?} bytes_done={bytes_seen} \
         keys_done_peak={keys_done_peak} phases={phases:?} writes={writes}",
        start_pause, worst_stall
    );
    assert!(
        start_pause < START_PAUSE_BUDGET,
        "{keys} keys: start pause {start_pause:?} exceeds {START_PAUSE_BUDGET:?}"
    );
    assert!(
        worst_stall < CONCURRENT_STALL_BUDGET,
        "{keys} keys: worst concurrent command stall {worst_stall:?} exceeds \
         {CONCURRENT_STALL_BUDGET:?}"
    );
    assert!(
        bytes_seen > 0,
        "{keys} keys: rewrite never reported completed bytes"
    );
    assert!(
        keys_done_peak > 0,
        "{keys} keys: rewrite never reported completed keys"
    );

    // The rewrite must leave the concurrent write and every earlier key intact.
    let value = format!("concurrent-{}", writes - 1);
    let expected = format!("${}\r\n{value}\r\n", value.len());
    assert_eq!(
        request(&mut control, &[b"GET", b"rewrite:concurrent"]),
        expected.as_bytes().to_vec()
    );
    assert!(request(&mut control, &[b"GET", b"rewrite:key:0"]).starts_with(b"$"));
    server.shutdown().unwrap();
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn cooperative_rewrite_keeps_its_start_pause_and_progress_budget() {
    measure(5_000);
    measure(40_000);
}
