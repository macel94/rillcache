#![forbid(unsafe_code)]
#![cfg(unix)]

//! Default owner-local plain-TCP reactor, opt-in owner-migration workers and the
//! explicit one-worker path.

use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

fn start(workers: Option<usize>) -> ServerHandle {
    let server = Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        socket_workers: workers,
        ..Config::default()
    })
    .unwrap()
    .spawn()
    .unwrap();
    assert!(server.scheduling_policy().readiness_io);
    assert!(!server.scheduling_policy().tokio_transport);
    server
}

struct Peer(BufReader<TcpStream>);
impl Peer {
    fn new(server: &ServerHandle) -> Self {
        let stream = TcpStream::connect(server.local_addr()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream.set_nodelay(true).unwrap();
        Self(BufReader::new(stream))
    }
    fn send(&mut self, args: &[&[u8]]) {
        let mut bytes = Vec::new();
        encode_request(args, &mut bytes);
        self.0.get_mut().write_all(&bytes).unwrap();
    }
    fn line(&mut self) -> Vec<u8> {
        let mut line = Vec::new();
        self.0.read_until(b'\n', &mut line).unwrap();
        assert!(line.ends_with(b"\r\n"));
        line
    }
    fn bulk(&mut self) -> Vec<u8> {
        let header = self.line();
        let len: usize = std::str::from_utf8(&header[1..header.len() - 2])
            .unwrap()
            .parse()
            .unwrap();
        let mut value = vec![0; len + 2];
        self.0.read_exact(&mut value).unwrap();
        assert!(value.ends_with(b"\r\n"));
        value.truncate(len);
        value
    }
    fn ping(&mut self) {
        self.send(&[b"PING"]);
        assert_eq!(self.line(), b"+PONG\r\n");
    }
}

#[test]
fn default_plain_tcp_uses_the_owner_local_reactor_and_serves_wire() {
    let server = start(None);
    assert_eq!(
        server.scheduling_policy().socket_workers,
        1,
        "the default keeps one owner-local reactor: it batches more pipeline-1 commands per readiness wake"
    );
    assert_eq!(server.scheduling_policy().client_handoff_lanes, 0);

    let mut peers: Vec<_> = (0..8).map(|_| Peer::new(&server)).collect();
    for (index, peer) in peers.iter_mut().enumerate() {
        let key = format!("socket-worker:{index}");
        peer.ping();
        peer.send(&[b"SET", key.as_bytes(), b"value"]);
        assert_eq!(peer.line(), b"+OK\r\n");
        peer.send(&[b"GET", key.as_bytes()]);
        assert_eq!(peer.bulk(), b"value");
        // A pipeline spanning local and data commands keeps reply order.
        peer.send(&[b"PING"]);
        peer.send(&[b"GET", key.as_bytes()]);
        peer.send(&[b"PING"]);
        assert_eq!(peer.line(), b"+PONG\r\n");
        assert_eq!(peer.bulk(), b"value");
        assert_eq!(peer.line(), b"+PONG\r\n");
    }
    server.shutdown().unwrap();
}

#[test]
fn concurrent_ping_accounting_and_reset_are_exact_with_one_two_or_four_reactors() {
    fn total(peer: &mut Peer) -> u64 {
        peer.send(&[b"INFO", b"stats"]);
        let info = String::from_utf8(peer.bulk()).unwrap();
        info.lines()
            .find_map(|line| line.strip_prefix("total_commands_processed:"))
            .unwrap()
            .parse()
            .unwrap()
    }

    for workers in [1, 2, 4] {
        let server = start(Some(workers));
        assert_eq!(server.scheduling_policy().socket_workers, workers);
        let mut admin = Peer::new(&server);
        for pipeline in [1, 16] {
            let peers: Vec<_> = (0..8).map(|_| Peer::new(&server)).collect();
            admin.send(&[b"CONFIG", b"RESETSTAT"]);
            assert_eq!(admin.line(), b"+OK\r\n");
            let barrier = std::sync::Barrier::new(peers.len());
            std::thread::scope(|scope| {
                for mut peer in peers {
                    let barrier = &barrier;
                    scope.spawn(move || {
                        let mut frame = Vec::new();
                        for _ in 0..pipeline {
                            encode_request(&[b"PING"], &mut frame);
                        }
                        barrier.wait();
                        for _ in 0..64 / pipeline {
                            peer.0.get_mut().write_all(&frame).unwrap();
                            for _ in 0..pipeline {
                                assert_eq!(peer.line(), b"+PONG\r\n");
                            }
                        }
                    });
                }
            });
            // Includes this INFO command; there is no sampled/batched accounting.
            assert_eq!(total(&mut admin), 8 * 64 + 1);
            admin.send(&[b"CONFIG", b"RESETSTAT"]);
            assert_eq!(admin.line(), b"+OK\r\n");
            assert_eq!(total(&mut admin), 1);
        }
        server.shutdown().unwrap();
    }
}

#[test]
fn explicit_one_worker_keeps_owner_local_path() {
    let server = start(Some(1));
    assert_eq!(server.scheduling_policy().socket_workers, 1);
    let mut peer = Peer::new(&server);
    peer.ping();
    peer.send(&[b"SET", b"k", b"v"]);
    assert_eq!(peer.line(), b"+OK\r\n");
    peer.send(&[b"GET", b"k"]);
    assert_eq!(peer.bulk(), b"v");
    server.shutdown().unwrap();
}

#[test]
fn migration_preserves_session_state_transactions_and_pipelines() {
    let server = start(None);
    let mut peer = Peer::new(&server);
    peer.send(&[b"CLIENT", b"SETNAME", b"mover"]);
    assert_eq!(peer.line(), b"+OK\r\n");
    // This data command migrates the connection; the name must survive.
    peer.send(&[b"SET", b"k", b"1"]);
    assert_eq!(peer.line(), b"+OK\r\n");
    peer.send(&[b"CLIENT", b"GETNAME"]);
    assert_eq!(peer.bulk(), b"mover");
    peer.send(&[b"MULTI"]);
    assert_eq!(peer.line(), b"+OK\r\n");
    peer.send(&[b"INCR", b"k"]);
    assert_eq!(peer.line(), b"+QUEUED\r\n");
    peer.send(&[b"PING"]);
    assert_eq!(peer.line(), b"+QUEUED\r\n");
    peer.send(&[b"EXEC"]);
    let header = peer.line();
    assert_eq!(header, b"*2\r\n");
    assert_eq!(peer.line(), b":2\r\n");
    assert_eq!(peer.line(), b"+PONG\r\n");
    // A mixed pipeline after migration stays ordered.
    peer.send(&[b"PING"]);
    peer.send(&[b"GET", b"k"]);
    peer.send(&[b"ECHO", b"done"]);
    assert_eq!(peer.line(), b"+PONG\r\n");
    assert_eq!(peer.bulk(), b"2");
    assert_eq!(peer.bulk(), b"done");
    server.shutdown().unwrap();
}

#[test]
fn pubsub_and_blocking_connections_are_owned_by_the_owner_reactor() {
    let server = start(None);
    let mut subscriber = Peer::new(&server);
    subscriber.send(&[b"SUBSCRIBE", b"chan"]);
    assert_eq!(subscriber.line(), b"*3\r\n");
    assert_eq!(subscriber.bulk(), b"subscribe");
    assert_eq!(subscriber.bulk(), b"chan");
    assert_eq!(subscriber.line(), b":1\r\n");
    let mut publisher = Peer::new(&server);
    publisher.send(&[b"PUBLISH", b"chan", b"hello"]);
    assert_eq!(publisher.line(), b":1\r\n");
    assert_eq!(subscriber.line(), b"*3\r\n");
    assert_eq!(subscriber.bulk(), b"message");
    assert_eq!(subscriber.bulk(), b"chan");
    assert_eq!(subscriber.bulk(), b"hello");

    let mut waiter = Peer::new(&server);
    waiter.send(&[b"BLPOP", b"queue", b"2"]);
    let mut pusher = Peer::new(&server);
    pusher.send(&[b"LPUSH", b"queue", b"job"]);
    assert_eq!(pusher.line(), b":1\r\n");
    assert_eq!(waiter.line(), b"*2\r\n");
    assert_eq!(waiter.bulk(), b"queue");
    assert_eq!(waiter.bulk(), b"job");
    server.shutdown().unwrap();
}

#[test]
fn large_local_echo_backpressure_then_data_migration_stays_ordered() {
    let server = start(None);
    let mut peer = Peer::new(&server);
    let value = vec![b'x'; 512 * 1024];
    let mut frame = Vec::new();
    encode_request(&[b"ECHO", &value], &mut frame);
    // ECHO is connection-local; the large reply can backpressure the worker.
    peer.0.get_mut().write_all(&frame).unwrap();
    peer.send(&[b"SET", b"after", b"1"]);
    assert_eq!(peer.bulk(), value);
    assert_eq!(peer.line(), b"+OK\r\n");
    peer.send(&[b"GET", b"after"]);
    assert_eq!(peer.bulk(), b"1");
    server.shutdown().unwrap();
}
