#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::Duration;

use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

fn start(config: Config) -> ServerHandle {
    let workers = match std::env::var("RILLCACHE_TEST_THREADS").as_deref() {
        Ok("0") | Err(_) => None,
        Ok("1") => Some(1),
        Ok("4") => Some(4),
        Ok(value) => panic!("invalid test worker mode {value}"),
    };
    Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        worker_threads: workers,
        ..config
    })
    .unwrap()
    .spawn()
    .unwrap()
}

fn temp_name(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "rillcache-acl-{label}-{}-{:?}.acl",
        std::process::id(),
        std::thread::current().id()
    ))
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
    assert_eq!(actual, reply);
}

fn read_to_end(stream: &mut TcpStream) -> Vec<u8> {
    let mut output = Vec::new();
    stream.read_to_end(&mut output).unwrap();
    output
}

#[test]
fn acl_file_users_restrict_commands_keys_and_are_observable() {
    let path = temp_name("users");
    std::fs::write(
        &path,
        b"# managed by the ACL test\n\
user default on nopass ~* &* +@all\n\
user alice on >alice-password resetkeys ~app:* resetchannels &events:* -@all +@read +@write +@transaction\n",
    )
    .unwrap();
    let server = start(Config {
        aclfile: Some(path.clone()),
        ..Config::default()
    });
    let mut admin = connect(&server);
    send(&mut admin, &[b"PING"], b"+PONG\r\n");
    send(&mut admin, &[b"ACL", b"WHOAMI"], b"$7\r\ndefault\r\n");
    let mut alice = connect(&server);
    send(
        &mut alice,
        &[b"AUTH", b"alice", b"alice-password"],
        b"+OK\r\n",
    );
    send(&mut alice, &[b"ACL", b"WHOAMI"], b"$5\r\nalice\r\n");
    send(&mut alice, &[b"SET", b"app:key", b"value"], b"+OK\r\n");
    send(&mut alice, &[b"GET", b"app:key"], b"$5\r\nvalue\r\n");
    send(
        &mut alice,
        &[b"GET", b"other:key"],
        b"-NOPERM this user has no permissions to access one of the keys used as arguments\r\n",
    );
    send(
        &mut alice,
        &[b"SET", b"other:key", b"value"],
        b"-NOPERM this user has no permissions to access one of the keys used as arguments\r\n",
    );
    send(
        &mut alice,
        &[b"PING"],
        b"-NOPERM this user has no permissions to run the 'ping' command\r\n",
    );
    send(&mut alice, &[b"DEL", b"app:key"], b":1\r\n");

    send(
        &mut admin,
        &[
            b"ACL",
            b"SETUSER",
            b"bob",
            b"on",
            b">bob-password",
            b"allkeys",
            b"allchannels",
            b"-@all",
            b"+@read",
        ],
        b"+OK\r\n",
    );
    let mut bob = connect(&server);
    send(&mut bob, &[b"AUTH", b"bob", b"bob-password"], b"+OK\r\n");
    send(&mut bob, &[b"GET", b"any"], b"$-1\r\n");
    send(
        &mut bob,
        &[b"SET", b"any", b"value"],
        b"-NOPERM this user has no permissions to run the 'set' command\r\n",
    );

    send(&mut admin, &[b"ACL", b"SAVE"], b"+OK\r\n");
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(saved.contains("user alice on #"));
    assert!(saved.contains("user bob on #"));
    assert!(!saved.contains("alice-password"));
    assert!(!saved.contains("bob-password"));
    send(&mut admin, &[b"ACL", b"LOAD"], b"+OK\r\n");

    send(
        &mut admin,
        &[b"ACL", b"DELUSER", b"alice", b"bob"],
        b":2\r\n",
    );
    send(
        &mut alice,
        &[b"GET", b"app:key"],
        b"-NOAUTH Authentication required.\r\n",
    );
    send(
        &mut bob,
        &[b"GET", b"any"],
        b"-NOAUTH Authentication required.\r\n",
    );

    server.shutdown().unwrap();
    let _ = std::fs::remove_file(&path);
}

