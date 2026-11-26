#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Barrier};
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

#[track_caller]
fn expect(stream: &mut TcpStream, expected: &[u8]) {
    let mut actual = vec![0; expected.len()];
    stream.read_exact(&mut actual).unwrap();
    assert_eq!(actual, expected);
}

fn read_line(stream: &mut TcpStream) -> Vec<u8> {
    let mut line = Vec::new();
    loop {
        let mut byte = [0];
        stream.read_exact(&mut byte).unwrap();
        line.push(byte[0]);
        if byte[0] == b'\n' {
            return line;
        }
    }
}

fn read_resp_frame(stream: &mut TcpStream) -> Vec<u8> {
    let mut prefix = [0];
    stream.read_exact(&mut prefix).unwrap();
    let mut frame = vec![prefix[0]];
    match prefix[0] {
        b'+' | b'-' | b':' => frame.extend(read_line(stream)),
        b'$' | b'*' => {
            let header = read_line(stream);
            let length = std::str::from_utf8(&header[..header.len() - 2])
                .unwrap()
                .parse::<i64>()
                .unwrap();
            frame.extend_from_slice(&header);
            if length >= 0 && prefix[0] == b'$' {
                let mut body = vec![0; length as usize + 2];
                stream.read_exact(&mut body).unwrap();
                frame.extend(body);
            } else if length > 0 {
                for _ in 0..length {
                    frame.extend(read_resp_frame(stream));
                }
            }
        }
        other => panic!("unsupported RESP frame prefix: {other:?}"),
    }
    frame
}

