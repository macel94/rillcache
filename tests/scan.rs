#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use rillcache::protocol::{encode_bulk, encode_request};
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
        .set_read_timeout(Some(Duration::from_secs(3)))
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

fn array(values: &[&[u8]]) -> Vec<u8> {
    let mut output = format!("*{}\r\n", values.len()).into_bytes();
    for value in values {
        encode_bulk(Some(value), &mut output);
    }
    output
}

fn resp_line(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    loop {
        let mut byte = [0];
        stream.read_exact(&mut byte).unwrap();
        if byte[0] == b'\r' {
            stream.read_exact(&mut byte).unwrap();
            assert_eq!(byte[0], b'\n');
            break;
        }
        bytes.push(byte[0]);
    }
    String::from_utf8(bytes).unwrap()
}

fn resp_bulk(stream: &mut TcpStream) -> Vec<u8> {
    let length: usize = resp_line(stream)
        .strip_prefix('$')
        .unwrap()
        .parse()
        .unwrap();
    let mut value = vec![0; length];
    stream.read_exact(&mut value).unwrap();
    let mut crlf = [0; 2];
    stream.read_exact(&mut crlf).unwrap();
    assert_eq!(&crlf, b"\r\n");
    value
}

fn scan_page(stream: &mut TcpStream, cursor: &str, options: &[&[u8]]) -> (String, Vec<Vec<u8>>) {
    let mut arguments = vec![b"SCAN".as_slice(), cursor.as_bytes()];
    arguments.extend_from_slice(options);
    let mut request = Vec::new();
    encode_request(&arguments, &mut request);
    stream.write_all(&request).unwrap();
    assert_eq!(resp_line(stream), "*2");
    let next = String::from_utf8(resp_bulk(stream)).unwrap();
    let length: usize = resp_line(stream)
        .strip_prefix('*')
        .unwrap()
        .parse()
        .unwrap();
    let values = (0..length).map(|_| resp_bulk(stream)).collect();
    (next, values)
}

#[test]
fn scan_traversal_filters_and_write_stability() {
    let server = start();
    let mut stream = connect(&server);
    let mut expected = Vec::new();
    for index in 0..25 {
        let key = format!("k{index:02}");
        send(&mut stream, &[b"SET", key.as_bytes(), b"v"], b"+OK\r\n");
        expected.push(key.into_bytes());
        if index % 5 == 0 {
            let hash = format!("h{index:02}");
            send(
                &mut stream,
                &[b"HSET", hash.as_bytes(), b"f", b"v"],
                b":1\r\n",
            );
            expected.push(hash.into_bytes());
        }
    }
    let mut cursor = "0".to_owned();
    let mut found = Vec::new();
    loop {
        let (next, values) = scan_page(&mut stream, &cursor, &[b"COUNT", b"5"]);
        found.extend(values);
        cursor = next;
        if cursor == "0" {
            break;
        }
    }
    found.sort();
    expected.sort();
    assert_eq!(found, expected);

    cursor = "0".to_owned();
    let mut matching = Vec::new();
    loop {
        let (next, values) = scan_page(&mut stream, &cursor, &[b"MATCH", b"k1*", b"COUNT", b"5"]);
        matching.extend(values);
        cursor = next;
        if cursor == "0" {
            break;
        }
    }
    matching.sort();
    assert_eq!(
        matching,
        (10..20)
            .map(|index| format!("k{index:02}").into_bytes())
            .collect::<Vec<_>>()
    );

    cursor = "0".to_owned();
    let mut hashes = Vec::new();
    loop {
        let (next, values) = scan_page(&mut stream, &cursor, &[b"TYPE", b"hash", b"COUNT", b"5"]);
        hashes.extend(values);
        cursor = next;
        if cursor == "0" {
            break;
        }
    }
    hashes.sort();
    assert_eq!(
        hashes,
        vec![
            b"h00".to_vec(),
            b"h05".to_vec(),
            b"h10".to_vec(),
            b"h15".to_vec(),
            b"h20".to_vec(),
        ]
    );

    send(&mut stream, &[b"FLUSHALL"], b"+OK\r\n");
    let mut originals = Vec::new();
    for index in 0..100 {
        let key = format!("key:{index:03}");
        send(&mut stream, &[b"SET", key.as_bytes(), b"v"], b"+OK\r\n");
        originals.push(key.into_bytes());
    }
    cursor = "0".to_owned();
    found.clear();
    let mut round = 0;
    loop {
        let (next, values) = scan_page(&mut stream, &cursor, &[b"COUNT", b"10"]);
        found.extend(values);
        cursor = next;
        if cursor == "0" {
            break;
        }
        if round < 10 {
            for index in 0..10 {
                let key = format!("added:{round}:{index}");
                send(&mut stream, &[b"SET", key.as_bytes(), b"v"], b"+OK\r\n");
            }
            round += 1;
        }
    }
    for original in originals {
        assert!(
            found.contains(&original),
            "SCAN missed stable key {original:?}"
        );
    }

    for arguments in [
        &[b"SCAN".as_slice(), b"not-a-cursor"][..],
        &[b"SCAN".as_slice(), b"0", b"COUNT", b"0"][..],
        &[b"SCAN".as_slice(), b"0", b"COUNT", b"-1"][..],
        &[b"SCAN".as_slice(), b"0", b"BOGUS"][..],
        &[b"SCAN".as_slice(), b"0", b"MATCH"][..],
    ] {
        send(
            &mut stream,
            arguments,
            b"-ERR syntax error or invalid cursor\r\n",
        );
    }
    server.shutdown().unwrap();
}

