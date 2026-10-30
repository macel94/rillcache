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
