#![forbid(unsafe_code)]

use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::thread;
use std::time::Duration;

const MODES: [(Option<usize>, bool); 4] = [
    (None, false),
    (None, true),
    (Some(1), false),
    (Some(4), false),
];

fn start(workers: Option<usize>, tokio: bool, max_clients: usize) -> ServerHandle {
    Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        worker_threads: workers,
        io_mode: if tokio {
            rillcache::server::IoMode::Tokio
        } else {
            rillcache::server::IoMode::Mio
        },
        max_clients,
        idle_timeout: Duration::from_secs(2),
        frame_assembly_timeout: Some(Duration::from_millis(250)),
        ..Config::default()
    })
    .unwrap()
    .spawn()
    .unwrap()
}

fn connect(server: &ServerHandle) -> TcpStream {
    let stream = TcpStream::connect(server.local_addr()).unwrap();
    stream.set_nodelay(true).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
}

fn ping(stream: &mut TcpStream) {
    let mut request = Vec::new();
    encode_request(&[b"PING"], &mut request);
    stream.write_all(&request).unwrap();
    let mut reply = [0; 7];
    stream.read_exact(&mut reply).unwrap();
    assert_eq!(&reply, b"+PONG\r\n");
}

#[test]
fn trickle_below_idle_timeout_expires_only_slow_frame_and_releases_admission() {
    for (workers, tokio) in MODES {
        let server = start(workers, tokio, 2);
        let healthy = connect(&server);
        let mut slow = connect(&server);
        slow.write_all(b"*").unwrap();
        let mut trickler = slow.try_clone().unwrap();
        let writer = thread::spawn(move || {
            // Each fragment arrives well inside the two-second inactivity limit,
            // while assembly of the 14-byte PING frame exceeds 250 ms.
            for byte in b"1\r\n$4\r\nPING\r\n" {
                thread::sleep(Duration::from_millis(60));
                if trickler.write_all(&[*byte]).is_err() {
                    break;
                }
            }
        });
        let mut slow_reply = Vec::new();
        slow.read_to_end(&mut slow_reply).unwrap();
        writer.join().unwrap();
        assert!(
            slow_reply.is_empty(),
            "partial frame must close without reply"
        );

        let mut healthy = healthy;
        ping(&mut healthy);
        // max_clients=2: after the slow connection's permit is dropped, a new
        // connection must be admitted while the healthy client remains open.
        let mut replacement = connect(&server);
        ping(&mut replacement);
        ping(&mut healthy);
        server.shutdown().unwrap();
    }
}

#[test]
fn partial_frame_expires_without_further_socket_activity() {
    for (workers, tokio) in MODES {
        let server = Server::bind(Config {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            worker_threads: workers,
            io_mode: if tokio {
                rillcache::server::IoMode::Tokio
            } else {
                rillcache::server::IoMode::Mio
            },
            idle_timeout: Duration::from_secs(2),
            frame_assembly_timeout: Some(Duration::from_millis(150)),
            ..Config::default()
        })
        .unwrap()
        .spawn()
        .unwrap();
        let mut peer = connect(&server);
        peer.write_all(b"*").unwrap();
        let started = std::time::Instant::now();
        let mut reply = Vec::new();
        peer.read_to_end(&mut reply).unwrap();
        assert!(reply.is_empty());
        assert!(started.elapsed() < Duration::from_millis(800));
        server.shutdown().unwrap();
    }
}

#[test]
fn complete_buffered_command_survives_pause_longer_than_assembly_deadline() {
    for (workers, tokio) in MODES {
        let server = start(workers, tokio, 8);
        let mut admin = connect(&server);
        let mut worker = connect(&server);
        let mut request = Vec::new();
        encode_request(&[b"CLIENT", b"PAUSE", b"700", b"ALL"], &mut request);
        encode_request(&[b"PING"], &mut request);
        worker.write_all(&request).unwrap();
        let mut reply = [0; 5];
        worker.read_exact(&mut reply).unwrap();
        assert_eq!(&reply, b"+OK\r\n");
        thread::sleep(Duration::from_millis(400));
        let mut unpause = Vec::new();
        encode_request(&[b"CLIENT", b"UNPAUSE"], &mut unpause);
        admin.write_all(&unpause).unwrap();
        admin.read_exact(&mut reply).unwrap();
        assert_eq!(&reply, b"+OK\r\n");
        let mut pong = [0; 7];
        worker.read_exact(&mut pong).unwrap();
        assert_eq!(&pong, b"+PONG\r\n");
        server.shutdown().unwrap();
    }
}

#[test]
fn complete_and_valid_fragmented_frames_are_not_rejected() {
    for (workers, tokio) in MODES {
        let server = start(workers, tokio, 8);
        let mut peer = connect(&server);
        peer.write_all(b"*1\r\n$4\r\nPI").unwrap();
        thread::sleep(Duration::from_millis(80));
        peer.write_all(b"NG\r\n*1\r\n$4\r\nPING\r\n").unwrap();
        let mut replies = [0; 14];
        peer.read_exact(&mut replies).unwrap();
        assert_eq!(&replies, b"+PONG\r\n+PONG\r\n");
        server.shutdown().unwrap();
    }
}
