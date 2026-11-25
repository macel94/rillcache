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

fn cursor_page(cursor: &str, values: &[&[u8]]) -> Vec<u8> {
    let mut output = format!("*2\r\n${}\r\n{cursor}\r\n", cursor.len()).into_bytes();
    output.extend_from_slice(&array(values));
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
    read_scan_page(stream, &arguments)
}

fn collection_scan_page(
    stream: &mut TcpStream,
    command: &[u8],
    key: &[u8],
    cursor: &str,
    options: &[&[u8]],
) -> (String, Vec<Vec<u8>>) {
    let mut arguments = vec![command, key, cursor.as_bytes()];
    arguments.extend_from_slice(options);
    read_scan_page(stream, &arguments)
}

fn read_scan_page(stream: &mut TcpStream, arguments: &[&[u8]]) -> (String, Vec<Vec<u8>>) {
    let mut request = Vec::new();
    encode_request(arguments, &mut request);
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

fn collect_scan(
    stream: &mut TcpStream,
    command: &[u8],
    key: &[u8],
    options: &[&[u8]],
) -> Vec<Vec<u8>> {
    let mut cursor = "0".to_owned();
    let mut values = Vec::new();
    loop {
        let (next, page) = collection_scan_page(stream, command, key, &cursor, options);
        values.extend(page);
        cursor = next;
        if cursor == "0" {
            break;
        }
    }
    values
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

    let hash_values = collect_scan(&mut stream, b"HSCAN", b"hash", &[b"COUNT", b"4"]);
    assert_eq!(hash_values.len(), fields.len() * 2);
    assert!(
        hash_values
            .chunks_exact(2)
            .all(|pair| pair[1].as_slice() == b"value")
    );
    let mut found_fields: Vec<Vec<u8>> = hash_values
        .chunks_exact(2)
        .map(|pair| pair[0].clone())
        .collect();
    found_fields.sort_unstable();
    let mut expected_fields: Vec<Vec<u8>> = fields
        .iter()
        .map(|field| field.as_bytes().to_vec())
        .collect();
    expected_fields.sort_unstable();
    assert_eq!(found_fields, expected_fields);

    let mut no_values = collect_scan(
        &mut stream,
        b"HSCAN",
        b"hash",
        &[b"COUNT", b"2", b"NOVALUES"],
    );
    no_values.sort_unstable();
    assert_eq!(no_values, expected_fields);

    send(
        &mut stream,
        &[b"SADD", b"set", b"m02", b"m00", b"m01", b"m04", b"m03"],
        b":5\r\n",
    );
    let mut set_members = collect_scan(&mut stream, b"SSCAN", b"set", &[b"COUNT", b"2"]);
    set_members.sort_unstable();
    assert_eq!(
        set_members,
        (0..5)
            .map(|i| format!("m{i:02}").into_bytes())
            .collect::<Vec<_>>()
    );

    let scores: Vec<String> = (0..5).map(|index| format!("{index}.5")).collect();
    let members: Vec<String> = (0..5).map(|index| format!("m{index:02}")).collect();
    let mut command = vec![b"ZADD".as_slice(), b"zset"];
    for (score, member) in scores.iter().zip(&members) {
        command.push(score.as_bytes());
        command.push(member.as_bytes());
    }
    send(&mut stream, &command, b":5\r\n");
    let zset_values = collect_scan(&mut stream, b"ZSCAN", b"zset", &[b"COUNT", b"2"]);
    assert_eq!(zset_values.len(), members.len() * 2);
    let mut found_scores: Vec<(Vec<u8>, Vec<u8>)> = zset_values
        .chunks_exact(2)
        .map(|pair| (pair[0].clone(), pair[1].clone()))
        .collect();
    found_scores.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    let mut expected_scores: Vec<(Vec<u8>, Vec<u8>)> = members
        .iter()
        .zip(&scores)
        .map(|(member, score)| (member.as_bytes().to_vec(), score.as_bytes().to_vec()))
        .collect();
    expected_scores.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    assert_eq!(found_scores, expected_scores);

    let mut no_scores = collect_scan(
        &mut stream,
        b"ZSCAN",
        b"zset",
        &[b"COUNT", b"2", b"NOSCORES"],
    );
    no_scores.sort_unstable();
    assert_eq!(
        no_scores,
        members
            .iter()
            .map(|member| member.as_bytes().to_vec())
            .collect::<Vec<_>>()
    );

    let mut matching = collect_scan(
        &mut stream,
        b"ZSCAN",
        b"zset",
        &[b"NOSCORES", b"MATCH", b"m0?"],
    );
    matching.sort_unstable();
    assert_eq!(
        matching,
        members
            .iter()
            .map(|member| member.as_bytes().to_vec())
            .collect::<Vec<_>>()
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

#[test]
fn sscan_cursor_keeps_remaining_members_after_set_shrinkage() {
    let server = start();
    let mut stream = connect(&server);
    let added: Vec<String> = (0..200).map(|index| index.to_string()).collect();
    let mut add: Vec<&[u8]> = vec![b"SADD", b"set", b"x"];
    add.extend(added.iter().map(|value| value.as_bytes()));
    send(&mut stream, &add, b":201\r\n");

    let (mut cursor, mut found) =
        collection_scan_page(&mut stream, b"SSCAN", b"set", "0", &[b"COUNT", b"10"]);
    assert_ne!(cursor, "0");
    let removed: Vec<String> = (100..200).map(|index| index.to_string()).collect();
    let mut remove: Vec<&[u8]> = vec![b"SREM", b"set"];
    remove.extend(removed.iter().map(|value| value.as_bytes()));
    send(&mut stream, &remove, b":100\r\n");
    while cursor != "0" {
        let (next, page) =
            collection_scan_page(&mut stream, b"SSCAN", b"set", &cursor, &[b"COUNT", b"10"]);
        found.extend(page);
        cursor = next;
    }
    for index in 0..100 {
        let stable = index.to_string().into_bytes();
        assert!(
            found.contains(&stable),
            "SSCAN missed stable member {index}"
        );
    }
    server.shutdown().unwrap();
}
