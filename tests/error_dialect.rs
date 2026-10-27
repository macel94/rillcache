#![forbid(unsafe_code)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use rillcache::engine::{Clock, ErrorDialect, Store};
use rillcache::protocol::encode_request;
use rillcache::{Config, Server};

fn short_error(dialect: ErrorDialect) -> &'static [u8] {
    match dialect {
        ErrorDialect::Redis => b"-ERR wrong number of key-value pairs\r\n",
        ErrorDialect::Valkey => b"-ERR syntax error\r\n",
    }
}

fn run(store: &Store, arguments: &[&[u8]]) -> Vec<u8> {
    let mut output = Vec::new();
    store.execute(arguments, &mut output);
    output
}

#[test]
fn library_dialects_only_change_short_pair_wording_and_preserve_live_state() {
    struct FixedClock;
    impl Clock for FixedClock {
        fn now_ms(&self) -> i64 {
            100
        }
    }
    assert_eq!(Config::default().error_dialect, ErrorDialect::Valkey);
    assert_eq!(
        run(&Store::new(32), &[b"MSETEX", b"2", b"k", b"v"]),
        short_error(ErrorDialect::Valkey)
    );
    for dialect in [ErrorDialect::Redis, ErrorDialect::Valkey] {
        let store = Store::with_clock(32, Arc::new(FixedClock)).with_error_dialect(dialect);
        let key = b"key\0\xff";
        assert_eq!(
            run(&store, &[b"SET", key, b"old", b"PX", b"500"]),
            b"+OK\r\n"
        );
        let watched = store.watch_snapshot(&[key]).unwrap();
        for count in [b"2".as_slice(), b"1073741823", b"2147483646", b"2147483647"] {
            assert_eq!(
                run(&store, &[b"mSeTeX", count, key, b"new"]),
                short_error(dialect)
            );
        }
        for count in [
            b"0".as_slice(),
            b"-1",
            b"+1",
            b"01",
            b"1.5",
            b"2147483648",
            b"9223372036854775808",
            b"1\0",
        ] {
            assert_eq!(
                run(&store, &[b"MSETEX", count, key, b"new"]),
                b"-ERR invalid numkeys value or out of range\r\n"
            );
        }
        assert_eq!(
            run(&store, &[b"MSETEX", b"2", key]),
            b"-ERR wrong number of arguments for 'msetex' command\r\n"
        );
        assert_eq!(
            run(&store, &[b"MSETEX", b"1", key, b"new", b"invalid"]),
            b"-ERR syntax error\r\n"
        );
        assert_eq!(
            run(&store, &[b"MSETEX", b"1", key, b"new", b"NX", b"XX"]),
            b"-ERR syntax error\r\n"
        );
        assert_eq!(run(&store, &[b"GET", key]), b"$3\r\nold\r\n");
        assert_eq!(run(&store, &[b"PTTL", key]), b":500\r\n");
        let mut transaction = Vec::new();
        store.execute_transaction(
            &[vec![b"GET".to_vec(), key.to_vec()]],
            &watched,
            &mut transaction,
        );
        assert_eq!(
            transaction, b"*1\r\n$3\r\nold\r\n",
            "rejected writes must not dirty WATCH"
        );
        assert_eq!(
            run(&store, &[b"MSETEX", b"1", key, b"new", b"XX", b"KEEPTTL"]),
            b":1\r\n"
        );
        assert_eq!(run(&store, &[b"GET", key]), b"$3\r\nnew\r\n");
        assert_eq!(run(&store, &[b"PTTL", key]), b":500\r\n");
    }
}

