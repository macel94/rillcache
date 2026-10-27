#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::time::Duration;

use rillcache::engine::FsyncPolicy;
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

fn start_persistent(dir: &Path) -> ServerHandle {
    Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        appendonly: true,
        appendfsync: FsyncPolicy::Always,
        dir: dir.to_path_buf(),
        appendfilename: "test.aof".into(),
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

fn command(stream: &mut TcpStream, args: &[&[u8]], expected: &[u8]) {
    let mut request = Vec::new();
    encode_request(args, &mut request);
    stream.write_all(&request).unwrap();
    assert_eq!(read_frame(stream, 0), expected, "{args:?}");
}

fn reply(stream: &mut TcpStream, args: &[&[u8]]) -> Vec<u8> {
    let mut request = Vec::new();
    encode_request(args, &mut request);
    stream.write_all(&request).unwrap();
    read_frame(stream, 0)
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn info(stream: &mut TcpStream) -> Vec<u8> {
    reply(stream, &[b"XINFO", b"STREAM", b"s"])
}

#[test]
fn xsetid_sets_stream_metadata_and_validates_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = connect(&server);

        command(
            &mut stream,
            &[b"XADD", b"s", b"1-0", b"f", b"v"],
            b"$3\r\n1-0\r\n",
        );
        command(
            &mut stream,
            &[b"XADD", b"s", b"5-0", b"f", b"v"],
            b"$3\r\n5-0\r\n",
        );
        let initial = info(&mut stream);
        assert!(contains(&initial, b"entries-added"));
        assert!(contains(&initial, b"2"));
        assert!(contains(&initial, b"max-deleted-entry-id"));
        assert!(contains(&initial, b"0-0"));

        command(&mut stream, &[b"XSETID", b"s", b"10-0"], b"+OK\r\n");
        let raised = info(&mut stream);
        assert!(contains(&raised, b"10-0"));
        command(
            &mut stream,
            &[b"XADD", b"s", b"11-0", b"f", b"v"],
            b"$4\r\n11-0\r\n",
        );
        let after_add = info(&mut stream);
        assert!(contains(&after_add, b"entries-added"));
        assert!(contains(&after_add, b"3"));
        command(
            &mut stream,
            &[b"XADD", b"s", b"10-0", b"f", b"v"],
            b"-ERR The ID specified in XADD is equal or smaller than the target stream top item\r\n",
        );
        command(&mut stream, &[b"XSETID", b"s", b"11-0"], b"+OK\r\n");
        command(
            &mut stream,
            &[b"XSETID", b"s", b"10-0"],
            b"-ERR The ID specified in XSETID is smaller than the target stream top item\r\n",
        );

        command(
            &mut stream,
            &[b"XSETID", b"s", b"11-0", b"ENTRIESADDED", b"20"],
            b"+OK\r\n",
        );
        let added = info(&mut stream);
        assert!(contains(&added, b"entries-added"));
        assert!(contains(&added, b"20"));
        command(
            &mut stream,
            &[b"XSETID", b"s", b"11-0", b"ENTRIESADDED", b"2"],
            b"-ERR The entries_added specified in XSETID is smaller than the target stream length\r\n",
        );
        command(
            &mut stream,
            &[b"XSETID", b"s", b"11-0", b"MAXDELETEDID", b"7-0"],
            b"+OK\r\n",
        );
        assert!(contains(&info(&mut stream), b"7-0"));
        command(
            &mut stream,
            &[b"XSETID", b"s", b"11-0", b"MAXDELETEDID", b"12-0"],
            b"-ERR The ID specified in XSETID is smaller than the provided max_deleted_entry_id\r\n",
        );
        command(
            &mut stream,
            &[b"XSETID", b"s", b"11-0", b"MAXDELETEDID", b"2-0"],
            b"+OK\r\n",
        );
        assert!(contains(&info(&mut stream), b"2-0"));

        command(&mut stream, &[b"XDEL", b"s", b"5-0"], b":1\r\n");
        let after_del = info(&mut stream);
        assert!(contains(&after_del, b"max-deleted-entry-id"));
        assert!(contains(&after_del, b"5-0"));
        assert!(contains(&after_del, b"20"));
        command(&mut stream, &[b"XTRIM", b"s", b"MAXLEN", b"1"], b":1\r\n");
        let after_trim = info(&mut stream);
        assert!(contains(&after_trim, b"5-0"));
        assert!(contains(&after_trim, b"entries-added"));
        assert!(contains(&after_trim, b"20"));

        command(
            &mut stream,
            &[b"XSETID", b"missing", b"1-0"],
            b"-ERR no such key\r\n",
        );
        command(&mut stream, &[b"SET", b"str", b"v"], b"+OK\r\n");
        command(
            &mut stream,
            &[b"XSETID", b"str", b"1-0"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );
        command(
            &mut stream,
            &[b"XSETID", b"s", b"bogus"],
            b"-ERR Invalid stream ID specified as stream command argument\r\n",
        );
        command(
            &mut stream,
            &[b"XSETID", b"s", b"11-0", b"BOGUS"],
            b"-ERR syntax error\r\n",
        );
        command(
            &mut stream,
            &[b"XSETID", b"s", b"11-0", b"ENTRIESADDED"],
            b"-ERR syntax error\r\n",
        );
        command(
            &mut stream,
            &[b"XSETID", b"s", b"11-0", b"ENTRIESADDED", b"x"],
            b"-ERR value is not an integer or out of range\r\n",
        );
        command(
            &mut stream,
            &[b"XSETID", b"s", b"11-0", b"ENTRIESADDED", b"-1"],
            b"-ERR entries_added must be positive\r\n",
        );
        command(
            &mut stream,
            &[b"XSETID", b"s", b"11-0", b"MAXDELETEDID"],
            b"-ERR syntax error\r\n",
        );
        command(
            &mut stream,
            &[b"XSETID", b"s", b"11-0", b"MAXDELETEDID", b"x"],
            b"-ERR Invalid stream ID specified as stream command argument\r\n",
        );
        command(
            &mut stream,
            &[b"XSETID", b"s"],
            b"-ERR wrong number of arguments for 'xsetid' command\r\n",
        );

        command(&mut stream, &[b"MULTI"], b"+OK\r\n");
        command(
            &mut stream,
            &[b"XSETID", b"s", b"12-0", b"ENTRIESADDED", b"21"],
            b"+QUEUED\r\n",
        );
        command(&mut stream, &[b"EXEC"], b"*1\r\n+OK\r\n");
        let queued = info(&mut stream);
        assert!(contains(&queued, b"12-0"));
        assert!(contains(&queued, b"21"));

        command(
            &mut stream,
            &[b"XADD", b"w", b"1-0", b"f", b"v"],
            b"$3\r\n1-0\r\n",
        );
        command(&mut stream, &[b"WATCH", b"w"], b"+OK\r\n");
        let mut other = connect(&server);
        command(&mut other, &[b"XSETID", b"w", b"5-0"], b"+OK\r\n");
        command(&mut stream, &[b"MULTI"], b"+OK\r\n");
        command(&mut stream, &[b"XLEN", b"w"], b"+QUEUED\r\n");
        command(&mut stream, &[b"EXEC"], b"*-1\r\n");

        server.shutdown().unwrap();
    }
}

#[test]
fn xsetid_state_survives_journal_restart() {
    let dir = std::env::temp_dir().join(format!(
        "rillcache-xsetid-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    {
        let server = start_persistent(&dir);
        let mut stream = connect(&server);
        command(
            &mut stream,
            &[b"XADD", b"s", b"5-0", b"f", b"v"],
            b"$3\r\n5-0\r\n",
        );
        command(
            &mut stream,
            &[
                b"XSETID",
                b"s",
                b"10-0",
                b"ENTRIESADDED",
                b"9",
                b"MAXDELETEDID",
                b"4-0",
            ],
            b"+OK\r\n",
        );
        server.shutdown().unwrap();
    }
    {
        let server = start_persistent(&dir);
        let mut stream = connect(&server);
        let restored = info(&mut stream);
        assert!(contains(&restored, b"last-generated-id"));
        assert!(contains(&restored, b"10-0"));
        assert!(contains(&restored, b"entries-added"));
        assert!(contains(&restored, b"9"));
        assert!(contains(&restored, b"max-deleted-entry-id"));
        assert!(contains(&restored, b"4-0"));
        command(
            &mut stream,
            &[b"XADD", b"s", b"11-0", b"f", b"v"],
            b"$4\r\n11-0\r\n",
        );
        command(
            &mut stream,
            &[b"XADD", b"s", b"6-0", b"f", b"v"],
            b"-ERR The ID specified in XADD is equal or smaller than the target stream top item\r\n",
        );
        server.shutdown().unwrap();
    }
    let _ = std::fs::remove_dir_all(&dir);
}
