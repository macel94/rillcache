#![forbid(unsafe_code)]
#![cfg(unix)]

use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::time::{Duration, Instant};

fn start() -> ServerHandle {
    let mut config = Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        // This suite owns the opt-in async engine; the default stays Mio.
        io_mode: rillcache::server::IoMode::Tokio,
        ..Config::default()
    };
    config.limits.max_bulk_bytes = 8 * 1024 * 1024;
    config.limits.max_frame_bytes = 16 * 1024 * 1024;
    let server = Server::bind(config).unwrap().spawn().unwrap();
    assert!(server.scheduling_policy().readiness_io);
    assert!(server.scheduling_policy().tokio_transport);
    assert!(server.scheduling_policy().tokio_worker_threads > 0);
    assert_eq!(server.scheduling_policy().client_handoff_lanes, 0);
    server
}

struct Peer(BufReader<TcpStream>);
impl Peer {
    fn new(server: &ServerHandle) -> Self {
        let stream = TcpStream::connect(server.local_addr()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream.set_nodelay(true).unwrap();
        Self(BufReader::new(stream))
    }
    fn send(&mut self, args: &[&[u8]]) {
        let mut bytes = Vec::new();
        encode_request(args, &mut bytes);
        self.0.get_mut().write_all(&bytes).unwrap();
    }
    fn exact(&mut self, expected: &[u8]) {
        let mut bytes = vec![0; expected.len()];
        self.0.read_exact(&mut bytes).unwrap();
        assert_eq!(bytes, expected);
    }
    fn line(&mut self) -> String {
        let mut line = String::new();
        self.0.read_line(&mut line).unwrap();
        assert!(line.ends_with("\r\n") && line.len() < 1024);
        line
    }
    fn bulk(&mut self) -> Vec<u8> {
        let line = self.line();
        let len: usize = line.strip_prefix('$').unwrap().trim().parse().unwrap();
        assert!(len <= 8 * 1024 * 1024);
        let mut bytes = vec![0; len];
        self.0.read_exact(&mut bytes).unwrap();
        self.exact(b"\r\n");
        bytes
    }
    fn ping(&mut self) {
        self.send(&[b"PING"]);
        self.exact(b"+PONG\r\n");
    }
}

#[test]
fn a_pipeline_crossing_read_and_command_quanta_drains_once_after_half_close() {
    let server = start();
    let mut peer = Peer::new(&server);
    let mut requests = Vec::new();
    let mut expected = Vec::new();
    for index in 1..=4097 {
        encode_request(&[b"INCR", b"counter"], &mut requests);
        expected.extend_from_slice(format!(":{index}\r\n").as_bytes());
    }
    peer.0.get_mut().write_all(&requests).unwrap();
    peer.0.get_mut().shutdown(Shutdown::Write).unwrap();
    let mut replies = Vec::new();
    peer.0.read_to_end(&mut replies).unwrap();
    assert_eq!(replies, expected);
    let mut observer = Peer::new(&server);
    observer.send(&[b"GET", b"counter"]);
    assert_eq!(observer.bulk(), b"4097");
    server.shutdown().unwrap();
}

#[test]
fn fragmented_binary_input_and_backpressured_output_do_not_starve_other_clients() {
    let server = start();
    let mut peer = Peer::new(&server);
    let mut observer = Peer::new(&server);
    let value: Vec<u8> = (0..4 * 1024 * 1024)
        .map(|index| (index % 256) as u8)
        .collect();
    let mut request = Vec::new();
    encode_request(&[b"ECHO", &value], &mut request);
    peer.0.get_mut().write_all(&request[..513]).unwrap();
    observer.ping(); // The incomplete frame must not block the event loop.
    peer.0.get_mut().write_all(&request[513..]).unwrap();
    for _ in 0..8 {
        observer.ping(); // Leave the multi-megabyte reply unread/backpressured.
    }
    assert_eq!(peer.bulk(), value);
    peer.ping(); // Writable-interest removal must not strand the next read edge.
    server.shutdown().unwrap();
}

#[test]
fn deferred_replies_and_pushes_wake_without_new_input_on_the_waiting_socket() {
    let server = start();
    let mut subscriber = Peer::new(&server);
    let mut waiter = Peer::new(&server);
    let mut publisher = Peer::new(&server);
    subscriber.send(&[b"SUBSCRIBE", b"c"]);
    subscriber.exact(b"*3\r\n$9\r\nsubscribe\r\n$1\r\nc\r\n:1\r\n");
    publisher.send(&[b"PUBLISH", b"c", b"hello"]);
    publisher.exact(b":1\r\n");
    subscriber.exact(b"*3\r\n$7\r\nmessage\r\n$1\r\nc\r\n$5\r\nhello\r\n");
    waiter.send(&[b"BLPOP", b"q", b"0"]);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        publisher.send(&[b"INFO", b"clients"]);
        if String::from_utf8(publisher.bulk())
            .unwrap()
            .contains("blocked_clients:1\r\n")
        {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    publisher.send(&[b"LPUSH", b"q", b"v"]);
    publisher.exact(b":1\r\n");
    waiter.exact(b"*2\r\n$1\r\nq\r\n$1\r\nv\r\n");
    waiter.ping();
    server.shutdown().unwrap();
}

#[test]
fn fin_behind_a_blocked_pipeline_releases_the_registered_client() {
    let server = start();
    let mut peer = Peer::new(&server);
    let mut observer = Peer::new(&server);
    peer.send(&[b"CLIENT", b"ID"]);
    let id = peer.line().trim().strip_prefix(':').unwrap().to_owned();
    let mut pipeline = Vec::new();
    encode_request(&[b"BLPOP", b"never", b"0"], &mut pipeline);
    encode_request(&[b"PING"], &mut pipeline);
    peer.0.get_mut().write_all(&pipeline).unwrap();
    peer.0.get_mut().shutdown(Shutdown::Write).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        observer.send(&[b"CLIENT", b"LIST", b"ID", id.as_bytes()]);
        if observer.bulk().is_empty() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "blocked disconnected client remained registered"
        );
        std::thread::yield_now();
    }
    observer.send(&[b"INFO", b"clients"]);
    assert!(
        String::from_utf8(observer.bulk())
            .unwrap()
            .contains("blocked_clients:0\r\n")
    );
    server.shutdown().unwrap();
}

#[test]
fn repeated_fd_reuse_keeps_replies_and_sessions_isolated() {
    let server = start();
    for index in 0..128 {
        let mut peer = Peer::new(&server);
        let value = format!("connection-{index}");
        peer.send(&[b"ECHO", value.as_bytes()]);
        assert_eq!(peer.bulk(), value.as_bytes());
        peer.send(&[b"QUIT"]);
        peer.exact(b"+OK\r\n");
        let mut end = [0];
        assert_eq!(peer.0.read(&mut end).unwrap(), 0);
    }
    server.shutdown().unwrap();
}

#[test]
fn shutdown_joins_with_idle_incomplete_and_blocked_clients() {
    let server = start();
    let _idle = Peer::new(&server);
    let mut incomplete = Peer::new(&server);
    incomplete
        .0
        .get_mut()
        .write_all(b"*2\r\n$4\r\nECHO\r\n")
        .unwrap();
    let mut waiter = Peer::new(&server);
    waiter.send(&[b"BLPOP", b"never", b"0"]);
    let mut observer = Peer::new(&server);
    observer.ping();
    server.shutdown().unwrap();
}
