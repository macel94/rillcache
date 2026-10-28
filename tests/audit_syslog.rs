#![forbid(unsafe_code)]

//! Off-host audit shipping over a best-effort UDP sink.

use std::io::{Read, Write};
use std::net::{TcpStream, UdpSocket};
use std::path::PathBuf;
use std::time::Duration;

use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

fn start(config: Config) -> ServerHandle {
    let workers = match std::env::var("RILLCACHE_TEST_THREADS").as_deref() {
        Ok("0") | Err(_) => None,
        Ok("1") => Some(1),
        Ok("4") => Some(4),
        Ok(value) => panic!("invalid test worker mode {value}"),
    };
    Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        worker_threads: workers,
        ..config
    })
    .unwrap()
    .spawn()
    .unwrap()
}

fn temp_name(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "rillcache-syslog-{label}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ))
}

fn connect(server: &ServerHandle) -> TcpStream {
    let stream = TcpStream::connect(server.local_addr()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream.set_nodelay(true).unwrap();
    stream
}

fn send(stream: &mut TcpStream, arguments: &[&[u8]], reply: &[u8]) {
    let mut bytes = Vec::new();
    encode_request(arguments, &mut bytes);
    stream.write_all(&bytes).unwrap();
    let mut actual = vec![0; reply.len()];
    stream.read_exact(&mut actual).unwrap();
    assert_eq!(actual, reply);
}

#[test]
fn syslog_sink_receives_the_same_redacted_denial_line() {
    let acl = temp_name("acl");
    std::fs::write(
        &acl,
        b"user default on nopass ~* &* +@all\n\
user reader on >reader-secret allkeys allchannels -@all +get\n",
    )
    .unwrap();
    let receiver = UdpSocket::bind("127.0.0.1:0").unwrap();
    receiver
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let server = start(Config {
        aclfile: Some(acl.clone()),
        audit_syslog: Some(receiver.local_addr().unwrap()),
        ..Config::default()
    });
    let mut reader = connect(&server);
    send(
        &mut reader,
        &[b"AUTH", b"reader", b"reader-secret"],
        b"+OK\r\n",
    );
    send(
        &mut reader,
        &[b"SET", b"secret-key", b"v"],
        b"-NOPERM this user has no permissions to run the 'set' command\r\n",
    );
    let mut buffer = [0u8; 2048];
    let (length, _) = receiver.recv_from(&mut buffer).unwrap();
    let line = std::str::from_utf8(&buffer[..length]).unwrap().trim_end();
    let fields: Vec<_> = line.split(' ').collect();
    assert_eq!(fields.len(), 5, "{line}");
    assert!(fields[0].chars().all(|ch| ch.is_ascii_digit()));
    assert_eq!(fields[1], "command");
    assert_eq!(fields[2], "toplevel");
    assert_eq!(fields[3], "726561646572");
    assert_eq!(fields[4], "736574");
    assert!(!line.contains("reader-secret"));
    assert!(!line.contains("secret-key"));
    server.shutdown().unwrap();
    let _ = std::fs::remove_file(acl);
}

#[test]
fn invalid_syslog_destination_fails_startup() {
    let config = Config {
        audit_syslog: Some("127.0.0.1:0".parse().unwrap()),
        ..Config::default()
    };
    assert!(Server::bind(config).is_err());
}
