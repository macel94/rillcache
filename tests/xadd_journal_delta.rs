#![forbid(unsafe_code)]

//! #710: a plain `XADD` journals only the appended entry, so journal growth is
//! linear in appended entries rather than proportional to the stream length, and
//! replay still restores every entry.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::time::Duration;

use rillcache::protocol::encode_request;
use rillcache::{Config, Server};

fn journal_bytes(directory: &Path) -> u64 {
    std::fs::read_dir(directory)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "aof"))
        .filter_map(|entry| entry.metadata().ok())
        .map(|metadata| metadata.len())
        .sum()
}

fn ask(stream: &mut TcpStream, args: &[&[u8]]) -> Vec<u8> {
    let mut request = Vec::new();
    encode_request(args, &mut request);
    stream.write_all(&request).unwrap();
    let mut buffer = [0u8; 16 * 1024];
    let read = stream.read(&mut buffer).unwrap();
    buffer[..read].to_vec()
}

#[test]
fn plain_xadd_journals_a_linear_number_of_bytes_per_entry() {
    let directory = std::env::temp_dir().join(format!(
        "rillcache-xadd-delta-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let value = "v".repeat(256);
    let mut config = Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        ..Config::default()
    };
    config.appendonly = true;
    config.appendfilename = "appendonly.aof".into();
    config.dir = directory.clone();

    let server = Server::bind(config.clone()).unwrap().spawn().unwrap();
    let mut stream = TcpStream::connect(server.local_addr()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();

    let mut sizes = Vec::new();
    for index in 0..300 {
        let reply = ask(&mut stream, &[b"XADD", b"s", b"*", b"f", value.as_bytes()]);
        assert!(reply.starts_with(b"$"), "XADD failed: {reply:?}");
        if (index + 1) % 100 == 0 {
            sizes.push(journal_bytes(&directory));
        }
    }
    assert_eq!(ask(&mut stream, &[b"XLEN", b"s"]), b":300\r\n");
    server.shutdown().unwrap();

    // Bytes appended for the last 100 entries must stay close to the entry size:
    // the whole-key re-encode cost ~80 KB per entry at this length.
    let last_100 = sizes[2] - sizes[1];
    assert!(
        last_100 < 100 * 1024,
        "journal grew {last_100} bytes for 100 entries (>1 KiB/entry means the \
         whole stream is being re-encoded per append)"
    );

    let restarted = Server::bind(config).unwrap().spawn().unwrap();
    let mut stream = TcpStream::connect(restarted.local_addr()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    assert_eq!(
        ask(&mut stream, &[b"XLEN", b"s"]),
        b":300\r\n",
        "replay must restore every appended entry"
    );
    let range = ask(&mut stream, &[b"XRANGE", b"s", b"-", b"+"]);
    assert!(
        range.starts_with(b"*300\r\n"),
        "XRANGE after replay: {:?}",
        &range[..32.min(range.len())]
    );
    restarted.shutdown().unwrap();
    std::fs::remove_dir_all(directory).ok();
}
