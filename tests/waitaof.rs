#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

fn temp_dir(label: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("rillcache-waitaof-{}-{label}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

fn start(workers: Option<usize>, appendonly: bool, dir: &Path) -> ServerHandle {
    Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        worker_threads: workers,
        appendonly,
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
    stream
}

fn read_frame(stream: &mut TcpStream, depth: usize) -> Vec<u8> {
    assert!(depth <= 8);
    let mut output = Vec::new();
    let mut prefix = [0];
    stream.read_exact(&mut prefix).unwrap();
    output.push(prefix[0]);
    match prefix[0] {
        b'+' | b'-' | b':' => {
            read_line_value(stream, &mut output);
        }
        b'$' => {
            let length = read_line_value(stream, &mut output).parse::<i64>().unwrap();
            if length >= 0 {
                let mut body = vec![0; length as usize + 2];
                stream.read_exact(&mut body).unwrap();
                output.extend_from_slice(&body);
            }
        }
        b'*' => {
            let count = read_line_value(stream, &mut output).parse::<i64>().unwrap();
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

fn read_line_value(stream: &mut TcpStream, output: &mut Vec<u8>) -> String {
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

fn command(stream: &mut TcpStream, args: &[&[u8]], expected: &[u8]) {
    let mut request = Vec::new();
    encode_request(args, &mut request);
    stream.write_all(&request).unwrap();
    assert_eq!(read_frame(stream, 0), expected, "{args:?}");
}

#[test]
fn waitaof_reports_no_local_aof_and_blocks_for_absent_replicas_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let dir = temp_dir(&format!("without-{workers:?}"));
        let server = start(workers, false, &dir);
        let mut stream = connect(&server);

        command(
            &mut stream,
            &[b"WAITAOF", b"0", b"0", b"0"],
            b"*2\r\n:0\r\n:0\r\n",
        );
        command(
            &mut stream,
            &[b"WAITAOF", b"1", b"0", b"0"],
            b"-ERR WAITAOF cannot be used when numlocal is set to 1 but the AOF is not enabled.\r\n",
        );
        command(
            &mut stream,
            &[b"WAITAOF", b"2", b"0", b"0"],
            b"-ERR numlocal can't be greater than 1\r\n",
        );
        command(
            &mut stream,
            &[b"WAITAOF", b"0", b"-1", b"0"],
            b"-ERR numreplicas can't be negative\r\n",
        );
        command(
            &mut stream,
            &[b"WAITAOF", b"0", b"0", b"-1"],
            b"-ERR timeout can't be negative\r\n",
        );
        command(
            &mut stream,
            &[b"WAITAOF", b"x", b"0", b"0"],
            b"-ERR value is not an integer or out of range\r\n",
        );
        command(
            &mut stream,
            &[b"WAITAOF", b"0", b"0"],
            b"-ERR wrong number of arguments for 'waitaof' command\r\n",
        );

        let started = Instant::now();
        command(
            &mut stream,
            &[b"WAITAOF", b"0", b"1", b"100"],
            b"*2\r\n:0\r\n:0\r\n",
        );
        assert!(started.elapsed() >= Duration::from_millis(50));
        assert!(started.elapsed() < Duration::from_secs(3));

        command(&mut stream, &[b"MULTI"], b"+OK\r\n");
        command(
            &mut stream,
            &[b"WAITAOF", b"0", b"1", b"500"],
            b"+QUEUED\r\n",
        );
        let started = Instant::now();
        command(&mut stream, &[b"EXEC"], b"*1\r\n*2\r\n:0\r\n:0\r\n");
        assert!(started.elapsed() < Duration::from_millis(400));

        server.shutdown().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn waitaof_confirms_local_aof_fsync_and_does_not_block_in_transactions() {
    for workers in [None, Some(1), Some(4)] {
        let dir = temp_dir(&format!("with-{workers:?}"));
        let server = start(workers, true, &dir);
        let mut stream = connect(&server);

        command(&mut stream, &[b"SET", b"k", b"v"], b"+OK\r\n");
        command(
            &mut stream,
            &[b"WAITAOF", b"1", b"0", b"0"],
            b"*2\r\n:1\r\n:0\r\n",
        );

        let started = Instant::now();
        command(
            &mut stream,
            &[b"WAITAOF", b"0", b"1", b"100"],
            b"*2\r\n:1\r\n:0\r\n",
        );
        assert!(started.elapsed() >= Duration::from_millis(50));
        assert!(started.elapsed() < Duration::from_secs(3));

        command(&mut stream, &[b"MULTI"], b"+OK\r\n");
        command(
            &mut stream,
            &[b"WAITAOF", b"1", b"1", b"500"],
            b"+QUEUED\r\n",
        );
        let started = Instant::now();
        command(&mut stream, &[b"EXEC"], b"*1\r\n*2\r\n:1\r\n:0\r\n");
        assert!(started.elapsed() < Duration::from_millis(400));

        command(
            &mut stream,
            &[b"WAITAOF", b"2", b"0", b"0"],
            b"-ERR numlocal can't be greater than 1\r\n",
        );

        server.shutdown().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
