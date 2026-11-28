#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use rillcache::protocol::encode_request;
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
        .set_read_timeout(Some(Duration::from_secs(5)))
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
    if !reply.is_empty() {
        let mut actual = vec![0; reply.len()];
        stream.read_exact(&mut actual).unwrap();
        assert_eq!(actual, reply, "command {arguments:?}");
    }
}

fn line(stream: &mut TcpStream) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        stream.read_exact(&mut byte).unwrap();
        bytes.push(byte[0]);
        assert!(bytes.len() <= 128);
        if bytes.ends_with(b"\r\n") {
            return bytes;
        }
    }
}

fn client_id(stream: &mut TcpStream) -> u64 {
    send(stream, &[b"CLIENT", b"ID"], b"");
    let header = line(stream);
    assert_eq!(header.first(), Some(&b':'));
    std::str::from_utf8(&header[1..header.len() - 2])
        .unwrap()
        .parse()
        .unwrap()
}

fn bulk(stream: &mut TcpStream) -> Vec<u8> {
    let header = line(stream);
    assert_eq!(header.first(), Some(&b'$'));
    let size: usize = std::str::from_utf8(&header[1..header.len() - 2])
        .unwrap()
        .parse()
        .unwrap();
    let mut body = vec![0; size + 2];
    stream.read_exact(&mut body).unwrap();
    assert_eq!(&body[size..], b"\r\n");
    body.truncate(size);
    body
}

fn client_list(stream: &mut TcpStream, filter: &[&[u8]]) -> String {
    let mut command = vec![b"CLIENT".as_slice(), b"LIST"];
    command.extend_from_slice(filter);
    send(stream, &command, b"");
    String::from_utf8(bulk(stream)).unwrap()
}

fn drain_hello(stream: &mut TcpStream) {
    let header = line(stream);
    assert_eq!(&header, b"%5\r\n");
    for _ in 0..10 {
        let value = line(stream);
        if value[0] == b'$' {
            let size: usize = std::str::from_utf8(&value[1..value.len() - 2])
                .unwrap()
                .parse()
                .unwrap();
            let mut payload = vec![0; size + 2];
            stream.read_exact(&mut payload).unwrap();
        }
    }
}

