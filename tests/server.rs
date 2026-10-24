#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

use rillcache::protocol::{Limits, encode_request};
use rillcache::{Config, Server, ServerHandle};

fn start(mut config: Config) -> ServerHandle {
    config.bind_addr = "127.0.0.1:0".parse().unwrap();
    if let Ok(mode) = std::env::var("RILLCACHE_TEST_THREADS") {
        config.worker_threads = match mode.as_str() {
            "0" => None,
            "1" => Some(1),
            "4" => Some(4),
            _ => panic!("test mode must be 0, 1 or 4"),
        };
    }
    Server::bind(config).unwrap().spawn().unwrap()
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

fn expect(stream: &mut TcpStream, reply: &[u8]) {
    let mut actual = vec![0; reply.len()];
    stream.read_exact(&mut actual).unwrap();
    assert_eq!(actual, reply);
}

fn send(stream: &mut TcpStream, arguments: &[&[u8]], reply: &[u8]) {
    let mut bytes = Vec::new();
    encode_request(arguments, &mut bytes);
    stream.write_all(&bytes).unwrap();
    expect(stream, reply);
}

#[test]
fn resp2_commands_are_binary_safe_and_shared_between_connections() {
    let server = start(Config::default());
    let mut first = connect(&server);
    let mut second = connect(&server);
    send(&mut first, &[b"pInG"], b"+PONG\r\n");
    send(
        &mut first,
        &[b"PING", b"a\0\xff\r\nb"],
        b"$6\r\na\0\xff\r\nb\r\n",
    );
    send(&mut first, &[b"ECHO", b""], b"$0\r\n\r\n");
    send(&mut first, &[b"GET", b"\xff\0"], b"$-1\r\n");
    send(&mut first, &[b"SET", b"\xff\0", b"\0\xff\r\n"], b"+OK\r\n");
    send(&mut second, &[b"GET", b"\xff\0"], b"$4\r\n\0\xff\r\n\r\n");
    send(
        &mut second,
        &[b"EXISTS", b"\xff\0", b"\xff\0", b"no"],
        b":2\r\n",
    );
    send(&mut second, &[b"DEL", b"\xff\0", b"\xff\0"], b":1\r\n");
    send(&mut first, &[b"GET", b"\xff\0"], b"$-1\r\n");
    server.shutdown().unwrap();
}

#[test]
fn fragmented_commands_and_large_pipelines_preserve_order() {
    let server = start(Config::default());
    let mut stream = connect(&server);
    for byte in b"*2\r\n$4\r\nECHO\r\n$3\r\na\0b\r\n" {
        stream.write_all(&[*byte]).unwrap();
    }
    expect(&mut stream, b"$3\r\na\0b\r\n");
    let mut pipeline = Vec::new();
    let mut expected = Vec::new();
    for number in 0..2000 {
        let value = number.to_string();
        encode_request(&[b"SET", b"key", value.as_bytes()], &mut pipeline);
        encode_request(&[b"GET", b"key"], &mut pipeline);
        expected.extend_from_slice(b"+OK\r\n");
        rillcache::protocol::encode_bulk(Some(value.as_bytes()), &mut expected);
    }
    stream.write_all(&pipeline).unwrap();
    expect(&mut stream, &expected);
}

#[test]
fn malformed_input_closes_only_the_offending_connection() {
    let server = start(Config::default());
    let mut broken = connect(&server);
    broken
        .write_all(b"*1\r\n$4\r\nPING\r\n*1\r\n$-1\r\n")
        .unwrap();
    let mut output = Vec::new();
    broken.read_to_end(&mut output).unwrap();
    assert_eq!(output, b"+PONG\r\n-ERR Protocol error: invalid length\r\n");
    send(&mut connect(&server), &[b"PING"], b"+PONG\r\n");
}

#[test]
fn semantic_errors_keep_the_connection_usable_and_do_not_write() {
    let server = start(Config::default());
    let mut stream = connect(&server);
    send(
        &mut stream,
        &[b"GET"],
        b"-ERR wrong number of arguments for 'get' command\r\n",
    );
    send(&mut stream, &[b"HELLO", b"3"], b"-ERR unknown command\r\n");
    send(
        &mut stream,
        &[b"SET", b"k", b"v", b"EX", b"1"],
        b"-ERR syntax error\r\n",
    );
    send(&mut stream, &[b"GET", b"k"], b"$-1\r\n");
    send(&mut stream, &[b"PING"], b"+PONG\r\n");
}

#[test]
fn quit_flushes_and_ignores_following_pipelined_commands() {
    let server = start(Config::default());
    let mut stream = connect(&server);
    let mut bytes = Vec::new();
    encode_request(&[b"QUIT"], &mut bytes);
    encode_request(&[b"SET", b"never", b"written"], &mut bytes);
    stream.write_all(&bytes).unwrap();
    let mut output = Vec::new();
    stream.read_to_end(&mut output).unwrap();
    assert_eq!(output, b"+OK\r\n");
    send(&mut connect(&server), &[b"GET", b"never"], b"$-1\r\n");
}

#[test]
fn half_close_flushes_complete_commands_and_reports_a_truncated_frame() {
    let server = start(Config::default());
    let mut stream = connect(&server);
    stream
        .write_all(b"*1\r\n$4\r\nPING\r\n*1\r\n$4\r\nPI")
        .unwrap();
    stream.shutdown(Shutdown::Write).unwrap();
    let mut output = Vec::new();
    stream.read_to_end(&mut output).unwrap();
    assert_eq!(
        output,
        b"+PONG\r\n-ERR Protocol error: unexpected end of stream\r\n"
    );
}

#[test]
fn oversized_advertised_payload_is_rejected_without_receiving_it() {
    let server = start(Config {
        limits: Limits {
            max_frame_bytes: 100,
            max_bulk_bytes: 20,
            max_arguments: 4,
        },
        ..Config::default()
    });
    let mut stream = connect(&server);
    stream.write_all(b"*2\r\n$4\r\nECHO\r\n$21\r\n").unwrap();
    let mut output = Vec::new();
    stream.read_to_end(&mut output).unwrap();
    assert_eq!(
        output,
        b"-ERR Protocol error: bulk string limit exceeded\r\n"
    );
}

#[test]
fn connection_limit_is_enforced_and_capacity_is_reused() {
    let server = start(Config {
        max_clients: 1,
        ..Config::default()
    });
    let mut first = connect(&server);
    send(&mut first, &[b"PING"], b"+PONG\r\n");
    let mut extra = connect(&server);
    let mut rejection = Vec::new();
    extra.read_to_end(&mut rejection).unwrap();
    assert_eq!(rejection, b"-ERR max number of clients reached\r\n");
    send(&mut first, &[b"QUIT"], b"+OK\r\n");
    let mut end = Vec::new();
    first.read_to_end(&mut end).unwrap();
    // The accept loop reaps finished workers before accepting another client.
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let mut next = connect(&server);
        let mut command = Vec::new();
        encode_request(&[b"PING"], &mut command);
        next.write_all(&command).unwrap();
        let mut prefix = [0; 1];
        next.read_exact(&mut prefix).unwrap();
        if prefix == *b"+" {
            expect(&mut next, b"PONG\r\n");
            break;
        }
        assert_eq!(prefix, *b"-");
        assert!(
            Instant::now() < deadline,
            "client capacity was not reclaimed"
        );
        thread::yield_now();
    }
}

