#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

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
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream.set_nodelay(true).unwrap();
    stream
}

#[derive(Debug, PartialEq, Eq)]
enum Value {
    Simple(Vec<u8>),
    Error(Vec<u8>),
    Int(i64),
    Bulk(Option<Vec<u8>>),
    Array(Option<Vec<Value>>),
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

fn send(stream: &mut TcpStream, arguments: &[&[u8]]) {
    let mut request = Vec::new();
    encode_request(arguments, &mut request);
    stream.write_all(&request).unwrap();
}

fn call(stream: &mut TcpStream, arguments: &[&[u8]]) -> Value {
    send(stream, arguments);
    read_value(stream, 0)
}

fn command(stream: &mut TcpStream, arguments: &[&[u8]], expected: &[u8]) {
    send(stream, arguments);
    let mut actual = vec![0; expected.len()];
    stream.read_exact(&mut actual).unwrap();
    assert_eq!(actual, expected, "{arguments:?}");
}

fn integer(stream: &mut TcpStream, arguments: &[&[u8]]) -> i64 {
    match call(stream, arguments) {
        Value::Int(value) => value,
        other => panic!("expected integer for {arguments:?}, got {other:?}"),
    }
}

#[test]
fn latency_configuration_shapes_and_errors_work_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = connect(&server);

        assert_eq!(
            call(
                &mut stream,
                &[b"CONFIG", b"GET", b"latency-monitor-threshold"]
            ),
            Value::Array(Some(vec![
                Value::Bulk(Some(b"latency-monitor-threshold".to_vec())),
                Value::Bulk(Some(b"0".to_vec())),
            ]))
        );
        assert_eq!(
            call(&mut stream, &[b"LATENCY", b"LATEST"]),
            Value::Array(Some(Vec::new()))
        );
        assert_eq!(
            call(&mut stream, &[b"LATENCY", b"HISTORY", b"command"]),
            Value::Array(Some(Vec::new()))
        );
        assert_eq!(integer(&mut stream, &[b"LATENCY", b"RESET"]), 0);
        match call(&mut stream, &[b"LATENCY", b"DOCTOR"]) {
            Value::Bulk(Some(report)) => {
                assert!(report.windows(8).any(|part| part == b"disabled"));
            }
            other => panic!("expected doctor bulk, got {other:?}"),
        }
        assert!(matches!(
            call(&mut stream, &[b"LATENCY", b"HELP"]),
            Value::Array(Some(ref lines)) if !lines.is_empty()
        ));
        assert_eq!(
            call(&mut stream, &[b"LATENCY", b"GRAPH", b"command"]),
            Value::Error(b"ERR LATENCY GRAPH is unsupported; use LATENCY HISTORY".to_vec())
        );
        assert_eq!(
            call(&mut stream, &[b"LATENCY", b"HISTOGRAM"]),
            Value::Error(b"ERR LATENCY HISTOGRAM is unsupported; use LATENCY HISTORY".to_vec())
        );
        assert!(matches!(
            call(&mut stream, &[b"LATENCY", b"BOGUS"]),
            Value::Error(ref error) if error.starts_with(b"ERR unknown subcommand")
        ));
        assert!(matches!(
            call(&mut stream, &[b"LATENCY"]),
            Value::Error(ref error) if error == b"ERR wrong number of arguments for 'latency' command"
        ));
        assert!(matches!(
            call(
                &mut stream,
                &[b"CONFIG", b"SET", b"latency-monitor-threshold", b"-1"]
            ),
            Value::Error(ref error) if error == b"ERR invalid latency-monitor-threshold"
        ));
        assert!(matches!(
            call(
                &mut stream,
                &[b"CONFIG", b"SET", b"latency-monitor-threshold", b"abc"]
            ),
            Value::Error(ref error) if error == b"ERR invalid latency-monitor-threshold"
        ));

        server.shutdown().unwrap();
    }
}

