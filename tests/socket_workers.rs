#![forbid(unsafe_code)]
#![cfg(unix)]

//! Default plain-TCP shared-store socket workers and the explicit one-worker path.

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
fn default_plain_tcp_uses_shared_store_workers_and_serves_wire() {
    let server = start(None);
    let cpus = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1);
    assert_eq!(
        server.scheduling_policy().socket_workers,
        cpus.clamp(1, 4),
        "default reactor count follows the allowed CPU hint"
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