#[test]
fn shutdown_interrupts_idle_and_partial_clients_without_waiting_for_timeout() {
    let server = start(Config::default());
    let mut idle = connect(&server);
    send(&mut idle, &[b"PING"], b"+PONG\r\n");
    idle.write_all(b"*2\r\n$4\r\nECHO\r\n$100\r\nx").unwrap();
    let start = Instant::now();
    server.shutdown().unwrap();
    assert!(start.elapsed() < Duration::from_secs(3));
    assert_eq!(idle.read(&mut [0]).unwrap(), 0);
}

#[test]
fn idle_clients_are_disconnected() {
    let server = start(Config {
        idle_timeout: Duration::from_millis(50),
        ..Config::default()
    });
    let mut stream = connect(&server);
    send(&mut stream, &[b"PING"], b"+PONG\r\n");
    assert_eq!(stream.read(&mut [0]).unwrap(), 0);
    send(&mut connect(&server), &[b"PING"], b"+PONG\r\n");
}

#[test]
fn exact_bulk_limit_and_empty_keys_cross_network_buffer_boundaries() {
    let server = start(Config::default());
    let mut stream = connect(&server);
    let value: Vec<u8> = (0..Limits::default().max_bulk_bytes)
        .map(|n| n as u8)
        .collect();
    send(&mut stream, &[b"SET", b"", &value], b"+OK\r\n");
    let mut expected = Vec::new();
    rillcache::protocol::encode_bulk(Some(&value), &mut expected);
    send(&mut stream, &[b"GET", b""], &expected);
    send(&mut stream, &[b"ECHO", &value], &expected);
    send(&mut stream, &[b"DEL", b""], b":1\r\n");
}

