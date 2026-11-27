#![forbid(unsafe_code)]

//! Fixed-worker polling latency contract (R3).
//!
//! `--threads N` runs N cooperative workers that poll with a short idle wait and
//! a bounded active window instead of blocking on OS readiness. That is a
//! latency/CPU tradeoff, not a real-time guarantee. This test pins the practical
//! contract: a worker answers after being idle, another client makes progress
//! while one client is busy, and more clients than workers still all get served
//! within a bounded time.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

fn start(threads: usize) -> ServerHandle {
    Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        worker_threads: Some(threads),
        ..Config::default()
    })
    .unwrap()
    .spawn()
    .unwrap()
}

fn connect(server: &ServerHandle) -> TcpStream {
    let stream = TcpStream::connect(server.local_addr()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    stream.set_nodelay(true).unwrap();
    stream
}

fn command(stream: &mut TcpStream, args: &[&[u8]], expected: &[u8]) {
    let mut request = Vec::new();
    encode_request(args, &mut request);
    stream.write_all(&request).unwrap();
    let mut reply = vec![0; expected.len()];
    stream.read_exact(&mut reply).unwrap();
    assert_eq!(reply, expected, "{args:?}");
}

#[test]
fn fixed_workers_serve_after_idle_under_mixed_and_oversubscribed_load() {
    for threads in [1usize, 4] {
        let server = start(threads);

        // Idle: the worker must answer after a quiet period instead of sleeping
        // until another event.
        std::thread::sleep(Duration::from_millis(300));
        let mut idle = connect(&server);
        let started = Instant::now();
        command(&mut idle, &[b"PING"], b"+PONG\r\n");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "threads={threads}: idle worker did not answer promptly"
        );

        // Mixed: one client floods PING while another client's GET must still
        // complete. The workers are cooperative, so this proves progress rather
        // than fairness.
        let address = server.local_addr();
        let busy = std::thread::spawn(move || {
            let mut stream = TcpStream::connect(address).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            stream.set_nodelay(true).unwrap();
            for _ in 0..2000 {
                let mut request = Vec::new();
                encode_request(&[b"PING"], &mut request);
                stream.write_all(&request).unwrap();
                let mut reply = [0u8; 7];
                stream.read_exact(&mut reply).unwrap();
                assert_eq!(&reply, b"+PONG\r\n");
            }
        });
        let mut other = connect(&server);
        let started = Instant::now();
        command(&mut other, &[b"GET", b"absent"], b"$-1\r\n");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "threads={threads}: a second client did not progress while one was busy"
        );
        busy.join().unwrap();

        // Oversubscribed: more clients than workers, all must be served within
        // the bound; concurrent clients share the same cooperative lanes.
        let clients = threads * 8;
        let connections: Vec<_> = (0..clients).map(|_| connect(&server)).collect();
        let started = Instant::now();
        for mut stream in connections {
            command(&mut stream, &[b"PING"], b"+PONG\r\n");
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "threads={threads}: {clients} clients were not all served"
        );
        server.shutdown().unwrap();
    }
}
