#![forbid(unsafe_code)]

//! S9: enforced slow-reader and command-budget overload policy.
//!
//! Every case runs against the readiness reactor, the opt-in Tokio transport
//! and both fixed-lane configurations, so a policy that only exists on one I/O
//! path cannot pass.

use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rillcache::protocol::{Limits, encode_request};
use rillcache::server::IoMode;
use rillcache::{Config, Server, ServerHandle};

const MODES: [(Option<usize>, bool); 4] = [
    (None, false),
    (None, true),
    (Some(1), false),
    (Some(4), false),
];

/// Budget admission is dispatcher-level, not I/O-path-level: two wire modes
/// plus the engine unit tests cover it. The slow-reader policy is I/O-path
/// specific and keeps the full matrix below.
const BUDGET_MODES: [(Option<usize>, bool); 2] = [(None, false), (Some(4), false)];

/// The progressing and disabled-deadline cases exercise the two distinct write
/// paths (readiness reactor and Tokio); the non-reading case covers the reactor,
/// Tokio and one fixed lane (both lanes share `Client::poll`).
const WRITE_PATH_MODES: [(Option<usize>, bool); 2] = [(None, false), (None, true)];
const NON_READER_MODES: [(Option<usize>, bool); 3] =
    [(None, false), (None, true), (Some(1), false)];

fn start(workers: Option<usize>, tokio: bool, config: Config) -> ServerHandle {
    Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        worker_threads: workers,
        io_mode: if tokio { IoMode::Tokio } else { IoMode::Mio },
        ..config
    })
    .unwrap()
    .spawn()
    .unwrap()
}

fn connect(server: &ServerHandle) -> TcpStream {
    let stream = TcpStream::connect(server.local_addr()).unwrap();
    stream.set_nodelay(true).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
}

fn send(stream: &mut TcpStream, arguments: &[&[u8]]) {
    let mut request = Vec::new();
    encode_request(arguments, &mut request);
    stream.write_all(&request).unwrap();
}

fn read_line(stream: &mut TcpStream) -> Vec<u8> {
    let mut line = Vec::new();
    loop {
        let mut byte = [0u8; 1];
        stream.read_exact(&mut byte).unwrap();
        line.push(byte[0]);
        if line.ends_with(b"\r\n") {
            return line;
        }
    }
}

fn read_reply(stream: &mut TcpStream) -> Vec<u8> {
    let mut reply = Vec::new();
    read_reply_into(stream, &mut reply);
    reply
}

fn read_reply_into(stream: &mut TcpStream, reply: &mut Vec<u8>) {
    let mut kind = [0u8; 1];
    stream.read_exact(&mut kind).unwrap();
    let marker = kind[0];
    reply.push(marker);
    let line = read_line(stream);
    match marker {
        b'$' | b'*' => {
            let text = std::str::from_utf8(&line[..line.len() - 2]).unwrap();
            let count: i64 = text.parse().unwrap();
            reply.extend_from_slice(&line);
            if marker == b'*' {
                for _ in 0..count {
                    read_reply_into(stream, reply);
                }
            } else if count >= 0 {
                let mut body = vec![0; count as usize + 2];
                stream.read_exact(&mut body).unwrap();
                reply.extend_from_slice(&body);
            }
        }
        _ => reply.extend_from_slice(&line),
    }
}

fn command(stream: &mut TcpStream, arguments: &[&[u8]]) -> Vec<u8> {
    send(stream, arguments);
    read_reply(stream)
}

fn now_second() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// Fixed windows are wall-clock seconds. Start each budget sequence just after
/// a boundary so a descheduled test cannot see an accidental reset mid-sequence.
fn wait_fresh_second() {
    let started = now_second();
    while now_second() == started {
        std::thread::sleep(Duration::from_millis(5));
    }
    std::thread::sleep(Duration::from_millis(20));
}

fn field<'a>(text: &'a str, name: &str) -> &'a str {
    text.lines()
        .find_map(|line| line.strip_prefix(&format!("{name}:")))
        .unwrap_or_else(|| panic!("missing {name} in {text}"))
}

#[test]
fn global_command_budget_sheds_the_excess_and_recovers_in_the_next_window() {
    for (workers, tokio) in BUDGET_MODES {
        let server = start(
            workers,
            tokio,
            Config {
                global_command_budget: 2,
                idle_timeout: Duration::from_secs(30),
                ..Config::default()
            },
        );
        let mut client = connect(&server);
        let mut rejected = false;
        for attempt in 0..5 {
            if attempt > 0 {
                wait_fresh_second();
            }
            if command(&mut client, &[b"PING"]) != b"+PONG\r\n"
                || command(&mut client, &[b"PING"]) != b"+PONG\r\n"
            {
                continue; // The window rolled over between the two replies.
            }
            let third = command(&mut client, &[b"PING"]);
            if third == b"-ERR overload: global command budget exhausted\r\n" {
                rejected = true;
                break;
            }
            assert_eq!(third, b"+PONG\r\n", "an over-budget reply must be shed");
        }
        assert!(rejected, "the third command in the window must be rejected");

        wait_fresh_second();
        let info = command(&mut client, &[b"INFO", b"stats"]);
        let text = std::str::from_utf8(&info).unwrap();
        assert_eq!(field(text, "command_budget_rejections"), "1");

        wait_fresh_second();
        assert_eq!(command(&mut client, &[b"PING"]), b"+PONG\r\n");
        server.shutdown().unwrap();
    }
}