#[test]
fn aggregate_frame_limit_does_not_limit_a_valid_pipeline() {
    let server = start(Config {
        limits: Limits {
            max_frame_bytes: 64,
            max_bulk_bytes: 32,
            max_arguments: 8,
        },
        ..Config::default()
    });
    let mut stream = connect(&server);
    let mut command = Vec::new();
    encode_request(&[b"ECHO", &[b'x'; 32]], &mut command);
    let mut expected = Vec::new();
    rillcache::protocol::encode_bulk(Some(&[b'x'; 32]), &mut expected);
    stream.write_all(&command.repeat(10)).unwrap();
    expect(&mut stream, &expected.repeat(10));
    let mut oversized = Vec::new();
    encode_request(&[b"SET", &[b'k'; 20], &[b'v'; 20]], &mut oversized);
    stream.write_all(&oversized).unwrap();
    let mut output = Vec::new();
    stream.read_to_end(&mut output).unwrap();
    assert_eq!(
        output,
        b"-ERR Protocol error: request frame limit exceeded\r\n"
    );
    send(&mut connect(&server), &[b"GET", &[b'k'; 20]], b"$-1\r\n");
}

#[test]
fn oom_over_the_wire_preserves_values_and_reclaims_deleted_payload() {
    let server = start(Config {
        max_data_bytes: 5,
        ..Config::default()
    });
    let mut stream = connect(&server);
    send(&mut stream, &[b"SET", b"k", b"1234"], b"+OK\r\n");
    send(
        &mut stream,
        &[b"SET", b"k", b"12345"],
        b"-OOM configured payload limit exceeded\r\n",
    );
    send(&mut stream, &[b"GET", b"k"], b"$4\r\n1234\r\n");
    send(&mut stream, &[b"DEL", b"k"], b":1\r\n");
    send(&mut stream, &[b"SET", b"new", b"12"], b"+OK\r\n");
}

#[test]
fn dropping_the_handle_joins_and_disconnects_active_clients() {
    let server = start(Config::default());
    let mut stream = connect(&server);
    send(&mut stream, &[b"PING"], b"+PONG\r\n");
    let started = Instant::now();
    drop(server);
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(stream.read(&mut [0]).unwrap(), 0);
}

#[test]
fn conditional_set_pipeline_preserves_values_and_error_order() {
    let server = start(Config::default());
    let mut stream = connect(&server);
    let cases: &[(&[&[u8]], &[u8])] = &[
        (&[b"SET", b"\0\xff", b"missing", b"XX"], b"$-1\r\n"),
        (&[b"SET", b"\0\xff", b"", b"NX", b"nx"], b"+OK\r\n"),
        (&[b"SET", b"\0\xff", b"wrong", b"NX"], b"$-1\r\n"),
        (&[b"GET", b"\0\xff"], b"$0\r\n\r\n"),
        (
            &[b"SET", b"\0\xff", b"wrong", b"XX", b"NX"],
            b"-ERR syntax error\r\n",
        ),
        (
            &[b"SET", b"\0\xff", b"\r\n\0\xff", b"xx", b"XX"],
            b"+OK\r\n",
        ),
        (&[b"GET", b"\0\xff"], b"$4\r\n\r\n\0\xff\r\n"),
    ];
    let mut pipeline = Vec::new();
    let mut expected = Vec::new();
    for (arguments, reply) in cases {
        encode_request(arguments, &mut pipeline);
        expected.extend_from_slice(reply);
    }
    stream.write_all(&pipeline).unwrap();
    expect(&mut stream, &expected);
    send(
        &mut connect(&server),
        &[b"GET", b"\0\xff"],
        b"$4\r\n\r\n\0\xff\r\n",
    );
}

