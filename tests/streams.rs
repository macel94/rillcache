#![forbid(unsafe_code)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

fn start(workers: Option<usize>) -> ServerHandle {
    start_with_budget(workers, Config::default().max_data_bytes)
}

fn start_with_budget(workers: Option<usize>, max_payload_bytes: usize) -> ServerHandle {
    let config = Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        worker_threads: workers,
        max_data_bytes: max_payload_bytes,
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

#[derive(Debug, PartialEq, Eq)]
enum Metadata {
    Bulk(Vec<u8>),
    Integer(i64),
    Array(Vec<Metadata>),
    Null,
}

impl Metadata {
    fn array(&self) -> &[Self] {
        let Self::Array(items) = self else {
            panic!("expected array: {self:?}")
        };
        items
    }
    fn bulk(&self) -> &[u8] {
        let Self::Bulk(bytes) = self else {
            panic!("expected bulk: {self:?}")
        };
        bytes
    }
    fn integer(&self) -> i64 {
        let Self::Integer(value) = self else {
            panic!("expected integer: {self:?}")
        };
        *value
    }
    fn field(&self, name: &[u8]) -> &Self {
        let (pairs, remainder) = self.array().as_chunks::<2>();
        assert!(remainder.is_empty());
        let mut matches = pairs.iter().filter(|pair| pair[0].bulk() == name);
        let value = &matches.next().expect("required metadata field")[1];
        assert!(matches.next().is_none(), "duplicate metadata field");
        value
    }
}

fn parse_metadata(bytes: &mut &[u8]) -> Metadata {
    let line_end = bytes.windows(2).position(|w| w == b"\r\n").unwrap();
    let marker = bytes[0];
    let length = std::str::from_utf8(&bytes[1..line_end])
        .unwrap()
        .parse::<i64>()
        .unwrap();
    *bytes = &bytes[line_end + 2..];
    match (marker, length) {
        (b':', value) => Metadata::Integer(value),
        (b'$' | b'*', -1) => Metadata::Null,
        (b'$', length) => {
            let length = usize::try_from(length).unwrap();
            let payload = bytes[..length].to_vec();
            assert_eq!(&bytes[length..length + 2], b"\r\n");
            *bytes = &bytes[length + 2..];
            Metadata::Bulk(payload)
        }
        (b'*', length) => Metadata::Array(
            (0..usize::try_from(length).unwrap())
                .map(|_| parse_metadata(bytes))
                .collect(),
        ),
        _ => panic!("unexpected metadata marker {marker:?}"),
    }
}

fn metadata(reply: &[u8]) -> Metadata {
    let mut bytes = reply;
    let value = parse_metadata(&mut bytes);
    assert!(bytes.is_empty(), "trailing metadata bytes");
    value
}

fn assert_group(info: &Metadata, name: &[u8], last: &[u8], expected: &[(&[u8], &[u8])]) {
    let group = info
        .field(b"groups")
        .array()
        .iter()
        .find(|g| g.field(b"name").bulk() == name)
        .unwrap();
    assert_eq!(group.field(b"last-delivered-id").bulk(), last);
    assert_eq!(group.field(b"pel-count").integer(), expected.len() as i64);
    assert_eq!(group.field(b"nacked-count").integer(), 0);
    let pending = group.field(b"pending").array();
    assert_eq!(pending.len(), expected.len());
    for (item, (id, owner)) in pending.iter().zip(expected) {
        let fields = item.array();
        assert_eq!(fields.len(), 4);
        assert_eq!(fields[0].bulk(), *id);
        assert_eq!(fields[1].bulk(), *owner);
        assert!(fields[2].integer() > 0);
        assert_eq!(fields[3].integer(), 1);
    }
    let consumers = group.field(b"consumers").array();
    let owners: std::collections::BTreeSet<_> = expected.iter().map(|(_, owner)| *owner).collect();
    assert_eq!(consumers.len(), owners.len());
    let mut observed = 0;
    for consumer in consumers {
        let name = consumer.field(b"name").bulk();
        let owned: Vec<_> = pending
            .iter()
            .filter(|p| p.array()[1].bulk() == name)
            .collect();
        assert_eq!(consumer.field(b"pel-count").integer(), owned.len() as i64);
        assert!(consumer.field(b"seen-time").integer() > 0);
        assert!(consumer.field(b"active-time").integer() > 0);
        let records = consumer.field(b"pending").array();
        assert_eq!(records.len(), owned.len());
        for (record, group_record) in records.iter().zip(owned) {
            let item = record.array();
            let group_item = group_record.array();
            assert_eq!(item.len(), 3);
            assert_eq!(item[0], group_item[0]);
            assert_eq!(item[1], group_item[2]);
            assert_eq!(item[2], group_item[3]);
        }
        observed += records.len();
    }
    assert_eq!(observed, expected.len());
}

#[test]
fn stream_rename_admission_and_expiry_are_atomic_over_tcp_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start_with_budget(workers, 7);
        let mut stream = TcpStream::connect(server.local_addr()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        assert_eq!(
            request(
                &mut stream,
                &mut reader,
                &[b"XADD", b"a", b"1", b"f", b"v\0"]
            ),
            b"$3\r\n1-0\r\n"
        );
        assert_eq!(
            request(
                &mut stream,
                &mut reader,
                &[b"PEXPIREAT", b"a", b"9223372036854775807"]
            ),
            b":1\r\n"
        );
        let before = request(
            &mut stream,
            &mut reader,
            &[b"XINFO", b"STREAM", b"a", b"FULL"],
        );
        for command in [b"RENAME".as_slice(), b"RENAMENX", b"COPY"] {
            assert_eq!(
                request(&mut stream, &mut reader, &[command, b"a", b"longer"]),
                b"-OOM configured payload limit exceeded\r\n"
            );
        }
        assert_eq!(
            request(
                &mut stream,
                &mut reader,
                &[b"XINFO", b"STREAM", b"a", b"FULL"]
            ),
            before
        );
        assert_eq!(
            request(&mut stream, &mut reader, &[b"RENAMENX", b"a", b"long"]),
            b":1\r\n"
        );
        assert_eq!(
            request(&mut stream, &mut reader, &[b"PEXPIRETIME", b"long"]),
            b":9223372036854775807\r\n"
        );
        assert_eq!(
            request(
                &mut stream,
                &mut reader,
                &[b"XINFO", b"STREAM", b"long", b"FULL"]
            ),
            before
        );
        assert_eq!(
            request(&mut stream, &mut reader, &[b"TYPE", b"a"]),
            b"+none\r\n"
        );
        assert_eq!(
            request(&mut stream, &mut reader, &[b"DEL", b"long"]),
            b":1\r\n"
        );
        assert_eq!(
            request(&mut stream, &mut reader, &[b"SET", b"a", b"123456"]),
            b"+OK\r\n"
        );
        assert_eq!(request(&mut stream, &mut reader, &[b"DBSIZE"]), b":1\r\n");
        server.shutdown().unwrap();
    }
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
        let noack_info = metadata(&request(
            &mut stream,
            &mut reader,
            &[b"XINFO", b"STREAM", b"x", b"FULL"],
        ));
        assert_group(
            &noack_info,
            b"g1",
            b"102-0",
            &[(b"100-0", b"Alice"), (b"101-0", b"Bob")],
        );
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
        let info = metadata(&copied);
        assert_eq!(info.field(b"length").integer(), 5);
        let ids: Vec<_> = info
            .field(b"entries")
            .array()
            .iter()
            .map(|e| e.array()[0].bulk())
            .collect();
        assert_eq!(ids, [b"100-0", b"101-0", b"102-0", b"104-0", b"105-0"]);
        assert_group(
            &info,
            b"g1",
            b"104-0",
            &[
                (b"100-0", b"Alice"),
                (b"101-0", b"Bob"),
                (b"102-0", b"Dave"),
                (b"103-0", b"Dave"),
                (b"104-0", b"Dave"),
            ],
        );
        assert_group(
            &info,
            b"g2",
            b"103-0",
            &[
                (b"100-0", b"Charlie"),
                (b"101-0", b"Charlie"),
                (b"102-0", b"Charlie"),
                (b"103-0", b"Charlie"),
            ],
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
            request(&mut stream, &mut reader, &[b"RENAME", b"copy", b"moved"]),
            b"+OK\r\n"
        );
        assert_eq!(
            request(&mut stream, &mut reader, &[b"RENAMENX", b"moved", b"x"]),
            b":0\r\n"
        );
        assert_eq!(
            request(&mut stream, &mut reader, &[b"RENAMENX", b"moved", b"copy"]),
            b":1\r\n"
        );
        // Mutating source entries and cursor cannot mutate the copied tree.
        assert_eq!(
            request(&mut stream, &mut reader, &[b"XDEL", b"x", b"100"]),
            b":1\r\n"
        );
        assert_eq!(
            request(
                &mut stream,
                &mut reader,
                &[b"XGROUP", b"SETID", b"x", b"g1", b"0"]
            ),
            b"+OK\r\n"
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
