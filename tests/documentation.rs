#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use rillcache::protocol::encode_request;
use rillcache::{Config, Server};

#[test]
fn onboarding_pages_links_and_logo_assets_pass_offline_checks() {
    let output = Command::new("python3")
        .arg("scripts/test_onboarding.py")
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")))
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .expect("Python 3 is required by the existing offline policy checks");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn readme_ping_set_get_example_works_without_configuration() {
    let server = Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        ..Config::default()
    })
    .unwrap()
    .spawn()
    .unwrap();
    let mut stream = TcpStream::connect(server.local_addr()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let cases: &[(&[&[u8]], &[u8])] = &[
        (&[b"PING"], b"+PONG\r\n"),
        (&[b"SET", b"greeting", b"hello"], b"+OK\r\n"),
        (&[b"GET", b"greeting"], b"$5\r\nhello\r\n"),
    ];
    for (args, expected) in cases {
        let mut request = Vec::new();
        encode_request(args, &mut request);
        stream.write_all(&request).unwrap();
        let mut actual = vec![0; expected.len()];
        stream.read_exact(&mut actual).unwrap();
        assert_eq!(actual.as_slice(), *expected);
    }
    drop(stream);
    server.shutdown().unwrap();
}
