#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use rillcache::engine::{EvictionPolicy, NotifyFlags};
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

fn start_limited() -> ServerHandle {
    Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        max_data_bytes: 10,
        maxmemory_policy: EvictionPolicy::NoEviction,
        ..Config::default()
    })
    .unwrap()
    .spawn()
    .unwrap()
}

fn start_notify() -> ServerHandle {
    Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        notify_keyspace_events: NotifyFlags::parse(b"KEA").unwrap(),
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

fn send(stream: &mut TcpStream, args: &[&[u8]]) {
    let mut request = Vec::new();
    encode_request(args, &mut request);
    stream.write_all(&request).unwrap();
}

fn command(stream: &mut TcpStream, args: &[&[u8]], expected: &[u8]) {
    send(stream, args);
    assert_eq!(read_frame(stream, 0), expected, "{args:?}");
}

fn reply(stream: &mut TcpStream, args: &[&[u8]]) -> Vec<u8> {
    send(stream, args);
    read_frame(stream, 0)
}

fn frame(parts: &[&[u8]]) -> Vec<u8> {
    let mut output = format!("*{}\r\n", parts.len()).into_bytes();
    for part in parts {
        output.extend_from_slice(format!("${}\r\n", part.len()).as_bytes());
        output.extend_from_slice(part);
        output.extend_from_slice(b"\r\n");
    }
    output
}

fn subscribe(stream: &mut TcpStream, kind: &[u8], pattern: &[u8]) {
    send(stream, &[kind, pattern]);
    let confirmation = read_frame(stream, 0);
    assert!(confirmation.starts_with(b"*3\r\n"), "{confirmation:?}");
}

fn expect_event(subscriber: &mut TcpStream, key: &[u8], event: &[u8]) {
    let mut channel = b"__keyspace@0__:".to_vec();
    channel.extend_from_slice(key);
    assert_eq!(
        read_frame(subscriber, 0),
        frame(&[b"pmessage", b"__keyspace@*", &channel, event])
    );
}

#[test]
fn list_move_family_moves_rotates_and_reports_errors_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = connect(&server);

        command(
            &mut stream,
            &[b"RPUSH", b"src", b"a", b"b", b"c"],
            b":3\r\n",
        );
        command(
            &mut stream,
            &[b"LMOVE", b"src", b"dst", b"LEFT", b"RIGHT"],
            b"$1\r\na\r\n",
        );
        command(
            &mut stream,
            &[b"LRANGE", b"src", b"0", b"-1"],
            b"*2\r\n$1\r\nb\r\n$1\r\nc\r\n",
        );
        command(
            &mut stream,
            &[b"LRANGE", b"dst", b"0", b"-1"],
            b"*1\r\n$1\r\na\r\n",
        );
        command(
            &mut stream,
            &[b"LMOVE", b"src", b"dst", b"RIGHT", b"LEFT"],
            b"$1\r\nc\r\n",
        );
        command(
            &mut stream,
            &[b"LRANGE", b"dst", b"0", b"-1"],
            b"*2\r\n$1\r\nc\r\n$1\r\na\r\n",
        );
        command(&mut stream, &[b"RPOPLPUSH", b"src", b"dst"], b"$1\r\nb\r\n");
        command(&mut stream, &[b"EXISTS", b"src"], b":0\r\n");
        command(
            &mut stream,
            &[b"LRANGE", b"dst", b"0", b"-1"],
            b"*3\r\n$1\r\nb\r\n$1\r\nc\r\n$1\r\na\r\n",
        );

        // Same-key moves rotate the list without losing or duplicating values.
        command(
            &mut stream,
            &[b"RPUSH", b"rot", b"1", b"2", b"3"],
            b":3\r\n",
        );
        command(&mut stream, &[b"RPOPLPUSH", b"rot", b"rot"], b"$1\r\n3\r\n");
        command(
            &mut stream,
            &[b"LRANGE", b"rot", b"0", b"-1"],
            b"*3\r\n$1\r\n3\r\n$1\r\n1\r\n$1\r\n2\r\n",
        );
        command(
            &mut stream,
            &[b"LMOVE", b"rot", b"rot", b"RIGHT", b"RIGHT"],
            b"$1\r\n2\r\n",
        );
        command(
            &mut stream,
            &[b"LRANGE", b"rot", b"0", b"-1"],
            b"*3\r\n$1\r\n3\r\n$1\r\n1\r\n$1\r\n2\r\n",
        );

        command(&mut stream, &[b"RPUSH", b"empty-src", b"x"], b":1\r\n");
        command(
            &mut stream,
            &[b"LMOVE", b"missing", b"nowhere", b"LEFT", b"LEFT"],
            b"$-1\r\n",
        );
        command(
            &mut stream,
            &[b"RPOPLPUSH", b"missing", b"nowhere"],
            b"$-1\r\n",
        );
        command(&mut stream, &[b"EXISTS", b"nowhere"], b":0\r\n");

        // Failed moves leave both sides untouched.
        command(&mut stream, &[b"SET", b"wrong", b"v"], b"+OK\r\n");
        command(&mut stream, &[b"RPUSH", b"typed", b"a", b"b"], b":2\r\n");
        command(
            &mut stream,
            &[b"LMOVE", b"typed", b"wrong", b"LEFT", b"LEFT"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );
        command(
            &mut stream,
            &[b"LRANGE", b"typed", b"0", b"-1"],
            b"*2\r\n$1\r\na\r\n$1\r\nb\r\n",
        );
        command(&mut stream, &[b"GET", b"wrong"], b"$1\r\nv\r\n");
        command(
            &mut stream,
            &[b"RPOPLPUSH", b"wrong", b"typed"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );

        // Destination TTL is preserved; a newly created destination is persistent.
        command(&mut stream, &[b"RPUSH", b"tt-src", b"a"], b":1\r\n");
        command(&mut stream, &[b"PEXPIRE", b"tt-src", b"100000"], b":1\r\n");
        command(&mut stream, &[b"RPUSH", b"tt-dst", b"b"], b":1\r\n");
        command(&mut stream, &[b"PEXPIRE", b"tt-dst", b"200000"], b":1\r\n");
        command(
            &mut stream,
            &[b"LMOVE", b"tt-src", b"tt-dst", b"LEFT", b"LEFT"],
            b"$1\r\na\r\n",
        );
        command(&mut stream, &[b"EXISTS", b"tt-src"], b":0\r\n");
        let ttl = reply(&mut stream, &[b"PTTL", b"tt-dst"]);
        let ttl = std::str::from_utf8(&ttl[1..ttl.len() - 2])
            .unwrap()
            .parse::<i64>()
            .unwrap();
        assert!(ttl > 0, "destination TTL was not preserved: {ttl}");
        command(&mut stream, &[b"RPUSH", b"tt-new-src", b"a"], b":1\r\n");
        command(
            &mut stream,
            &[b"PEXPIRE", b"tt-new-src", b"100000"],
            b":1\r\n",
        );
        command(
            &mut stream,
            &[b"LMOVE", b"tt-new-src", b"tt-new-dst", b"LEFT", b"RIGHT"],
            b"$1\r\na\r\n",
        );
        command(&mut stream, &[b"PTTL", b"tt-new-dst"], b":-1\r\n");
        // A source that still has elements keeps its TTL.
        command(&mut stream, &[b"RPUSH", b"tt-keep", b"a", b"b"], b":2\r\n");
        command(&mut stream, &[b"PEXPIRE", b"tt-keep", b"100000"], b":1\r\n");
        command(
            &mut stream,
            &[b"LMOVE", b"tt-keep", b"tt-take", b"LEFT", b"LEFT"],
            b"$1\r\na\r\n",
        );
        let ttl = reply(&mut stream, &[b"PTTL", b"tt-keep"]);
        let ttl = std::str::from_utf8(&ttl[1..ttl.len() - 2])
            .unwrap()
            .parse::<i64>()
            .unwrap();
        assert!(ttl > 0, "source TTL was not preserved: {ttl}");

        command(
            &mut stream,
            &[b"LMOVE", b"src", b"dst"],
            b"-ERR wrong number of arguments for 'lmove' command\r\n",
        );
        command(
            &mut stream,
            &[b"LMOVE", b"src", b"dst", b"BAD", b"LEFT"],
            b"-ERR syntax error\r\n",
        );
        command(
            &mut stream,
            &[b"RPOPLPUSH", b"src"],
            b"-ERR wrong number of arguments for 'rpoplpush' command\r\n",
        );
        command(
            &mut stream,
            &[b"BLMOVE", b"src", b"dst", b"LEFT"],
            b"-ERR wrong number of arguments for 'blmove' command\r\n",
        );
        command(
            &mut stream,
            &[b"BRPOPLPUSH", b"src"],
            b"-ERR wrong number of arguments for 'brpoplpush' command\r\n",
        );

        server.shutdown().unwrap();
    }
}

#[test]
fn blmove_and_brpoplpush_block_wake_time_out_and_run_nonblocking_in_multi() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = connect(&server);

        // Immediate source wins without blocking.
        command(&mut stream, &[b"RPUSH", b"ready", b"a"], b":1\r\n");
        command(
            &mut stream,
            &[b"BLMOVE", b"ready", b"ready-dst", b"LEFT", b"RIGHT", b"0"],
            b"$1\r\na\r\n",
        );
        command(
            &mut stream,
            &[b"LRANGE", b"ready-dst", b"0", b"-1"],
            b"*1\r\n$1\r\na\r\n",
        );

        // A wake triggered by another connection finishes the move.
        send(
            &mut stream,
            &[b"BLMOVE", b"missing", b"woken", b"LEFT", b"LEFT", b"0"],
        );
        std::thread::sleep(Duration::from_millis(80));
        let mut writer = connect(&server);
        command(&mut writer, &[b"RPUSH", b"missing", b"v"], b":1\r\n");
        assert_eq!(read_frame(&mut stream, 0), b"$1\r\nv\r\n");
        command(
            &mut stream,
            &[b"LRANGE", b"woken", b"0", b"-1"],
            b"*1\r\n$1\r\nv\r\n",
        );

        send(
            &mut stream,
            &[b"BRPOPLPUSH", b"br-missing", b"br-dst", b"0"],
        );
        std::thread::sleep(Duration::from_millis(80));
        command(&mut writer, &[b"RPUSH", b"br-missing", b"w"], b":1\r\n");
        assert_eq!(read_frame(&mut stream, 0), b"$1\r\nw\r\n");
        command(
            &mut stream,
            &[b"LRANGE", b"br-dst", b"0", b"-1"],
            b"*1\r\n$1\r\nw\r\n",
        );

        command(
            &mut stream,
            &[b"BLMOVE", b"nobody", b"nodst", b"LEFT", b"RIGHT", b"0.05"],
            b"*-1\r\n",
        );
        command(
            &mut stream,
            &[b"BRPOPLPUSH", b"nobody", b"nodst", b"0.05"],
            b"*-1\r\n",
        );
        command(
            &mut stream,
            &[b"BLMOVE", b"any", b"where", b"BAD", b"LEFT", b"0"],
            b"-ERR syntax error\r\n",
        );
        command(
            &mut stream,
            &[b"BLMOVE", b"any", b"where", b"LEFT", b"RIGHT", b"-1"],
            b"-ERR timeout is negative\r\n",
        );
        command(
            &mut stream,
            &[b"BRPOPLPUSH", b"any", b"where", b"bad"],
            b"-ERR timeout is not a float or out of range\r\n",
        );
        command(&mut stream, &[b"SET", b"wrong", b"v"], b"+OK\r\n");
        command(
            &mut stream,
            &[b"BLMOVE", b"wrong", b"where", b"LEFT", b"LEFT", b"0"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );

        // Inside MULTI both commands execute once, without blocking.
        command(&mut stream, &[b"RPUSH", b"multi", b"a"], b":1\r\n");
        command(&mut stream, &[b"MULTI"], b"+OK\r\n");
        command(
            &mut stream,
            &[b"BLMOVE", b"multi", b"multi-dst", b"LEFT", b"RIGHT", b"0"],
            b"+QUEUED\r\n",
        );
        command(
            &mut stream,
            &[b"BRPOPLPUSH", b"multi-dst", b"multi", b"0"],
            b"+QUEUED\r\n",
        );
        command(&mut stream, &[b"EXEC"], b"*2\r\n$1\r\na\r\n$1\r\na\r\n");
        command(
            &mut stream,
            &[b"LRANGE", b"multi", b"0", b"-1"],
            b"*1\r\n$1\r\na\r\n",
        );

        command(&mut stream, &[b"MULTI"], b"+OK\r\n");
        command(
            &mut stream,
            &[b"BRPOPLPUSH", b"empty", b"empty-dst", b"0"],
            b"+QUEUED\r\n",
        );
        command(&mut stream, &[b"EXEC"], b"*1\r\n$-1\r\n");

        // A blocking move from another client invalidates WATCH.
        command(&mut stream, &[b"RPUSH", b"watched", b"a"], b":1\r\n");
        command(&mut stream, &[b"WATCH", b"watched"], b"+OK\r\n");
        let mut other = connect(&server);
        command(
            &mut other,
            &[
                b"BLMOVE",
                b"watched",
                b"watched-dst",
                b"LEFT",
                b"LEFT",
                b"0",
            ],
            b"$1\r\na\r\n",
        );
        command(&mut stream, &[b"MULTI"], b"+OK\r\n");
        command(&mut stream, &[b"LLEN", b"watched"], b"+QUEUED\r\n");
        command(&mut stream, &[b"EXEC"], b"*-1\r\n");

        server.shutdown().unwrap();
    }
}

