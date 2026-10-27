#![forbid(unsafe_code)]

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

fn temp_name(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "rillcache-acl-{label}-{}-{:?}.acl",
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

fn read_to_end(stream: &mut TcpStream) -> Vec<u8> {
    let mut output = Vec::new();
    stream.read_to_end(&mut output).unwrap();
    output
}

#[test]
fn acl_file_users_restrict_commands_keys_and_are_observable() {
    let path = temp_name("users");
    std::fs::write(
        &path,
        b"# managed by the ACL test\n\
user default on nopass ~* &* +@all\n\
user alice on >alice-password resetkeys ~app:* resetchannels &events:* -@all +@read +@write +@transaction\n",
    )
    .unwrap();
    let server = start(Config {
        aclfile: Some(path.clone()),
        ..Config::default()
    });
    let mut admin = connect(&server);
    send(&mut admin, &[b"PING"], b"+PONG\r\n");
    send(&mut admin, &[b"ACL", b"WHOAMI"], b"$7\r\ndefault\r\n");
    let mut alice = connect(&server);
    send(
        &mut alice,
        &[b"AUTH", b"alice", b"alice-password"],
        b"+OK\r\n",
    );
    send(&mut alice, &[b"ACL", b"WHOAMI"], b"$5\r\nalice\r\n");
    send(&mut alice, &[b"SET", b"app:key", b"value"], b"+OK\r\n");
    send(&mut alice, &[b"GET", b"app:key"], b"$5\r\nvalue\r\n");
    send(
        &mut alice,
        &[b"GET", b"other:key"],
        b"-NOPERM this user has no permissions to access one of the keys used as arguments\r\n",
    );
    send(
        &mut alice,
        &[b"SET", b"other:key", b"value"],
        b"-NOPERM this user has no permissions to access one of the keys used as arguments\r\n",
    );
    send(
        &mut alice,
        &[b"PING"],
        b"-NOPERM this user has no permissions to run the 'ping' command\r\n",
    );
    send(&mut alice, &[b"DEL", b"app:key"], b":1\r\n");

    send(
        &mut admin,
        &[
            b"ACL",
            b"SETUSER",
            b"bob",
            b"on",
            b">bob-password",
            b"allkeys",
            b"allchannels",
            b"-@all",
            b"+@read",
        ],
        b"+OK\r\n",
    );
    let mut bob = connect(&server);
    send(&mut bob, &[b"AUTH", b"bob", b"bob-password"], b"+OK\r\n");
    send(&mut bob, &[b"GET", b"any"], b"$-1\r\n");
    send(
        &mut bob,
        &[b"SET", b"any", b"value"],
        b"-NOPERM this user has no permissions to run the 'set' command\r\n",
    );

    send(&mut admin, &[b"ACL", b"SAVE"], b"+OK\r\n");
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(saved.contains("user alice on #"));
    assert!(saved.contains("user bob on #"));
    assert!(!saved.contains("alice-password"));
    assert!(!saved.contains("bob-password"));
    send(&mut admin, &[b"ACL", b"LOAD"], b"+OK\r\n");

    send(
        &mut admin,
        &[b"ACL", b"DELUSER", b"alice", b"bob"],
        b":2\r\n",
    );
    send(
        &mut alice,
        &[b"GET", b"app:key"],
        b"-NOAUTH Authentication required.\r\n",
    );
    send(
        &mut bob,
        &[b"GET", b"any"],
        b"-NOAUTH Authentication required.\r\n",
    );

    server.shutdown().unwrap();
    let _ = std::fs::remove_file(&path);
}

#[test]
fn acl_file_and_authentication_failures_are_visible_at_startup() {
    let missing = temp_name("missing");
    let error = Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        aclfile: Some(missing),
        ..Config::default()
    })
    .err()
    .unwrap();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);

    let no_default = temp_name("no-default");
    std::fs::write(
        &no_default,
        b"user alice on nopass allkeys allchannels +@all\n",
    )
    .unwrap();
    assert!(
        Server::bind(Config {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            aclfile: Some(no_default.clone()),
            ..Config::default()
        })
        .is_err()
    );
    std::fs::remove_file(&no_default).unwrap();

    let invalid = temp_name("invalid");
    std::fs::write(
        &invalid,
        b"user default on nopass allkeys allchannels +@all\nuser alice on +@no-such-category\n",
    )
    .unwrap();
    assert!(
        Server::bind(Config {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            aclfile: Some(invalid.clone()),
            ..Config::default()
        })
        .is_err()
    );
    std::fs::remove_file(&invalid).unwrap();

    let error = Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        requirepass: Some(b"pw".to_vec()),
        aclfile: Some(temp_name("both")),
        ..Config::default()
    })
    .err()
    .unwrap();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
}

