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

#[test]
fn scan_pages_sorted_keys_with_wrapping_cursor_and_filters() {
    let server = start();
    let mut stream = connect(&server);
    for index in 0..25 {
        let key = format!("k{index:02}");
        send(&mut stream, &[b"SET", key.as_bytes(), b"v"], b"+OK\r\n");
        if index % 5 == 0 {
            send(
                &mut stream,
                &[b"HSET", format!("h{index:02}").as_bytes(), b"f", b"v"],
                b":1\r\n",
            );
        }
    }
    send(
        &mut stream,
        &[b"SCAN", b"0", b"COUNT", b"5"],
        &cursor_page("5", &[b"h00", b"h05", b"h10", b"h15", b"h20"]),
    );
    send(
        &mut stream,
        &[b"SCAN", b"20", b"COUNT", b"10"],
        &cursor_page(
            "0",
            &[
                b"k15", b"k16", b"k17", b"k18", b"k19", b"k20", b"k21", b"k22", b"k23", b"k24",
            ],
        ),
    );
    send(
        &mut stream,
        &[b"SCAN", b"24", b"COUNT", b"10"],
        &cursor_page("0", &[b"k19", b"k20", b"k21", b"k22", b"k23", b"k24"]),
    );
    send(
        &mut stream,
        &[b"SCAN", b"25"],
        &cursor_page("0", &[b"k20", b"k21", b"k22", b"k23", b"k24"]),
    );
    send(&mut stream, &[b"SCAN", b"30"], &cursor_page("0", &[]));
    send(
        &mut stream,
        &[b"SCAN", b"0", b"MATCH", b"k1*", b"COUNT", b"100"],
        &cursor_page(
            "0",
            &[
                b"k10", b"k11", b"k12", b"k13", b"k14", b"k15", b"k16", b"k17", b"k18", b"k19",
            ],
        ),
    );
    send(
        &mut stream,
        &[b"SCAN", b"0", b"TYPE", b"hash", b"COUNT", b"100"],
        &cursor_page("0", &[b"h00", b"h05", b"h10", b"h15", b"h20"]),
    );
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
