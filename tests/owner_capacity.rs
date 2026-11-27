#![forbid(unsafe_code)]

//! Declared single-owner capacity envelope (R1).
//!
//! The default profile runs one data owner. Its capacity is a declared envelope
//! of enforced admission bounds, not a throughput SLO: one owner CPU, a logical
//! payload budget and a connection limit. This test proves the declared numbers
//! default correctly and that exceeding the connection bound is an explicit
//! refusal rather than a silent drop.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

fn start(workers: Option<usize>, max_clients: usize) -> ServerHandle {
    Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        worker_threads: workers,
        max_clients,
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
}

fn ping(stream: &mut TcpStream) -> std::io::Result<()> {
    let mut request = Vec::new();
    encode_request(&[b"PING"], &mut request);
    stream.write_all(&request)?;
    let mut reply = [0u8; 7];
    stream.read_exact(&mut reply)?;
    assert_eq!(&reply, b"+PONG\r\n");
    Ok(())
}

#[test]
fn the_declared_single_owner_envelope_is_the_default_and_enforced() {
    let declared = Config::default();
    assert_eq!(
        declared.storage_owners, None,
        "the default data path must remain single-owner"
    );
    assert_eq!(declared.max_clients, 128);

    for workers in [None, Some(1), Some(4)] {
        let server = start(workers, 2);
        let mut first = connect(&server);
        ping(&mut first).unwrap();
        let mut second = connect(&server);
        ping(&mut second).unwrap();

        // The declared connection bound is an explicit refusal, not a silent
        // reset or an accepted-then-ignored client.
        let mut refused = connect(&server);
        let mut reply = Vec::new();
        refused.read_to_end(&mut reply).unwrap();
        assert_eq!(reply, b"-ERR max number of clients reached\r\n");

        // Existing clients keep working after the refusal.
        ping(&mut second).unwrap();

        // Closing a client frees capacity within the declared bound; retry
        // rather than assuming the owner has noticed the close immediately.
        drop(first);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let mut replacement = connect(&server);
            if ping(&mut replacement).is_ok() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "capacity was not released after an admitted client closed"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        ping(&mut second).unwrap();
        server.shutdown().unwrap();
    }
}
