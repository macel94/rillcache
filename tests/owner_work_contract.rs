#![forbid(unsafe_code)]

//! R4: the O(N) commands are explicitly outside the latency contract, the
//! measured occupancy guidance is published, and small commands still work.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::time::Duration;

use rillcache::protocol::encode_request;
use rillcache::{Config, Server};

#[test]
fn o_n_commands_are_excluded_from_the_latency_contract_with_measured_guidance() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let profile = std::fs::read_to_string(manifest.join("docs/production-profile.md"))
        .expect("production profile");
    for statement in [
        "no per-command",
        "deadline, cancellation or CPU quota",
        "excluded from",
        "owner-work-guidance.md",
    ] {
        assert!(
            profile.contains(statement),
            "the profile must keep the statement {statement:?}"
        );
    }

    let guidance = std::fs::read_to_string(manifest.join("docs/owner-work-guidance.md"))
        .expect("owner work guidance");
    for row in [
        "KEYS *",
        "SCAN full iteration",
        "ZRANGEBYSCORE all",
        "HGETALL",
    ] {
        assert!(
            guidance.contains(row),
            "the measured table must keep {row:?}"
        );
    }
    assert!(
        guidance.contains("no per-command deadline, cancellation or CPU quota"),
        "guidance must state that no deadline or cancellation exists"
    );

    // A small keyspace command still works in every execution mode.
    for workers in [None, Some(1), Some(4)] {
        let server = Server::bind(Config {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            worker_threads: workers,
            ..Config::default()
        })
        .unwrap()
        .spawn()
        .unwrap();
        let mut stream = TcpStream::connect(server.local_addr()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        for index in 0..10 {
            let mut request = Vec::new();
            encode_request(
                &[b"SET", format!("k{index}").as_bytes(), b"v"],
                &mut request,
            );
            stream.write_all(&request).unwrap();
            let mut reply = [0u8; 5];
            stream.read_exact(&mut reply).unwrap();
            assert_eq!(&reply, b"+OK\r\n");
        }
        let mut request = Vec::new();
        encode_request(&[b"KEYS", b"*"], &mut request);
        stream.write_all(&request).unwrap();
        let mut buffer = [0u8; 4096];
        let read = stream.read(&mut buffer).unwrap();
        assert!(
            buffer[..read].starts_with(b"*10\r\n"),
            "KEYS must still return the small keyspace: {}",
            String::from_utf8_lossy(&buffer[..read])
        );
        server.shutdown().unwrap();
    }
}
