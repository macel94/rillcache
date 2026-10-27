#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

fn start() -> ServerHandle {
    let workers = match std::env::var("RILLCACHE_TEST_THREADS").as_deref() {
        Ok("0") | Err(_) => None,
        Ok("1") => Some(1),
        Ok("4") => Some(4),
        Ok(value) => panic!("invalid test worker mode {value}"),
    };
    Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        worker_threads: workers,
        ..Config::default()
    })
    .unwrap()
    .spawn()
    .unwrap()
}

fn connect(server: &ServerHandle) -> TcpStream {
    let stream = TcpStream::connect(server.local_addr()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream.set_nodelay(true).unwrap();
    stream
}

fn send(stream: &mut TcpStream, arguments: &[&[u8]], expected: &[u8]) {
    let mut request = Vec::new();
    encode_request(arguments, &mut request);
    stream.write_all(&request).unwrap();
    if !expected.is_empty() {
        let mut actual = vec![0; expected.len()];
        stream
            .read_exact(&mut actual)
            .unwrap_or_else(|error| panic!("no reply for {arguments:?}: {error}"));
        assert_eq!(actual, expected, "command {arguments:?}");
    }
}

fn read_line(stream: &mut TcpStream, output: &mut Vec<u8>) -> String {
    let mut line = Vec::new();
    let mut byte = [0];
    loop {
        stream.read_exact(&mut byte).unwrap();
        if byte[0] == b'\n' {
            assert_eq!(line.pop(), Some(b'\r'));
            output.extend_from_slice(&line);
            output.extend_from_slice(b"\r\n");
            return String::from_utf8(line).unwrap();
        }
        line.push(byte[0]);
    }
}

fn read_frame(stream: &mut TcpStream) -> Vec<u8> {
    let mut output = Vec::new();
    let mut prefix = [0];
    stream.read_exact(&mut prefix).unwrap();
    output.push(prefix[0]);
    match prefix[0] {
        b'+' | b'-' | b':' => {
            read_line(stream, &mut output);
        }
        b'$' => {
            let length = read_line(stream, &mut output).parse::<i64>().unwrap();
            if length >= 0 {
                let mut body = vec![0; length as usize + 2];
                stream.read_exact(&mut body).unwrap();
                output.extend_from_slice(&body);
            }
        }
        b'*' => {
            let count = read_line(stream, &mut output).parse::<i64>().unwrap();
            for _ in 0..count.max(0) {
                output.extend_from_slice(&read_frame(stream));
            }
        }
        other => panic!("unexpected prefix {other:?}"),
    }
    output
}

fn bulk(frame: &[u8]) -> Vec<u8> {
    assert_eq!(frame.first(), Some(&b'$'));
    let newline = frame
        .windows(2)
        .position(|window| window == b"\r\n")
        .unwrap();
    let size: usize = std::str::from_utf8(&frame[1..newline])
        .unwrap()
        .parse()
        .unwrap();
    frame[newline + 2..newline + 2 + size].to_vec()
}

#[test]
fn genpass_dryrun_and_the_denial_log_are_available_to_administrators() {
    let server = start();
    let mut admin = connect(&server);
    send(&mut admin, &[b"ACL", b"GENPASS"], b"");
    let password = bulk(&read_frame(&mut admin));
    assert_eq!(password.len(), 64);
    assert!(password.iter().all(u8::is_ascii_hexdigit));
    send(&mut admin, &[b"ACL", b"GENPASS", b"128"], b"");
    let password = bulk(&read_frame(&mut admin));
    assert_eq!(password.len(), 32);
    send(
        &mut admin,
        &[b"ACL", b"GENPASS", b"0"],
        b"-ERR GENPASS bits must be 1..=1024\r\n",
    );

    send(
        &mut admin,
        &[
            b"ACL",
            b"SETUSER",
            b"alice",
            b"on",
            b">pw",
            b"resetkeys",
            b"~app:*",
            b"resetchannels",
            b"&allowed:*",
            b"-@all",
            b"+@read",
            b"+@write",
            b"+@pubsub",
        ],
        b"+OK\r\n",
    );
    send(
        &mut admin,
        &[b"ACL", b"DRYRUN", b"alice", b"GET", b"app:k"],
        b"+OK\r\n",
    );
    send(
        &mut admin,
        &[b"ACL", b"DRYRUN", b"alice", b"GET", b"other"],
        b"-NOPERM this user has no permissions to access one of the keys used as arguments\r\n",
    );
    send(
        &mut admin,
        &[b"ACL", b"DRYRUN", b"alice", b"PING"],
        b"-NOPERM this user has no permissions to run the 'ping' command\r\n",
    );
    send(
        &mut admin,
        &[b"ACL", b"DRYRUN", b"missing", b"GET", b"k"],
        b"-ERR User not found\r\n",
    );

    let mut alice = connect(&server);
    send(&mut alice, &[b"AUTH", b"alice", b"pw"], b"+OK\r\n");
    send(
        &mut alice,
        &[b"GET", b"other"],
        b"-NOPERM this user has no permissions to access one of the keys used as arguments\r\n",
    );
    send(
        &mut alice,
        &[b"PUBLISH", b"other", b"x"],
        b"-NOPERM this user has no permissions to access one of the channels used as arguments\r\n",
    );
    send(
        &mut alice,
        &[b"PING"],
        b"-NOPERM this user has no permissions to run the 'ping' command\r\n",
    );
    send(&mut admin, &[b"ACL", b"LOG"], b"");
    let log = read_frame(&mut admin);
    assert!(log.starts_with(b"*3\r\n"), "{log:?}");
    assert!(log.windows(5).any(|window| window == b"alice"));
    assert!(log.windows(5).any(|window| window == b"other"));
    assert!(log.windows(7).any(|window| window == b"channel"), "{log:?}");

    send(&mut admin, &[b"ACL", b"LOG", b"RESET"], b"+OK\r\n");
    send(&mut admin, &[b"ACL", b"LOG"], b"*0\r\n");
    server.shutdown().unwrap();
}
