#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

fn start(requirepass: Option<&[u8]>) -> ServerHandle {
    let workers = match std::env::var("RILLCACHE_TEST_THREADS").as_deref() {
        Ok("0") | Err(_) => None,
        Ok("1") => Some(1),
        Ok("4") => Some(4),
        Ok(value) => panic!("invalid test worker mode {value}"),
    };
    Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        worker_threads: workers,
        requirepass: requirepass.map(<[u8]>::to_vec),
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
    let mut actual = vec![0; reply.len()];
    stream.read_exact(&mut actual).unwrap();
    assert_eq!(actual, reply, "command {arguments:?}");
}

#[test]
fn reset_clears_pubsub_transactions_protocol_and_authentication() {
    let server = start(Some(b"pw"));
    let mut stream = connect(&server);
    send(&mut stream, &[b"AUTH", b"pw"], b"+OK\r\n");
    // Drain HELLO 3.
    let mut request = Vec::new();
    encode_request(&[b"HELLO", b"3"], &mut request);
    stream.write_all(&request).unwrap();
    let mut prefix = [0u8; 4];
    stream.read_exact(&mut prefix).unwrap();
    assert_eq!(&prefix, b"%5\r\n");
    drain_values(&mut stream, 10);

    send(&mut stream, &[b"MULTI"], b"+OK\r\n");
    send(&mut stream, &[b"SET", b"k", b"v"], b"+QUEUED\r\n");
    send(&mut stream, &[b"RESET"], b"+RESET\r\n");
    // RESET returned to RESP2 and deauthenticated the connection, so even EXEC
    // is rejected before the now-empty transaction state is observable.
    send(
        &mut stream,
        &[b"EXEC"],
        b"-NOAUTH Authentication required.\r\n",
    );
    send(
        &mut stream,
        &[b"GET", b"k"],
        b"-NOAUTH Authentication required.\r\n",
    );
    send(&mut stream, &[b"AUTH", b"pw"], b"+OK\r\n");
    send(&mut stream, &[b"EXEC"], b"-ERR EXEC without MULTI\r\n");
    send(&mut stream, &[b"GET", b"k"], b"$-1\r\n");

    // A subscriber can reset back to normal command mode.
    send(
        &mut stream,
        &[b"SUBSCRIBE", b"ch"],
        b"*3\r\n$9\r\nsubscribe\r\n$2\r\nch\r\n:1\r\n",
    );
    send(
        &mut stream,
        &[b"GET", b"k"],
        b"-ERR Can't execute 'get': only (P|S)SUBSCRIBE / (P|S)UNSUBSCRIBE / PING / QUIT / RESET are allowed in this context\r\n",
    );
    send(&mut stream, &[b"RESET"], b"+RESET\r\n");
    send(
        &mut stream,
        &[b"GET", b"k"],
        b"-NOAUTH Authentication required.\r\n",
    );
    send(&mut stream, &[b"AUTH", b"pw"], b"+OK\r\n");
    send(&mut stream, &[b"GET", b"k"], b"$-1\r\n");
    send(&mut stream, &[b"PING"], b"+PONG\r\n");

    // RESET is available without authentication and still deauthenticates.
    let mut other = connect(&server);
    send(&mut other, &[b"RESET"], b"+RESET\r\n");
    send(
        &mut other,
        &[b"PING"],
        b"-NOAUTH Authentication required.\r\n",
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
