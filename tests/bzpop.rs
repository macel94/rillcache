#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

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
        .set_read_timeout(Some(Duration::from_secs(5)))
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

#[test]
fn bzpop_pops_immediately_and_reports_errors_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = connect(&server);

        command(
            &mut stream,
            &[b"ZADD", b"pop", b"1", b"a", b"2", b"b", b"3", b"c"],
            b":3\r\n",
        );
        command(
            &mut stream,
            &[b"BZPOPMIN", b"missing", b"pop", b"0"],
            b"*3\r\n$3\r\npop\r\n$1\r\na\r\n$1\r\n1\r\n",
        );
        command(
            &mut stream,
            &[b"BZPOPMAX", b"pop", b"missing", b"0"],
            b"*3\r\n$3\r\npop\r\n$1\r\nc\r\n$1\r\n3\r\n",
        );
        command(&mut stream, &[b"EXISTS", b"pop"], b":1\r\n");

        command(
            &mut stream,
            &[b"ZADD", b"pt", b"1", b"a", b"2", b"b"],
            b":2\r\n",
        );
        command(&mut stream, &[b"PEXPIRE", b"pt", b"100000"], b":1\r\n");
        command(
            &mut stream,
            &[b"BZPOPMIN", b"pt", b"0"],
            b"*3\r\n$2\r\npt\r\n$1\r\na\r\n$1\r\n1\r\n",
        );
        let ttl = reply(&mut stream, &[b"PTTL", b"pt"]);
        let ttl = std::str::from_utf8(&ttl[1..ttl.len() - 2])
            .unwrap()
            .parse::<i64>()
            .unwrap();
        assert!(ttl > 0);

        command(&mut stream, &[b"ZADD", b"first", b"1", b"a"], b":1\r\n");
        command(&mut stream, &[b"RPUSH", b"list", b"x"], b":1\r\n");
        command(
            &mut stream,
            &[b"BZPOPMIN", b"first", b"list", b"0"],
            b"*3\r\n$5\r\nfirst\r\n$1\r\na\r\n$1\r\n1\r\n",
        );
        command(
            &mut stream,
            &[b"BZPOPMAX", b"list", b"first", b"0"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );

        command(
            &mut stream,
            &[b"BZPOPMIN", b"first", b"nope"],
            b"-ERR timeout is not a float or out of range\r\n",
        );
        command(
            &mut stream,
            &[b"BZPOPMAX", b"first", b"-1"],
            b"-ERR timeout is negative\r\n",
        );
        command(
            &mut stream,
            &[b"BZPOPMIN", b"first", b"inf"],
            b"-ERR timeout is out of range\r\n",
        );
        command(
            &mut stream,
            &[b"BZPOPMIN"],
            b"-ERR wrong number of arguments for 'bzpopmin' command\r\n",
        );
        command(
            &mut stream,
            &[b"BZPOPMAX", b"0"],
            b"-ERR wrong number of arguments for 'bzpopmax' command\r\n",
        );

        server.shutdown().unwrap();
    }
}

#[test]
fn bzpop_times_out_and_wakes_blocked_clients_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut blocked = connect(&server);
        let mut other = connect(&server);

        let started = Instant::now();
        command(&mut blocked, &[b"BZPOPMIN", b"missing", b"0.1"], b"*-1\r\n");
        assert!(started.elapsed() >= Duration::from_millis(50));
        assert!(started.elapsed() < Duration::from_secs(4));

        send(&mut blocked, &[b"BZPOPMIN", b"wake", b"0"]);
        std::thread::sleep(Duration::from_millis(100));
        command(&mut other, &[b"ZADD", b"wake", b"5", b"m"], b":1\r\n");
        assert_eq!(
            read_frame(&mut blocked, 0),
            b"*3\r\n$4\r\nwake\r\n$1\r\nm\r\n$1\r\n5\r\n"
        );

        send(&mut blocked, &[b"BZPOPMAX", b"future", b"other", b"0"]);
        std::thread::sleep(Duration::from_millis(100));
        command(
            &mut other,
            &[b"ZADD", b"future", b"1", b"a", b"7", b"z"],
            b":2\r\n",
        );
        assert_eq!(
            read_frame(&mut blocked, 0),
            b"*3\r\n$6\r\nfuture\r\n$1\r\nz\r\n$1\r\n7\r\n"
        );

        server.shutdown().unwrap();
    }
}

#[test]
fn bzpop_executes_non_blocking_inside_transactions_and_respects_watch() {
    let server = start(None);
    let mut stream = connect(&server);
    let mut other = connect(&server);

    command(
        &mut stream,
        &[b"ZADD", b"q", b"1", b"a", b"2", b"b"],
        b":2\r\n",
    );
    command(&mut stream, &[b"MULTI"], b"+OK\r\n");
    command(&mut stream, &[b"BZPOPMIN", b"q", b"0"], b"+QUEUED\r\n");
    command(
        &mut stream,
        &[b"EXEC"],
        b"*1\r\n*3\r\n$1\r\nq\r\n$1\r\na\r\n$1\r\n1\r\n",
    );

    command(&mut stream, &[b"MULTI"], b"+OK\r\n");
    command(&mut stream, &[b"BZPOPMAX", b"q", b"-1"], b"+QUEUED\r\n");
    command(
        &mut stream,
        &[b"EXEC"],
        b"*1\r\n-ERR timeout is negative\r\n",
    );

    command(&mut stream, &[b"MULTI"], b"+OK\r\n");
    command(
        &mut stream,
        &[b"BZPOPMIN", b"missing", b"1"],
        b"+QUEUED\r\n",
    );
    command(&mut stream, &[b"EXEC"], b"*1\r\n*-1\r\n");

    command(&mut stream, &[b"ZADD", b"w", b"1", b"a"], b":1\r\n");
    command(&mut stream, &[b"WATCH", b"w"], b"+OK\r\n");
    command(&mut other, &[b"ZADD", b"w", b"2", b"b"], b":1\r\n");
    command(&mut stream, &[b"MULTI"], b"+OK\r\n");
    command(&mut stream, &[b"BZPOPMAX", b"w", b"1"], b"+QUEUED\r\n");
    command(&mut stream, &[b"EXEC"], b"*-1\r\n");
    command(
        &mut stream,
        &[b"ZRANGE", b"w", b"0", b"-1"],
        b"*2\r\n$1\r\na\r\n$1\r\nb\r\n",
    );

    server.shutdown().unwrap();
}
