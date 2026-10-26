#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

fn start(workers: Option<usize>) -> ServerHandle {
    let config = Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        worker_threads: workers,
        ..Config::default()
    };
    Server::bind(config).unwrap().spawn().unwrap()
}

fn command(stream: &mut TcpStream, args: &[&[u8]], expected: &[u8]) {
    let mut request = Vec::new();
    encode_request(args, &mut request);
    stream.write_all(&request).unwrap();
    let mut reply = vec![0; expected.len()];
    stream.read_exact(&mut reply).unwrap();
    assert_eq!(reply, expected);
}

#[test]
fn keyspace_commands_work_over_tcp_in_all_execution_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = TcpStream::connect(server.local_addr()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();

        command(&mut stream, &[b"TYPE", b"absent"], b"+none\r\n");
        command(&mut stream, &[b"SET", b"a\0\xff", b"v\0"], b"+OK\r\n");
        command(&mut stream, &[b"TYPE", b"a\0\xff"], b"+string\r\n");
        command(
            &mut stream,
            &[b"TOUCH", b"a\0\xff", b"a\0\xff", b"missing"],
            b":2\r\n",
        );
        command(&mut stream, &[b"RENAME", b"a\0\xff", b"b\xff"], b"+OK\r\n");
        command(&mut stream, &[b"GET", b"b\xff"], b"$2\r\nv\0\r\n");
        command(&mut stream, &[b"RENAMENX", b"b\xff", b"c"], b":1\r\n");
        command(&mut stream, &[b"SET", b"b\xff", b"occupied"], b"+OK\r\n");
        command(&mut stream, &[b"RENAMENX", b"c", b"b\xff"], b":0\r\n");
        command(&mut stream, &[b"DBSIZE"], b":2\r\n");

        server.shutdown().unwrap();
    }
}
