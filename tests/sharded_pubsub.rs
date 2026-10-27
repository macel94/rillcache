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
    stream
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

fn command(stream: &mut TcpStream, args: &[&[u8]], expected: &[u8]) {
    let mut request = Vec::new();
    encode_request(args, &mut request);
    stream.write_all(&request).unwrap();
    assert_eq!(read_frame(stream, 0), expected, "{args:?}");
}

#[test]
fn sharded_pubsub_works_over_tcp_in_all_execution_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut subscriber = connect(&server);
        let mut publisher = connect(&server);
        let mut normal = connect(&server);

        let mut request = Vec::new();
        encode_request(&[b"SSUBSCRIBE", b"shard:one", b"shard:two"], &mut request);
        subscriber.write_all(&request).unwrap();
        let mut confirmations = read_frame(&mut subscriber, 0);
        confirmations.extend_from_slice(&read_frame(&mut subscriber, 0));
        assert_eq!(
            confirmations,
            b"*3\r\n$10\r\nssubscribe\r\n$9\r\nshard:one\r\n:1\r\n*3\r\n$10\r\nssubscribe\r\n$9\r\nshard:two\r\n:2\r\n"
        );
        command(
            &mut publisher,
            &[b"PUBSUB", b"SHARDCHANNELS"],
            b"*2\r\n$9\r\nshard:one\r\n$9\r\nshard:two\r\n",
        );
        command(
            &mut publisher,
            &[
                b"PUBSUB",
                b"SHARDNUMSUB",
                b"shard:one",
                b"shard:two",
                b"other",
            ],
            b"*6\r\n$9\r\nshard:one\r\n:1\r\n$9\r\nshard:two\r\n:1\r\n$5\r\nother\r\n:0\r\n",
        );
        command(&mut publisher, &[b"PUBSUB", b"CHANNELS"], b"*0\r\n");
        command(
            &mut publisher,
            &[b"SPUBLISH", b"shard:one", b"hello"],
            b":1\r\n",
        );
        assert_eq!(
            read_frame(&mut subscriber, 0),
            b"*3\r\n$8\r\nsmessage\r\n$9\r\nshard:one\r\n$5\r\nhello\r\n"
        );
        command(
            &mut publisher,
            &[b"PUBLISH", b"shard:one", b"normal"],
            b":0\r\n",
        );

        command(
            &mut normal,
            &[b"SUBSCRIBE", b"shard:one"],
            b"*3\r\n$9\r\nsubscribe\r\n$9\r\nshard:one\r\n:1\r\n",
        );
        command(
            &mut publisher,
            &[b"SPUBLISH", b"shard:one", b"shard"],
            b":1\r\n",
        );
        command(
            &mut publisher,
            &[b"PUBLISH", b"shard:one", b"normal"],
            b":1\r\n",
        );
        assert_eq!(
            read_frame(&mut normal, 0),
            b"*3\r\n$7\r\nmessage\r\n$9\r\nshard:one\r\n$6\r\nnormal\r\n"
        );
        assert_eq!(
            read_frame(&mut subscriber, 0),
            b"*3\r\n$8\r\nsmessage\r\n$9\r\nshard:one\r\n$5\r\nshard\r\n"
        );

        command(
            &mut subscriber,
            &[b"GET", b"k"],
            b"-ERR Can't execute 'get': only (P|S)SUBSCRIBE / (P|S)UNSUBSCRIBE / PING / QUIT / RESET are allowed in this context\r\n",
        );
        command(
            &mut subscriber,
            &[b"SPUBLISH", b"shard:one", b"x"],
            b"-ERR Can't execute 'spublish': only (P|S)SUBSCRIBE / (P|S)UNSUBSCRIBE / PING / QUIT / RESET are allowed in this context\r\n",
        );
        command(
            &mut publisher,
            &[b"PUBSUB", b"SHARDCHANNELS", b"a", b"b"],
            b"-ERR unknown subcommand or wrong number of arguments for 'shardchannels'. Try PUBSUB HELP.\r\n",
        );
        command(
            &mut publisher,
            &[b"SPUBLISH", b"shard:one"],
            b"-ERR wrong number of arguments for 'spublish' command\r\n",
        );
        command(
            &mut publisher,
            &[b"SSUBSCRIBE"],
            b"-ERR wrong number of arguments for 'ssubscribe' command\r\n",
        );

        command(&mut subscriber, &[b"RESET"], b"+RESET\r\n");
        command(
            &mut publisher,
            &[b"PUBSUB", b"SHARDNUMSUB", b"shard:one", b"shard:two"],
            b"*4\r\n$9\r\nshard:one\r\n:0\r\n$9\r\nshard:two\r\n:0\r\n",
        );
        command(&mut subscriber, &[b"PING"], b"+PONG\r\n");

        server.shutdown().unwrap();
    }
}
