#![forbid(unsafe_code)]

//! Seeded command-state model checks for the binary-safe string/keyspace subset.
//! Failures report a fixed seed and operation index so every sequence reproduces.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

const SEED: u64 = 0x6d6f_6465_6c5f_7631;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut value = self.0;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.0 = value;
        value
    }

    fn below(&mut self, limit: usize) -> usize {
        (self.next() % limit as u64) as usize
    }

    fn bytes(&mut self, length: usize) -> Vec<u8> {
        (0..length).map(|_| self.next() as u8).collect()
    }
}

fn key(index: usize) -> Vec<u8> {
    vec![b'k', 0, index as u8, 0xff]
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
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream.set_nodelay(true).unwrap();
    stream
}

fn line(stream: &mut TcpStream, frame: &mut Vec<u8>) -> Vec<u8> {
    let mut value = Vec::new();
    loop {
        let mut byte = [0];
        stream.read_exact(&mut byte).unwrap();
        value.push(byte[0]);
        frame.push(byte[0]);
        if value.ends_with(b"\r\n") {
            value.truncate(value.len() - 2);
            return value;
        }
    }
}

fn read_frame(stream: &mut TcpStream) -> Vec<u8> {
    let mut frame = Vec::new();
    let mut prefix = [0];
    stream.read_exact(&mut prefix).unwrap();
    frame.push(prefix[0]);
    match prefix[0] {
        b'+' | b'-' | b':' => {
            line(stream, &mut frame);
        }
        b'$' => {
            let length = std::str::from_utf8(&line(stream, &mut frame))
                .unwrap()
                .parse::<i64>()
                .unwrap();
            if length >= 0 {
                let mut body = vec![0; length as usize + 2];
                stream.read_exact(&mut body).unwrap();
                assert!(body.ends_with(b"\r\n"));
                frame.extend_from_slice(&body);
            }
        }
        b'*' => {
            let count = std::str::from_utf8(&line(stream, &mut frame))
                .unwrap()
                .parse::<i64>()
                .unwrap();
            for _ in 0..count.max(0) {
                frame.extend_from_slice(&read_frame(stream));
            }
        }
        other => panic!("unexpected RESP prefix {other:?}"),
    }
    frame
}

fn bulk(value: Option<&[u8]>) -> Vec<u8> {
    let Some(value) = value else {
        return b"$-1\r\n".to_vec();
    };
    let mut frame = format!("${}\r\n", value.len()).into_bytes();
    frame.extend_from_slice(value);
    frame.extend_from_slice(b"\r\n");
    frame
}

fn integer(value: usize) -> Vec<u8> {
    format!(":{value}\r\n").into_bytes()
}

fn array(values: impl IntoIterator<Item = Option<Vec<u8>>>) -> Vec<u8> {
    let values: Vec<_> = values.into_iter().collect();
    let mut frame = format!("*{}\r\n", values.len()).into_bytes();
    for value in values {
        frame.extend_from_slice(&bulk(value.as_deref()));
    }
    frame
}

fn check_command(
    stream: &mut TcpStream,
    arguments: &[Vec<u8>],
    expected: &[u8],
    seed: u64,
    mode: Option<usize>,
    step: usize,
) {
    let slices: Vec<&[u8]> = arguments.iter().map(Vec::as_slice).collect();
    let mut request = Vec::new();
    encode_request(&slices, &mut request);
    stream.write_all(&request).unwrap();
    let actual = read_frame(stream);
    assert_eq!(
        actual, expected,
        "seed={seed:#x} mode={mode:?} step={step} command={arguments:?}"
    );
}

#[test]
fn seeded_string_keyspace_sequences_match_a_reference_model_in_every_mode() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = connect(&server);
        let mut rng = Rng(SEED);
        let mut model = BTreeMap::<Vec<u8>, Vec<u8>>::new();

        for step in 0..600 {
            let selected_key = key(rng.below(32));
            let (arguments, expected) = match rng.below(6) {
                0 => {
                    let value_length = rng.below(48);
                    let value = rng.bytes(value_length);
                    model.insert(selected_key.clone(), value.clone());
                    (
                        vec![b"SET".to_vec(), selected_key, value],
                        b"+OK\r\n".to_vec(),
                    )
                }
                1 => {
                    let expected = bulk(model.get(&selected_key).map(Vec::as_slice));
                    (vec![b"GET".to_vec(), selected_key], expected)
                }
                2 => {
                    let suffix_length = rng.below(24);
                    let suffix = rng.bytes(suffix_length);
                    let value = model.entry(selected_key.clone()).or_default();
                    value.extend_from_slice(&suffix);
                    (
                        vec![b"APPEND".to_vec(), selected_key, suffix],
                        integer(value.len()),
                    )
                }
                3 => {
                    let removed = model.remove(&selected_key).is_some();
                    (
                        vec![b"DEL".to_vec(), selected_key],
                        integer(usize::from(removed)),
                    )
                }
                4 => {
                    let length = model.get(&selected_key).map_or(0, Vec::len);
                    (vec![b"STRLEN".to_vec(), selected_key], integer(length))
                }
                _ => {
                    let count = 1 + rng.below(5);
                    let keys: Vec<_> = (0..count).map(|_| key(rng.below(32))).collect();
                    let expected =
                        array(keys.iter().map(|candidate| model.get(candidate).cloned()));
                    let mut arguments = vec![b"MGET".to_vec()];
                    arguments.extend(keys);
                    (arguments, expected)
                }
            };
            check_command(&mut stream, &arguments, &expected, SEED, workers, step);
        }

        for index in 0..32 {
            let selected_key = key(index);
            let expected = bulk(model.get(&selected_key).map(Vec::as_slice));
            check_command(
                &mut stream,
                &[b"GET".to_vec(), selected_key],
                &expected,
                SEED,
                workers,
                600 + index,
            );
        }
        server.shutdown().unwrap();
    }
}