#[test]
fn client_list_reports_names_metadata_and_pubsub_state() {
    let server = start();
    let mut admin = connect(&server);
    let mut worker = connect(&server);
    let worker_id = client_id(&mut worker);
    send(
        &mut worker,
        &[b"CLIENT", b"SETNAME", b"worker-b"],
        b"+OK\r\n",
    );
    send(
        &mut worker,
        &[b"CLIENT", b"SETINFO", b"LIB-NAME", b"test-lib"],
        b"+OK\r\n",
    );
    send(
        &mut worker,
        &[b"CLIENT", b"SETINFO", b"LIB-VER", b"9.9"],
        b"+OK\r\n",
    );
    send(&mut worker, &[b"HELLO", b"3"], b"");
    drain_hello(&mut worker);

    let all = client_list(&mut admin, &[]);
    assert!(all.contains(&format!("id={worker_id} ")), "{all}");
    assert!(all.contains("name=worker-b"), "{all}");
    assert!(all.contains("resp=3"), "{all}");
    assert!(all.contains("lib-name=test-lib"), "{all}");
    assert!(all.contains("lib-ver=9.9"), "{all}");
    assert!(all.contains("addr=127.0.0.1:"), "{all}");
    assert!(
        all.contains(&format!("laddr={}", server.local_addr())),
        "{all}"
    );
    let only = client_list(&mut admin, &[b"ID", worker_id.to_string().as_bytes()]);
    assert!(only.contains(&format!("id={worker_id} ")));
    assert_eq!(only.lines().count(), 1);

    send(
        &mut worker,
        &[b"SUBSCRIBE", b"ch"],
        b">3\r\n$9\r\nsubscribe\r\n$2\r\nch\r\n:1\r\n",
    );
    send(
        &mut worker,
        &[b"PSUBSCRIBE", b"news*"],
        b">3\r\n$10\r\npsubscribe\r\n$5\r\nnews*\r\n:2\r\n",
    );
    send(
        &mut worker,
        &[b"SSUBSCRIBE", b"shard"],
        b">3\r\n$10\r\nssubscribe\r\n$5\r\nshard\r\n:1\r\n",
    );
    let pubsub = client_list(&mut admin, &[b"TYPE", b"pubsub"]);
    assert!(pubsub.contains(&format!("id={worker_id} ")), "{pubsub}");
    assert!(pubsub.contains("flags=P"), "{pubsub}");
    assert!(pubsub.contains("sub=1 psub=1 ssub=1"), "{pubsub}");
    assert_eq!(pubsub.lines().count(), 1);
    send(&mut worker, &[b"CLIENT", b"INFO"], b"");
    let own = String::from_utf8(bulk(&mut worker)).unwrap();
    assert!(own.contains("sub=1 psub=1 ssub=1"), "{own}");

    // Sharded-only subscribers still have the pub/sub client type and P flag,
    // while their regular channel count stays zero.
    let mut shard_only = connect(&server);
    let shard_only_id = client_id(&mut shard_only);
    send(&mut shard_only, &[b"HELLO", b"3"], b"");
    drain_hello(&mut shard_only);
    send(
        &mut shard_only,
        &[b"SSUBSCRIBE", b"shard:one"],
        b">3\r\n$10\r\nssubscribe\r\n$9\r\nshard:one\r\n:1\r\n",
    );
    let shard_clients = client_list(&mut admin, &[b"TYPE", b"pubsub"]);
    assert!(
        shard_clients.contains(&format!("id={shard_only_id} ")),
        "{shard_clients}"
    );
    assert!(
        shard_clients.contains("flags=P db=0 sub=0 psub=0 ssub=1"),
        "{shard_clients}"
    );
    let normal_clients = client_list(&mut admin, &[b"TYPE", b"normal"]);
    assert!(!normal_clients.contains(&format!("id={shard_only_id} ")));
    send(&mut shard_only, &[b"CLIENT", b"INFO"], b"");
    let shard_info = String::from_utf8(bulk(&mut shard_only)).unwrap();
    assert!(
        shard_info.contains("flags=P sub=0 psub=0 ssub=1"),
        "{shard_info}"
    );
    send(
        &mut shard_only,
        &[b"SUNSUBSCRIBE", b"shard:one"],
        b">3\r\n$12\r\nsunsubscribe\r\n$9\r\nshard:one\r\n:0\r\n",
    );
    let shard_record = client_list(&mut admin, &[b"ID", shard_only_id.to_string().as_bytes()]);
    assert!(
        shard_record.contains("flags=N db=0 sub=0 psub=0 ssub=0"),
        "{shard_record}"
    );
    let normal_clients = client_list(&mut admin, &[b"TYPE", b"normal"]);
    assert!(
        normal_clients.contains(&format!("id={shard_only_id} ")),
        "{normal_clients}"
    );
    server.shutdown().unwrap();
}

#[test]
fn client_kill_removes_connections_and_protects_the_caller() {
    let server = start();
    let mut admin = connect(&server);
    let admin_id = client_id(&mut admin);
    let mut worker = connect(&server);
    let worker_id = client_id(&mut worker);
    let addr = client_list(&mut admin, &[b"ID", worker_id.to_string().as_bytes()])
        .split_whitespace()
        .find_map(|field| field.strip_prefix("addr="))
        .unwrap()
        .to_string();

    send(
        &mut admin,
        &[b"CLIENT", b"KILL", b"ID", admin_id.to_string().as_bytes()],
        b":0\r\n",
    );
    send(&mut admin, &[b"PING"], b"+PONG\r\n");
    send(
        &mut admin,
        &[b"CLIENT", b"KILL", b"SKIPME", b"no", b"ID", b"999999999"],
        b":0\r\n",
    );
    send(
        &mut admin,
        &[b"CLIENT", b"KILL", b"SKIPME", b"maybe"],
        b"-ERR syntax error\r\n",
    );
    send(
        &mut admin,
        &[b"CLIENT", b"KILL", b"ID", worker_id.to_string().as_bytes()],
        b":1\r\n",
    );
    let mut finished = Vec::new();
    let _ = worker.read_to_end(&mut finished);
    let list = client_list(&mut admin, &[]);
    assert!(!list.contains(&format!("id={worker_id} ")), "{list}");

    let mut legacy = connect(&server);
    let legacy_id = client_id(&mut legacy);
    let legacy_addr = client_list(&mut admin, &[b"ID", legacy_id.to_string().as_bytes()])
        .split_whitespace()
        .find_map(|field| field.strip_prefix("addr="))
        .unwrap()
        .to_string();
    send(
        &mut admin,
        &[b"CLIENT", b"KILL", legacy_addr.as_bytes()],
        b"+OK\r\n",
    );
    send(
        &mut admin,
        &[b"CLIENT", b"KILL", b"ADDR", addr.as_bytes()],
        b":0\r\n",
    );

    send(&mut admin, &[b"CLIENT", b"INFO"], b"");
    let own = String::from_utf8(bulk(&mut admin)).unwrap();
    assert!(
        own.contains(&format!("laddr={}", server.local_addr())),
        "{own}"
    );
    assert!(own.contains("flags=N"), "{own}");
    assert!(own.contains("sub=0"), "{own}");
    assert!(own.contains("psub=0"), "{own}");
    assert!(own.contains("watch=0"), "{own}");

    let info = client_list(&mut admin, &[]);
    assert!(info.contains(&format!("id={admin_id} ")), "{info}");
    assert_eq!(info.lines().count(), 1);
    send(&mut admin, &[b"INFO", b"clients"], b"");
    let clients = String::from_utf8(bulk(&mut admin)).unwrap();
    assert!(clients.contains("connected_clients:1"), "{clients}");
    server.shutdown().unwrap();
}

