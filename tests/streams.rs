#![forbid(unsafe_code)]

use std::io::{BufRead, BufReader, Read, Write};
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

fn read_reply(reader: &mut BufReader<TcpStream>) -> Vec<u8> {
    let mut marker = [0];
    reader.read_exact(&mut marker).unwrap();
    let mut reply = marker.to_vec();
    let mut line = Vec::new();
    reader.read_until(b'\n', &mut line).unwrap();
    assert!(line.ends_with(b"\r\n"));
    reply.extend_from_slice(&line);

    match marker[0] {
        b'$' => {
            let length = std::str::from_utf8(&line[..line.len() - 2])
                .unwrap()
                .parse::<isize>()
                .unwrap();
            if length >= 0 {
                let mut payload = vec![0; usize::try_from(length).unwrap() + 2];
                reader.read_exact(&mut payload).unwrap();
                reply.extend_from_slice(&payload);
            }
        }
        b'*' => {
            let count = std::str::from_utf8(&line[..line.len() - 2])
                .unwrap()
                .parse::<isize>()
                .unwrap();
            for _ in 0..count.max(0) {
                reply.extend_from_slice(&read_reply(reader));
            }
        }
        b'+' | b'-' | b':' => {}
        other => panic!("unexpected RESP marker {other:?}"),
    }
    reply
}

fn request(stream: &mut TcpStream, reader: &mut BufReader<TcpStream>, args: &[&[u8]]) -> Vec<u8> {
    let mut frame = Vec::new();
    encode_request(args, &mut frame);
    stream.write_all(&frame).unwrap();
    read_reply(reader)
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[test]
fn stream_group_copy_and_delete_scenario_works_over_tcp_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = TcpStream::connect(server.local_addr()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());

        for (id, field) in [
            (b"100".as_slice(), b"a".as_slice()),
            (b"101", b"b"),
            (b"102", b"c"),
            (b"103", b"e"),
            (b"104", b"f"),
            (b"105", b"g"),
        ] {
            let reply = request(&mut stream, &mut reader, &[b"XADD", b"x", id, field, b"1"]);
            assert!(reply.starts_with(b"$"));
        }
        assert_eq!(
            request(
                &mut stream,
                &mut reader,
                &[b"XGROUP", b"CREATE", b"x", b"g1", b"0"]
            ),
            b"+OK\r\n"
        );
        assert_eq!(
            request(
                &mut stream,
                &mut reader,
                &[b"XGROUP", b"CREATE", b"x", b"g2", b"0"]
            ),
            b"+OK\r\n"
        );

        let alice = request(
            &mut stream,
            &mut reader,
            &[
                b"XREADGROUP",
                b"GROUP",
                b"g1",
                b"Alice",
                b"COUNT",
                b"1",
                b"STREAMS",
                b"x",
                b">",
            ],
        );
        assert!(contains(&alice, b"100-0"));
        let bob = request(
            &mut stream,
            &mut reader,
            &[
                b"XREADGROUP",
                b"GROUP",
                b"g1",
                b"Bob",
                b"COUNT",
                b"1",
                b"STREAMS",
                b"x",
                b">",
            ],
        );
        assert!(contains(&bob, b"101-0"));
        let noack = request(
            &mut stream,
            &mut reader,
            &[
                b"XREADGROUP",
                b"GROUP",
                b"g1",
                b"Bob",
                b"NOACK",
                b"COUNT",
                b"1",
                b"STREAMS",
                b"x",
                b">",
            ],
        );
        assert!(contains(&noack, b"102-0"));
        let charlie = request(
            &mut stream,
            &mut reader,
            &[
                b"XREADGROUP",
                b"GROUP",
                b"g2",
                b"Charlie",
                b"COUNT",
                b"4",
                b"STREAMS",
                b"x",
                b">",
            ],
        );
        for id in [b"100-0".as_slice(), b"101-0", b"102-0", b"103-0"] {
            assert!(contains(&charlie, id));
        }
        assert_eq!(
            request(
                &mut stream,
                &mut reader,
                &[b"XGROUP", b"SETID", b"x", b"g1", b"101-0"],
            ),
            b"+OK\r\n"
        );
        let dave = request(
            &mut stream,
            &mut reader,
            &[
                b"XREADGROUP",
                b"GROUP",
                b"g1",
                b"Dave",
                b"COUNT",
                b"3",
                b"STREAMS",
                b"x",
                b">",
            ],
        );
        for id in [b"102-0".as_slice(), b"103-0", b"104-0"] {
            assert!(contains(&dave, id));
        }
        assert_eq!(
            request(&mut stream, &mut reader, &[b"XDEL", b"x", b"103"]),
            b":1\r\n"
        );
        assert_eq!(
            request(&mut stream, &mut reader, &[b"COPY", b"x", b"copy"]),
            b":1\r\n"
        );
        let copied = request(
            &mut stream,
            &mut reader,
            &[b"XINFO", b"STREAM", b"copy", b"FULL"],
        );
        assert_eq!(
            copied,
            request(
                &mut stream,
                &mut reader,
                &[b"XINFO", b"STREAM", b"x", b"FULL"],
            )
        );
        assert_eq!(
            request(&mut stream, &mut reader, &[b"DEL", b"x"]),
            b":1\r\n"
        );
        assert_eq!(
            copied,
            request(
                &mut stream,
                &mut reader,
                &[b"XINFO", b"STREAM", b"copy", b"FULL"],
            )
        );
        assert_eq!(
            request(&mut stream, &mut reader, &[b"TYPE", b"copy"]),
            b"+stream\r\n"
        );
        assert!(request(&mut stream, &mut reader, &[b"GET", b"copy"]).starts_with(b"-WRONGTYPE"));

        let mut previous = None;
        for _ in 0..1000 {
            let reply = request(
                &mut stream,
                &mut reader,
                &[b"XADD", b"generated", b"*", b"item", b"2", b"value", b"b"],
            );
            assert!(reply.starts_with(b"$"));
            let line_end = reply.iter().position(|byte| *byte == b'\n').unwrap();
            let id = std::str::from_utf8(&reply[line_end + 1..reply.len() - 2]).unwrap();
            let (millisecond, sequence) = id.split_once('-').unwrap();
            let current = (
                millisecond.parse::<u64>().unwrap(),
                sequence.parse::<u64>().unwrap(),
            );
            assert!(previous.is_none_or(|last| current > last));
            previous = Some(current);
        }
        assert_eq!(
            request(&mut stream, &mut reader, &[b"XLEN", b"generated"]),
            b":1000\r\n"
        );
        assert_eq!(
            request(
                &mut stream,
                &mut reader,
                &[b"COPY", b"generated", b"generated-copy"],
            ),
            b":1\r\n"
        );
        assert_eq!(
            request(&mut stream, &mut reader, &[b"XLEN", b"generated-copy"]),
            b":1000\r\n"
        );

        server.shutdown().unwrap();
    }
}
