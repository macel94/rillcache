#![forbid(unsafe_code)]

//! C9: the stream metadata divergences are declared in the profile and the
//! supported fields and exact-trim semantics are pinned by tests.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::time::Duration;

use rillcache::protocol::encode_request;
use rillcache::{Config, Server};

fn ask(stream: &mut TcpStream, args: &[&[u8]]) -> Vec<u8> {
    let mut request = Vec::new();
    encode_request(args, &mut request);
    stream.write_all(&request).unwrap();
    let mut buffer = [0u8; 64 * 1024];
    let read = stream.read(&mut buffer).unwrap();
    buffer[..read].to_vec()
}

#[test]
fn stream_metadata_divergences_are_declared_and_pinned() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let profile = std::fs::read_to_string(manifest.join("docs/production-profile.md"))
        .expect("production profile");
    for statement in ["nacked-count", "XTRIM", "stream-trim-exact"] {
        assert!(
            profile.contains(statement),
            "the profile must declare the stream divergence {statement:?}"
        );
    }
    let corpus = std::fs::read_to_string(manifest.join("ci/upstream/differential-v1.json"))
        .expect("differential corpus");
    assert!(
        corpus.contains("stream-trim-exact"),
        "the pinned differential corpus must exercise exact stream trimming"
    );

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
        for id in [b"1-1".as_slice(), b"2-1", b"3-1"] {
            assert!(ask(&mut stream, &[b"XADD", b"s", id, b"f", b"v"]).starts_with(b"$"));
        }
        assert_eq!(
            ask(&mut stream, &[b"XTRIM", b"s", b"MAXLEN", b"2"]),
            b":1\r\n"
        );
        assert_eq!(ask(&mut stream, &[b"XLEN", b"s"]), b":2\r\n");
        assert!(ask(&mut stream, &[b"XGROUP", b"CREATE", b"s", b"g", b"0"]).starts_with(b"+"));
        let groups = ask(&mut stream, &[b"XINFO", b"GROUPS", b"s"]);
        for field in [b"entries-read".as_slice(), b"lag", b"last-delivered-id"] {
            assert!(
                groups.windows(field.len()).any(|window| window == field),
                "XINFO GROUPS must report {field:?}: {}",
                String::from_utf8_lossy(&groups)
            );
        }
        assert!(
            !groups.windows(12).any(|window| window == b"nacked-count"),
            "the declared divergence is that XINFO GROUPS omits nacked-count"
        );
        server.shutdown().unwrap();
    }
}
