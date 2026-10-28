#![forbid(unsafe_code)]

//! Persistent ACL denial audit: bounded lines, rotation, restart append and
//! visible write-failure accounting.

use std::io::{Read, Write};
use std::net::TcpStream;
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

fn temp_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "rillcache-audit-{label}-{}-{:?}",
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

fn read_frame(stream: &mut TcpStream, depth: usize) -> Vec<u8> {
    assert!(depth <= 8);
    let mut output = Vec::new();
    let mut prefix = [0u8];
    stream.read_exact(&mut prefix).unwrap();
    output.push(prefix[0]);
    let mut line = Vec::new();
    loop {
        let mut byte = [0u8];
        stream.read_exact(&mut byte).unwrap();
        if byte[0] == b'\n' {
            assert_eq!(line.pop(), Some(b'\r'));
            output.extend_from_slice(&line);
            output.extend_from_slice(b"\r\n");
            break;
        }
        line.push(byte[0]);
    }
    match prefix[0] {
        b'$' => {
            let length = std::str::from_utf8(&line).unwrap().parse::<i64>().unwrap();
            if length >= 0 {
                let mut body = vec![0; length as usize + 2];
                stream.read_exact(&mut body).unwrap();
                output.extend_from_slice(&body);
            }
        }
        b'*' => {
            let count = std::str::from_utf8(&line).unwrap().parse::<i64>().unwrap();
            for _ in 0..count.max(0) {
                output.extend_from_slice(&read_frame(stream, depth + 1));
            }
        }
        _ => {}
    }
    output
}

fn request(stream: &mut TcpStream, arguments: &[&[u8]]) -> Vec<u8> {
    let mut bytes = Vec::new();
    encode_request(arguments, &mut bytes);
    stream.write_all(&bytes).unwrap();
    read_frame(stream, 0)
}

fn audit_acl(path: &PathBuf) {
    std::fs::write(
        path,
        b"user default on nopass ~* &* +@all\n\
user reader on >reader-secret allkeys allchannels -@all +get\n",
    )
    .unwrap();
}

#[test]
fn audit_records_denials_bounded_hex_and_appends_across_restart() {
    let acl = temp_path("acl");
    let audit = temp_path("audit");
    audit_acl(&acl);
    let config = Config {
        aclfile: Some(acl.clone()),
        audit_log: Some(audit.clone()),
        audit_log_max_bytes: 4096,
        ..Config::default()
    };

    {
        let server = start(config.clone());
        let mut admin = connect(&server);
        send(&mut admin, &[b"PING"], b"+PONG\r\n");
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
        send(&mut reader, &[b"GET", b"secret-key"], b"$-1\r\n");

        let contents = std::fs::read_to_string(&audit).unwrap();
        let lines: Vec<_> = contents.lines().collect();
        assert_eq!(lines.len(), 1, "{contents}");
        let fields: Vec<_> = lines[0].split(' ').collect();
        assert_eq!(fields.len(), 5, "{contents}");
        assert!(fields[0].chars().all(|ch| ch.is_ascii_digit()));
        assert_eq!(fields[1], "command");
        assert_eq!(fields[2], "toplevel");
        assert_eq!(fields[3], "726561646572");
        assert_eq!(fields[4], "736574");
        assert!(!contents.contains("reader-secret"));
        assert!(!contents.contains("secret-key"));
        let before_reset = std::fs::metadata(&audit).unwrap().len();
        send(&mut admin, &[b"ACL", b"LOG", b"RESET"], b"+OK\r\n");
        assert_eq!(std::fs::metadata(&audit).unwrap().len(), before_reset);
        server.shutdown().unwrap();
    }

    {
        let server = start(config.clone());
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
        let contents = std::fs::read_to_string(&audit).unwrap();
        assert_eq!(contents.lines().count(), 2, "{contents}");
        server.shutdown().unwrap();
    }

    let _ = std::fs::remove_file(&acl);
    let _ = std::fs::remove_file(&audit);
    let mut rotated = audit.into_os_string();
    rotated.push(".1");
    let _ = std::fs::remove_file(PathBuf::from(rotated));
}

#[test]
fn audit_rotates_one_generation_and_keeps_the_old_file() {
    let acl = temp_path("rotate-acl");
    let audit = temp_path("rotate-audit");
    audit_acl(&acl);
    let server = start(Config {
        aclfile: Some(acl.clone()),
        audit_log: Some(audit.clone()),
        audit_log_max_bytes: 1024,
        ..Config::default()
    });
    let mut reader = connect(&server);
    send(
        &mut reader,
        &[b"AUTH", b"reader", b"reader-secret"],
        b"+OK\r\n",
    );
    for _ in 0..40 {
        send(
            &mut reader,
            &[b"SET", b"secret-key", b"v"],
            b"-NOPERM this user has no permissions to run the 'set' command\r\n",
        );
    }
    let mut rotated = audit.as_os_str().to_os_string();
    rotated.push(".1");
    assert!(PathBuf::from(rotated).exists());
    assert!(std::fs::metadata(&audit).unwrap().len() <= 1024);
    server.shutdown().unwrap();

    let _ = std::fs::remove_file(&acl);
    let _ = std::fs::remove_file(&audit);
    let mut rotated = audit.into_os_string();
    rotated.push(".1");
    let _ = std::fs::remove_file(PathBuf::from(rotated));
}

#[test]
fn audit_open_failure_is_a_startup_error() {
    let config = Config {
        audit_log: Some(temp_path("missing-directory").join("audit.log")),
        ..Config::default()
    };
    assert!(Server::bind(config).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn audit_write_failure_is_visible_in_info_stats() {
    let acl = temp_path("full-acl");
    audit_acl(&acl);
    let server = start(Config {
        aclfile: Some(acl.clone()),
        audit_log: Some(PathBuf::from("/dev/full")),
        audit_log_max_bytes: 1024,
        ..Config::default()
    });
    let mut admin = connect(&server);
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
    let stats = request(&mut admin, &[b"INFO", b"stats"]);
    let stats = String::from_utf8_lossy(&stats);
    assert!(stats.contains("audit_log_write_failures:1"), "{stats}");
    server.shutdown().unwrap();
    let _ = std::fs::remove_file(acl);
}
