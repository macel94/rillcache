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
        match mode.as_str() {
            "0" => config.worker_threads = None,
            "1" => config.worker_threads = Some(1),
            "4" => config.worker_threads = Some(4),
            "mio" => {
                config.worker_threads = None;
                config.io_mode = rillcache::server::IoMode::Mio;
            }
            // Explicit async coverage independent of automatic selection.
            "tokio" => {
                config.worker_threads = None;
                config.io_mode = rillcache::server::IoMode::Tokio;
            }
            _ => panic!("test mode must be 0, 1, 4, mio or tokio"),
        }
    }
    Server::bind(config).unwrap().spawn().unwrap()
}

#[test]
fn automatic_and_explicit_network_selection_reports_captured_topology() {
    let mode = std::env::var("RILLCACHE_TEST_THREADS").unwrap_or_else(|_| "0".into());
    if !matches!(mode.as_str(), "0" | "mio" | "tokio") {
        return;
    }
    let server = start(Config::default());
    let policy = server.scheduling_policy();
    if mode == "tokio"
        || mode == "0" && thread::available_parallelism().is_ok_and(|n| n.get() >= 2) && cfg!(unix)
    {
        assert!(policy.readiness_io);
        assert!(policy.tokio_transport);
        let expected = thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1)
            .clamp(1, 64);
        assert_eq!(policy.tokio_worker_threads, expected);
        assert!(!policy.tokio_compatibility_owner);
    } else {
        assert_eq!(Config::default().io_mode, rillcache::server::IoMode::Auto);
        assert!(policy.readiness_io);
        assert!(!policy.tokio_transport);
        assert_eq!(policy.tokio_worker_threads, 0);
    }
    server.shutdown().unwrap();
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

