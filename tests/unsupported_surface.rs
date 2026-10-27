#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

fn start(workers: Option<usize>) -> ServerHandle {
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
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream.set_nodelay(true).unwrap();
    stream
}

fn read_line(stream: &mut TcpStream, output: &mut Vec<u8>) -> String {
    let mut line = Vec::new();
    loop {
        let mut byte = [0];
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

fn read_frame(stream: &mut TcpStream, depth: usize) -> Vec<u8> {
    assert!(depth <= 8);
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
                assert!(body.ends_with(b"\r\n"));
                output.extend_from_slice(&body);
            }
        }
        b'*' => {
            let count = read_line(stream, &mut output).parse::<i64>().unwrap();
            if count >= 0 {
                for _ in 0..count {
                    output.extend_from_slice(&read_frame(stream, depth + 1));
                }
            }
        }
        other => panic!("unexpected RESP prefix {other:?}"),
    }
    output
}

fn command(stream: &mut TcpStream, args: &[&[u8]], expected: &[u8]) {
    let mut request = Vec::new();
    encode_request(args, &mut request);
    stream.write_all(&request).unwrap();
    assert_eq!(read_frame(stream, 0), expected, "{args:?}");
}

fn reply(stream: &mut TcpStream, args: &[&[u8]]) -> Vec<u8> {
    let mut request = Vec::new();
    encode_request(args, &mut request);
    stream.write_all(&request).unwrap();
    read_frame(stream, 0)
}

#[test]
fn cluster_module_and_wait_surface_is_explicit_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = connect(&server);
        let disabled = b"-ERR This instance has cluster support disabled\r\n";

        for args in [
            vec![b"CLUSTER".as_slice(), b"INFO"],
            vec![b"CLUSTER", b"MYID"],
            vec![b"CLUSTER", b"SLOTS"],
            vec![b"CLUSTER", b"KEYSLOT", b"foo"],
            vec![b"READONLY"],
            vec![b"READWRITE"],
            vec![b"ASKING"],
        ] {
            command(&mut stream, &args, disabled);
        }
        assert!(reply(&mut stream, &[b"CLUSTER", b"HELP"]).starts_with(b"*1\r\n"));
        command(
            &mut stream,
            &[b"CLUSTER"],
            b"-ERR wrong number of arguments for 'cluster' command\r\n",
        );

        command(&mut stream, &[b"MODULE", b"LIST"], b"*0\r\n");
        assert!(reply(&mut stream, &[b"MODULE", b"HELP"]).starts_with(b"*1\r\n"));
        command(
            &mut stream,
            &[b"MODULE", b"LOAD", b"missing.so"],
            b"-ERR MODULE loading is not supported by this server\r\n",
        );
        command(
            &mut stream,
            &[b"MODULE", b"UNLOAD", b"name"],
            b"-ERR MODULE loading is not supported by this server\r\n",
        );
        command(
            &mut stream,
            &[b"MODULE", b"BOGUS"],
            b"-ERR unknown MODULE subcommand\r\n",
        );
        command(
            &mut stream,
            &[b"MODULE"],
            b"-ERR wrong number of arguments for 'module' command\r\n",
        );

        command(&mut stream, &[b"WAIT", b"0", b"0"], b":0\r\n");
        command(&mut stream, &[b"WAIT", b"1", b"10"], b":0\r\n");
        command(&mut stream, &[b"WAIT", b"-1", b"0"], b":0\r\n");
        command(
            &mut stream,
            &[b"WAIT", b"0", b"-1"],
            b"-ERR timeout is negative\r\n",
        );
        command(
            &mut stream,
            &[b"WAIT", b"0", b"x"],
            b"-ERR timeout is not an integer or out of range\r\n",
        );
        command(
            &mut stream,
            &[b"WAIT", b"x", b"0"],
            b"-ERR value is not an integer or out of range\r\n",
        );
        command(
            &mut stream,
            &[b"WAIT", b"0"],
            b"-ERR wrong number of arguments for 'wait' command\r\n",
        );

        command(&mut stream, &[b"MSET", b"k", b"v"], b"+OK\r\n");
        command(&mut stream, &[b"GET", b"k"], b"$1\r\nv\r\n");
        server.shutdown().unwrap();
    }
}