#[test]
fn client_kill_maxage_removes_only_old_connections() {
    let server = start();
    let mut admin = connect(&server);
    let admin_id = client_id(&mut admin);
    let mut old_one = connect(&server);
    let old_one_id = client_id(&mut old_one);
    let mut old_two = connect(&server);
    let old_two_id = client_id(&mut old_two);
    // Use the sleep itself as the threshold. The old clients are always at
    // least that old, while the young client would need to wait two seconds
    // before the KILL is dispatched to become eligible under load.
    std::thread::sleep(Duration::from_secs(2));
    let mut young = connect(&server);
    let young_id = client_id(&mut young);

    send(
        &mut admin,
        &[b"CLIENT", b"KILL", b"MAXAGE", b"2000"],
        b":2\r\n",
    );
    let mut finished = Vec::new();
    let _ = old_one.read_to_end(&mut finished);
    let mut finished = Vec::new();
    let _ = old_two.read_to_end(&mut finished);
    send(&mut young, &[b"PING"], b"+PONG\r\n");

    let list = client_list(&mut admin, &[]);
    assert!(list.contains(&format!("id={admin_id} ")), "{list}");
    assert!(list.contains(&format!("id={young_id} ")), "{list}");
    assert!(!list.contains(&format!("id={old_one_id} ")), "{list}");
    assert!(!list.contains(&format!("id={old_two_id} ")), "{list}");

    send(
        &mut admin,
        &[b"CLIENT", b"KILL", b"MAXAGE", b"nope"],
        b"-ERR invalid client MAXAGE\r\n",
    );
    server.shutdown().unwrap();
}

#[test]
fn client_kill_filters_by_local_address() {
    let server = start();
    let mut admin = connect(&server);
    let mut victim = connect(&server);
    let victim_id = client_id(&mut victim);
    let local = server.local_addr().to_string();

    let list = client_list(&mut admin, &[]);
    assert!(list.contains(&format!("laddr={local}")), "{list}");

    send(
        &mut admin,
        &[
            b"CLIENT",
            b"KILL",
            b"LADDR",
            b"127.0.0.1:1",
            b"ID",
            victim_id.to_string().as_bytes(),
        ],
        b":0\r\n",
    );
    send(
        &mut admin,
        &[
            b"CLIENT",
            b"KILL",
            b"LADDR",
            local.as_bytes(),
            b"ID",
            victim_id.to_string().as_bytes(),
        ],
        b":1\r\n",
    );
    let mut finished = Vec::new();
    let _ = victim.read_to_end(&mut finished);
    send(&mut admin, &[b"PING"], b"+PONG\r\n");
    server.shutdown().unwrap();
}