#[cfg(unix)]
#[test]
fn tokio_applies_live_connection_limit_to_admission_and_task_start() {
    let server = Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        io_mode: rillcache::server::IoMode::Tokio,
        max_clients: 1,
        ..Config::default()
    })
    .unwrap()
    .spawn()
    .unwrap();
    let mut admin = connect(&server);
    send(&mut admin, &[b"PING"]);
    expect(&mut admin, b"+PONG\r\n");
    expect(
        &mut connect(&server),
        b"-ERR max number of clients reached\r\n",
    );

    send(&mut admin, &[b"CONFIG", b"SET", b"maxclients", b"2"]);
    expect(&mut admin, b"+OK\r\n");
    let mut second = connect(&server);
    send(&mut second, &[b"PING"]);
    expect(&mut second, b"+PONG\r\n");

    send(&mut admin, &[b"CONFIG", b"SET", b"maxclients", b"1"]);
    expect(&mut admin, b"+OK\r\n");
    expect(
        &mut connect(&server),
        b"-ERR max number of clients reached\r\n",
    );
    for client in [&mut admin, &mut second] {
        send(client, &[b"PING"]);
        expect(client, b"+PONG\r\n");
    }
    send(&mut admin, &[b"CONFIG", b"SET", b"maxclients", b"3"]);
    expect(&mut admin, b"+OK\r\n");
    let mut third = connect(&server);
    send(&mut third, &[b"PING"]);
    expect(&mut third, b"+PONG\r\n");
    server.shutdown().unwrap();
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
        assert_eq!(error, b"-ERR Protocol error: invalid bulk length\r\n");
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

        // Binary members are preserved when authorized dynamic lookups execute.
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
fn per_user_connection_caps_are_atomic_and_release_slots_in_every_mode() {
    for (workers, async_io) in MODES {
        let path = std::env::temp_dir().join(format!(
            "rillcache-user-cap-{}-{workers:?}-{async_io}.acl",
            std::process::id()
        ));
        std::fs::write(&path, b"user default on nopass ~* &* +@all\n").unwrap();
        let server = Server::bind(Config {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            worker_threads: workers,
            io_mode: if async_io {
                rillcache::server::IoMode::Tokio
            } else {
                rillcache::server::IoMode::Mio
            },
            aclfile: Some(path.clone()),
            ..Config::default()
        })
        .unwrap()
        .spawn()
        .unwrap();
        let mut admin = connect(&server);
        send(
            &mut admin,
            &[
                b"ACL",
                b"SETUSER",
                b"alice",
                b"on",
                b">pw",
                b"+@all",
                b"maxconnections=1",
            ],
        );
        expect(&mut admin, b"+OK\r\n");
        send(
            &mut admin,
            &[
                b"ACL",
                b"SETUSER",
                b"bob",
                b"on",
                b">pw",
                b"+@all",
                b"maxconnections=1",
            ],
        );
        expect(&mut admin, b"+OK\r\n");
        send(&mut admin, &[b"ACL", b"LIST"]);
        let acl_list = String::from_utf8(read_resp_frame(&mut admin)).unwrap();
        assert!(acl_list.contains("user alice"), "{acl_list}");
        assert!(acl_list.contains("maxconnections=1"), "{acl_list}");
        send(&mut admin, &[b"ACL", b"GETUSER", b"alice"]);
        let details = String::from_utf8(read_resp_frame(&mut admin)).unwrap();
        assert!(details.contains("maxconnections"), "{details}");

        let mut left = connect(&server);
        let mut right = connect(&server);
        let barrier = Arc::new(Barrier::new(2));
        let left_barrier = Arc::clone(&barrier);
        let left_thread = std::thread::spawn(move || {
            left_barrier.wait();
            send(&mut left, &[b"AUTH", b"alice", b"pw"]);
            let reply = read_line(&mut left);
            (left, reply)
        });
        let right_barrier = Arc::clone(&barrier);
        let right_thread = std::thread::spawn(move || {
            right_barrier.wait();
            send(&mut right, &[b"AUTH", b"alice", b"pw"]);
            let reply = read_line(&mut right);
            (right, reply)
        });
        let (left, left_reply) = left_thread.join().unwrap();
        let (right, right_reply) = right_thread.join().unwrap();
        assert_ne!(
            left_reply, right_reply,
            "both simultaneous AUTHs: {left_reply:?}"
        );
        assert!(
            [left_reply.as_slice(), right_reply.as_slice()].contains(&b"+OK\r\n".as_slice()),
            "{left_reply:?} {right_reply:?}"
        );
        assert!(
            [left_reply.as_slice(), right_reply.as_slice()]
                .contains(&b"-ERR max connections reached for user\r\n".as_slice()),
            "{left_reply:?} {right_reply:?}"
        );
        let (mut winner, mut loser) = if left_reply == b"+OK\r\n" {
            (left, right)
        } else {
            (right, left)
        };

        send(&mut winner, &[b"AUTH", b"alice", b"pw"]);
        expect(&mut winner, b"+OK\r\n");
        send(
            &mut admin,
            &[b"ACL", b"SETUSER", b"alice", b"maxconnections=0"],
        );
        expect(&mut admin, b"+OK\r\n");
        send(&mut loser, &[b"AUTH", b"alice", b"pw"]);
        expect(&mut loser, b"+OK\r\n");
        send(
            &mut admin,
            &[b"ACL", b"SETUSER", b"alice", b"maxconnections=1"],
        );
        expect(&mut admin, b"+OK\r\n");
        send(&mut winner, &[b"PING"]);
        expect(&mut winner, b"+PONG\r\n");
        send(&mut loser, &[b"PING"]);
        expect(&mut loser, b"+PONG\r\n");

        let mut third = connect(&server);
        send(&mut third, &[b"AUTH", b"alice", b"pw"]);
        expect(&mut third, b"-ERR max connections reached for user\r\n");
        send(&mut third, &[b"AUTH", b"bob", b"pw"]);
        expect(&mut third, b"+OK\r\n");
        send(&mut winner, &[b"AUTH", b"bob", b"pw"]);
        expect(&mut winner, b"-ERR max connections reached for user\r\n");
        send(&mut winner, &[b"PING"]);
        expect(&mut winner, b"+PONG\r\n");

        send(&mut third, &[b"QUIT"]);
        expect(&mut third, b"+OK\r\n");
        drop(third);
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            send(&mut winner, &[b"AUTH", b"bob", b"pw"]);
            let reply = read_line(&mut winner);
            if reply == b"+OK\r\n" {
                break;
            }
            assert_eq!(reply, b"-ERR max connections reached for user\r\n");
            assert!(
                std::time::Instant::now() < deadline,
                "switch did not release old identity"
            );
            std::thread::yield_now();
        }

        let mut fourth = connect(&server);
        send(&mut fourth, &[b"AUTH", b"bob", b"pw"]);
        expect(&mut fourth, b"-ERR max connections reached for user\r\n");
        send(&mut fourth, &[b"HELLO", b"3", b"AUTH", b"bob", b"pw"]);
        expect(&mut fourth, b"-ERR max connections reached for user\r\n");
        send(&mut fourth, &[b"HELLO"]);
        let hello = read_resp_frame(&mut fourth);
        assert!(
            hello.starts_with(b"*10\r\n"),
            "HELLO unexpectedly changed protocol: {hello:?}"
        );

        send(&mut winner, &[b"RESET"]);
        expect(&mut winner, b"+RESET\r\n");
        send(&mut fourth, &[b"AUTH", b"bob", b"pw"]);
        expect(&mut fourth, b"+OK\r\n");

        send(&mut loser, &[b"QUIT"]);
        expect(&mut loser, b"+OK\r\n");
        drop(loser);
        let mut released = connect(&server);
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            send(&mut released, &[b"AUTH", b"alice", b"pw"]);
            let reply = read_line(&mut released);
            if reply == b"+OK\r\n" {
                break;
            }
            assert_eq!(reply, b"-ERR max connections reached for user\r\n");
            assert!(
                std::time::Instant::now() < deadline,
                "disconnect did not release quota"
            );
            std::thread::yield_now();
        }

        server.shutdown().unwrap();
        let _ = std::fs::remove_file(path);
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
