#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

#[derive(Debug, PartialEq, Eq)]
enum Value {
    Simple(Vec<u8>),
    Error(Vec<u8>),
    Int(i64),
    Bulk(Option<Vec<u8>>),
    Array(Option<Vec<Value>>),
}

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

fn read_line(stream: &mut TcpStream) -> Vec<u8> {
    let mut line = Vec::new();
    loop {
        let mut byte = [0];
        stream.read_exact(&mut byte).unwrap();
        if byte[0] == b'\n' {
            assert_eq!(line.pop(), Some(b'\r'));
            return line;
        }
        line.push(byte[0]);
    }
}

fn read_value(stream: &mut TcpStream, depth: usize) -> Value {
    assert!(depth <= 8);
    let mut prefix = [0];
    stream.read_exact(&mut prefix).unwrap();
    match prefix[0] {
        b'+' => Value::Simple(read_line(stream)),
        b'-' => Value::Error(read_line(stream)),
        b':' => Value::Int(
            std::str::from_utf8(&read_line(stream))
                .unwrap()
                .parse()
                .unwrap(),
        ),
        b'$' => {
            let length: i64 = std::str::from_utf8(&read_line(stream))
                .unwrap()
                .parse()
                .unwrap();
            if length < 0 {
                Value::Bulk(None)
            } else {
                let mut body = vec![0; length as usize + 2];
                stream.read_exact(&mut body).unwrap();
                body.truncate(length as usize);
                Value::Bulk(Some(body))
            }
        }
        b'*' => {
            let count: i64 = std::str::from_utf8(&read_line(stream))
                .unwrap()
                .parse()
                .unwrap();
            if count < 0 {
                Value::Array(None)
            } else {
                Value::Array(Some(
                    (0..count).map(|_| read_value(stream, depth + 1)).collect(),
                ))
            }
        }
        other => panic!("unexpected RESP prefix {other:?}"),
    }
}

fn call(stream: &mut TcpStream, arguments: &[&[u8]]) -> Value {
    let mut request = Vec::new();
    encode_request(arguments, &mut request);
    stream.write_all(&request).unwrap();
    read_value(stream, 0)
}

fn ok(stream: &mut TcpStream, arguments: &[&[u8]]) {
    assert_eq!(call(stream, arguments), Value::Simple(b"OK".to_vec()));
}

fn integer(stream: &mut TcpStream, arguments: &[&[u8]]) -> i64 {
    match call(stream, arguments) {
        Value::Int(value) => value,
        other => panic!("expected integer for {arguments:?}, got {other:?}"),
    }
}

fn entries(stream: &mut TcpStream, count: &[u8]) -> Vec<Vec<Value>> {
    match call(stream, &[b"SLOWLOG", b"GET", count]) {
        Value::Array(Some(entries)) => entries
            .into_iter()
            .map(|entry| match entry {
                Value::Array(Some(fields)) => fields,
                other => panic!("expected entry array, got {other:?}"),
            })
            .collect(),
        other => panic!("expected entry array list, got {other:?}"),
    }
}

fn entry_argv(fields: &[Value]) -> Vec<Vec<u8>> {
    match fields.get(3) {
        Some(Value::Array(Some(arguments))) => arguments
            .iter()
            .map(|argument| match argument {
                Value::Bulk(Some(bytes)) => bytes.clone(),
                other => panic!("expected bulk argument, got {other:?}"),
            })
            .collect(),
        other => panic!("expected argv array, got {other:?}"),
    }
}

fn entry_duration(fields: &[Value]) -> i64 {
    match fields.get(2) {
        Some(Value::Int(value)) => *value,
        other => panic!("expected duration integer, got {other:?}"),
    }
}

fn entry_name(fields: &[Value]) -> Vec<u8> {
    match fields.get(5) {
        Some(Value::Bulk(Some(bytes))) => bytes.clone(),
        other => panic!("expected client name bulk, got {other:?}"),
    }
}

fn bulk_values(value: &Value, output: &mut Vec<Vec<u8>>) {
    match value {
        Value::Bulk(Some(bytes)) => output.push(bytes.clone()),
        Value::Array(Some(values)) => {
            for value in values {
                bulk_values(value, output);
            }
        }
        _ => {}
    }
}