#[test]
fn acl_denial_keeps_pipelines_ordered_and_quit_available() {
    let server = start(Config::default());
    let mut admin = connect(&server);
    send(
        &mut admin,
        &[
            b"ACL", b"SETUSER", b"reader", b"on", b">pw", b"allkeys", b"-@all", b"+@read",
        ],
        b"+OK\r\n",
    );
    let mut stream = connect(&server);
    let mut pipeline = Vec::new();
    encode_request(&[b"AUTH", b"reader", b"pw"], &mut pipeline);
    encode_request(&[b"GET", b"key"], &mut pipeline);
    encode_request(&[b"SET", b"key", b"v"], &mut pipeline);
    encode_request(&[b"GET", b"key"], &mut pipeline);
    encode_request(&[b"QUIT"], &mut pipeline);
    stream.write_all(&pipeline).unwrap();
    let expected = b"+OK\r\n$-1\r\n-NOPERM this user has no permissions to run the 'set' command\r\n$-1\r\n+OK\r\n";
    assert_eq!(read_to_end(&mut stream), expected);
    server.shutdown().unwrap();
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

#[test]
fn acl_selectors_authorize_independent_key_scopes_and_round_trip() {
    let path = temp_name("selectors");
    std::fs::write(&path, b"user default on nopass ~* &* +@all\n").unwrap();
    let config = Config {
        aclfile: Some(path.clone()),
        ..Config::default()
    };

    {
        let server = start(config.clone());
        let mut admin = connect(&server);
        send(
            &mut admin,
            &[
                b"ACL",
                b"SETUSER",
                b"alice",
                b"on",
                b">secret",
                b"-@all",
                b"+get",
                b"allkeys",
                b"(+set ~app:*)",
            ],
            b"+OK\r\n",
        );
        let list = String::from_utf8(request(&mut admin, &[b"ACL", b"LIST"])).unwrap();
        assert!(list.contains("(+set ~app:*)"), "{list}");
        let details =
            String::from_utf8(request(&mut admin, &[b"ACL", b"GETUSER", b"alice"])).unwrap();
        assert!(details.contains("selectors"), "{details}");
        assert!(details.contains("+set"), "{details}");
        assert!(details.contains("~app:*"), "{details}");
        send(&mut admin, &[b"ACL", b"SAVE"], b"+OK\r\n");

        let mut alice = connect(&server);
        send(&mut alice, &[b"AUTH", b"alice", b"secret"], b"+OK\r\n");
        send(&mut alice, &[b"SET", b"app:1", b"v"], b"+OK\r\n");
        send(&mut alice, &[b"GET", b"app:1"], b"$1\r\nv\r\n");
        send(&mut alice, &[b"GET", b"other"], b"$-1\r\n");
        send(
            &mut alice,
            &[b"SET", b"other", b"v"],
            b"-NOPERM this user has no permissions to access one of the keys used as arguments\r\n",
        );
        server.shutdown().unwrap();
    }

    {
        let server = start(config);
        let mut alice = connect(&server);
        send(&mut alice, &[b"AUTH", b"alice", b"secret"], b"+OK\r\n");
        send(&mut alice, &[b"SET", b"app:2", b"v"], b"+OK\r\n");
        send(
            &mut alice,
            &[b"SET", b"other", b"v"],
            b"-NOPERM this user has no permissions to access one of the keys used as arguments\r\n",
        );
        server.shutdown().unwrap();
    }
    let _ = std::fs::remove_file(&path);
}
