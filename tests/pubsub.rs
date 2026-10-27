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
    let mut actual = vec![0; reply.len()];
    stream.read_exact(&mut actual).unwrap();
    assert_eq!(actual, reply, "command {arguments:?}");
}

fn bulk(value: &[u8], output: &mut Vec<u8>) {
    output.extend_from_slice(format!("${}\r\n", value.len()).as_bytes());
    output.extend_from_slice(value);
    output.extend_from_slice(b"\r\n");
}

fn message(channel: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut output = b"*3\r\n$7\r\nmessage\r\n".to_vec();
    bulk(channel, &mut output);
    bulk(payload, &mut output);
    output
}

fn smessage(channel: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut output = b"*3\r\n$8\r\nsmessage\r\n".to_vec();
    bulk(channel, &mut output);
    bulk(payload, &mut output);
    output
}

fn pmessage(pattern: &[u8], channel: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut output = b"*4\r\n$8\r\npmessage\r\n".to_vec();
    bulk(pattern, &mut output);
    bulk(channel, &mut output);
    bulk(payload, &mut output);
    output
}

#[test]
fn publish_and_spublish_execute_from_multi_and_deliver_at_exec() {
    let server = start();
    let mut normal = connect(&server);
    let mut shard = connect(&server);
    let mut publisher = connect(&server);
    send(
        &mut normal,
        &[b"SUBSCRIBE", b"normal"],
        b"*3\r\n$9\r\nsubscribe\r\n$6\r\nnormal\r\n:1\r\n",
    );
    send(
        &mut shard,
        &[b"SSUBSCRIBE", b"shard"],
        b"*3\r\n$10\r\nssubscribe\r\n$5\r\nshard\r\n:1\r\n",
    );
    send(&mut publisher, &[b"MULTI"], b"+OK\r\n");
    send(
        &mut publisher,
        &[b"PUBLISH", b"normal", b"hello"],
        b"+QUEUED\r\n",
    );
    send(
        &mut publisher,
        &[b"SPUBLISH", b"shard", b"hello"],
        b"+QUEUED\r\n",
    );
    send(&mut publisher, &[b"EXEC"], b"*2\r\n:1\r\n:1\r\n");

    let expected = message(b"normal", b"hello");
    let mut actual = vec![0; expected.len()];
    normal.read_exact(&mut actual).unwrap();
    assert_eq!(actual, expected);
    let expected = smessage(b"shard", b"hello");
    let mut actual = vec![0; expected.len()];
    shard.read_exact(&mut actual).unwrap();
    assert_eq!(actual, expected);
    server.shutdown().unwrap();
}

#[test]
fn subscribe_publish_patterns_and_introspection_work_over_the_wire() {
    let server = start();
    let mut subscriber = connect(&server);
    let mut publisher = connect(&server);
    send(
        &mut subscriber,
        &[b"SUBSCRIBE", b"news", b"\0bin"],
        b"*3\r\n$9\r\nsubscribe\r\n$4\r\nnews\r\n:1\r\n*3\r\n$9\r\nsubscribe\r\n$4\r\n\0bin\r\n:2\r\n",
    );
    send(
        &mut publisher,
        &[b"PUBLISH", b"news", b"hello\0world"],
        b":1\r\n",
    );
    let mut expected = Vec::new();
    expected.extend_from_slice(&message(b"news", b"hello\0world"));
    let mut actual = vec![0; expected.len()];
    subscriber.read_exact(&mut actual).unwrap();
    assert_eq!(actual, expected);

    send(
        &mut subscriber,
        &[b"PSUBSCRIBE", b"news.*"],
        b"*3\r\n$10\r\npsubscribe\r\n$6\r\nnews.*\r\n:3\r\n",
    );
    send(
        &mut publisher,
        &[b"PUBLISH", b"news.tech", b"binary\xff"],
        b":1\r\n",
    );
    let expected = pmessage(b"news.*", b"news.tech", b"binary\xff");
    let mut actual = vec![0; expected.len()];
    subscriber.read_exact(&mut actual).unwrap();
    assert_eq!(actual, expected);

    send(
        &mut publisher,
        &[b"PUBSUB", b"CHANNELS"],
        b"*2\r\n$4\r\n\0bin\r\n$4\r\nnews\r\n",
    );
    send(
        &mut publisher,
        &[b"PUBSUB", b"NUMSUB", b"news", b"missing"],
        b"*4\r\n$4\r\nnews\r\n:1\r\n$7\r\nmissing\r\n:0\r\n",
    );
    send(&mut publisher, &[b"PUBSUB", b"NUMPAT"], b":1\r\n");

    send(
        &mut subscriber,
        &[b"UNSUBSCRIBE"],
        b"*3\r\n$11\r\nunsubscribe\r\n$4\r\n\0bin\r\n:2\r\n*3\r\n$11\r\nunsubscribe\r\n$4\r\nnews\r\n:1\r\n",
    );
    send(
        &mut subscriber,
        &[b"PUNSUBSCRIBE"],
        b"*3\r\n$12\r\npunsubscribe\r\n$6\r\nnews.*\r\n:0\r\n",
    );
    send(&mut publisher, &[b"PUBLISH", b"news", b"gone"], b":0\r\n");
    server.shutdown().unwrap();
}

#[test]
fn resp3_push_frames_and_subscriber_mode_replies_are_explicit() {
    let server = start();
    let mut subscriber = connect(&server);
    let mut publisher = connect(&server);
    // Drain the HELLO 3 map.
    send(&mut subscriber, &[b"HELLO", b"3"], b"");
    let mut prefix = [0u8; 4];
    subscriber.read_exact(&mut prefix).unwrap();
    assert_eq!(&prefix, b"%5\r\n");
    drain_values(&mut subscriber, 10);
    send(
        &mut subscriber,
        &[b"SUBSCRIBE", b"push"],
        b">3\r\n$9\r\nsubscribe\r\n$4\r\npush\r\n:1\r\n",
    );
    send(&mut publisher, &[b"PUBLISH", b"push", b"v"], b":1\r\n");
    let mut expected = b">3\r\n$7\r\nmessage\r\n".to_vec();
    bulk(b"push", &mut expected);
    bulk(b"v", &mut expected);
    let mut actual = vec![0; expected.len()];
    subscriber.read_exact(&mut actual).unwrap();
    assert_eq!(actual, expected);
    server.shutdown().unwrap();
}

#[test]
fn resp2_subscriber_mode_rejects_data_commands_but_allows_ping_and_quit() {
    let server = start();
    let mut subscriber = connect(&server);
    send(
        &mut subscriber,
        &[b"SUBSCRIBE", b"ch"],
        b"*3\r\n$9\r\nsubscribe\r\n$2\r\nch\r\n:1\r\n",
    );
    send(
        &mut subscriber,
        &[b"GET", b"k"],
        b"-ERR Can't execute 'get': only (P|S)SUBSCRIBE / (P|S)UNSUBSCRIBE / PING / QUIT / RESET are allowed in this context\r\n",
    );
    send(
        &mut subscriber,
        &[b"PING"],
        b"*2\r\n$4\r\npong\r\n$0\r\n\r\n",
    );
    send(&mut subscriber, &[b"QUIT"], b"+OK\r\n");
    assert_eq!(subscriber.read(&mut [0]).unwrap(), 0);
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
