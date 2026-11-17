#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

/// Mode matrix: proven default, opt-in async engine, and fixed workers.
const MODES: [(Option<usize>, bool); 4] = [
    (None, false),
    (None, true),
    (Some(1), false),
    (Some(4), false),
];

fn start(workers: Option<usize>, async_io: bool) -> ServerHandle {
    Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        worker_threads: workers,
        io_mode: if async_io {
            rillcache::server::IoMode::Tokio
        } else {
            rillcache::server::IoMode::Mio
        },
        ..Config::default()
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

fn send(stream: &mut TcpStream, args: &[&[u8]]) {
    let mut request = Vec::new();
    encode_request(args, &mut request);
    stream.write_all(&request).unwrap();
}

fn expect(stream: &mut TcpStream, expected: &[u8]) {
    let mut actual = vec![0; expected.len()];
    stream.read_exact(&mut actual).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn fragmented_and_pipelined_wire_errors_are_isolated_in_every_mode() {
    for (workers, async_io) in MODES {
        let server = start(workers, async_io);
        let mut healthy = connect(&server);
        let mut fragmented = connect(&server);
        for part in b"*2\r\n$4\r\nECHO\r\n$3\r\na\0b\r\n".chunks(2) {
            fragmented.write_all(part).unwrap();
        }
        expect(&mut fragmented, b"$3\r\na\0b\r\n");

        let mut pipeline = Vec::new();
        let commands: &[&[&[u8]]] = &[
            &[b"SET", b"binary", b"v\0\xff"],
            &[b"GET", b"binary"],
            &[b"GET"],
            &[b"PING"],
        ];
        for args in commands {
            encode_request(args, &mut pipeline);
        }
        healthy.write_all(&pipeline).unwrap();
        expect(
            &mut healthy,
            b"+OK\r\n$3\r\nv\0\xff\r\n-ERR wrong number of arguments for 'get' command\r\n+PONG\r\n",
        );

        let mut malformed = connect(&server);
        malformed.write_all(b"*1\r\n$-1\r\n").unwrap();
        let mut error = Vec::new();
        malformed.read_to_end(&mut error).unwrap();
        assert_eq!(error, b"-ERR Protocol error: invalid length\r\n");
        send(&mut healthy, &[b"PING"]);
        expect(&mut healthy, b"+PONG\r\n");
        server.shutdown().unwrap();
    }
}

#[test]
fn blocked_and_pubsub_sessions_progress_and_join_on_shutdown_in_every_mode() {
    for (workers, async_io) in MODES {
        let server = start(workers, async_io);
        let mut subscriber = connect(&server);
        send(&mut subscriber, &[b"SUBSCRIBE", b"events"]);
        expect(
            &mut subscriber,
            b"*3\r\n$9\r\nsubscribe\r\n$6\r\nevents\r\n:1\r\n",
        );

        let mut blocked = connect(&server);
        send(&mut blocked, &[b"BLPOP", b"queue", b"0"]);
        let mut publisher = connect(&server);
        send(&mut publisher, &[b"PUBLISH", b"events", b"ready"]);
        expect(&mut publisher, b":1\r\n");
        expect(
            &mut subscriber,
            b"*3\r\n$7\r\nmessage\r\n$6\r\nevents\r\n$5\r\nready\r\n",
        );
        send(&mut publisher, &[b"LPUSH", b"queue", b"item"]);
        expect(&mut publisher, b":1\r\n");
        expect(&mut blocked, b"*2\r\n$5\r\nqueue\r\n$4\r\nitem\r\n");

        let start = std::time::Instant::now();
        server.shutdown().unwrap();
        assert!(start.elapsed() < Duration::from_secs(3));
        assert_eq!(subscriber.read(&mut [0]).unwrap(), 0);
        assert_eq!(blocked.read(&mut [0]).unwrap(), 0);
    }
}

#[test]
fn sort_dynamic_acl_is_fail_closed_and_store_is_atomic_in_every_mode() {
    for (workers, async_io) in MODES {
        let server = start(workers, async_io);
        let mut admin = connect(&server);
        let member = b"a\0\xff";
        let mut weight_key = b"weight:".to_vec();
        weight_key.extend_from_slice(member);
        let mut data_key = b"data:".to_vec();
        data_key.extend_from_slice(member);
        send(&mut admin, &[b"RPUSH", b"source:allowed", member]);
        expect(&mut admin, b":1\r\n");
        send(&mut admin, &[b"RPUSH", b"source:denied", member]);
        expect(&mut admin, b":1\r\n");
        send(&mut admin, &[b"SET", &weight_key, b"1"]);
        expect(&mut admin, b"+OK\r\n");
        send(&mut admin, &[b"SET", &data_key, b"secret"]);
        expect(&mut admin, b"+OK\r\n");
        send(&mut admin, &[b"SET", b"destination:allowed", b"before"]);
        expect(&mut admin, b"+OK\r\n");
        send(
            &mut admin,
            &[
                b"ACL",
                b"SETUSER",
                b"sorter",
                b"on",
                b"nopass",
                b"-@all",
                b"+sort",
                b"+sort_ro",
                b"%R~source:allowed",
                b"%W~destination:allowed",
            ],
        );
        expect(&mut admin, b"+OK\r\n");
        send(
            &mut admin,
            &[
                b"ACL",
                b"SETUSER",
                b"sorter-all",
                b"on",
                b"nopass",
                b"-@all",
                b"+sort_ro",
                b"allkeys",
            ],
        );
        expect(&mut admin, b"+OK\r\n");

        let mut restricted = connect(&server);
        send(&mut restricted, &[b"AUTH", b"sorter", b""]);
        expect(&mut restricted, b"+OK\r\n");
        let denied =
            b"-NOPERM this user has no permissions to access one of the keys used as arguments\r\n";
        send(
            &mut restricted,
            &[b"SORT", b"source:allowed", b"BY", b"weight:*"],
        );
        expect(&mut restricted, denied);
        send(
            &mut restricted,
            &[
                b"SORT",
                b"source:allowed",
                b"BY",
                b"weight:*",
                b"STORE",
                b"destination:allowed",
            ],
        );
        expect(&mut restricted, denied);
        send(
            &mut restricted,
            &[b"SORT_RO", b"source:allowed", b"GET", b"data:*"],
        );
        expect(&mut restricted, denied);
        send(
            &mut restricted,
            &[b"SORT", b"source:allowed", b"STORE", b"destination:blocked"],
        );
        expect(&mut restricted, denied);
        send(&mut restricted, &[b"SORT_RO", b"source:denied"]);
        expect(&mut restricted, denied);
        send(
            &mut admin,
            &[
                b"ACL",
                b"DRYRUN",
                b"sorter",
                b"SORT_RO",
                b"source:allowed",
                b"GET",
                b"data:*",
            ],
        );
        expect(&mut admin, denied);

        // A dynamic-reference denial occurs before STORE can alter its target.
        send(&mut admin, &[b"GET", b"destination:allowed"]);
        expect(&mut admin, b"$6\r\nbefore\r\n");
        send(&mut admin, &[b"GET", b"destination:blocked"]);
        expect(&mut admin, b"$-1\r\n");

        // Hash/data lookups with binary members work for an explicit all-key user.
        let mut all_keys = connect(&server);
        send(&mut all_keys, &[b"AUTH", b"sorter-all", b""]);
        expect(&mut all_keys, b"+OK\r\n");
        send(
            &mut all_keys,
            &[
                b"SORT_RO",
                b"source:allowed",
                b"BY",
                b"weight:*",
                b"GET",
                b"data:*",
            ],
        );
        expect(&mut all_keys, b"*1\r\n$6\r\nsecret\r\n");
        server.shutdown().unwrap();
    }
}

#[test]
fn backpressured_reply_does_not_stall_healthy_clients_or_shutdown_in_every_mode() {
    for (workers, async_io) in MODES {
        let server = start(workers, async_io);
        let mut healthy = connect(&server);
        let value = vec![b'x'; 1024 * 1024];
        send(&mut healthy, &[b"SET", b"large", &value]);
        expect(&mut healthy, b"+OK\r\n");

        let mut slow = connect(&server);
        let mut request = Vec::new();
        encode_request(&[b"GET", b"large"], &mut request);
        slow.write_all(&request.repeat(1024)).unwrap();
        for _ in 0..16 {
            send(&mut healthy, &[b"PING"]);
            expect(&mut healthy, b"+PONG\r\n");
        }
        let start = std::time::Instant::now();
        server.shutdown().unwrap();
        assert!(start.elapsed() < Duration::from_secs(3));
        let mut pending = Vec::new();
        let result = slow.read_to_end(&mut pending);
        assert!(
            result.is_ok()
                || result
                    .as_ref()
                    .is_err_and(|error| error.kind() == std::io::ErrorKind::ConnectionReset),
            "shutdown did not close the slow client: {result:?}"
        );
        assert!(
            pending.len() < 1024 * 1024 * 1024,
            "server drained the full slow-client pipeline"
        );
    }
}
