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
fn pubsub_introspection_filters_and_rejects_acl_channel_disclosures() {
    let server = start();
    let mut admin = connect(&server);
    send(
        &mut admin,
        &[
            b"ACL",
            b"SETUSER",
            b"observer",
            b"on",
            b">pw",
            b"allkeys",
            b"-@all",
            b"+@pubsub",
            b"resetchannels",
            b"&allowed:*",
        ],
        b"+OK\r\n",
    );

    let mut normal_subscriber = connect(&server);
    send(
        &mut normal_subscriber,
        &[b"SUBSCRIBE", b"allowed:live"],
        b"",
    );
    read_frame(&mut normal_subscriber);
    let mut hidden_subscriber = connect(&server);
    send(&mut hidden_subscriber, &[b"SUBSCRIBE", b"secret:live"], b"");
    read_frame(&mut hidden_subscriber);
    let mut binary_subscriber = connect(&server);
    send(
        &mut binary_subscriber,
        &[b"SUBSCRIBE", b"allowed:\0", b"secret:\xff", b""],
        b"",
    );
    for _ in 0..3 {
        read_frame(&mut binary_subscriber);
    }
    let mut shard_subscriber = connect(&server);
    send(
        &mut shard_subscriber,
        &[b"SSUBSCRIBE", b"allowed:shard"],
        b"",
    );
    read_frame(&mut shard_subscriber);
    let mut hidden_shard_subscriber = connect(&server);
    send(
        &mut hidden_shard_subscriber,
        &[b"SSUBSCRIBE", b"secret:shard"],
        b"",
    );
    read_frame(&mut hidden_shard_subscriber);

    send(
        &mut admin,
        &[
            b"ACL",
            b"SETUSER",
            b"selected",
            b"on",
            b">pw2",
            b"allkeys",
            b"-@all",
            b"resetchannels",
            b"(+@pubsub &allowed:*)",
        ],
        b"+OK\r\n",
    );
    let mut selector_user = connect(&server);
    send(
        &mut selector_user,
        &[b"AUTH", b"selected", b"pw2"],
        b"+OK\r\n",
    );
    send(&mut selector_user, &[b"PUBSUB", b"CHANNELS"], b"");
    let selector_channels = read_frame(&mut selector_user);
    assert!(
        selector_channels
            .windows(12)
            .any(|window| window == b"allowed:live")
    );
    assert!(
        !selector_channels
            .windows(11)
            .any(|window| window == b"secret:live")
    );

    let mut observer = connect(&server);
    send(&mut observer, &[b"AUTH", b"observer", b"pw"], b"+OK\r\n");
    send(&mut observer, &[b"PUBSUB", b"CHANNELS"], b"");
    let channels = read_frame(&mut observer);
    assert!(channels.windows(12).any(|window| window == b"allowed:live"));
    assert!(channels.windows(9).any(|window| window == b"allowed:\0"));
    assert!(!channels.windows(11).any(|window| window == b"secret:live"));
    assert!(!channels.windows(8).any(|window| window == b"secret:\xff"));
    assert!(!channels.windows(6).any(|window| window == b"$0\r\n\r\n"));

    send(&mut observer, &[b"PUBSUB", b"CHANNELS", b"allowed:*"], b"");
    let patterned = read_frame(&mut observer);
    assert!(
        patterned
            .windows(12)
            .any(|window| window == b"allowed:live")
    );
    assert!(!patterned.windows(11).any(|window| window == b"secret:live"));
    send(&mut observer, &[b"PUBSUB", b"NUMSUB", b"allowed:live"], b"");
    assert_eq!(
        read_frame(&mut observer),
        b"*2\r\n$12\r\nallowed:live\r\n:1\r\n"
    );
    send(
        &mut observer,
        &[b"PUBSUB", b"NUMSUB", b"allowed:live", b"secret:live"],
        b"-NOPERM this user has no permissions to access one of the channels used as arguments\r\n",
    );
    send(
        &mut observer,
        &[b"PUBSUB", b"NUMSUB", b"allowed:live", b""],
        b"-NOPERM this user has no permissions to access one of the channels used as arguments\r\n",
    );
    send(&mut observer, &[b"PUBSUB", b"SHARDCHANNELS"], b"");
    let shard_channels = read_frame(&mut observer);
    assert!(
        shard_channels
            .windows(13)
            .any(|window| window == b"allowed:shard")
    );
    assert!(
        !shard_channels
            .windows(12)
            .any(|window| window == b"secret:shard")
    );
    send(
        &mut observer,
        &[b"PUBSUB", b"SHARDNUMSUB", b"allowed:shard"],
        b"",
    );
    assert_eq!(
        read_frame(&mut observer),
        b"*2\r\n$13\r\nallowed:shard\r\n:1\r\n"
    );
    send(
        &mut observer,
        &[b"PUBSUB", b"SHARDNUMSUB", b"allowed:shard", b"secret:shard"],
        b"-NOPERM this user has no permissions to access one of the channels used as arguments\r\n",
    );
    send(
        &mut observer,
        &[b"PUBSUB", b"NUMPAT"],
        b"-NOPERM this user has no permissions to access one of the channels used as arguments\r\n",
    );

    // The default user remains an independent unrestricted control.
    send(&mut admin, &[b"PUBSUB", b"CHANNELS"], b"");
    let unrestricted = read_frame(&mut admin);
    assert!(
        unrestricted
            .windows(11)
            .any(|window| window == b"secret:live")
    );
    assert!(
        unrestricted
            .windows(8)
            .any(|window| window == b"secret:\xff")
    );
    send(&mut admin, &[b"PUBSUB", b"NUMSUB", b""], b"");
    let empty_channel = read_frame(&mut admin);
    assert!(
        empty_channel
            .windows(6)
            .any(|window| window == b"$0\r\n\r\n"),
        "{empty_channel:?}"
    );
    send(&mut admin, &[b"PUBSUB", b"NUMPAT"], b"");
    assert_eq!(read_frame(&mut admin), b":0\r\n");
    server.shutdown().unwrap();
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