fn read_frame(reader: &mut impl BufRead) -> Vec<u8> {
    let mut output = Vec::new();
    reader.read_until(b'\n', &mut output).unwrap();
    assert!(output.ends_with(b"\r\n"));
    let count = || {
        std::str::from_utf8(&output[1..output.len() - 2])
            .unwrap()
            .parse::<i64>()
            .unwrap()
    };
    match output[0] {
        b'$' => {
            let length = count();
            assert!((-1..=2 * 1024 * 1024).contains(&length));
            if length >= 0 {
                let mut body = vec![0; length as usize + 2];
                reader.read_exact(&mut body).unwrap();
                assert!(body.ends_with(b"\r\n"));
                output.extend_from_slice(&body);
            }
        }
        b'*' | b'%' => {
            let elements = count();
            assert!((-1..=1024).contains(&elements));
            for _ in 0..elements.max(0) * if output[0] == b'%' { 2 } else { 1 } {
                output.extend_from_slice(&read_frame(reader));
            }
        }
        b'+' | b'-' | b':' | b'_' => {}
        prefix => panic!("unexpected reply prefix {prefix}"),
    }
    output
}

fn call(client: &mut BufReader<TcpStream>, arguments: &[&[u8]]) -> Vec<u8> {
    let mut wire = Vec::new();
    encode_request(arguments, &mut wire);
    client.get_mut().write_all(&wire).unwrap();
    read_frame(client)
}

#[test]
fn configured_servers_preserve_dialect_in_fragmented_pipelines_resp3_and_exec() {
    let workers = match std::env::var("RILLCACHE_TEST_THREADS").as_deref() {
        Ok("0") | Err(_) => None,
        Ok("1") => Some(1),
        Ok("4") => Some(4),
        Ok(other) => panic!("invalid worker mode {other}"),
    };
    for dialect in [ErrorDialect::Redis, ErrorDialect::Valkey] {
        let server = Server::bind(Config {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            worker_threads: workers,
            error_dialect: dialect,
            ..Config::default()
        })
        .unwrap()
        .spawn()
        .unwrap();
        let stream = TcpStream::connect(server.local_addr()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream.set_nodelay(true).unwrap();
        let mut client = BufReader::new(stream);
        assert_eq!(
            call(
                &mut client,
                &[b"SET", b"k", b"old", b"PXAT", b"4000000000000"]
            ),
            b"+OK\r\n"
        );
        let mut wire = Vec::new();
        encode_request(&[b"MSETEX", b"2", b"k", b"new"], &mut wire);
        encode_request(&[b"GET", b"k"], &mut wire);
        encode_request(&[b"PEXPIRETIME", b"k"], &mut wire);
        for fragment in wire.chunks(7) {
            client.get_mut().write_all(fragment).unwrap();
        }
        assert_eq!(read_frame(&mut client), short_error(dialect));
        assert_eq!(read_frame(&mut client), b"$3\r\nold\r\n");
        assert_eq!(read_frame(&mut client), b":4000000000000\r\n");
        assert_eq!(call(&mut client, &[b"MULTI"]), b"+OK\r\n");
        assert_eq!(
            call(&mut client, &[b"MSETEX", b"2147483647", b"k", b"new"]),
            b"+QUEUED\r\n"
        );
        assert_eq!(call(&mut client, &[b"GET", b"k"]), b"+QUEUED\r\n");
        let mut expected = b"*2\r\n".to_vec();
        expected.extend_from_slice(short_error(dialect));
        expected.extend_from_slice(b"$3\r\nold\r\n");
        assert_eq!(call(&mut client, &[b"EXEC"]), expected);
        assert!(call(&mut client, &[b"HELLO", b"3"]).starts_with(b"%"));
        assert_eq!(
            call(&mut client, &[b"MSETEX", b"2", b"k", b"new"]),
            short_error(dialect)
        );
        assert_eq!(
            call(&mut client, &[b"MSETEX", b"1", b"k", b"new"]),
            b":1\r\n"
        );
        assert_eq!(call(&mut client, &[b"GET", b"k"]), b"$3\r\nnew\r\n");
        assert_eq!(call(&mut client, &[b"QUIT"]), b"+OK\r\n");
        assert_eq!(client.read(&mut [0]).unwrap(), 0);
        server.shutdown().unwrap();
    }
}
