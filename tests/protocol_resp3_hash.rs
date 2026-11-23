#![forbid(unsafe_code)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use rillcache::protocol::encode_request;
use rillcache::{Config, Server};

fn send(client: &mut BufReader<TcpStream>, args: &[&[u8]]) {
    let mut request = Vec::new();
    encode_request(args, &mut request);
    client.get_mut().write_all(&request).unwrap();
}

fn expect(client: &mut BufReader<TcpStream>, bytes: &[u8]) {
    let mut actual = vec![0; bytes.len()];
    client.read_exact(&mut actual).unwrap();
    assert_eq!(actual, bytes);
}

fn hello3(client: &mut BufReader<TcpStream>) {
    send(client, &[b"HELLO", b"3"]);
    expect(client, b"%5\r\n");
    for _ in 0..10 {
        let mut line = String::new();
        client.read_line(&mut line).unwrap();
        if let Some(length) = line.strip_prefix('$') {
            let mut bytes = vec![0; length.trim_end().parse::<usize>().unwrap() + 2];
            client.read_exact(&mut bytes).unwrap();
            assert!(bytes.ends_with(b"\r\n"));
        } else {
            assert!(line.starts_with(':'));
        }
    }
}

#[test]
fn hrandfield_withvalues_nests_only_resp3_pairs_including_exec() {
    for worker_threads in [None, Some(1), Some(4)] {
        let server = Server::bind(Config {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            worker_threads,
            ..Config::default()
        })
        .unwrap()
        .spawn()
        .unwrap();
        let stream = TcpStream::connect(server.local_addr()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut client = BufReader::new(stream);
        send(&mut client, &[b"HSET", b"h", b"f\0", b"v\r\n$"]);
        expect(&mut client, b":1\r\n");
        send(&mut client, &[b"HRANDFIELD", b"h", b"1", b"WITHVALUES"]);
        expect(&mut client, b"*2\r\n$2\r\nf\0\r\n$4\r\nv\r\n$\r\n");

        hello3(&mut client);
        send(&mut client, &[b"hrandfield", b"h", b"-2", b"withvalues"]);
        expect(
            &mut client,
            b"*2\r\n*2\r\n$2\r\nf\0\r\n$4\r\nv\r\n$\r\n*2\r\n$2\r\nf\0\r\n$4\r\nv\r\n$\r\n",
        );
        send(&mut client, &[b"HRANDFIELD", b"h", b"1"]);
        expect(&mut client, b"*1\r\n$2\r\nf\0\r\n");
        send(
            &mut client,
            &[b"HRANDFIELD", b"missing", b"1", b"WITHVALUES"],
        );
        expect(&mut client, b"*0\r\n");

        send(&mut client, &[b"MULTI"]);
        send(&mut client, &[b"HRANDFIELD", b"h", b"1", b"WITHVALUES"]);
        send(&mut client, &[b"GET", b"missing"]);
        send(&mut client, &[b"HRANDFIELD", b"h", b"bad", b"WITHVALUES"]);
        send(&mut client, &[b"EXEC"]);
        send(&mut client, &[b"PING"]);
        expect(
            &mut client,
            b"+OK\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n*3\r\n*1\r\n*2\r\n$2\r\nf\0\r\n$4\r\nv\r\n$\r\n_\r\n-ERR value is not an integer or out of range\r\n+PONG\r\n",
        );
    }
}