fn info_clients(stream: &mut TcpStream) -> String {
    let mut command = Vec::new();
    encode_request(&[b"INFO", b"clients"], &mut command);
    stream.write_all(&command).unwrap();
    let mut header = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        stream.read_exact(&mut byte).unwrap();
        header.push(byte[0]);
        if header.ends_with(b"\r\n") {
            break;
        }
    }
    assert_eq!(header.first(), Some(&b'$'));
    let size: usize = std::str::from_utf8(&header[1..header.len() - 2])
        .unwrap()
        .parse()
        .unwrap();
    let mut body = vec![0; size + 2];
    stream.read_exact(&mut body).unwrap();
    assert_eq!(&body[size..], b"\r\n");
    body.truncate(size);
    String::from_utf8(body).unwrap()
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
fn transactions_queue_execute_watch_and_preserve_wire_order() {
    let server = start(Config::default());
    let mut first = connect(&server);
    let mut second = connect(&server);
    send(&mut first, &[b"MULTI"], b"+OK\r\n");
    send(&mut first, &[b"SET", b"tx\0key", b"v\xff"], b"+QUEUED\r\n");
    send(&mut first, &[b"GET", b"tx\0key"], b"+QUEUED\r\n");
    send(&mut first, &[b"EXEC"], b"*2\r\n+OK\r\n$2\r\nv\xff\r\n");
    send(&mut first, &[b"WATCH", b"tx\0key"], b"+OK\r\n");
    send(&mut second, &[b"SET", b"tx\0key", b"other"], b"+OK\r\n");
    send(&mut first, &[b"MULTI"], b"+OK\r\n");
    send(&mut first, &[b"SET", b"tx\0key", b"queued"], b"+QUEUED\r\n");
    send(&mut first, &[b"EXEC"], b"*-1\r\n");
    send(&mut first, &[b"GET", b"tx\0key"], b"$5\r\nother\r\n");
    send(&mut first, &[b"UNWATCH"], b"+OK\r\n");
    send(&mut first, &[b"MULTI"], b"+OK\r\n");
    send(&mut first, &[b"SET", b"tx\0key", b"final"], b"+QUEUED\r\n");
    send(&mut first, &[b"EXEC"], b"*1\r\n+OK\r\n");
    send(&mut first, &[b"GET", b"tx\0key"], b"$5\r\nfinal\r\n");
    send(&mut first, &[b"MULTI"], b"+OK\r\n");
    send(
        &mut first,
        &[b"SET", b"tx\0key"],
        b"-ERR wrong number of arguments for 'set' command\r\n",
    );
    send(
        &mut first,
        &[b"EXEC"],
        b"-EXECABORT Transaction discarded because of previous errors.\r\n",
    );
    send(&mut first, &[b"GET", b"tx\0key"], b"$5\r\nfinal\r\n");
    send(&mut second, &[b"PING"], b"+PONG\r\n");
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
fn inline_commands_are_quoted_binary_safe_fragmented_and_pipeline_ordered() {
    let server = start(Config::default());
    let mut stream = connect(&server);
    let bytes =
        b"\r\nSET \"key with space\" \"value\\n\\x00\\xff\"\nGET \"key with space\"\r\nPING\n";
    for fragment in bytes.chunks(3) {
        stream.write_all(fragment).unwrap();
    }
    expect(&mut stream, b"+OK\r\n$8\r\nvalue\n\0\xff\r\n+PONG\r\n");
    let mut malformed = connect(&server);
    malformed.write_all(b"SET \"unterminated\r\n").unwrap();
    let mut output = Vec::new();
    malformed.read_to_end(&mut output).unwrap();
    assert_eq!(output, b"-ERR Protocol error: invalid inline command\r\n");
    send(&mut connect(&server), &[b"PING"], b"+PONG\r\n");
}

#[test]
fn configured_limits_admit_four_megabyte_values_and_bound_inline_lines() {
    let server = start(Config {
        limits: Limits {
            max_bulk_bytes: 8 * 1024 * 1024,
            max_frame_bytes: 16 * 1024 * 1024,
            ..Limits::default()
        },
        max_data_bytes: 8 * 1024 * 1024,
        ..Config::default()
    });
    let mut stream = connect(&server);
    let value = vec![b'x'; 4_000_000];
    send(&mut stream, &[b"SET", b"large", &value], b"+OK\r\n");
    let mut expected = Vec::new();
    rillcache::protocol::encode_bulk(Some(&value), &mut expected);
    send(&mut stream, &[b"GET", b"large"], &expected);
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
    send(
        &mut stream,
        &[b"NO-SUCH-COMMAND"],
        b"-ERR unknown command\r\n",
    );
    send(
        &mut stream,
        &[b"SET", b"k", b"v", b"UNSUPPORTED", b"1"],
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
fn runtime_connection_limit_changes_are_enforced_and_reported() {
    let server = start(Config::default());
    let mut admin = connect(&server);
    send(
        &mut admin,
        &[b"CONFIG", b"GET", b"maxclients"],
        b"*2\r\n$10\r\nmaxclients\r\n$3\r\n128\r\n",
    );
    send(
        &mut admin,
        &[b"CONFIG", b"SET", b"maxclients", b"2"],
        b"+OK\r\n",
    );
    let mut second = connect(&server);
    send(&mut second, &[b"PING"], b"+PONG\r\n");
    let mut rejected = connect(&server);
    let mut rejection = Vec::new();
    rejected.read_to_end(&mut rejection).unwrap();
    assert_eq!(rejection, b"-ERR max number of clients reached\r\n");
    send(
        &mut admin,
        &[b"CONFIG", b"GET", b"maxclients"],
        b"*2\r\n$10\r\nmaxclients\r\n$1\r\n2\r\n",
    );
    assert!(
        info_clients(&mut admin).contains("maxclients:2\r\n"),
        "INFO clients must report the live limit"
    );

    // Lowering the limit keeps admitted clients; raising it admits another one.
    send(&mut second, &[b"PING"], b"+PONG\r\n");
    send(
        &mut admin,
        &[b"CONFIG", b"SET", b"maxclients", b"3"],
        b"+OK\r\n",
    );
    let mut third = connect(&server);
    send(&mut third, &[b"PING"], b"+PONG\r\n");
    assert!(
        info_clients(&mut admin).contains("maxclients:3\r\n"),
        "INFO clients must reflect the raised limit"
    );

    // Lowering below the current count keeps admitted clients but rejects new ones.
    send(
        &mut admin,
        &[b"CONFIG", b"SET", b"maxclients", b"1"],
        b"+OK\r\n",
    );
    send(&mut second, &[b"PING"], b"+PONG\r\n");
    send(&mut third, &[b"PING"], b"+PONG\r\n");
    let mut fourth = connect(&server);
    let mut rejection = Vec::new();
    fourth.read_to_end(&mut rejection).unwrap();
    assert_eq!(rejection, b"-ERR max number of clients reached\r\n");

    // Invalid updates do not change the effective limit.
    send(
        &mut admin,
        &[b"CONFIG", b"SET", b"maxclients", b"0"],
        b"-ERR invalid maxclients\r\n",
    );
    send(
        &mut admin,
        &[b"CONFIG", b"SET", b"maxclients", b"nope"],
        b"-ERR invalid maxclients\r\n",
    );
    send(
        &mut admin,
        &[b"CONFIG", b"GET", b"maxclients"],
        b"*2\r\n$10\r\nmaxclients\r\n$1\r\n1\r\n",
    );
    server.shutdown().unwrap();
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
        // Verify eventual idle disconnect, not a 50 ms shared-host scheduling budget.
        idle_timeout: Duration::from_millis(250),
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
fn repeated_shutdown_with_connections_in_flight_is_not_a_worker_failure() {
    for _ in 0..32 {
        let server = start(Config::default());
        let addr = server.local_addr();
        let ready = std::sync::Barrier::new(2);
        thread::scope(|scope| {
            let ready_ref = &ready;
            let connector = scope.spawn(move || {
                ready_ref.wait();
                for _ in 0..8 {
                    if let Ok(mut stream) =
                        TcpStream::connect_timeout(&addr, Duration::from_millis(200))
                    {
                        let _ = stream.write_all(b"*1\r\n$4\r\nPING\r\n");
                    }
                }
            });
            ready.wait();
            server.shutdown().unwrap();
            connector.join().unwrap();
        });
    }
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
fn extended_string_conditions_digest_and_msetex_work_in_all_modes() {
    const REDIS_VECTOR: &[u8] = b"v8lf0c11xh8ymlqztfd3eeq16kfn4sspw7fqmnuuq3k3t75em5wdizgcdw7uc26nnf961u2jkfzkjytls2kwlj7626sd";
    for worker_threads in [None, Some(1), Some(4)] {
        let server = start(Config {
            worker_threads,
            ..Config::default()
        });
        let mut stream = connect(&server);
        send(&mut stream, &[b"SET", b"foo", REDIS_VECTOR], b"+OK\r\n");
        send(
            &mut stream,
            &[b"DIGEST", b"foo"],
            b"$16\r\n00006c38adf31777\r\n",
        );
        send(
            &mut stream,
            &[b"SET", b"foo", b"updated", b"IFDEQ", b"00006C38ADF31777"],
            b"+OK\r\n",
        );
        send(
            &mut stream,
            &[b"SET", b"foo", b"no", b"IFEQ", b"wrong"],
            b"$-1\r\n",
        );
        send(&mut stream, &[b"GET", b"foo"], b"$7\r\nupdated\r\n");
        send(
            &mut stream,
            &[b"DELEX", b"foo", b"IFNE", b"updated"],
            b":0\r\n",
        );
        send(&mut stream, &[b"DELIFEQ", b"foo", b"wrong"], b":0\r\n");
        send(
            &mut stream,
            &[b"SET", b"delete-eq", b"binary\0\xff"],
            b"+OK\r\n",
        );
        send(
            &mut stream,
            &[b"DELIFEQ", b"delete-eq", b"wrong"],
            b":0\r\n",
        );
        send(
            &mut stream,
            &[b"DELIFEQ", b"delete-eq", b"binary\0\xff"],
            b":1\r\n",
        );
        send(
            &mut stream,
            &[b"DELEX", b"foo", b"IFEQ", b"updated"],
            b":1\r\n",
        );
        send(
            &mut stream,
            &[b"SET", b"a", b"old", b"PX", b"4102444800000"],
            b"+OK\r\n",
        );
        send(&mut stream, &[b"SET", b"b", b"old"], b"+OK\r\n");
        send(
            &mut stream,
            &[
                b"MSETEX",
                b"2",
                b"a",
                b"new",
                b"b",
                b"two",
                b"XX",
                b"PXAT",
                b"4102444800123",
            ],
            b":1\r\n",
        );
        send(
            &mut stream,
            &[b"MGET", b"a", b"b"],
            b"*2\r\n$3\r\nnew\r\n$3\r\ntwo\r\n",
        );
        send(&mut stream, &[b"PEXPIRETIME", b"a"], b":4102444800123\r\n");
        send(
            &mut stream,
            &[b"MSETEX", b"2", b"a", b"x", b"missing", b"y", b"XX"],
            b":0\r\n",
        );
        send(&mut stream, &[b"GET", b"a"], b"$3\r\nnew\r\n");
        send(
            &mut stream,
            &[b"SET", b"a", b"bad", b"IFDEQ", b"short"],
            b"-ERR must be exactly 16 hexadecimal characters\r\n",
        );
        send(&mut stream, &[b"GET", b"a"], b"$3\r\nnew\r\n");
        server.shutdown().unwrap();
    }
}

#[test]
fn conditional_strings_binary_type_and_oom_errors_keep_the_wire_usable() {
    let server = start(Config {
        max_data_bytes: 32,
        ..Config::default()
    });
    let mut stream = connect(&server);
    send(&mut stream, &[b"SET", b"\0\xff", b""], b"+OK\r\n");
    send(
        &mut stream,
        &[b"DIGEST", b"\0\xff"],
        b"$16\r\n2d06800538d394c2\r\n",
    );
    send(
        &mut stream,
        &[
            b"SET",
            b"\0\xff",
            b"\0\xff",
            b"IFDEQ",
            b"2D06800538D394C2",
            b"GET",
        ],
        b"$0\r\n\r\n",
    );
    send(
        &mut stream,
        &[b"DELEX", b"\0\xff", b"IFEQ", b"no"],
        b":0\r\n",
    );
    send(
        &mut stream,
        &[b"SET", b"\0\xff", &[b'x'; 32], b"IFEQ", b"\0\xff", b"GET"],
        b"-OOM configured payload limit exceeded\r\n",
    );
    send(&mut stream, &[b"GET", b"\0\xff"], b"$2\r\n\0\xff\r\n");
    send(&mut stream, &[b"DELIFEQ", b"\0\xff", b"\0\xff"], b":1\r\n");
    send(
        &mut stream,
        &[b"XADD", b"k", b"1-0", b"f", b"v"],
        b"$3\r\n1-0\r\n",
    );
    send(
        &mut stream,
        &[b"DIGEST", b"k"],
        b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
    );
    send(
        &mut stream,
        &[b"SET", b"k", b"v", b"IFEQ", b""],
        b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
    );
    send(
        &mut stream,
        &[b"DELEX", b"k", b"IFEQ", b""],
        b"-ERR Key should be of string type if conditions are specified\r\n",
    );
    send(
        &mut stream,
        &[b"DELIFEQ", b"k", b""],
        b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
    );
    send(
        &mut stream,
        &[b"MGET", b"k", b"missing"],
        b"*2\r\n$-1\r\n$-1\r\n",
    );
    send(
        &mut stream,
        &[b"MSETEX", b"2", b"k", b"bad"],
        b"-ERR syntax error\r\n",
    );
    send(&mut stream, &[b"XLEN", b"k"], b":1\r\n");
    send(
        &mut stream,
        &[b"MSETEX", b"1", b"k", b"new", b"XX"],
        b":1\r\n",
    );
    send(&mut stream, &[b"GET", b"k"], b"$3\r\nnew\r\n");
    send(&mut stream, &[b"DELEX", b"k"], b":1\r\n");
    send(&mut stream, &[b"PING"], b"+PONG\r\n");
    server.shutdown().unwrap();
}

#[test]
fn concurrent_value_compares_have_one_tcp_winner_in_every_mode() {
    use std::io::{BufRead, BufReader};
    for worker_threads in [None, Some(1), Some(4)] {
        let server = start(Config {
            worker_threads,
            ..Config::default()
        });
        send(
            &mut connect(&server),
            &[b"SET", b"race", b"old"],
            b"+OK\r\n",
        );
        let streams: Vec<_> = (0..8).map(|_| connect(&server)).collect();
        let barrier = std::sync::Barrier::new(8);
        let replies = thread::scope(|scope| {
            let workers: Vec<_> = streams
                .into_iter()
                .map(|mut stream| {
                    let barrier = &barrier;
                    scope.spawn(move || {
                        let mut command = Vec::new();
                        encode_request(&[b"SET", b"race", b"new", b"IFEQ", b"old"], &mut command);
                        barrier.wait();
                        stream.write_all(&command).unwrap();
                        let mut line = String::new();
                        BufReader::new(stream).read_line(&mut line).unwrap();
                        assert!(line == "+OK\r\n" || line == "$-1\r\n");
                        line == "+OK\r\n"
                    })
                })
                .collect();
            workers
                .into_iter()
                .map(|worker| worker.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert_eq!(replies.iter().filter(|&&won| won).count(), 1);
        send(&mut connect(&server), &[b"GET", b"race"], b"$3\r\nnew\r\n");
        server.shutdown().unwrap();
    }
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
fn expiration_aliases_and_numeric_errors_preserve_pipeline_order() {
    let server = start(Config::default());
    let cases: &[(&[&[u8]], &[u8])] = &[
        (&[b"SET", b"k", b"0", b"PXAT", b"4102444800123"], b"+OK\r\n"),
        (&[b"INCR", b"k"], b":1\r\n"),
        (&[b"DECRBY", b"k", b"-2"], b":3\r\n"),
        (&[b"PEXPIRETIME", b"k"], b":4102444800123\r\n"),
        (&[b"SET", b"k", b"x", b"NX", b"GET"], b"$1\r\n3\r\n"),
        (
            &[b"SET", b"k", b"x", b"NX", b"EX", b"0"],
            b"-ERR invalid expire time in 'set' command\r\n",
        ),
        (&[b"GETEX", b"k", b"PERSIST"], b"$1\r\n3\r\n"),
        (&[b"PTTL", b"k"], b":-1\r\n"),
        (&[b"PEXPIREAT", b"k", b"1"], b":1\r\n"),
        (&[b"GET", b"k"], b"$-1\r\n"),
        (&[b"TTL", b"k"], b":-2\r\n"),
    ];
    let mut bytes = Vec::new();
    let mut expected = Vec::new();
    for (args, reply) in cases {
        encode_request(args, &mut bytes);
        expected.extend_from_slice(reply);
    }
    let mut stream = connect(&server);
    stream.write_all(&bytes).unwrap();
    expect(&mut stream, &expected);
}

#[test]
fn concurrent_tcp_increments_on_one_key_have_no_lost_or_duplicate_results() {
    use std::io::{BufRead, BufReader};
    const CLIENTS: usize = 12;
    const COMMANDS: usize = 256;
    let server = start(Config::default());
    send(
        &mut connect(&server),
        &[b"SET", b"count", b"0", b"PXAT", b"4102444800123"],
        b"+OK\r\n",
    );
    let mut streams: Vec<_> = (0..CLIENTS).map(|_| connect(&server)).collect();
    for stream in &mut streams {
        send(stream, &[b"PING"], b"+PONG\r\n");
    }
    let ready = std::sync::Barrier::new(CLIENTS);
    let values = thread::scope(|scope| {
        let workers: Vec<_> = streams
            .into_iter()
            .map(|mut stream| {
                let ready = &ready;
                scope.spawn(move || {
                    let mut bytes = Vec::new();
                    for _ in 0..COMMANDS {
                        encode_request(&[b"INCR", b"count"], &mut bytes);
                    }
                    ready.wait();
                    stream.write_all(&bytes).unwrap();
                    let mut reader = BufReader::new(stream);
                    let mut values = Vec::new();
                    for _ in 0..COMMANDS {
                        let mut line = String::new();
                        reader.read_line(&mut line).unwrap();
                        values.push(
                            line.strip_prefix(':')
                                .unwrap()
                                .trim_end()
                                .parse::<usize>()
                                .unwrap(),
                        );
                    }
                    values
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|worker| worker.join().unwrap())
            .collect::<std::collections::BTreeSet<_>>()
    });
    assert_eq!(values, (1..=CLIENTS * COMMANDS).collect());
    send(
        &mut connect(&server),
        &[b"GET", b"count"],
        b"$4\r\n3072\r\n",
    );
    send(
        &mut connect(&server),
        &[b"PEXPIRETIME", b"count"],
        b":4102444800123\r\n",
    );
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