#[test]
fn client_kill_skipme_no_can_kill_the_caller_and_absent_types_are_empty() {
    let server = start();
    let mut admin = connect(&server);
    let admin_id = client_id(&mut admin);
    let mut witness = connect(&server);
    let witness_id = client_id(&mut witness);

    let list = client_list(&mut admin, &[b"TYPE", b"master"]);
    assert!(list.is_empty(), "{list}");
    let list = client_list(&mut admin, &[b"TYPE", b"replica"]);
    assert!(list.is_empty(), "{list}");
    let list = client_list(&mut admin, &[b"TYPE", b"slave"]);
    assert!(list.is_empty(), "{list}");
    send(
        &mut admin,
        &[
            b"CLIENT",
            b"KILL",
            b"TYPE",
            b"replica",
            b"ID",
            witness_id.to_string().as_bytes(),
        ],
        b":0\r\n",
    );
    send(&mut witness, &[b"PING"], b"+PONG\r\n");

    // SKIPME no with a non-caller target behaves like the default.
    send(
        &mut admin,
        &[
            b"CLIENT",
            b"KILL",
            b"SKIPME",
            b"no",
            b"ID",
            witness_id.to_string().as_bytes(),
        ],
        b":1\r\n",
    );
    let mut finished = Vec::new();
    let _ = witness.read_to_end(&mut finished);

    // Killing the caller closes its socket; the reply may race with the close.
    let mut victim = connect(&server);
    let victim_id = client_id(&mut victim);
    let mut killer = connect(&server);
    let killer_id = client_id(&mut killer);
    send(
        &mut killer,
        &[
            b"CLIENT",
            b"KILL",
            b"SKIPME",
            b"no",
            b"ID",
            killer_id.to_string().as_bytes(),
        ],
        b"",
    );
    let mut reply = vec![0; 4];
    let _ = killer.read(&mut reply);
    let list = client_list(&mut victim, &[]);
    assert!(!list.contains(&format!("id={killer_id} ")), "{list}");
    assert!(list.contains(&format!("id={victim_id} ")), "{list}");
    assert!(list.contains(&format!("id={admin_id} ")), "{list}");
    send(&mut victim, &[b"PING"], b"+PONG\r\n");
    server.shutdown().unwrap();
}

/// O3: the documented per-client resource fields. `fd` is the server-side
/// socket descriptor and `idle` is seconds since that client's last command,
/// sampled once per accepted command; a non-negative `fd` is required on Unix
/// and the field stays present (with -1) where the platform cannot report one.
#[test]
fn client_info_and_list_report_fd_and_idle_with_documented_units() {
    let server = start();
    let mut observer = connect(&server);
    let mut worker = connect(&server);
    send(&mut worker, &[b"PING"], b"+PONG\r\n");

    let listed = client_list(&mut observer, &[]);
    let worker_id = client_id(&mut worker);
    let mut found = None;
    for line in listed.lines() {
        let fields: std::collections::HashMap<&str, &str> = line
            .split(' ')
            .filter_map(|field| field.split_once('='))
            .collect();
        if fields.get("id").and_then(|id| id.parse::<u64>().ok()) == Some(worker_id) {
            found = Some(fields);
        }
    }
    let fields = found.expect("CLIENT LIST must contain the worker");
    let fd: i64 = fields.get("fd").expect("fd field").parse().unwrap();
    let idle: u64 = fields.get("idle").expect("idle field").parse().unwrap();
    if cfg!(unix) {
        assert!(fd >= 0, "unix should report a real descriptor, got {fd}");
    }
    assert!(
        idle < 60,
        "a just-active client must report small idle, got {idle}"
    );

    // CLIENT INFO (self) reports the same documented fields.
    let mut self_info = connect(&server);
    send(&mut self_info, &[b"CLIENT", b"INFO"], b"");
    let info = String::from_utf8(bulk(&mut self_info)).unwrap();
    assert!(info.contains(" fd="), "CLIENT INFO must report fd:\n{info}");
    assert!(
        info.contains(" idle="),
        "CLIENT INFO must report idle:\n{info}"
    );

    // Idle is a sampled second-granularity value: after a pause it grows.
    std::thread::sleep(Duration::from_millis(1_100));
    let listed = client_list(&mut observer, &[]);
    let idle_after = listed
        .lines()
        .find(|line| line.contains(&format!("id={worker_id} ")))
        .and_then(|line| {
            line.split(' ')
                .find_map(|field| field.strip_prefix("idle="))
        })
        .expect("worker row");
    assert!(
        idle_after.parse::<u64>().unwrap() >= 1,
        "idle must advance after a second without commands"
    );
    server.shutdown().unwrap();
}
