#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use rillcache::protocol::encode_request;
use rillcache::server::IoMode;
use rillcache::{Config, Server, ServerHandle};

const MODES: [(Option<usize>, IoMode); 4] = [
    (None, IoMode::Mio),
    (None, IoMode::Tokio),
    (Some(1), IoMode::Mio),
    (Some(4), IoMode::Mio),
];

#[derive(Debug, PartialEq, Eq)]
enum Reply {
    Simple(Vec<u8>),
    Error(Vec<u8>),
    Integer(i64),
    Bulk(Option<Vec<u8>>),
    Array(Vec<Reply>),
}

fn start(workers: Option<usize>, io_mode: IoMode) -> ServerHandle {
    Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        worker_threads: workers,
        io_mode,
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

fn read_reply(stream: &mut TcpStream) -> Reply {
    let mut prefix = [0];
    stream.read_exact(&mut prefix).unwrap();
    match prefix[0] {
        b'+' => Reply::Simple(read_line(stream)),
        b'-' => Reply::Error(read_line(stream)),
        b':' => Reply::Integer(
            std::str::from_utf8(&read_line(stream))
                .unwrap()
                .parse()
                .unwrap(),
        ),
        b'$' => {
            let size = std::str::from_utf8(&read_line(stream))
                .unwrap()
                .parse::<i64>()
                .unwrap();
            if size < 0 {
                Reply::Bulk(None)
            } else {
                let mut bytes = vec![0; size as usize + 2];
                stream.read_exact(&mut bytes).unwrap();
                assert_eq!(&bytes[size as usize..], b"\r\n");
                bytes.truncate(size as usize);
                Reply::Bulk(Some(bytes))
            }
        }
        b'*' => {
            let count = std::str::from_utf8(&read_line(stream))
                .unwrap()
                .parse::<i64>()
                .unwrap();
            Reply::Array((0..count.max(0)).map(|_| read_reply(stream)).collect())
        }
        prefix => panic!("unexpected RESP prefix {prefix:?}"),
    }
}

fn command(stream: &mut TcpStream, arguments: &[&[u8]]) -> Reply {
    let mut frame = Vec::new();
    encode_request(arguments, &mut frame);
    stream.write_all(&frame).unwrap();
    read_reply(stream)
}

fn bulk_array(reply: Reply) -> Vec<String> {
    let Reply::Array(items) = reply else {
        panic!("expected RESP array, got {reply:?}");
    };
    items
        .into_iter()
        .map(|item| match item {
            Reply::Bulk(Some(bytes)) => String::from_utf8(bytes).unwrap(),
            other => panic!("expected bulk item, got {other:?}"),
        })
        .collect()
}

struct CategoryCase {
    category: &'static [u8],
    args: &'static [&'static [u8]],
    runtime_prefix: &'static [u8],
}

const CASES: &[CategoryCase] = &[
    CategoryCase {
        category: b"all",
        args: &[b"PING"],
        runtime_prefix: b"+PONG\r\n",
    },
    CategoryCase {
        category: b"admin",
        args: &[b"LASTSAVE"],
        runtime_prefix: b":",
    },
    CategoryCase {
        category: b"bitmap",
        args: &[b"SETBIT", b"bits", b"0", b"1"],
        runtime_prefix: b":",
    },
    CategoryCase {
        category: b"blocking",
        args: &[b"WAIT", b"0", b"0"],
        runtime_prefix: b":",
    },
    CategoryCase {
        category: b"connection",
        args: &[b"PING"],
        runtime_prefix: b"+PONG\r\n",
    },
    CategoryCase {
        category: b"dangerous",
        args: &[b"INFO"],
        runtime_prefix: b"$",
    },
    CategoryCase {
        category: b"fast",
        args: &[b"PING"],
        runtime_prefix: b"+PONG\r\n",
    },
    CategoryCase {
        category: b"generic",
        args: &[b"OBJECT", b"HELP"],
        runtime_prefix: b"*",
    },
    CategoryCase {
        category: b"geo",
        args: &[b"GEOADD", b"geo", b"13", b"38", b"Rome"],
        runtime_prefix: b":",
    },
    CategoryCase {
        category: b"hash",
        args: &[b"HSET", b"hash", b"field", b"value"],
        runtime_prefix: b":",
    },
    CategoryCase {
        category: b"hyperloglog",
        args: &[b"PFADD", b"hll", b"element"],
        runtime_prefix: b":",
    },
    CategoryCase {
        category: b"keyspace",
        args: &[b"DBSIZE"],
        runtime_prefix: b":",
    },
    CategoryCase {
        category: b"list",
        args: &[b"LPUSH", b"list", b"value"],
        runtime_prefix: b":",
    },
    CategoryCase {
        category: b"pubsub",
        args: &[b"PUBSUB", b"NUMPAT"],
        runtime_prefix: b":",
    },
    CategoryCase {
        category: b"read",
        args: &[b"GET", b"missing"],
        runtime_prefix: b"$-1\r\n",
    },
    CategoryCase {
        category: b"set",
        args: &[b"SADD", b"set", b"member"],
        runtime_prefix: b":",
    },
    CategoryCase {
        category: b"slow",
        args: &[b"INFO"],
        runtime_prefix: b"$",
    },
    CategoryCase {
        category: b"sortedset",
        args: &[b"ZADD", b"sorted", b"1", b"member"],
        runtime_prefix: b":",
    },
    CategoryCase {
        category: b"stream",
        args: &[b"XADD", b"stream", b"*", b"field", b"value"],
        runtime_prefix: b"$",
    },
    CategoryCase {
        category: b"string",
        args: &[b"SET", b"string", b"value"],
        runtime_prefix: b"+OK\r\n",
    },
    CategoryCase {
        category: b"transaction",
        args: &[b"MULTI"],
        runtime_prefix: b"+OK\r\n",
    },
    CategoryCase {
        category: b"write",
        args: &[b"SET", b"write", b"value"],
        runtime_prefix: b"+OK\r\n",
    },
    CategoryCase {
        category: b"zset",
        args: &[b"ZADD", b"zset", b"1", b"member"],
        runtime_prefix: b":",
    },
];