#[test]
fn collection_scans_page_hash_set_and_zset_members() {
    let server = start();
    let mut stream = connect(&server);
    let mut command = vec![b"HSET".as_slice(), b"hash"];
    let fields: Vec<String> = (0..10).map(|index| format!("f{index:02}")).collect();
    for field in &fields {
        command.push(field.as_bytes());
        command.push(b"value");
    }
    send(
        &mut stream,
        &command,
        format!(":{}\r\n", fields.len()).as_bytes(),
    );

    let mut page = String::new();
    page.push_str("*2\r\n$1\r\n4\r\n*8\r\n");
    for field in &fields[..4] {
        page.push_str(&format!("$3\r\n{field}\r\n$5\r\nvalue\r\n"));
    }
    send(
        &mut stream,
        &[b"HSCAN", b"hash", b"0", b"COUNT", b"4"],
        page.as_bytes(),
    );
    let mut page = String::new();
    page.push_str("*2\r\n$1\r\n0\r\n*4\r\n");
    for field in &fields[8..] {
        page.push_str(&format!("$3\r\n{field}\r\n$5\r\nvalue\r\n"));
    }
    send(
        &mut stream,
        &[b"HSCAN", b"hash", b"8", b"COUNT", b"4"],
        page.as_bytes(),
    );
    send(
        &mut stream,
        &[b"HSCAN", b"hash", b"0", b"COUNT", b"2", b"NOVALUES"],
        &cursor_page("2", &[b"f00", b"f01"]),
    );

    send(
        &mut stream,
        &[b"SADD", b"set", b"m02", b"m00", b"m01", b"m04", b"m03"],
        b":5\r\n",
    );
    send(
        &mut stream,
        &[b"SSCAN", b"set", b"0", b"COUNT", b"2"],
        &cursor_page("2", &[b"m00", b"m01"]),
    );
    send(
        &mut stream,
        &[b"SSCAN", b"set", b"4", b"COUNT", b"2"],
        &cursor_page("0", &[b"m04"]),
    );

    let scores: Vec<String> = (0..5).map(|index| format!("{index}.5")).collect();
    let members: Vec<String> = (0..5).map(|index| format!("m{index:02}")).collect();
    let mut command = vec![b"ZADD".as_slice(), b"zset"];
    for (score, member) in scores.iter().zip(&members) {
        command.push(score.as_bytes());
        command.push(member.as_bytes());
    }
    send(&mut stream, &command, b":5\r\n");
    let mut page = String::new();
    page.push_str("*2\r\n$1\r\n2\r\n*4\r\n$3\r\nm00\r\n$3\r\n0.5\r\n$3\r\nm01\r\n$3\r\n1.5\r\n");
    send(
        &mut stream,
        &[b"ZSCAN", b"zset", b"0", b"COUNT", b"2"],
        page.as_bytes(),
    );
    send(
        &mut stream,
        &[b"ZSCAN", b"zset", b"0", b"COUNT", b"2", b"NOSCORES"],
        &cursor_page("2", &[b"m00", b"m01"]),
    );
    send(
        &mut stream,
        &[b"ZSCAN", b"zset", b"0", b"NOSCORES", b"MATCH", b"m0?"],
        &cursor_page("0", &[b"m00", b"m01", b"m02", b"m03", b"m04"]),
    );
    send(
        &mut stream,
        &[b"ZSCAN", b"zset", b"0", b"NOVALUES"],
        b"-ERR syntax error or invalid cursor\r\n",
    );

    send(
        &mut stream,
        &[b"HSCAN", b"missing", b"0"],
        &cursor_page("0", &[]),
    );
    send(
        &mut stream,
        &[b"SSCAN", b"missing", b"0"],
        &cursor_page("0", &[]),
    );
    send(
        &mut stream,
        &[b"ZSCAN", b"missing", b"0"],
        &cursor_page("0", &[]),
    );
    send(&mut stream, &[b"SET", b"string", b"x"], b"+OK\r\n");
    send(
        &mut stream,
        &[b"HSCAN", b"string", b"0"],
        b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
    );
    send(
        &mut stream,
        &[b"SSCAN", b"string", b"0"],
        b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
    );
    send(
        &mut stream,
        &[b"ZSCAN", b"string", b"0"],
        b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
    );
    server.shutdown().unwrap();
}