#[test]
fn list_move_oom_leaves_both_keys_unchanged() {
    let server = start_limited();
    let mut stream = connect(&server);
    command(&mut stream, &[b"RPUSH", b"s", b"v"], b":1\r\n");
    command(
        &mut stream,
        &[b"LMOVE", b"s", b"destination", b"LEFT", b"LEFT"],
        b"-OOM configured payload limit exceeded\r\n",
    );
    command(
        &mut stream,
        &[b"LRANGE", b"s", b"0", b"-1"],
        b"*1\r\n$1\r\nv\r\n",
    );
    command(&mut stream, &[b"EXISTS", b"destination"], b":0\r\n");
    // Same-key rotation must not grow the payload and must not fail.
    command(&mut stream, &[b"RPOPLPUSH", b"s", b"s"], b"$1\r\nv\r\n");
    command(
        &mut stream,
        &[b"LRANGE", b"s", b"0", b"-1"],
        b"*1\r\n$1\r\nv\r\n",
    );
    server.shutdown().unwrap();
}

#[test]
fn list_move_notifies_destination_push_source_pop_and_del() {
    let server = start_notify();
    let mut writer = connect(&server);
    command(
        &mut writer,
        &[b"RPUSH", b"src", b"a", b"b", b"c"],
        b":3\r\n",
    );
    command(&mut writer, &[b"RPUSH", b"dst", b"z"], b":1\r\n");
    let mut subscriber = connect(&server);
    subscribe(&mut subscriber, b"PSUBSCRIBE", b"__keyspace@*");

    command(
        &mut writer,
        &[b"LMOVE", b"src", b"dst", b"LEFT", b"RIGHT"],
        b"$1\r\na\r\n",
    );
    expect_event(&mut subscriber, b"dst", b"rpush");
    expect_event(&mut subscriber, b"src", b"lpop");

    command(&mut writer, &[b"RPOPLPUSH", b"src", b"dst"], b"$1\r\nc\r\n");
    expect_event(&mut subscriber, b"dst", b"lpush");
    expect_event(&mut subscriber, b"src", b"rpop");

    // The move empties the source, so a generic del follows the pop.
    command(&mut writer, &[b"RPOPLPUSH", b"src", b"dst"], b"$1\r\nb\r\n");
    expect_event(&mut subscriber, b"dst", b"lpush");
    expect_event(&mut subscriber, b"src", b"rpop");
    expect_event(&mut subscriber, b"src", b"del");

    // Blocking forms emit the same events as their non-blocking equivalents.
    command(&mut writer, &[b"RPUSH", b"bsrc", b"x"], b":1\r\n");
    expect_event(&mut subscriber, b"bsrc", b"rpush");
    command(
        &mut writer,
        &[b"BLMOVE", b"bsrc", b"bdst", b"LEFT", b"LEFT", b"0"],
        b"$1\r\nx\r\n",
    );
    expect_event(&mut subscriber, b"bdst", b"lpush");
    expect_event(&mut subscriber, b"bsrc", b"lpop");
    expect_event(&mut subscriber, b"bsrc", b"del");

    server.shutdown().unwrap();
}