#[test]
fn acl_file_and_authentication_failures_are_visible_at_startup() {
    let missing = temp_name("missing");
    let error = Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        aclfile: Some(missing),
        ..Config::default()
    })
    .err()
    .unwrap();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);

    let no_default = temp_name("no-default");
    std::fs::write(
        &no_default,
        b"user alice on nopass allkeys allchannels +@all\n",
    )
    .unwrap();
    assert!(
        Server::bind(Config {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            aclfile: Some(no_default.clone()),
            ..Config::default()
        })
        .is_err()
    );
    std::fs::remove_file(&no_default).unwrap();

    let invalid = temp_name("invalid");
    std::fs::write(
        &invalid,
        b"user default on nopass allkeys allchannels +@all\nuser alice on +@no-such-category\n",
    )
    .unwrap();
    assert!(
        Server::bind(Config {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            aclfile: Some(invalid.clone()),
            ..Config::default()
        })
        .is_err()
    );
    std::fs::remove_file(&invalid).unwrap();

    let error = Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        requirepass: Some(b"pw".to_vec()),
        aclfile: Some(temp_name("both")),
        ..Config::default()
    })
    .err()
    .unwrap();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
}

#[test]
fn acl_denial_keeps_pipelines_ordered_and_quit_available() {
    let server = start(Config::default());
    let mut admin = connect(&server);
    send(
        &mut admin,
        &[
            b"ACL", b"SETUSER", b"reader", b"on", b">pw", b"allkeys", b"-@all", b"+@read",
        ],
        b"+OK\r\n",
    );
    let mut stream = connect(&server);
    let mut pipeline = Vec::new();
    encode_request(&[b"AUTH", b"reader", b"pw"], &mut pipeline);
    encode_request(&[b"GET", b"key"], &mut pipeline);
    encode_request(&[b"SET", b"key", b"v"], &mut pipeline);
    encode_request(&[b"GET", b"key"], &mut pipeline);
    encode_request(&[b"QUIT"], &mut pipeline);
    stream.write_all(&pipeline).unwrap();
    let expected = b"+OK\r\n$-1\r\n-NOPERM this user has no permissions to run the 'set' command\r\n$-1\r\n+OK\r\n";
    assert_eq!(read_to_end(&mut stream), expected);
    server.shutdown().unwrap();
}

fn read_frame(stream: &mut TcpStream, depth: usize) -> Vec<u8> {
    assert!(depth <= 8);
    let mut output = Vec::new();
    let mut prefix = [0u8];
    stream.read_exact(&mut prefix).unwrap();
    output.push(prefix[0]);
    let mut line = Vec::new();
    loop {
        let mut byte = [0u8];
        stream.read_exact(&mut byte).unwrap();
        if byte[0] == b'\n' {
            assert_eq!(line.pop(), Some(b'\r'));
            output.extend_from_slice(&line);
            output.extend_from_slice(b"\r\n");
            break;
        }
        line.push(byte[0]);
    }
    match prefix[0] {
        b'$' => {
            let length = std::str::from_utf8(&line).unwrap().parse::<i64>().unwrap();
            if length >= 0 {
                let mut body = vec![0; length as usize + 2];
                stream.read_exact(&mut body).unwrap();
                output.extend_from_slice(&body);
            }
        }
        b'*' => {
            let count = std::str::from_utf8(&line).unwrap().parse::<i64>().unwrap();
            for _ in 0..count.max(0) {
                output.extend_from_slice(&read_frame(stream, depth + 1));
            }
        }
        _ => {}
    }
    output
}

fn request(stream: &mut TcpStream, arguments: &[&[u8]]) -> Vec<u8> {
    let mut bytes = Vec::new();
    encode_request(arguments, &mut bytes);
    stream.write_all(&bytes).unwrap();
    read_frame(stream, 0)
}

