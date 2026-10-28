#![forbid(unsafe_code)]

//! Hex audit keyring rotation coverage for server signing and collector verification.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream, UdpSocket};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use rillcache::engine::verify_audit_line;
use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

const NEW_KEY: &str = "0123456789abcdef";
const OLD_KEY: &str = "fedcba9876543210";
const NEW_HEX: &str = "30313233343536373839616263646566";
const OLD_HEX: &str = "66656463626139383736353433323130";
const SIGNED_LINE: &str = "1700000000000 command toplevel 616c696365 736574 9a6efad980470f46df31f9fd5790964a48b74191182cc31ea7897dabebc8f02e";

fn temp_name(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "rillcache-audit-keyring-{label}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ))
}

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
fn server_signs_with_the_first_keyring_key() {
    let acl = temp_name("server-acl");
    let audit = temp_name("server-audit");
    let keyring = temp_name("server-keyring");
    std::fs::write(
        &acl,
        b"user default on nopass ~* &* +@all\n\
user reader on >reader-secret allkeys allchannels -@all +get\n",
    )
    .unwrap();
    std::fs::write(&keyring, format!("{NEW_HEX}\n{OLD_HEX}\n")).unwrap();
    let server = start(Config {
        aclfile: Some(acl.clone()),
        audit_log: Some(audit.clone()),
        audit_hmac_keyring_file: Some(keyring.clone()),
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
    let line = std::fs::read_to_string(&audit).unwrap();
    assert!(verify_audit_line(NEW_KEY.as_bytes(), line.trim_end()));
    assert!(!verify_audit_line(OLD_KEY.as_bytes(), line.trim_end()));
    server.shutdown().unwrap();
    for path in [&acl, &audit, &keyring] {
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn collector_accepts_any_active_keyring_key() {
    let output = temp_name("collector-output");
    let keyring = temp_name("collector-keyring");
    assert!(verify_audit_line(NEW_KEY.as_bytes(), SIGNED_LINE));
    assert!(!verify_audit_line(OLD_KEY.as_bytes(), SIGNED_LINE));
    // The matching signing key is deliberately second: verify real rotation,
    // not merely the same first-key behavior as the server.
    std::fs::write(&keyring, format!("{OLD_HEX}\n{NEW_HEX}\n")).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_rillcache-audit-collector"))
        .args([
            "--bind",
            "127.0.0.1:0",
            "--output",
            output.to_str().unwrap(),
            "--max-datagrams",
            "1",
            "--hmac-keyring-file",
            keyring.to_str().unwrap(),
            "--print-address",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let address: SocketAddr = line
        .trim()
        .strip_prefix("listening on ")
        .expect("collector address")
        .parse()
        .unwrap();
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    let tampered = SIGNED_LINE.replacen("736574", "736575", 1);
    socket.send_to(tampered.as_bytes(), address).unwrap();
    socket.send_to(SIGNED_LINE.as_bytes(), address).unwrap();
    let status = child.wait().unwrap();
    assert!(status.success(), "{status:?}");
    assert_eq!(
        std::fs::read_to_string(&output).unwrap(),
        format!("{SIGNED_LINE}\n")
    );
    let _ = std::fs::remove_file(output);
    let _ = std::fs::remove_file(keyring);
}

#[test]
fn keyring_and_single_key_are_mutually_exclusive_and_invalid_keys_fail() {
    let key = temp_name("single-key");
    let keyring = temp_name("mutual-keyring");
    std::fs::write(&key, b"0123456789abcdef\n").unwrap();
    std::fs::write(&keyring, format!("{NEW_HEX}\n")).unwrap();
    let config = Config {
        audit_hmac_key_file: Some(key.clone()),
        audit_hmac_keyring_file: Some(keyring.clone()),
        ..Config::default()
    };
    assert!(Server::bind(config).is_err());
    let short = temp_name("short-keyring");
    std::fs::write(&short, b"00\n").unwrap();
    let config = Config {
        audit_hmac_keyring_file: Some(short.clone()),
        ..Config::default()
    };
    assert!(Server::bind(config).is_err());
    std::fs::write(&keyring, "#".repeat(64 * 1024 + 1)).unwrap();
    assert!(
        Server::bind(Config {
            audit_hmac_keyring_file: Some(keyring.clone()),
            ..Config::default()
        })
        .is_err()
    );
    assert!(rillcache::engine::read_audit_keyring(&keyring).is_err());
    let _ = std::fs::remove_file(key);
    let _ = std::fs::remove_file(keyring);
    let _ = std::fs::remove_file(short);
}
