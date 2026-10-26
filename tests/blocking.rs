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

fn send(stream: &mut TcpStream, arguments: &[&[u8]], reply: &[u8]) {
    let mut bytes = Vec::new();
    encode_request(arguments, &mut bytes);
    stream.write_all(&bytes).unwrap();
    if !reply.is_empty() {
        let mut actual = vec![0; reply.len()];
        stream.read_exact(&mut actual).unwrap();
        assert_eq!(actual, reply, "command {arguments:?}");
    }
}

fn pipeline(stream: &mut TcpStream, commands: &[&[&[u8]]]) {
    let mut bytes = Vec::new();
    for command in commands {
        encode_request(command, &mut bytes);
    }
    stream.write_all(&bytes).unwrap();
}

fn expect(stream: &mut TcpStream, reply: &[u8]) {
    let mut actual = vec![0; reply.len()];
    stream.read_exact(&mut actual).unwrap();
    assert_eq!(actual, reply);
}

#[test]
fn blpop_brpop_wait_for_push_preserve_key_order_and_time_out() {
    let server = start();
    let mut waiter = connect(&server);
    let mut pusher = connect(&server);

    send(&mut waiter, &[b"BLPOP", b"queue", b"0"], b"");
    send(&mut pusher, &[b"RPUSH", b"queue", b"one"], b":1\r\n");
    expect(&mut waiter, b"*2\r\n$5\r\nqueue\r\n$3\r\none\r\n");

    send(&mut pusher, &[b"RPUSH", b"k2", b"two"], b":1\r\n");
    send(&mut waiter, &[b"BLPOP", b"k1", b"k2", b"0"], b"");
    expect(&mut waiter, b"*2\r\n$2\r\nk2\r\n$3\r\ntwo\r\n");

    send(&mut pusher, &[b"RPUSH", b"right", b"a", b"b"], b":2\r\n");
    send(&mut waiter, &[b"BRPOP", b"right", b"0"], b"");
    expect(&mut waiter, b"*2\r\n$5\r\nright\r\n$1\r\nb\r\n");
    send(&mut waiter, &[b"BRPOP", b"right", b"0"], b"");
    expect(&mut waiter, b"*2\r\n$5\r\nright\r\n$1\r\na\r\n");

    send(&mut waiter, &[b"BLPOP", b"missing", b"0.05"], b"");
    expect(&mut waiter, b"*-1\r\n");

    send(&mut pusher, &[b"SET", b"string", b"x"], b"+OK\r\n");
    send(
        &mut waiter,
        &[b"BLPOP", b"string", b"0"],
        b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
    );
    send(
        &mut waiter,
        &[b"BLPOP", b"k", b"not-a-timeout"],
        b"-ERR timeout is not a float or out of range\r\n",
    );
    server.shutdown().unwrap();
}

#[test]
fn commands_pipelined_while_blocked_run_after_the_pop_reply() {
    let server = start();
    let mut waiter = connect(&server);
    let mut pusher = connect(&server);
    pipeline(
        &mut waiter,
        &[
            &[b"BLPOP", b"q", b"0"],
            &[b"PING"],
            &[b"SET", b"after", b"ok"],
        ],
    );
    send(&mut pusher, &[b"RPUSH", b"q", b"v"], b":1\r\n");
    expect(&mut waiter, b"*2\r\n$1\r\nq\r\n$1\r\nv\r\n+PONG\r\n+OK\r\n");
    send(&mut pusher, &[b"GET", b"after"], b"$2\r\nok\r\n");
    server.shutdown().unwrap();
}

#[test]
fn blocked_connections_are_not_dropped_by_the_idle_timeout() {
    let server = Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        idle_timeout: Duration::from_millis(100),
        ..Config::default()
    })
    .unwrap()
    .spawn()
    .unwrap();
    let mut waiter = connect(&server);
    send(&mut waiter, &[b"BLPOP", b"queue", b"0"], b"");
    std::thread::sleep(Duration::from_millis(250));
    let mut pusher = connect(&server);
    send(&mut pusher, &[b"RPUSH", b"queue", b"late"], b":1\r\n");
    expect(&mut waiter, b"*2\r\n$5\r\nqueue\r\n$4\r\nlate\r\n");
    server.shutdown().unwrap();
}

#[test]
fn resp3_blocking_timeout_is_a_null_and_blocking_commands_are_rejected_in_multi() {
    let server = start();
    let mut waiter = connect(&server);
    let mut request = Vec::new();
    encode_request(&[b"HELLO", b"3"], &mut request);
    waiter.write_all(&request).unwrap();
    let mut prefix = [0u8; 4];
    waiter.read_exact(&mut prefix).unwrap();
    assert_eq!(&prefix, b"%5\r\n");
    drain_values(&mut waiter, 10);
    send(&mut waiter, &[b"BLPOP", b"missing", b"0.05"], b"");
    expect(&mut waiter, b"_\r\n");

    // RESP3 still supports normal commands and MULTI validation.
    send(&mut waiter, &[b"MULTI"], b"+OK\r\n");
    send(
        &mut waiter,
        &[b"BLPOP", b"q", b"0"],
        b"-ERR session commands are not supported inside MULTI\r\n",
    );
    send(
        &mut waiter,
        &[b"EXEC"],
        b"-EXECABORT Transaction discarded because of previous errors.\r\n",
    );
    server.shutdown().unwrap();
}

fn drain_values(stream: &mut TcpStream, count: usize) {
    for _ in 0..count {
        let mut header = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            stream.read_exact(&mut byte).unwrap();
            header.push(byte[0]);
            if header.ends_with(b"\r\n") {
                break;
            }
        }
        match header[0] {
            b'$' => {
                let size: usize = std::str::from_utf8(&header[1..header.len() - 2])
                    .unwrap()
                    .parse()
                    .unwrap();
                let mut payload = vec![0; size + 2];
                stream.read_exact(&mut payload).unwrap();
            }
            b':' => {}
            other => panic!("unexpected HELLO value {other:?}"),
        }
    }
}