#[test]
fn acl_selectors_authorize_independent_key_scopes_and_round_trip() {
    let path = temp_name("selectors");
    std::fs::write(&path, b"user default on nopass ~* &* +@all\n").unwrap();
    let config = Config {
        aclfile: Some(path.clone()),
        ..Config::default()
    };

    {
        let server = start(config.clone());
        let mut admin = connect(&server);
        send(
            &mut admin,
            &[
                b"ACL",
                b"SETUSER",
                b"alice",
                b"on",
                b">secret",
                b"-@all",
                b"+get",
                b"allkeys",
                b"(+set ~app:*)",
            ],
            b"+OK\r\n",
        );
        let list = String::from_utf8(request(&mut admin, &[b"ACL", b"LIST"])).unwrap();
        assert!(list.contains("(+set ~app:*)"), "{list}");
        let details =
            String::from_utf8(request(&mut admin, &[b"ACL", b"GETUSER", b"alice"])).unwrap();
        assert!(details.contains("selectors"), "{details}");
        assert!(details.contains("+set"), "{details}");
        assert!(details.contains("~app:*"), "{details}");
        send(&mut admin, &[b"ACL", b"SAVE"], b"+OK\r\n");

        let mut alice = connect(&server);
        send(&mut alice, &[b"AUTH", b"alice", b"secret"], b"+OK\r\n");
        send(&mut alice, &[b"SET", b"app:1", b"v"], b"+OK\r\n");
        send(&mut alice, &[b"GET", b"app:1"], b"$1\r\nv\r\n");
        send(&mut alice, &[b"GET", b"other"], b"$-1\r\n");
        send(
            &mut alice,
            &[b"SET", b"other", b"v"],
            b"-NOPERM this user has no permissions to access one of the keys used as arguments\r\n",
        );
        server.shutdown().unwrap();
    }

    {
        let server = start(config);
        let mut alice = connect(&server);
        send(&mut alice, &[b"AUTH", b"alice", b"secret"], b"+OK\r\n");
        send(&mut alice, &[b"SET", b"app:2", b"v"], b"+OK\r\n");
        send(
            &mut alice,
            &[b"SET", b"other", b"v"],
            b"-NOPERM this user has no permissions to access one of the keys used as arguments\r\n",
        );
        server.shutdown().unwrap();
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn acl_key_modes_scope_reads_and_writes_and_round_trip() {
    let path = temp_name("key-modes");
    std::fs::write(&path, b"user default on nopass ~* &* +@all\n").unwrap();
    let config = Config {
        aclfile: Some(path.clone()),
        ..Config::default()
    };

    {
        let server = start(config.clone());
        let mut admin = connect(&server);
        send(
            &mut admin,
            &[
                b"ACL",
                b"SETUSER",
                b"writer",
                b"on",
                b">secret",
                b"-@all",
                b"+get",
                b"+set",
                b"+del",
                b"+lpush",
                b"%R~r:*",
                b"%W~w:*",
                b"%RW~both:*",
                b"(+set %W~sel:*)",
            ],
            b"+OK\r\n",
        );
        let list = String::from_utf8(request(&mut admin, &[b"ACL", b"LIST"])).unwrap();
        assert!(list.contains("%R~r:*"), "{list}");
        assert!(list.contains("%W~w:*"), "{list}");
        assert!(list.contains("~both:*"), "{list}");
        assert!(!list.contains("%RW~both:*"), "{list}");
        assert!(list.contains("(+set %W~sel:*)"), "{list}");
        let details =
            String::from_utf8(request(&mut admin, &[b"ACL", b"GETUSER", b"writer"])).unwrap();
        assert!(details.contains("%R~r:*"), "{details}");
        assert!(details.contains("%W~w:*"), "{details}");
        send(
            &mut admin,
            &[b"ACL", b"DRYRUN", b"writer", b"GET", b"w:1"],
            b"-NOPERM this user has no permissions to access one of the keys used as arguments\r\n",
        );
        send(
            &mut admin,
            &[b"ACL", b"DRYRUN", b"writer", b"GET", b"r:1"],
            b"+OK\r\n",
        );
        send(&mut admin, &[b"ACL", b"SAVE"], b"+OK\r\n");

        let mut writer = connect(&server);
        send(&mut writer, &[b"AUTH", b"writer", b"secret"], b"+OK\r\n");
        send(&mut writer, &[b"GET", b"r:1"], b"$-1\r\n");
        send(
            &mut writer,
            &[b"SET", b"r:1", b"v"],
            b"-NOPERM this user has no permissions to access one of the keys used as arguments\r\n",
        );
        send(&mut writer, &[b"SET", b"w:1", b"v"], b"+OK\r\n");
        send(
            &mut writer,
            &[b"GET", b"w:1"],
            b"-NOPERM this user has no permissions to access one of the keys used as arguments\r\n",
        );
        send(
            &mut writer,
            &[b"LPUSH", b"w:list", b"v"],
            b"-NOPERM this user has no permissions to access one of the keys used as arguments\r\n",
        );
        send(&mut writer, &[b"DEL", b"w:1"], b":1\r\n");
        send(&mut writer, &[b"SET", b"both:1", b"v"], b"+OK\r\n");
        send(&mut writer, &[b"GET", b"both:1"], b"$1\r\nv\r\n");
        send(&mut writer, &[b"SET", b"sel:1", b"v"], b"+OK\r\n");
        send(
            &mut writer,
            &[b"GET", b"sel:1"],
            b"-NOPERM this user has no permissions to access one of the keys used as arguments\r\n",
        );
        server.shutdown().unwrap();
    }

    {
        let server = start(config);
        let mut writer = connect(&server);
        send(&mut writer, &[b"AUTH", b"writer", b"secret"], b"+OK\r\n");
        send(&mut writer, &[b"GET", b"r:2"], b"$-1\r\n");
        send(&mut writer, &[b"SET", b"w:2", b"v"], b"+OK\r\n");
        send(&mut writer, &[b"SET", b"sel:2", b"v"], b"+OK\r\n");
        send(
            &mut writer,
            &[b"GET", b"w:2"],
            b"-NOPERM this user has no permissions to access one of the keys used as arguments\r\n",
        );
        server.shutdown().unwrap();
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn acl_key_modes_scope_mixed_dynamic_and_option_dependent_specs() {
    let server = start(Config::default());
    let mut admin = connect(&server);
    send(&mut admin, &[b"SET", b"src:bit", b"\xff"], b"+OK\r\n");
    send(
        &mut admin,
        &[b"SET", b"src:copy", b"copy-value"],
        b"+OK\r\n",
    );
    send(
        &mut admin,
        &[
            b"XGROUP",
            b"CREATE",
            b"stream:read",
            b"g",
            b"0",
            b"MKSTREAM",
        ],
        b"+OK\r\n",
    );
    send(&mut admin, &[b"RPUSH", b"sort:read", b"b", b"a"], b":2\r\n");
    send(
        &mut admin,
        &[b"GEOADD", b"geo:read", b"1", b"2", b"member"],
        b":1\r\n",
    );
    send(
        &mut admin,
        &[
            b"ACL",
            b"SETUSER",
            b"scoped",
            b"on",
            b">pw",
            b"-@all",
            b"+@all",
            b"%R~read:*",
            b"%W~write:*",
            b"%RW~rw:*",
            b"%R~src:*",
            b"%W~dst:*",
            b"%R~stream:*",
            b"%R~hll:*",
            b"%W~hll:*",
            b"%R~only-hll:*",
            b"%R~sort:*",
            b"%W~sortdst:*",
            b"%R~geo:*",
            b"%W~geodst:*",
            b"(+set %W~selector:*)",
        ],
        b"+OK\r\n",
    );
    let mut client = connect(&server);
    send(&mut client, &[b"AUTH", b"scoped", b"pw"], b"+OK\r\n");

    // Write-only grants permit replacement without granting read access.
    send(&mut client, &[b"SET", b"write:\0key", b"v"], b"+OK\r\n");
    send(
        &mut client,
        &[b"GET", b"write:\0key"],
        b"-NOPERM this user has no permissions to access one of the keys used as arguments\r\n",
    );
    send(
        &mut client,
        &[b"SET", b"read:key", b"v"],
        b"-NOPERM this user has no permissions to access one of the keys used as arguments\r\n",
    );
    send(&mut client, &[b"SET", b"rw:key", b"v"], b"+OK\r\n");
    send(&mut client, &[b"GET", b"rw:key"], b"$1\r\nv\r\n");

    // Mixed specs check each key position, not the command-wide WRITE flag.
    send(
        &mut client,
        &[b"BITOP", b"AND", b"dst:bit", b"src:bit"],
        b":1\r\n",
    );
    send(&mut client, &[b"COPY", b"src:copy", b"dst:copy"], b":1\r\n");
    send(
        &mut client,
        &[b"COPY", b"write:copy", b"read:copy"],
        b"-NOPERM this user has no permissions to access one of the keys used as arguments\r\n",
    );
    // PFCOUNT has an RW key spec despite the command-level READONLY flag.
    send(&mut client, &[b"PFCOUNT", b"hll:missing"], b":0\r\n");
    send(
        &mut client,
        &[b"PFCOUNT", b"only-hll:missing"],
        b"-NOPERM this user has no permissions to access one of the keys used as arguments\r\n",
    );

    // SORT and GEORADIUS access read-only sources and write-only destinations.
    let sorted = request(
        &mut client,
        &[b"SORT", b"sort:read", b"ALPHA", b"STORE", b"sortdst:result"],
    );
    assert_eq!(sorted, b":2\r\n");
    let geo = request(
        &mut client,
        &[
            b"GEORADIUS",
            b"geo:read",
            b"1",
            b"2",
            b"1",
            b"KM",
            b"STORE",
            b"geodst:result",
        ],
    );
    assert!(!geo.starts_with(b"-NOPERM"), "{geo:?}");

    // STREAMS key/ID halves are dynamic key specs and ACL checked.
    let xread = request(&mut client, &[b"XREAD", b"STREAMS", b"stream:read", b"0-0"]);
    assert!(!xread.starts_with(b"-NOPERM"), "{xread:?}");
    send(
        &mut client,
        &[b"XREAD", b"STREAMS", b"write:stream", b"0-0"],
        b"-NOPERM this user has no permissions to access one of the keys used as arguments\r\n",
    );
    let group_read = request(
        &mut client,
        &[
            b"XREADGROUP",
            b"GROUP",
            b"g",
            b"c",
            b"STREAMS",
            b"stream:read",
            b">",
        ],
    );
    assert!(!group_read.starts_with(b"-NOPERM"), "{group_read:?}");

    // The root permits the command but the selector grants only SET on its key.
    send(&mut client, &[b"SET", b"selector:1", b"v"], b"+OK\r\n");
    send(
        &mut client,
        &[b"GET", b"selector:1"],
        b"-NOPERM this user has no permissions to access one of the keys used as arguments\r\n",
    );
    send(
        &mut admin,
        &[b"ACL", b"DRYRUN", b"scoped", b"GET", b"write:\0key"],
        b"-NOPERM this user has no permissions to access one of the keys used as arguments\r\n",
    );
    server.shutdown().unwrap();
}

#[test]
fn acl_first_argument_rules_scope_subcommands_and_round_trip() {
    let path = temp_name("first-arg");
    std::fs::write(&path, b"user default on nopass ~* &* +@all\n").unwrap();
    let server = start(Config {
        aclfile: Some(path.clone()),
        ..Config::default()
    });
    let mut admin = connect(&server);
    send(
        &mut admin,
        &[
            b"ACL",
            b"SETUSER",
            b"config-reader",
            b"on",
            b">secret",
            b"allkeys",
            b"allchannels",
            b"-@all",
            b"+config|get",
        ],
        b"+OK\r\n",
    );
    let list = String::from_utf8(request(&mut admin, &[b"ACL", b"LIST"])).unwrap();
    assert!(list.contains("+config|get"), "{list}");
    let details =
        String::from_utf8(request(&mut admin, &[b"ACL", b"GETUSER", b"config-reader"])).unwrap();
    assert!(details.contains("+config|get"), "{details}");
    send(
        &mut admin,
        &[
            b"ACL",
            b"DRYRUN",
            b"config-reader",
            b"CONFIG",
            b"GET",
            b"maxmemory",
        ],
        b"+OK\r\n",
    );
    send(
        &mut admin,
        &[
            b"ACL",
            b"DRYRUN",
            b"config-reader",
            b"CONFIG",
            b"SET",
            b"maxmemory",
            b"1",
        ],
        b"-NOPERM this user has no permissions to run the 'config' command\r\n",
    );

    let mut reader = connect(&server);
    send(
        &mut reader,
        &[b"AUTH", b"config-reader", b"secret"],
        b"+OK\r\n",
    );
    let get = request(&mut reader, &[b"CONFIG", b"GET", b"maxmemory"]);
    assert!(
        String::from_utf8_lossy(&get).contains("maxmemory"),
        "{get:?}"
    );
    send(
        &mut reader,
        &[b"CONFIG", b"SET", b"maxmemory", b"1"],
        b"-NOPERM this user has no permissions to run the 'config' command\r\n",
    );
    send(&mut admin, &[b"ACL", b"SAVE"], b"+OK\r\n");
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(saved.contains("+config|get"), "{saved}");
    send(&mut admin, &[b"ACL", b"LOAD"], b"+OK\r\n");
    send(
        &mut reader,
        &[b"CONFIG", b"SET", b"maxmemory", b"1"],
        b"-NOPERM this user has no permissions to run the 'config' command\r\n",
    );

    send(
        &mut admin,
        &[
            b"ACL",
            b"SETUSER",
            b"config-except-set",
            b"on",
            b">secret2",
            b"allkeys",
            b"allchannels",
            b"-@all",
            b"+config",
            b"-config|set",
        ],
        b"+OK\r\n",
    );
    let mut except_set = connect(&server);
    send(
        &mut except_set,
        &[b"AUTH", b"config-except-set", b"secret2"],
        b"+OK\r\n",
    );
    let get = request(&mut except_set, &[b"CONFIG", b"GET", b"maxmemory"]);
    assert!(
        String::from_utf8_lossy(&get).contains("maxmemory"),
        "{get:?}"
    );
    send(
        &mut except_set,
        &[b"CONFIG", b"SET", b"maxmemory", b"1"],
        b"-NOPERM this user has no permissions to run the 'config' command\r\n",
    );

    send(
        &mut admin,
        &[
            b"ACL",
            b"SETUSER",
            b"selector-reader",
            b"on",
            b">secret3",
            b"allkeys",
            b"allchannels",
            b"-@all",
            b"(+config|get)",
        ],
        b"+OK\r\n",
    );
    let mut selector = connect(&server);
    send(
        &mut selector,
        &[b"AUTH", b"selector-reader", b"secret3"],
        b"+OK\r\n",
    );
    let get = request(&mut selector, &[b"CONFIG", b"GET", b"maxmemory"]);
    assert!(
        String::from_utf8_lossy(&get).contains("maxmemory"),
        "{get:?}"
    );
    send(
        &mut selector,
        &[b"CONFIG", b"SET", b"maxmemory", b"1"],
        b"-NOPERM this user has no permissions to run the 'config' command\r\n",
    );

    server.shutdown().unwrap();
    let _ = std::fs::remove_file(&path);
}