#[test]
fn slowlog_records_orders_truncates_and_bounds_entries_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = connect(&server);

        assert_eq!(
            call(
                &mut stream,
                &[b"CONFIG", b"GET", b"slowlog-log-slower-than"]
            ),
            Value::Array(Some(vec![
                Value::Bulk(Some(b"slowlog-log-slower-than".to_vec())),
                Value::Bulk(Some(b"10000".to_vec())),
            ]))
        );
        assert_eq!(
            call(&mut stream, &[b"CONFIG", b"GET", b"slowlog-max-len"]),
            Value::Array(Some(vec![
                Value::Bulk(Some(b"slowlog-max-len".to_vec())),
                Value::Bulk(Some(b"128".to_vec())),
            ]))
        );

        // Disable, reset, then enable so the expected entry count is exact.
        ok(
            &mut stream,
            &[b"CONFIG", b"SET", b"slowlog-log-slower-than", b"-1"],
        );
        ok(&mut stream, &[b"SLOWLOG", b"RESET"]);
        ok(
            &mut stream,
            &[b"CONFIG", b"SET", b"slowlog-log-slower-than", b"0"],
        );
        assert_eq!(
            call(&mut stream, &[b"PING"]),
            Value::Simple(b"PONG".to_vec())
        );
        ok(&mut stream, &[b"SET", b"k", b"v"]);
        assert_eq!(
            call(&mut stream, &[b"GET", b"k"]),
            Value::Bulk(Some(b"v".to_vec()))
        );
        let newest = entries(&mut stream, b"2");
        assert_eq!(newest.len(), 2);
        assert_eq!(entry_argv(&newest[0]), vec![b"GET".to_vec(), b"k".to_vec()]);
        assert_eq!(
            entry_argv(&newest[1]),
            vec![b"SET".to_vec(), b"k".to_vec(), b"v".to_vec()]
        );
        assert!(entry_duration(&newest[0]) >= 0);
        assert_eq!(entry_name(&newest[0]), Vec::<u8>::new());
        match newest[0].get(4) {
            Some(Value::Bulk(Some(addr))) => assert!(!addr.is_empty()),
            other => panic!("expected client address, got {other:?}"),
        }
        // The preceding SLOWLOG GET was itself recorded.
        assert_eq!(integer(&mut stream, &[b"SLOWLOG", b"LEN"]), 4);
        assert_eq!(entries(&mut stream, b"0").len(), 0);

        assert!(matches!(
            call(&mut stream, &[b"SLOWLOG", b"GET", b"-2"]),
            Value::Error(ref error) if error == b"ERR count should be greater than or equal to -1"
        ));
        assert!(matches!(
            call(&mut stream, &[b"SLOWLOG", b"GET", b"bad"]),
            Value::Error(ref error) if error == b"ERR value is not an integer or out of range"
        ));
        assert!(matches!(
            call(&mut stream, &[b"SLOWLOG", b"BOGUS"]),
            Value::Error(ref error) if error.starts_with(b"ERR unknown subcommand")
        ));
        assert!(matches!(
            call(&mut stream, &[b"SLOWLOG"]),
            Value::Error(ref error) if error == b"ERR wrong number of arguments for 'slowlog' command"
        ));
        assert!(matches!(
            call(&mut stream, &[b"SLOWLOG", b"HELP"]),
            Value::Array(Some(ref lines)) if !lines.is_empty()
        ));

        ok(&mut stream, &[b"CLIENT", b"SETNAME", b"tester"]);
        let named = entries(&mut stream, b"1");
        assert_eq!(entry_name(&named[0]), b"tester");
        assert_eq!(entry_argv(&named[0])[0], b"CLIENT");

        // Long arguments and argument counts are truncated like Redis.
        let long = vec![b'x'; 200];
        ok(&mut stream, &[b"SET", b"long", &long]);
        let long_entry = entries(&mut stream, b"1");
        let arguments = entry_argv(&long_entry[0]);
        assert_eq!(arguments.len(), 3);
        assert_eq!(arguments[2].len(), 128 + b"... (72 more bytes)".len());
        assert!(arguments[2].ends_with(b"... (72 more bytes)"));

        let mut many: Vec<&[u8]> = vec![b"HSET", b"h"];
        let fields: Vec<String> = (0..40).map(|index| format!("f{index}")).collect();
        let values: Vec<String> = (0..40).map(|index| format!("v{index}")).collect();
        for (field, value) in fields.iter().zip(&values) {
            many.push(field.as_bytes());
            many.push(value.as_bytes());
        }
        assert_eq!(integer(&mut stream, &many), 40);
        let many_entry = entries(&mut stream, b"1");
        let arguments = entry_argv(&many_entry[0]);
        assert_eq!(arguments.len(), 32);
        assert_eq!(arguments[31], b"... (51 more arguments)");

        ok(&mut stream, &[b"CONFIG", b"SET", b"slowlog-max-len", b"1"]);
        assert_eq!(integer(&mut stream, &[b"SLOWLOG", b"LEN"]), 1);
        ok(&mut stream, &[b"CONFIG", b"SET", b"slowlog-max-len", b"0"]);
        assert_eq!(integer(&mut stream, &[b"SLOWLOG", b"LEN"]), 0);
        assert_eq!(
            call(&mut stream, &[b"PING"]),
            Value::Simple(b"PONG".to_vec())
        );
        assert_eq!(integer(&mut stream, &[b"SLOWLOG", b"LEN"]), 0);

        server.shutdown().unwrap();
    }
}