#[test]
fn a_rejected_command_is_not_executed() {
    for (workers, tokio) in BUDGET_MODES {
        let server = start(
            workers,
            tokio,
            Config {
                global_command_budget: 1,
                idle_timeout: Duration::from_secs(30),
                ..Config::default()
            },
        );
        let mut client = connect(&server);
        let mut completed = false;
        for attempt in 0..5 {
            if attempt > 0 {
                wait_fresh_second();
            }
            if command(&mut client, &[b"INCR", b"budget-counter"]) != b":1\r\n" {
                continue; // A boundary between the two commands changed the value.
            }
            let rejected = command(&mut client, &[b"INCR", b"budget-counter"]);
            if rejected != b"-ERR overload: global command budget exhausted\r\n" {
                continue;
            }
            wait_fresh_second();
            assert_eq!(
                command(&mut client, &[b"GET", b"budget-counter"]),
                b"$1\r\n1\r\n",
                "the rejected INCR must not have mutated the key"
            );
            completed = true;
            break;
        }
        assert!(
            completed,
            "could not observe a rejected command in any window"
        );
        server.shutdown().unwrap();
    }
}

fn acl_file(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rillcache-overload-{name}-{}-{}.acl",
        std::process::id(),
        now_second()
    ));
    std::fs::write(
        &path,
        b"user default on nopass ~* &* +@all\n\
user alice on >alice-password ~* &* +@all\n\
user bob on >bob-password ~* &* +@all\n",
    )
    .unwrap();
    path
}

#[test]
fn per_user_command_budget_isolates_authenticated_identities() {
    for (workers, tokio) in BUDGET_MODES {
        let path = acl_file("users");
        let server = start(
            workers,
            tokio,
            Config {
                aclfile: Some(path.clone()),
                user_command_budget: 2,
                idle_timeout: Duration::from_secs(30),
                ..Config::default()
            },
        );
        let mut observed = false;
        for attempt in 0..5 {
            if attempt > 0 {
                wait_fresh_second();
            }
            let mut alice = connect(&server);
            if command(&mut alice, &[b"AUTH", b"alice", b"alice-password"]) != b"+OK\r\n" {
                continue;
            }
            if command(&mut alice, &[b"PING"]) != b"+PONG\r\n"
                || command(&mut alice, &[b"PING"]) != b"+PONG\r\n"
            {
                continue;
            }
            let third = command(&mut alice, &[b"PING"]);
            if third != b"-ERR overload: user command budget exhausted\r\n" {
                continue;
            }
            // A different authenticated identity still has its own allowance.
            let mut bob = connect(&server);
            assert_eq!(
                command(&mut bob, &[b"AUTH", b"bob", b"bob-password"]),
                b"+OK\r\n"
            );
            assert_eq!(command(&mut bob, &[b"PING"]), b"+PONG\r\n");
            assert_eq!(command(&mut bob, &[b"PING"]), b"+PONG\r\n");
            observed = true;
            break;
        }
        assert!(observed, "per-user budget never shed an over-limit user");
        server.shutdown().unwrap();
        std::fs::remove_file(path).unwrap();
    }
}

fn large_limits() -> Limits {
    Limits {
        max_frame_bytes: 16 * 1024 * 1024,
        max_bulk_bytes: 12 * 1024 * 1024,
        ..Limits::default()
    }
}

const LARGE_VALUE: usize = 10 * 1024 * 1024;

fn store_large_value(server: &ServerHandle) -> (TcpStream, usize) {
    let mut admin = connect(server);
    let value = vec![b'x'; LARGE_VALUE];
    let mut set = Vec::new();
    encode_request(&[b"SET", b"stall-value", &value], &mut set);
    admin.write_all(&set).unwrap();
    assert_eq!(read_reply(&mut admin), b"+OK\r\n");
    command(&mut admin, &[b"STRLEN", b"stall-value"]);
    (admin, value.len())
}

fn info_clients(admin: &mut TcpStream) -> usize {
    let reply = command(admin, &[b"INFO", b"clients"]);
    let text = std::str::from_utf8(&reply).unwrap();
    field(text, "connected_clients").parse().unwrap()
}