fn set_category(admin: &mut TcpStream, category: &[u8]) {
    let mut rule = b"+@".to_vec();
    rule.extend_from_slice(category);
    assert_eq!(
        command(
            admin,
            &[
                b"ACL",
                b"SETUSER",
                b"matrix",
                b"on",
                b"nopass",
                b"allkeys",
                b"allchannels",
                b"-@all",
                &rule,
            ],
        ),
        Reply::Simple(b"OK".to_vec()),
        "category {category:?}"
    );
}

#[test]
fn acl_categories_cat_dryrun_dispatch_and_selector_precedence_match_in_every_mode() {
    let expected_categories = [
        "admin",
        "all",
        "bitmap",
        "blocking",
        "connection",
        "dangerous",
        "fast",
        "generic",
        "geo",
        "hash",
        "hyperloglog",
        "keyspace",
        "list",
        "pubsub",
        "read",
        "set",
        "slow",
        "sortedset",
        "stream",
        "string",
        "transaction",
        "write",
        "zset",
    ];
    for (workers, io_mode) in MODES {
        let server = start(workers, io_mode);
        let mut admin = connect(&server);
        let categories = bulk_array(command(&mut admin, &[b"ACL", b"CAT"]));
        assert_eq!(categories, expected_categories);
        assert_eq!(
            bulk_array(command(&mut admin, &[b"ACL", b"CAT", b"sortedset"])),
            bulk_array(command(&mut admin, &[b"ACL", b"CAT", b"zset"]))
        );
        let fast = bulk_array(command(&mut admin, &[b"ACL", b"CAT", b"fast"]));
        let slow = bulk_array(command(&mut admin, &[b"ACL", b"CAT", b"slow"]));
        for name in ["bitfield_ro", "xadd", "xdel", "time"] {
            assert!(
                fast.iter().any(|entry| entry == name),
                "@fast omitted {name}"
            );
            assert!(
                !slow.iter().any(|entry| entry == name),
                "@slow included {name}"
            );
        }
        assert!(slow.iter().any(|entry| entry == "del"));
        assert!(!fast.iter().any(|entry| entry == "del"));
        assert!(
            !bulk_array(command(&mut admin, &[b"ACL", b"CAT", b"connection"]))
                .iter()
                .any(|entry| entry == "time")
        );
        assert!(
            !bulk_array(command(&mut admin, &[b"ACL", b"CAT", b"read"]))
                .iter()
                .any(|entry| entry == "time")
        );
        let mut user = connect(&server);

        for case in CASES {
            let category_name = std::str::from_utf8(case.category).unwrap();
            let entries = bulk_array(command(&mut admin, &[b"ACL", b"CAT", case.category]));
            let filtered = bulk_array(command(
                &mut admin,
                &[b"COMMAND", b"LIST", b"FILTERBY", b"ACLCAT", case.category],
            ));
            assert_eq!(filtered, entries, "COMMAND LIST category {category_name}");
            set_category(&mut admin, case.category);
            for entry in &entries {
                let mut parts = entry.split('|');
                let command_name = parts.next().unwrap().as_bytes();
                let mut dryrun = vec![b"ACL".as_slice(), b"DRYRUN", b"matrix", command_name];
                if let Some(subcommand) = parts.next() {
                    dryrun.push(subcommand.as_bytes());
                }
                assert_eq!(
                    command(&mut admin, &dryrun),
                    Reply::Simple(b"OK".to_vec()),
                    "@{category_name} entry {entry}"
                );
            }
            let command_name = String::from_utf8_lossy(case.args[0]).to_ascii_lowercase();
            let subcommand = case
                .args
                .get(1)
                .map(|arg| String::from_utf8_lossy(arg).to_ascii_lowercase());
            let listed = if (case.args[0].eq_ignore_ascii_case(b"OBJECT")
                || case.args[0].eq_ignore_ascii_case(b"PUBSUB"))
                && let Some(subcommand) = subcommand
            {
                format!("{command_name}|{subcommand}")
            } else {
                command_name
            };
            assert!(
                entries.contains(&listed),
                "@{category_name} omitted {listed}"
            );

            let mut dryrun = vec![b"ACL".as_slice(), b"DRYRUN", b"matrix"];
            dryrun.extend_from_slice(case.args);
            assert_eq!(
                command(&mut admin, &dryrun),
                Reply::Simple(b"OK".to_vec()),
                "@{category_name} {listed}"
            );
            let runtime_reply = command(&mut user, case.args).as_bytes();
            assert!(
                runtime_reply.starts_with(case.runtime_prefix),
                "runtime @{category_name} {listed}: {runtime_reply:?}"
            );
            if case.category == b"transaction" {
                assert_eq!(
                    command(&mut user, &[b"DISCARD"]),
                    Reply::Simple(b"OK".to_vec())
                );
            }
        }

        set_category(&mut admin, b"read");
        assert!(matches!(
            command(&mut admin, &[b"ACL", b"DRYRUN", b"matrix", b"TIME"]),
            Reply::Error(_)
        ));
        set_category(&mut admin, b"connection");
        assert!(matches!(
            command(&mut admin, &[b"ACL", b"DRYRUN", b"matrix", b"TIME"]),
            Reply::Error(_)
        ));
        set_category(&mut admin, b"fast");
        assert_eq!(
            command(&mut admin, &[b"ACL", b"DRYRUN", b"matrix", b"TIME"]),
            Reply::Simple(b"OK".to_vec())
        );
        assert!(command(&mut user, &[b"TIME"]).as_bytes().starts_with(b"*"));

        // Root rules deny GET; a selector can grant it only with its own key scope.
        assert_eq!(
            command(&mut admin, &[b"SET", b"first:key", b"one"]),
            Reply::Simple(b"OK".to_vec())
        );
        assert_eq!(
            command(&mut admin, &[b"SET", b"second:key", b"two"]),
            Reply::Simple(b"OK".to_vec())
        );
        assert_eq!(
            command(
                &mut admin,
                &[
                    b"ACL",
                    b"SETUSER",
                    b"selector",
                    b"on",
                    b"nopass",
                    b"-@all",
                    b"allkeys",
                    b"(+@read -get %R~first:*)",
                    b"(+@read -get +get %R~second:*)",
                ],
            ),
            Reply::Simple(b"OK".to_vec())
        );
        assert_eq!(
            command(
                &mut admin,
                &[b"ACL", b"DRYRUN", b"selector", b"GET", b"second:key"]
            ),
            Reply::Simple(b"OK".to_vec())
        );
        assert!(matches!(
            command(
                &mut admin,
                &[b"ACL", b"DRYRUN", b"selector", b"GET", b"first:key"]
            ),
            Reply::Error(_)
        ));
        let mut selector = connect(&server);
        assert_eq!(
            command(&mut selector, &[b"AUTH", b"selector", b""]),
            Reply::Simple(b"OK".to_vec())
        );
        assert_eq!(
            command(&mut selector, &[b"GET", b"second:key"]),
            Reply::Bulk(Some(b"two".to_vec()))
        );
        assert!(matches!(
            command(&mut selector, &[b"GET", b"first:key"]),
            Reply::Error(_)
        ));
        assert!(matches!(
            command(&mut selector, &[b"GET", b"other:key"]),
            Reply::Error(_)
        ));
        assert!(matches!(
            command(&mut selector, &[b"SET", b"second:key", b"no"]),
            Reply::Error(_)
        ));
        server.shutdown().unwrap();
    }
}

trait ReplyBytes {
    fn as_bytes(&self) -> Vec<u8>;
}

impl ReplyBytes for Reply {
    fn as_bytes(&self) -> Vec<u8> {
        match self {
            Reply::Simple(bytes) => [b"+".as_slice(), bytes, b"\r\n"].concat(),
            Reply::Error(bytes) => [b"-".as_slice(), bytes, b"\r\n"].concat(),
            Reply::Integer(number) => format!(":{number}\r\n").into_bytes(),
            Reply::Bulk(Some(bytes)) => {
                [format!("${}\r\n", bytes.len()).as_bytes(), bytes, b"\r\n"].concat()
            }
            Reply::Bulk(None) => b"$-1\r\n".to_vec(),
            Reply::Array(items) => {
                let mut bytes = format!("*{}\r\n", items.len()).into_bytes();
                for item in items {
                    bytes.extend(item.as_bytes());
                }
                bytes
            }
        }
    }
}