#[test]
fn slowlog_validates_config_logs_blocking_time_and_skips_queued_commands() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = connect(&server);

        assert!(matches!(
            call(
                &mut stream,
                &[b"CONFIG", b"SET", b"slowlog-log-slower-than", b"-2"]
            ),
            Value::Error(ref error) if error == b"ERR invalid slowlog-log-slower-than"
        ));
        assert!(matches!(
            call(&mut stream, &[b"CONFIG", b"SET", b"slowlog-max-len", b"-1"]),
            Value::Error(ref error) if error.starts_with(b"ERR slowlog-max-len")
        ));
        assert!(matches!(
            call(
                &mut stream,
                &[b"CONFIG", b"SET", b"slowlog-max-len", b"1000001"]
            ),
            Value::Error(ref error) if error.starts_with(b"ERR slowlog-max-len")
        ));
        assert_eq!(
            call(
                &mut stream,
                &[
                    b"CONFIG",
                    b"GET",
                    b"slowlog-log-slower-than",
                    b"slowlog-max-len"
                ]
            ),
            Value::Array(Some(vec![
                Value::Bulk(Some(b"slowlog-log-slower-than".to_vec())),
                Value::Bulk(Some(b"10000".to_vec())),
                Value::Bulk(Some(b"slowlog-max-len".to_vec())),
                Value::Bulk(Some(b"128".to_vec())),
            ]))
        );

        // A fast command below the default threshold is not recorded.
        ok(&mut stream, &[b"SLOWLOG", b"RESET"]);
        assert_eq!(
            call(&mut stream, &[b"PING"]),
            Value::Simple(b"PONG".to_vec())
        );
        assert_eq!(integer(&mut stream, &[b"SLOWLOG", b"LEN"]), 0);

        // A timed-out blocking pop records the total wait duration.
        assert_eq!(
            call(&mut stream, &[b"BLPOP", b"missing", b"0.05"]),
            Value::Array(None)
        );
        assert_eq!(integer(&mut stream, &[b"SLOWLOG", b"LEN"]), 1);
        let blocked = entries(&mut stream, b"1");
        assert_eq!(
            entry_argv(&blocked[0]),
            vec![b"BLPOP".to_vec(), b"missing".to_vec(), b"0.05".to_vec()]
        );
        assert!(entry_duration(&blocked[0]) >= 30_000);

        // EXEC is logged once; commands queued by MULTI are not logged separately.
        ok(
            &mut stream,
            &[b"CONFIG", b"SET", b"slowlog-log-slower-than", b"0"],
        );
        ok(&mut stream, &[b"SLOWLOG", b"RESET"]);
        ok(&mut stream, &[b"MULTI"]);
        assert_eq!(
            call(&mut stream, &[b"SET", b"queued", b"v"]),
            Value::Simple(b"QUEUED".to_vec())
        );
        match call(&mut stream, &[b"EXEC"]) {
            Value::Array(Some(replies)) => assert_eq!(replies.len(), 1),
            other => panic!("expected EXEC array, got {other:?}"),
        }
        let all = entries(&mut stream, b"-1");
        assert_eq!(all.len(), 3);
        assert!(
            all.iter().all(|entry| entry_argv(entry)[0] != b"SET"),
            "queued SET must not be logged separately: {all:?}"
        );
        assert!(all.iter().any(|entry| entry_argv(entry)[0] == b"EXEC"));
        // The preceding SLOWLOG GET was itself recorded.
        assert_eq!(integer(&mut stream, &[b"SLOWLOG", b"LEN"]), 4);

        server.shutdown().unwrap();
    }
}

#[test]
fn slowlog_redacts_credentials() {
    let server = start(None);
    let mut stream = connect(&server);
    ok(
        &mut stream,
        &[b"CONFIG", b"SET", b"slowlog-log-slower-than", b"0"],
    );
    ok(&mut stream, &[b"CONFIG", b"SET", b"requirepass", b"secret"]);
    ok(&mut stream, &[b"AUTH", b"secret"]);
    assert!(matches!(
        call(
            &mut stream,
            &[b"HELLO", b"2", b"AUTH", b"default", b"secret"]
        ),
        Value::Array(Some(_))
    ));

    let all = call(&mut stream, &[b"SLOWLOG", b"GET", b"-1"]);
    let mut bulks = Vec::new();
    bulk_values(&all, &mut bulks);
    assert!(
        !bulks
            .iter()
            .any(|value| value.windows(6).any(|part| part == b"secret")),
        "credentials leaked into slowlog: {bulks:?}"
    );
    assert!(
        bulks.iter().any(|value| value == b"(redacted)"),
        "expected redaction markers: {bulks:?}"
    );
    server.shutdown().unwrap();
}
