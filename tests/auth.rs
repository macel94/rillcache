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
fn password_authentication_gates_reads_writes_and_admin_commands() {
    let server = start(Some(b"correct horse"));
    let mut visitor = connect(&server);
    for arguments in [
        &[b"GET".as_slice(), b"key"][..],
        &[b"SET".as_slice(), b"key", b"value"][..],
        &[b"INFO".as_slice()][..],
        &[b"CONFIG".as_slice(), b"GET", b"*"][..],
    ] {
        send(
            &mut visitor,
            arguments,
            b"-NOAUTH Authentication required.\r\n",
        );
    }
    send(
        &mut visitor,
        &[b"AUTH", b"wrong"],
        b"-WRONGPASS invalid username-password pair or user is disabled.\r\n",
    );
    send(
        &mut visitor,
        &[b"AUTH", b"default", b"correct horse"],
        b"+OK\r\n",
    );
    send(&mut visitor, &[b"SET", b"key", b"value"], b"+OK\r\n");
    send(&mut visitor, &[b"GET", b"key"], b"$5\r\nvalue\r\n");

    let mut second = connect(&server);
    send(
        &mut second,
        &[b"PING"],
        b"-NOAUTH Authentication required.\r\n",
    );
    send(&mut second, &[b"AUTH", b"correct horse"], b"+OK\r\n");
    send(&mut second, &[b"GET", b"key"], b"$5\r\nvalue\r\n");
    server.shutdown().unwrap();
}

#[test]
fn unauthenticated_pipeline_preserves_order_and_quit_is_available() {
    let server = start(Some(b"pw"));
    let mut stream = connect(&server);
    let mut pipeline = Vec::new();
    encode_request(&[b"GET", b"k"], &mut pipeline);
    encode_request(&[b"AUTH", b"pw"], &mut pipeline);
    encode_request(&[b"SET", b"k", b"v"], &mut pipeline);
    encode_request(&[b"GET", b"k"], &mut pipeline);
    stream.write_all(&pipeline).unwrap();
    let expected = b"-NOAUTH Authentication required.\r\n+OK\r\n+OK\r\n$1\r\nv\r\n";
    let mut actual = vec![0; expected.len()];
    stream.read_exact(&mut actual).unwrap();
    assert_eq!(actual, expected);

    let mut visitor = connect(&server);
    send(&mut visitor, &[b"QUIT"], b"+OK\r\n");
    assert_eq!(visitor.read(&mut [0]).unwrap(), 0);
    server.shutdown().unwrap();
}

#[test]
fn runtime_requirepass_rotation_is_atomic_redacted_and_reversible() {
    let server = start(None);
    let mut admin = connect(&server);
    send(&mut admin, &[b"PING"], b"+PONG\r\n");
    send(
        &mut admin,
        &[b"CONFIG", b"SET", b"requirepass", b"first"],
        b"+OK\r\n",
    );
    send(
        &mut admin,
        &[b"PING"],
        b"-NOAUTH Authentication required.\r\n",
    );
    send(&mut admin, &[b"AUTH", b"first"], b"+OK\r\n");
    send(
        &mut admin,
        &[b"CONFIG", b"GET", b"requirepass"],
        b"*2\r\n$11\r\nrequirepass\r\n$10\r\n<redacted>\r\n",
    );
    send(
        &mut admin,
        &[b"CONFIG", b"SET", b"requirepass", b"second"],
        b"+OK\r\n",
    );

    let mut other = connect(&server);
    send(
        &mut other,
        &[b"AUTH", b"first"],
        b"-WRONGPASS invalid username-password pair or user is disabled.\r\n",
    );
    send(&mut other, &[b"AUTH", b"second"], b"+OK\r\n");

    send(
        &mut admin,
        &[b"CONFIG", b"SET", b"requirepass", b""],
        b"+OK\r\n",
    );
    let mut fresh = connect(&server);
    send(&mut fresh, &[b"PING"], b"+PONG\r\n");
    send(
        &mut fresh,
        &[b"CONFIG", b"GET", b"requirepass"],
        b"*2\r\n$11\r\nrequirepass\r\n$0\r\n\r\n",
    );
    server.shutdown().unwrap();
}

#[test]
fn hello_auth_negotiates_resp3_in_one_reply() {
    let server = start(Some(b"pw"));
    let mut stream = connect(&server);
    let mut request = Vec::new();
    encode_request(
        &[
            b"HELLO", b"3", b"AUTH", b"default", b"pw", b"SETNAME", b"client",
        ],
        &mut request,
    );
    stream.write_all(&request).unwrap();
    let mut prefix = [0u8; 4];
    stream.read_exact(&mut prefix).unwrap();
    assert_eq!(&prefix, b"%5\r\n");
    // Drain the five key/value pairs as RESP3 bulk/integer values.
    let mut fields = 0;
    while fields < 10 {
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
            other => panic!("unexpected HELLO field prefix {other:?}"),
        }
        fields += 1;
    }
    let mut request = Vec::new();
    encode_request(&[b"GET", b"k"], &mut request);
    stream.write_all(&request).unwrap();
    let mut reply = [0; 3];
    stream.read_exact(&mut reply).unwrap();
    assert_eq!(&reply, b"_\r\n");
    server.shutdown().unwrap();
}