#[test]
fn concurrent_set_nx_has_exactly_one_winner() {
    const CLIENTS: usize = 16;
    let server = start(Config::default());
    let mut streams: Vec<_> = (0..CLIENTS).map(|_| connect(&server)).collect();
    for stream in &mut streams {
        send(stream, &[b"PING"], b"+PONG\r\n");
    }
    let ready = std::sync::Barrier::new(CLIENTS);
    let replies = thread::scope(|scope| {
        let workers: Vec<_> = streams
            .into_iter()
            .enumerate()
            .map(|(client, mut stream)| {
                let ready = &ready;
                scope.spawn(move || {
                    let value = format!("client:{client}");
                    let mut command = Vec::new();
                    encode_request(&[b"SET", b"race", value.as_bytes(), b"NX"], &mut command);
                    ready.wait();
                    stream.write_all(&command).unwrap();
                    let mut reply = [0; 5];
                    stream.read_exact(&mut reply).unwrap();
                    assert!(reply == *b"+OK\r\n" || reply == *b"$-1\r\n");
                    (value, reply == *b"+OK\r\n")
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>()
    });
    let winners: Vec<_> = replies.iter().filter(|(_, success)| *success).collect();
    assert_eq!(winners.len(), 1);
    let mut expected = Vec::new();
    rillcache::protocol::encode_bulk(Some(winners[0].0.as_bytes()), &mut expected);
    send(&mut connect(&server), &[b"GET", b"race"], &expected);
}

#[test]
fn conditional_set_oom_and_rejections_leave_the_connection_usable() {
    let server = start(Config {
        max_data_bytes: 5,
        ..Config::default()
    });
    let mut stream = connect(&server);
    send(&mut stream, &[b"SET", b"k", b"1234", b"NX"], b"+OK\r\n");
    send(&mut stream, &[b"SET", b"k", b"12345", b"NX"], b"$-1\r\n");
    send(
        &mut stream,
        &[b"SET", b"missing", b"12345", b"XX"],
        b"$-1\r\n",
    );
    send(
        &mut stream,
        &[b"SET", b"k", b"12345", b"XX"],
        b"-OOM configured payload limit exceeded\r\n",
    );
    send(&mut stream, &[b"GET", b"k"], b"$4\r\n1234\r\n");
    send(&mut stream, &[b"SET", b"k", b"1", b"XX"], b"+OK\r\n");
    send(&mut stream, &[b"SET", b"x", b"12", b"NX"], b"+OK\r\n");
}

#[test]
fn a_backpressured_large_reply_client_does_not_block_other_clients_or_shutdown() {
    let server = start(Config::default());
    let mut healthy = connect(&server);
    let value = vec![b'x'; 1024 * 1024];
    send(&mut healthy, &[b"SET", b"large", &value], b"+OK\r\n");
    let mut slow = connect(&server);
    let mut command = Vec::new();
    encode_request(&[b"GET", b"large"], &mut command);
    slow.write_all(&command.repeat(1000)).unwrap();
    // Never drain the slow client's potentially 1 GiB of requested replies.
    // This exercises bounded application buffering and nonblocking backpressure.
    for _ in 0..32 {
        send(&mut healthy, &[b"PING"], b"+PONG\r\n");
    }
    server.shutdown().unwrap();
}

#[test]
fn idle_clients_do_not_exhaust_fixed_worker_lanes() {
    let server = start(Config::default());
    let mut idle: Vec<_> = (0..24).map(|_| connect(&server)).collect();
    for stream in &mut idle {
        send(stream, &[b"PING"], b"+PONG\r\n");
    }
    let mut busy = connect(&server);
    send(&mut busy, &[b"SET", b"shared", b"value"], b"+OK\r\n");
    for stream in &mut idle {
        send(stream, &[b"GET", b"shared"], b"$5\r\nvalue\r\n");
    }
    server.shutdown().unwrap();
}

#[test]
fn concurrent_clients_do_not_lose_or_cross_contaminate_values() {
    let server = start(Config::default());
    thread::scope(|scope| {
        for client in 0..12 {
            let server = &server;
            scope.spawn(move || {
                let mut stream = connect(server);
                let key = format!("client:{client}");
                for iteration in 0..100 {
                    let value = format!("{client}:{iteration}");
                    send(
                        &mut stream,
                        &[b"SET", key.as_bytes(), value.as_bytes()],
                        b"+OK\r\n",
                    );
                    let mut expected = Vec::new();
                    rillcache::protocol::encode_bulk(Some(value.as_bytes()), &mut expected);
                    send(&mut stream, &[b"GET", key.as_bytes()], &expected);
                }
            });
        }
    });
}