#[test]
fn latency_records_orders_and_resets_measured_commands_in_all_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = connect(&server);
        command(
            &mut stream,
            &[b"CONFIG", b"SET", b"latency-monitor-threshold", b"1"],
            b"+OK\r\n",
        );

        // Build a large list in bounded batches, then measure an in-place SORT
        // (one command with few arguments keeps clear of the request limits).
        for batch in 0..100 {
            let mut push: Vec<&[u8]> = vec![b"RPUSH", b"sortable"];
            let values: Vec<Vec<u8>> = (0..1_000)
                .map(|index| (batch * 1_000 + index).to_string().into_bytes())
                .collect();
            push.extend(values.iter().map(Vec::as_slice));
            send(&mut stream, &push);
            match read_value(&mut stream, 0) {
                Value::Int(length) => assert_eq!(length, (batch + 1) * 1_000),
                other => panic!("unexpected RPUSH reply: {other:?}"),
            }
        }
        command(
            &mut stream,
            &[b"SORT", b"sortable", b"STORE", b"sorted"],
            b":100000\r\n",
        );

        let latest = match call(&mut stream, &[b"LATENCY", b"LATEST"]) {
            Value::Array(Some(events)) => events,
            other => panic!("expected latest array, got {other:?}"),
        };
        let command_event = latest
            .iter()
            .find_map(|event| match event {
                Value::Array(Some(fields))
                    if matches!(fields.first(), Some(Value::Bulk(Some(name))) if name == b"command") =>
                {
                    Some(fields)
                }
                _ => None,
            })
            .expect("slow command event was not recorded");
        assert_eq!(command_event.len(), 4);
        let latest_ms = match &command_event[2] {
            Value::Int(value) => *value,
            other => panic!("expected latest latency, got {other:?}"),
        };
        let max_ms = match &command_event[3] {
            Value::Int(value) => *value,
            other => panic!("expected max latency, got {other:?}"),
        };
        assert!(latest_ms >= 1, "latest latency {latest_ms}");
        assert!(max_ms >= latest_ms);

        let history = match call(&mut stream, &[b"LATENCY", b"HISTORY", b"COMMAND"]) {
            Value::Array(Some(samples)) => samples,
            other => panic!("expected history array, got {other:?}"),
        };
        assert!(!history.is_empty());
        assert!(history.len() <= 160);
        let mut previous_timestamp = 0;
        for sample in &history {
            match sample {
                Value::Array(Some(pair)) => {
                    assert_eq!(pair.len(), 2);
                    let timestamp = match &pair[0] {
                        Value::Int(value) => *value,
                        other => panic!("expected timestamp, got {other:?}"),
                    };
                    let latency = match &pair[1] {
                        Value::Int(value) => *value,
                        other => panic!("expected latency, got {other:?}"),
                    };
                    assert!(timestamp >= previous_timestamp);
                    assert!(latency >= 1);
                    previous_timestamp = timestamp;
                }
                other => panic!("expected sample pair, got {other:?}"),
            }
        }

        match call(&mut stream, &[b"LATENCY", b"DOCTOR"]) {
            Value::Bulk(Some(report)) => {
                assert!(report.windows(7).any(|part| part == b"command"));
                assert!(report.windows(4).any(|part| part == b"1 ms"));
            }
            other => panic!("expected doctor report, got {other:?}"),
        }

        assert_eq!(integer(&mut stream, &[b"LATENCY", b"RESET", b"command"]), 1);
        assert_eq!(
            call(&mut stream, &[b"LATENCY", b"HISTORY", b"command"]),
            Value::Array(Some(Vec::new()))
        );
        assert_eq!(integer(&mut stream, &[b"LATENCY", b"RESET", b"unknown"]), 0);
        assert_eq!(
            call(&mut stream, &[b"LATENCY", b"LATEST"]),
            Value::Array(Some(Vec::new()))
        );

        // Raising the threshold above the measured durations stops new samples.
        command(
            &mut stream,
            &[b"CONFIG", b"SET", b"latency-monitor-threshold", b"100000"],
            b"+OK\r\n",
        );
        command(
            &mut stream,
            &[b"SORT", b"sortable", b"STORE", b"sorted"],
            b":100000\r\n",
        );
        assert_eq!(
            call(&mut stream, &[b"LATENCY", b"LATEST"]),
            Value::Array(Some(Vec::new()))
        );

        server.shutdown().unwrap();
    }
}
