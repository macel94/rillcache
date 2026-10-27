#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

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

fn read_line(stream: &mut TcpStream, output: &mut Vec<u8>) -> Vec<u8> {
    let mut line = Vec::new();
    loop {
        let mut byte = [0];
        stream.read_exact(&mut byte).unwrap();
        if byte[0] == b'\n' {
            assert_eq!(line.pop(), Some(b'\r'));
            output.extend_from_slice(&line);
            output.extend_from_slice(b"\r\n");
            return line;
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
            let length: i64 = std::str::from_utf8(&read_line(stream, &mut output))
                .unwrap()
                .parse()
                .unwrap();
            if length >= 0 {
                let mut body = vec![0; length as usize + 2];
                stream.read_exact(&mut body).unwrap();
                output.extend_from_slice(&body);
            }
        }
        b'*' => {
            let count: i64 = std::str::from_utf8(&read_line(stream, &mut output))
                .unwrap()
                .parse()
                .unwrap();
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

fn send(stream: &mut TcpStream, arguments: &[&[u8]]) {
    let mut request = Vec::new();
    encode_request(arguments, &mut request);
    stream.write_all(&request).unwrap();
}

fn command(stream: &mut TcpStream, arguments: &[&[u8]], expected: &[u8]) {
    send(stream, arguments);
    assert_eq!(read_frame(stream, 0), expected, "{arguments:?}");
}

#[test]
fn write_pause_defers_writes_but_not_reads_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut admin = connect(&server);
        let mut worker = connect(&server);
        command(&mut worker, &[b"SET", b"k", b"v"], b"+OK\r\n");

        command(
            &mut admin,
            &[b"CLIENT", b"PAUSE", b"5000", b"WRITE"],
            b"+OK\r\n",
        );
        // Reads are not part of a WRITE pause.
        command(&mut worker, &[b"GET", b"k"], b"$1\r\nv\r\n");
        command(&mut worker, &[b"PING"], b"+PONG\r\n");
        send(&mut worker, &[b"INFO", b"server"]);
        assert!(read_frame(&mut worker, 0).starts_with(b"$"));

        // The deferred write must not be visible or answered before unpause.
        send(&mut worker, &[b"SET", b"k", b"v2"]);
        std::thread::sleep(Duration::from_millis(100));
        command(&mut admin, &[b"GET", b"k"], b"$1\r\nv\r\n");
        command(&mut admin, &[b"CLIENT", b"UNPAUSE"], b"+OK\r\n");
        assert_eq!(read_frame(&mut worker, 0), b"+OK\r\n");
        command(&mut worker, &[b"GET", b"k"], b"$2\r\nv2\r\n");

        // A finite pause expires without another client's intervention.
        command(
            &mut admin,
            &[b"CLIENT", b"PAUSE", b"2000", b"WRITE"],
            b"+OK\r\n",
        );
        send(&mut worker, &[b"SET", b"k", b"v3"]);
        // The reply itself is held until the finite pause expires. Do not race a
        // separate admin read against the deadline under loaded CI.
        assert_eq!(read_frame(&mut worker, 0), b"+OK\r\n");
        command(&mut worker, &[b"GET", b"k"], b"$2\r\nv3\r\n");

        // A zero-length pause is a no-op.
        command(
            &mut admin,
            &[b"CLIENT", b"PAUSE", b"0", b"WRITE"],
            b"+OK\r\n",
        );
        command(&mut worker, &[b"SET", b"k", b"v4"], b"+OK\r\n");

        assert_eq!(
            gated(&mut admin, &[b"CLIENT", b"PAUSE", b"abc"]),
            b"-ERR timeout is not an integer or out of range\r\n"
        );
        assert_eq!(
            gated(&mut admin, &[b"CLIENT", b"PAUSE", b"-1"]),
            b"-ERR timeout is negative\r\n"
        );
        assert_eq!(
            gated(&mut admin, &[b"CLIENT", b"PAUSE", b"10", b"BOTH"]),
            b"-ERR CLIENT PAUSE mode must be WRITE or ALL\r\n"
        );

        server.shutdown().unwrap();
    }
}

#[test]
fn unpause_all_mode_pipelining_and_management_exemption_work() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut admin = connect(&server);
        let mut worker = connect(&server);

        // ALL pauses reads too, and UNPAUSE releases the deferred command.
        command(
            &mut admin,
            &[b"CLIENT", b"PAUSE", b"5000", b"ALL"],
            b"+OK\r\n",
        );
        send(&mut worker, &[b"PING"]);
        std::thread::sleep(Duration::from_millis(80));
        let started = Instant::now();
        command(&mut admin, &[b"CLIENT", b"UNPAUSE"], b"+OK\r\n");
        assert_eq!(read_frame(&mut worker, 0), b"+PONG\r\n");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "UNPAUSE did not release the paused read"
        );

        // Management commands are exempt, so a connection can pause and unpause
        // itself even in ALL mode.
        command(
            &mut admin,
            &[b"CLIENT", b"PAUSE", b"60000", b"ALL"],
            b"+OK\r\n",
        );
        command(&mut admin, &[b"CLIENT", b"UNPAUSE"], b"+OK\r\n");
        command(&mut admin, &[b"PING"], b"+PONG\r\n");

        // Pipelined commands keep wire order across the pause boundary.
        command(
            &mut worker,
            &[b"CLIENT", b"PAUSE"],
            b"-ERR wrong number of arguments for 'client|pause' command\r\n",
        );
        command(
            &mut admin,
            &[b"CLIENT", b"PAUSE", b"200", b"WRITE"],
            b"+OK\r\n",
        );
        send(&mut worker, &[b"SET", b"pipe", b"v"]);
        send(&mut worker, &[b"GET", b"pipe"]);
        assert_eq!(read_frame(&mut worker, 0), b"+OK\r\n");
        assert_eq!(read_frame(&mut worker, 0), b"$1\r\nv\r\n");

        // A queued write waits until the pause ends, then MULTI/EXEC keeps order.
        command(
            &mut admin,
            &[b"CLIENT", b"PAUSE", b"200", b"WRITE"],
            b"+OK\r\n",
        );
        send(&mut worker, &[b"MULTI"]);
        send(&mut worker, &[b"SET", b"multi", b"v"]);
        send(&mut worker, &[b"EXEC"]);
        assert_eq!(read_frame(&mut worker, 0), b"+OK\r\n");
        assert_eq!(read_frame(&mut worker, 0), b"+QUEUED\r\n");
        assert_eq!(read_frame(&mut worker, 0), b"*1\r\n+OK\r\n");
        command(&mut worker, &[b"GET", b"multi"], b"$1\r\nv\r\n");

        // UNPAUSE outside a pause is harmless.
        command(&mut admin, &[b"CLIENT", b"UNPAUSE"], b"+OK\r\n");

        server.shutdown().unwrap();
    }
}

#[test]
fn paused_connections_do_not_block_shutdown() {
    let server = start(Some(1));
    let mut admin = connect(&server);
    let mut worker = connect(&server);
    command(
        &mut admin,
        &[b"CLIENT", b"PAUSE", b"60000", b"ALL"],
        b"+OK\r\n",
    );
    send(&mut worker, &[b"PING"]);
    std::thread::sleep(Duration::from_millis(80));
    let started = Instant::now();
    server.shutdown().unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "paused connection delayed shutdown: {:?}",
        started.elapsed()
    );
}

fn gated(stream: &mut TcpStream, arguments: &[&[u8]]) -> Vec<u8> {
    send(stream, arguments);
    read_frame(stream, 0)
}