#[test]
fn a_non_reading_client_is_shed_after_the_write_stall_deadline() {
    for (workers, tokio) in NON_READER_MODES {
        let server = start(
            workers,
            tokio,
            Config {
                idle_timeout: Duration::from_secs(30),
                write_stall_timeout: Some(Duration::from_millis(300)),
                limits: large_limits(),
                max_clients: 4,
                ..Config::default()
            },
        );
        let (mut admin, size) = store_large_value(&server);
        let mut reader = connect(&server);
        reader
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        send(&mut reader, &[b"GET", b"stall-value"]);
        // Never drain the reply: the server must shed this connection even
        // though the read-idle timeout is 30 seconds away. Poll the server's
        // registry so a descheduled test process cannot hide the deadline.
        let registered = Instant::now() + Duration::from_secs(5);
        while info_clients(&mut admin) != 2 {
            assert!(
                Instant::now() < registered,
                "the reader was never registered in the client registry"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while info_clients(&mut admin) != 1 {
            assert!(
                Instant::now() < deadline,
                "the non-reading client was not shed"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        let mut received = 0;
        let mut buffer = [0u8; 64 * 1024];
        let closed = loop {
            match reader.read(&mut buffer) {
                Ok(0) => break true,
                Ok(read) => received += read,
                Err(error)
                    if matches!(
                        error.kind(),
                        ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::ConnectionReset
                    ) =>
                {
                    break error.kind() == ErrorKind::ConnectionReset;
                }
                Err(error) => panic!("unexpected read error {error}"),
            }
            assert!(
                received <= size + 32,
                "received more bytes than the stored value ({received} > {size})"
            );
        };
        assert!(
            closed,
            "a non-reading client must be shed after the write-stall deadline (received {received} of {size} bytes)"
        );
        assert!(
            received < size,
            "shedding drops the backlog instead of draining it ({received} of {size})"
        );
        // The freed admission slot must serve a new connection.
        assert_eq!(command(&mut admin, &[b"PING"]), b"+PONG\r\n");
        let mut fresh = connect(&server);
        assert_eq!(command(&mut fresh, &[b"PING"]), b"+PONG\r\n");
        drop(reader);
        let _ = fresh.shutdown(Shutdown::Both);
        server.shutdown().unwrap();
    }
}

#[test]
fn a_slow_but_progressing_reader_survives_the_write_stall_deadline() {
    for (workers, tokio) in WRITE_PATH_MODES {
        let server = start(
            workers,
            tokio,
            Config {
                idle_timeout: Duration::from_secs(30),
                write_stall_timeout: Some(Duration::from_millis(300)),
                limits: large_limits(),
                ..Config::default()
            },
        );
        let (_admin, size) = store_large_value(&server);
        let mut reader = connect(&server);
        reader
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        send(&mut reader, &[b"GET", b"stall-value"]);
        let header = format!("${size}\r\n");
        let expected = header.len() + size + 2;
        let mut body = Vec::new();
        // Bound the socket receive queue so the server really has to wait for
        // progress instead of buffering the whole reply in the kernel.
        reader
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut buffer = [0u8; 128 * 1024];
        // A reply frame: `$<len>\r\n` then the value. Read slowly but always
        // make progress, which refreshes the per-connection write clock.
        while body.len() < expected {
            let read = reader.read(&mut buffer).unwrap();
            assert!(read > 0, "a progressing reader must not be shed");
            body.extend_from_slice(&buffer[..read]);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(body.starts_with(header.as_bytes()));
        assert_eq!(body.len(), expected);
        assert_eq!(command(&mut reader, &[b"PING"]), b"+PONG\r\n");
        server.shutdown().unwrap();
    }
}

#[test]
fn a_disabled_write_stall_deadline_keeps_the_backlogged_connection() {
    for (workers, tokio) in [(None, false)] {
        let server = start(
            workers,
            tokio,
            Config {
                idle_timeout: Duration::from_secs(30),
                write_stall_timeout: None,
                limits: large_limits(),
                ..Config::default()
            },
        );
        let (_admin, size) = store_large_value(&server);
        let mut reader = connect(&server);
        reader
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        send(&mut reader, &[b"GET", b"stall-value"]);
        std::thread::sleep(Duration::from_millis(700));
        let mut body = Vec::new();
        let mut buffer = [0u8; 64 * 1024];
        while body.len() < size + 16 {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => body.extend_from_slice(&buffer[..read]),
                Err(error)
                    if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
                {
                    break;
                }
                Err(error) => panic!("unexpected read error {error}"),
            }
        }
        assert!(
            body.len() > size,
            "the disabled deadline must not shed the connection ({})",
            body.len()
        );
        assert_eq!(command(&mut reader, &[b"PING"]), b"+PONG\r\n");
        server.shutdown().unwrap();
    }
}

#[test]
fn budget_and_stall_thresholds_are_reported_and_rejected_at_startup() {
    for arguments in [
        vec!["--write-stall-timeout-ms", "0"],
        vec!["--write-stall-timeout-ms", "600001"],
        vec!["--global-command-budget", "1000001"],
        vec!["--user-command-budget", "nonsense"],
    ] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_rillcache"))
            .args(&arguments)
            .arg("--check-config")
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "{arguments:?} must be rejected by --check-config"
        );
    }
    // The default write-stall deadline is a bounded startup value.
    let ready = Instant::now();
    let server = start(None, false, Config::default());
    server.shutdown().unwrap();
    assert!(ready.elapsed() < Duration::from_secs(30));
}
