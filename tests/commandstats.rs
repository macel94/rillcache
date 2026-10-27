#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

fn start(workers: Option<usize>, password: Option<&[u8]>) -> ServerHandle {
    Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        worker_threads: workers,
        requirepass: password.map(<[u8]>::to_vec),
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

fn reply(stream: &mut TcpStream, args: &[&[u8]]) -> Vec<u8> {
    let mut request = Vec::new();
    encode_request(args, &mut request);
    stream.write_all(&request).unwrap();
    read_frame(stream, 0)
}

fn command(stream: &mut TcpStream, args: &[&[u8]], expected: &[u8]) {
    assert_eq!(reply(stream, args), expected, "{args:?}");
}

fn bulk_text(reply: &[u8]) -> String {
    assert_eq!(reply.first(), Some(&b'$'), "{reply:?}");
    let header_end = reply
        .windows(2)
        .position(|window| window == b"\r\n")
        .unwrap();
    let length = std::str::from_utf8(&reply[1..header_end])
        .unwrap()
        .parse::<usize>()
        .unwrap();
    let body = &reply[header_end + 2..header_end + 2 + length];
    std::str::from_utf8(body).unwrap().to_string()
}

fn stat_line<'a>(stats: &'a str, command: &str) -> Option<&'a str> {
    let prefix = format!("cmdstat_{command}:");
    stats.lines().find(|line| line.starts_with(&prefix))
}

#[test]
fn info_commandstats_records_calls_failures_and_resets_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers, None);
        let mut stream = connect(&server);

        // Statistics are opt-in so the default dispatch path stays free.
        command(
            &mut stream,
            &[b"CONFIG", b"GET", b"commandstats-enabled"],
            b"*2\r\n$20\r\ncommandstats-enabled\r\n$2\r\nno\r\n",
        );
        command(&mut stream, &[b"PING"], b"+PONG\r\n");
        let disabled = bulk_text(&reply(&mut stream, &[b"INFO", b"commandstats"]));
        assert!(stat_line(&disabled, "ping").is_none(), "{disabled}");
        command(
            &mut stream,
            &[b"CONFIG", b"SET", b"commandstats-enabled", b"bogus"],
            b"-ERR commandstats-enabled must be yes or no\r\n",
        );
        command(
            &mut stream,
            &[b"CONFIG", b"SET", b"commandstats-enabled", b"yes"],
            b"+OK\r\n",
        );

        command(&mut stream, &[b"PING"], b"+PONG\r\n");
        command(&mut stream, &[b"SET", b"k", b"v"], b"+OK\r\n");
        command(&mut stream, &[b"GET", b"k"], b"$1\r\nv\r\n");
        let failed = reply(&mut stream, &[b"INCR", b"k"]);
        assert_eq!(failed.first(), Some(&b'-'));
        let unknown = reply(&mut stream, &[b"NOSUCHCOMMAND"]);
        assert!(unknown.starts_with(b"-ERR unknown command"));

        let stats = bulk_text(&reply(&mut stream, &[b"INFO", b"commandstats"]));
        assert!(stats.starts_with("# Commandstats\r\n"), "{stats}");
        assert!(stat_line(&stats, "ping").is_some(), "{stats}");
        assert!(stat_line(&stats, "set").is_some(), "{stats}");
        assert!(stat_line(&stats, "get").is_some(), "{stats}");
        let incr = stat_line(&stats, "incr").expect("INCR statistics");
        assert!(incr.contains("calls=1"), "{incr}");
        assert!(incr.contains("failed_calls=1"), "{incr}");
        assert!(incr.contains("rejected_calls=0"), "{incr}");
        assert!(stat_line(&stats, "nosuchcommand").is_none(), "{stats}");

        let all = bulk_text(&reply(&mut stream, &[b"INFO", b"all"]));
        assert!(all.contains("# Commandstats\r\n"), "{all}");
        assert!(all.contains("cmdstat_set:calls=1"), "{all}");

        command(&mut stream, &[b"CONFIG", b"RESETSTAT"], b"+OK\r\n");
        let reset = bulk_text(&reply(&mut stream, &[b"INFO", b"commandstats"]));
        assert!(stat_line(&reset, "set").is_none(), "{reset}");
        assert!(stat_line(&reset, "get").is_none(), "{reset}");
        assert!(stat_line(&reset, "incr").is_none(), "{reset}");

        server.shutdown().unwrap();
    }
}

#[test]
fn info_commandstats_counts_authentication_rejections_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers, Some(b"secret"));
        let mut operator = connect(&server);
        command(&mut operator, &[b"AUTH", b"secret"], b"+OK\r\n");
        command(
            &mut operator,
            &[b"CONFIG", b"SET", b"commandstats-enabled", b"yes"],
            b"+OK\r\n",
        );

        let mut stream = connect(&server);
        let denied = reply(&mut stream, &[b"SET", b"k", b"v"]);
        assert!(denied.starts_with(b"-NOAUTH"), "{denied:?}");
        command(&mut stream, &[b"AUTH", b"secret"], b"+OK\r\n");

        let stats = bulk_text(&reply(&mut operator, &[b"INFO", b"commandstats"]));
        let set = stat_line(&stats, "set").expect("SET statistics");
        assert!(set.contains("calls=1"), "{set}");
        assert!(set.contains("rejected_calls=1"), "{set}");
        assert!(set.contains("failed_calls=1"), "{set}");
        let auth = stat_line(&stats, "auth").expect("AUTH statistics");
        assert!(auth.contains("calls=1"), "{auth}");
        assert!(auth.contains("rejected_calls=0"), "{auth}");
        let config = stat_line(&stats, "config").expect("CONFIG statistics");
        assert!(config.contains("calls=1"), "{config}");

        server.shutdown().unwrap();
    }
}
