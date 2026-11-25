#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

fn start(mode: Option<usize>) -> ServerHandle {
    let server = Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        worker_threads: mode,
        ..Config::default()
    })
    .unwrap();
    server.spawn().unwrap()
}

fn request(stream: &mut TcpStream, args: &[&[u8]]) -> Vec<u8> {
    let mut request = Vec::new();
    encode_request(args, &mut request);
    stream.write_all(&request).unwrap();
    let mut reply = Vec::new();
    let mut remaining = 2 * 1024 * 1024;
    read_frame(stream, &mut reply, &mut remaining, 0);
    reply
}

fn read_frame(stream: &mut impl Read, output: &mut Vec<u8>, remaining: &mut usize, depth: usize) {
    assert!(depth <= 16, "RESP nesting limit exceeded");
    let mut prefix = [0];
    stream.read_exact(&mut prefix).expect("RESP frame prefix");
    push_bounded(output, remaining, &prefix);
    match prefix[0] {
        b'+' | b'-' | b':' => {
            let line = read_line(stream, output, remaining);
            if prefix[0] == b':' {
                line.parse::<i64>().expect("RESP integer");
            }
        }
        b'$' => {
            let length = read_line(stream, output, remaining)
                .parse::<i64>()
                .expect("RESP bulk length");
            if length == -1 {
                return;
            }
            assert!(
                (0..=2 * 1024 * 1024).contains(&length),
                "RESP bulk limit exceeded"
            );
            let mut body = vec![0; length as usize + 2];
            stream.read_exact(&mut body).expect("complete RESP bulk");
            assert!(body.ends_with(b"\r\n"), "RESP bulk terminator");
            push_bounded(output, remaining, &body);
        }
        b'*' => {
            let count = read_line(stream, output, remaining)
                .parse::<i64>()
                .expect("RESP array length");
            if count == -1 {
                return;
            }
            assert!((0..=1024).contains(&count), "RESP array limit exceeded");
            for _ in 0..count {
                read_frame(stream, output, remaining, depth + 1);
            }
        }
        other => panic!("unexpected RESP type {other:?}"),
    }
}

fn read_line(stream: &mut impl Read, output: &mut Vec<u8>, remaining: &mut usize) -> String {
    let mut line = Vec::new();
    loop {
        let mut byte = [0];
        stream.read_exact(&mut byte).expect("complete RESP line");
        push_bounded(output, remaining, &byte);
        if byte[0] == b'\n' {
            assert_eq!(line.pop(), Some(b'\r'), "RESP line terminator");
            return String::from_utf8(line).expect("ASCII RESP line");
        }
        assert!(line.len() < 128, "RESP line limit exceeded");
        line.push(byte[0]);
    }
}

fn push_bounded(output: &mut Vec<u8>, remaining: &mut usize, bytes: &[u8]) {
    *remaining = remaining
        .checked_sub(bytes.len())
        .expect("RESP reply limit exceeded");
    output.extend_from_slice(bytes);
}

fn connect(server: &ServerHandle) -> TcpStream {
    let stream = TcpStream::connect(server.local_addr()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
}

#[test]
fn script_and_function_flush_do_not_imply_execution_support() {
    let absent = b"-ERR unknown command\r\n";
    let unsupported = b"-ERR unsupported subcommand or syntax error\r\n";
    let cases: &[(&[&[u8]], &[u8])] = &[
        (&[b"EVAL", b"return 1", b"0"], absent),
        (
            &[
                b"EVALSHA",
                b"0000000000000000000000000000000000000000",
                b"0",
            ],
            absent,
        ),
        (&[b"FCALL", b"example", b"0"], absent),
        (&[b"FCALL_RO", b"example", b"0"], absent),
        (&[b"SCRIPT", b"LOAD", b"return 1"], unsupported),
        (
            &[
                b"SCRIPT",
                b"EXISTS",
                b"0000000000000000000000000000000000000000",
            ],
            unsupported,
        ),
        (
            &[b"FUNCTION", b"LOAD", b"#!lua name=example\n"],
            unsupported,
        ),
        (&[b"FUNCTION", b"LIST"], unsupported),
    ];
    for mode in [None, Some(1), Some(4)] {
        let server = start(mode);
        let mut client = connect(&server);
        assert_eq!(request(&mut client, &[b"SET", b"key", b"v\0"]), b"+OK\r\n");
        assert_eq!(
            request(&mut client, &[b"SCRIPT", b"FLUSH", b"SYNC"]),
            b"+OK\r\n"
        );
        assert_eq!(
            request(&mut client, &[b"FUNCTION", b"FLUSH", b"ASYNC"]),
            b"+OK\r\n"
        );
        assert_eq!(
            request(
                &mut client,
                &[
                    b"COMMAND",
                    b"INFO",
                    b"EVAL",
                    b"EVALSHA",
                    b"FCALL",
                    b"FCALL_RO"
                ]
            ),
            b"*4\r\n*-1\r\n*-1\r\n*-1\r\n*-1\r\n"
        );
        for &(args, expected) in cases {
            assert_eq!(request(&mut client, args), expected, "{args:?}");
        }
        assert_eq!(request(&mut client, &[b"GET", b"key"]), b"$2\r\nv\0\r\n");
        server.shutdown().unwrap();
    }
}

#[test]
fn topology_probes_stay_standalone_after_writes_and_rejected_reconfiguration() {
    let role = b"*3\r\n$6\r\nmaster\r\n:0\r\n*0\r\n";
    let replication = "# Replication\r\nrole:master\r\nconnected_slaves:0\r\nmaster_failover_state:no-failover\r\nmaster_repl_offset:0\r\n";
    let info = format!("${}\r\n{replication}\r\n", replication.len());
    for mode in [None, Some(1), Some(4)] {
        let server = start(mode);
        let mut client = connect(&server);
        assert_eq!(request(&mut client, &[b"ROLE"]), role);
        assert_eq!(
            request(&mut client, &[b"INFO", b"replication"]),
            info.as_bytes()
        );
        assert_eq!(request(&mut client, &[b"SET", b"key", b"v\0"]), b"+OK\r\n");
        for subcommand in [b"SHARDS".as_slice(), b"SLOTS", b"NODES"] {
            assert_eq!(
                request(&mut client, &[b"CLUSTER", subcommand]),
                b"-ERR This instance has cluster support disabled\r\n",
                "{subcommand:?}"
            );
        }
        assert_eq!(
            request(&mut client, &[b"REPLICAOF", b"127.0.0.1", b"1"]),
            b"-ERR replication is not supported by this standalone server\r\n"
        );
        assert_eq!(
            request(&mut client, &[b"REPLICAOF", b"NO", b"ONE"]),
            b"+OK\r\n"
        );
        assert_eq!(request(&mut client, &[b"ROLE"]), role);
        assert_eq!(
            request(&mut client, &[b"INFO", b"replication"]),
            info.as_bytes()
        );
        assert_eq!(request(&mut client, &[b"GET", b"key"]), b"$2\r\nv\0\r\n");
        server.shutdown().unwrap();
    }
}

#[test]
fn command_discovery_has_sorted_truthful_resp2_metadata() {
    for mode in [None, Some(1), Some(4)] {
        let server = start(mode);
        let mut client = connect(&server);
        assert_eq!(request(&mut client, &[b"COMMAND", b"COUNT"]), b":245\r\n");
        let all_info = request(&mut client, &[b"COMMAND", b"INFO"]);
        assert!(all_info.starts_with(b"*245\r\n*6\r\n$3\r\nacl\r\n"));
        assert_eq!(
            request(&mut client, &[b"COMMAND", b"INFO", b"MSETEX", b"DIGEST", b"DELIFEQ"]),
            b"*3\r\n*6\r\n$6\r\nmsetex\r\n:-4\r\n*2\r\n+write\r\n+denyoom\r\n:2\r\n:-1\r\n:2\r\n*6\r\n$6\r\ndigest\r\n:2\r\n*2\r\n+readonly\r\n+fast\r\n:1\r\n:1\r\n:1\r\n*6\r\n$7\r\ndelifeq\r\n:3\r\n*2\r\n+write\r\n+fast\r\n:1\r\n:1\r\n:1\r\n"
        );
        assert_eq!(
            request(&mut client, &[b"COMMAND", b"INFO", b"gEt", b"does-not-exist", b"DEL"]),
            b"*3\r\n*6\r\n$3\r\nget\r\n:2\r\n*2\r\n+readonly\r\n+fast\r\n:1\r\n:1\r\n:1\r\n*-1\r\n*6\r\n$3\r\ndel\r\n:-2\r\n*1\r\n+write\r\n:1\r\n:-1\r\n:1\r\n"
        );
        let expected_names: &[&[u8]] = &[
            b"acl",
            b"append",
            b"asking",
            b"auth",
            b"bgrewriteaof",
            b"bgsave",
            b"bitcount",
            b"bitfield",
            b"bitfield_ro",
            b"bitop",
            b"bitpos",
            b"blmove",
            b"blmpop",
            b"blpop",
            b"brpop",
            b"brpoplpush",
            b"bzmpop",
            b"bzpopmax",
            b"bzpopmin",
            b"client",
            b"cluster",
            b"command",
            b"config",
            b"copy",
            b"dbsize",
            b"decr",
            b"decrby",
            b"del",
            b"delex",
            b"delifeq",
            b"digest",
            b"discard",
            b"echo",
            b"exec",
            b"exists",
            b"expire",
            b"expireat",
            b"expiretime",
            b"failover",
            b"flushall",
            b"flushdb",
            b"function",
            b"geoadd",
            b"geodist",
            b"geohash",
            b"geopos",
            b"georadius",
            b"georadius_ro",
            b"georadiusbymember",
            b"georadiusbymember_ro",
            b"geosearch",
            b"geosearchstore",
            b"get",
            b"getbit",
            b"getdel",
            b"getex",
            b"getrange",
            b"getset",
            b"hdel",
            b"hello",
            b"hexists",
            b"hexpire",
            b"hexpireat",
            b"hexpiretime",
            b"hget",
            b"hgetall",
            b"hgetdel",
            b"hgetex",
            b"hincrby",
            b"hincrbyfloat",
            b"hkeys",
            b"hlen",
            b"hmget",
            b"hmset",
            b"hpersist",
            b"hpexpire",
            b"hpexpireat",
            b"hpexpiretime",
            b"hpttl",
            b"hrandfield",
            b"hscan",
            b"hset",
            b"hsetex",
            b"hsetnx",
            b"hstrlen",
            b"httl",
            b"hvals",
            b"incr",
            b"incrby",
            b"incrbyfloat",
            b"info",
            b"keys",
            b"lastsave",
            b"latency",
            b"lcs",
            b"lindex",
            b"linsert",
            b"llen",
            b"lmove",
            b"lmpop",
            b"lpop",
            b"lpos",
            b"lpush",
            b"lpushx",
            b"lrange",
            b"lrem",
            b"lset",
            b"ltrim",
            b"memory",
            b"mget",
            b"module",
            b"monitor",
            b"move",
            b"mset",
            b"msetex",
            b"msetnx",
            b"multi",
            b"object",
            b"persist",
            b"pexpire",
            b"pexpireat",
            b"pexpiretime",
            b"pfadd",
            b"pfcount",
            b"pfdebug",
            b"pfmerge",
            b"pfselftest",
            b"ping",
            b"psetex",
            b"psubscribe",
            b"psync",
            b"pttl",
            b"publish",
            b"pubsub",
            b"punsubscribe",
            b"quit",
            b"randomkey",
            b"readonly",
            b"readwrite",
            b"rename",
            b"renamenx",
            b"replconf",
            b"replicaof",
            b"reset",
            b"role",
            b"rpop",
            b"rpoplpush",
            b"rpush",
            b"rpushx",
            b"sadd",
            b"save",
            b"scan",
            b"scard",
            b"script",
            b"sdiff",
            b"sdiffcard",
            b"sdiffstore",
            b"select",
            b"set",
            b"setbit",
            b"setex",
            b"setnx",
            b"setrange",
            b"shutdown",
            b"sinter",
            b"sintercard",
            b"sinterstore",
            b"sismember",
            b"slaveof",
            b"slowlog",
            b"smembers",
            b"smismember",
            b"smove",
            b"sort",
            b"sort_ro",
            b"spop",
            b"spublish",
            b"srandmember",
            b"srem",
            b"sscan",
            b"ssubscribe",
            b"strlen",
            b"subscribe",
            b"substr",
            b"sunion",
            b"sunioncard",
            b"sunionstore",
            b"sunsubscribe",
            b"swapdb",
            b"sync",
            b"time",
            b"touch",
            b"ttl",
            b"type",
            b"unlink",
            b"unsubscribe",
            b"unwatch",
            b"wait",
            b"waitaof",
            b"watch",
            b"xack",
            b"xadd",
            b"xautoclaim",
            b"xclaim",
            b"xdel",
            b"xgroup",
            b"xinfo",
            b"xlen",
            b"xpending",
            b"xrange",
            b"xread",
            b"xreadgroup",
            b"xrevrange",
            b"xsetid",
            b"xtrim",
            b"zadd",
            b"zcard",
            b"zcount",
            b"zdiff",
            b"zdiffstore",
            b"zincrby",
            b"zinter",
            b"zintercard",
            b"zinterstore",
            b"zlexcount",
            b"zmpop",
            b"zmscore",
            b"zpopmax",
            b"zpopmin",
            b"zrandmember",
            b"zrange",
            b"zrangebylex",
            b"zrangebyscore",
            b"zrangestore",
            b"zrank",
            b"zrem",
            b"zremrangebylex",
            b"zremrangebyrank",
            b"zremrangebyscore",
            b"zrevrange",
            b"zrevrangebylex",
            b"zrevrangebyscore",
            b"zrevrank",
            b"zscan",
            b"zscore",
            b"zunion",
            b"zunionstore",
        ];
        let mut expected_list = Vec::new();
        write!(&mut expected_list, "*{}\r\n", expected_names.len()).unwrap();
        for name in expected_names {
            write!(&mut expected_list, "${}\r\n", name.len()).unwrap();
            expected_list.extend_from_slice(name);
            expected_list.extend_from_slice(b"\r\n");
        }
        assert_eq!(request(&mut client, &[b"COMMAND", b"LIST"]), expected_list);
        assert_eq!(request(&mut client, &[b"COMMAND"]), all_info);

        // Rebuild the complete reply from bounded, targeted lookups. Uppercase
        // names and reversed order distinguish lookup from just
        // returning the registry in its sorted order.
        let mut metadata = Vec::new();
        write!(&mut metadata, "*{}\r\n", expected_names.len()).unwrap();
        for names in expected_names.chunks(24) {
            let upper: Vec<_> = names
                .iter()
                .rev()
                .map(|name| name.to_ascii_uppercase())
                .collect();
            let mut args: Vec<&[u8]> = vec![b"COMMAND", b"INFO"];
            args.extend(upper.iter().map(Vec::as_slice));
            let reply = request(&mut client, &args);
            let header = format!("*{}\r\n", names.len());
            let mut remaining = reply
                .strip_prefix(header.as_bytes())
                .expect("targeted INFO array");
            let mut rows = Vec::new();
            for name in names.iter().rev() {
                let mut row = Vec::new();
                let mut budget = remaining.len();
                read_frame(&mut remaining, &mut row, &mut budget, 0);
                let expected = format!(
                    "*6\r\n${}\r\n{}\r\n",
                    name.len(),
                    std::str::from_utf8(name).unwrap()
                );
                assert!(
                    row.starts_with(expected.as_bytes()),
                    "incorrect metadata row for {}",
                    String::from_utf8_lossy(name)
                );
                rows.push(row);
            }
            assert!(remaining.is_empty(), "unexpected metadata after batch");
            for row in rows.into_iter().rev() {
                metadata.extend(row);
            }
        }
        assert_eq!(metadata, all_info);
        let info = request(&mut client, &[b"INFO"]);
        let text = std::str::from_utf8(&info).unwrap();
        assert_eq!(info.first(), Some(&b'$'));
        assert!(text.contains("rillcache_version:0.1.0"));
        assert!(text.contains("rillcache_payload_scope:key_and_value_bytes_not_RSS"));
        assert!(text.contains("db0:keys=0,expires=0"));
        assert!(!text.contains("redis_version:"));
        let rss = text
            .lines()
            .find_map(|line| line.strip_prefix("used_memory_rss:"))
            .expect("INFO memory reports process RSS");
        assert!(rss.parse::<u64>().is_ok(), "{rss:?}");
        assert!(text.contains("used_memory_rss_human:"));
        assert!(text.contains("used_memory_rss_scope:"));
        if text.contains("used_memory_cgroup:") {
            assert!(text.contains("used_memory_cgroup_human:"));
            assert!(text.contains("max_memory_cgroup:"));
            assert!(text.contains("max_memory_cgroup_human:"));
            assert!(text.contains("used_memory_cgroup_scope:"));
        }
        server.shutdown().unwrap();
    }
}

#[test]
fn info_payload_headroom_tracks_admission_and_deletion() {
    for mode in [None, Some(1), Some(4)] {
        let server = Server::bind(Config {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            worker_threads: mode,
            max_data_bytes: 8,
            ..Config::default()
        })
        .unwrap()
        .spawn()
        .unwrap();
        let mut client = connect(&server);
        let check = |client: &mut TcpStream, section: &[u8], used, remaining| {
            let info = request(client, &[b"INFO", section]);
            let text = std::str::from_utf8(&info).unwrap();
            for expected in [
                format!("rillcache_payload_bytes:{used}"),
                "rillcache_payload_limit_bytes:8".to_owned(),
                format!("rillcache_payload_headroom_bytes:{remaining}"),
                "rillcache_payload_scope:key_and_value_bytes_not_RSS".to_owned(),
            ] {
                assert!(text.lines().any(|line| line == expected), "{text}");
            }
        };
        check(&mut client, b"memory", 0, 8);
        assert_eq!(
            request(&mut client, &[b"SET", b"x", b"1234567"]),
            b"+OK\r\n"
        );
        check(&mut client, b"memory", 8, 0);
        assert_eq!(
            request(&mut client, &[b"SET", b"y", b"v"]),
            b"-OOM configured payload limit exceeded\r\n"
        );
        check(&mut client, b"all", 8, 0);
        assert_eq!(request(&mut client, &[b"DEL", b"x"]), b":1\r\n");
        check(&mut client, b"memory", 0, 8);
        assert_eq!(
            request(
                &mut client,
                &[b"CONFIG", b"GET", b"MAX-DATA-BYTES", b"maxmemory"]
            ),
            b"*2\r\n$9\r\nmaxmemory\r\n$1\r\n8\r\n"
        );
        assert_eq!(
            request(&mut client, &[
                b"CONFIG", b"SET", b"maxmemory", b"16", b"appendonly", b"yes",
            ]),
            b"-ERR appendonly and appendfilename are startup-only; restart with the desired options\r\n"
        );
        check(&mut client, b"memory", 0, 8);
        assert_eq!(
            request(&mut client, &[b"CONFIG", b"SET", b"maxmemory", b"16"]),
            b"+OK\r\n"
        );
        assert_eq!(
            request(&mut client, &[b"CONFIG", b"GET", b"max-data-bytes"]),
            b"*2\r\n$9\r\nmaxmemory\r\n$2\r\n16\r\n"
        );
        server.shutdown().unwrap();
    }
}

#[test]
fn info_reports_payload_and_live_keyspace_and_rejects_unknown_sections() {
    for mode in [None, Some(1), Some(4)] {
        let server = start(mode);
        let mut client = connect(&server);
        assert_eq!(request(&mut client, &[b"SET", b"k", b"abc"]), b"+OK\r\n");
        let memory = request(&mut client, &[b"INFO", b"MEMORY"]);
        assert!(
            std::str::from_utf8(&memory)
                .unwrap()
                .contains("rillcache_payload_bytes:4\r\n")
        );
        let keyspace = request(&mut client, &[b"INFO", b"KEYSPACE"]);
        assert!(
            std::str::from_utf8(&keyspace)
                .unwrap()
                .contains("db0:keys=1,expires=0,avg_ttl=0,subexpiry=0\r\n")
        );
        assert_eq!(
            request(&mut client, &[b"INFO", b"anything\0SECRET"]),
            b"-ERR unsupported INFO section\r\n"
        );
        assert_eq!(
            request(&mut client, &[b"INFO", b"SERVER", b"MEMORY"]),
            b"-ERR wrong number of arguments for 'info' command\r\n"
        );
        server.shutdown().unwrap();
    }
}

#[test]
fn unsupported_command_options_are_rejected_without_reflecting_input() {
    for mode in [None, Some(1), Some(4)] {
        let server = start(mode);
        let mut client = connect(&server);
        assert_eq!(
            request(
                &mut client,
                &[
                    b"COMMAND",
                    b"LIST",
                    b"FILTERBY",
                    b"MODULE",
                    b"secret\0bytes"
                ]
            ),
            b"*0\r\n"
        );
        assert_eq!(
            request(&mut client, &[b"COMMAND", b"LIST", b"FILTERBY", b"PATTERN"]),
            b"-ERR syntax error\r\n"
        );
        assert_eq!(
            request(
                &mut client,
                &[b"COMMAND", b"LIST", b"FILTERBY", b"BOGUS", b"x"]
            ),
            b"-ERR syntax error\r\n"
        );
        assert_eq!(
            request(&mut client, &[b"COMMAND", b"UNKNOWN", b"secret\0bytes"]),
            b"-ERR unsupported COMMAND subcommand\r\n"
        );
        assert_eq!(
            request(&mut client, &[b"COMMAND", b"COUNT", b"secret"]),
            b"-ERR wrong number of arguments for 'command|count' command\r\n"
        );
        assert_eq!(request(&mut client, &[b"PING"]), b"+PONG\r\n");
        server.shutdown().unwrap();
    }
}

#[test]
fn command_getkeys_reuses_acl_key_specs() {
    for mode in [None, Some(1), Some(4)] {
        let server = start(mode);
        let mut client = connect(&server);
        assert_eq!(
            request(&mut client, &[b"COMMAND", b"GETKEYS", b"GET", b"k"]),
            b"*1\r\n$1\r\nk\r\n"
        );
        assert_eq!(
            request(
                &mut client,
                &[b"COMMAND", b"GETKEYS", b"MSET", b"a", b"1", b"b", b"2"]
            ),
            b"*2\r\n$1\r\na\r\n$1\r\nb\r\n"
        );
        assert_eq!(
            request(
                &mut client,
                &[
                    b"COMMAND", b"GETKEYS", b"MSETEX", b"2", b"a", b"1", b"b", b"2", b"EX", b"60",
                ]
            ),
            b"*2\r\n$1\r\na\r\n$1\r\nb\r\n"
        );
        assert_eq!(
            request(
                &mut client,
                &[
                    b"COMMAND", b"GETKEYS", b"SORT", b"src", b"LIMIT", b"0", b"1", b"STORE",
                    b"out",
                ]
            ),
            b"*2\r\n$3\r\nsrc\r\n$3\r\nout\r\n"
        );
        assert_eq!(
            request(
                &mut client,
                &[
                    b"COMMAND", b"GETKEYS", b"ZUNION", b"2", b"a", b"b", b"WEIGHTS", b"1", b"2",
                ]
            ),
            b"*2\r\n$1\r\na\r\n$1\r\nb\r\n"
        );
        assert_eq!(
            request(
                &mut client,
                &[
                    b"COMMAND",
                    b"GETKEYS",
                    b"SUNIONCARD",
                    b"2",
                    b"a",
                    b"b",
                    b"APPROX",
                    b"LIMIT",
                    b"1",
                ]
            ),
            b"*2\r\n$1\r\na\r\n$1\r\nb\r\n"
        );
        assert_eq!(
            request(
                &mut client,
                &[
                    b"COMMAND",
                    b"GETKEYS",
                    b"SDIFFCARD",
                    b"2",
                    b"a",
                    b"b",
                    b"LIMIT",
                    b"1",
                ]
            ),
            b"*2\r\n$1\r\na\r\n$1\r\nb\r\n"
        );
        assert_eq!(
            request(
                &mut client,
                &[
                    b"COMMAND", b"GETKEYS", b"BLMPOP", b"0", b"2", b"a", b"b", b"LEFT"
                ]
            ),
            b"*2\r\n$1\r\na\r\n$1\r\nb\r\n"
        );
        assert_eq!(
            request(
                &mut client,
                &[b"COMMAND", b"GETKEYS", b"LMPOP", b"2", b"a", b"b", b"LEFT"]
            ),
            b"*2\r\n$1\r\na\r\n$1\r\nb\r\n"
        );
        assert_eq!(
            request(
                &mut client,
                &[
                    b"COMMAND", b"GETKEYS", b"LMPOP", b"2", b"a", b"b", b"RIGHT", b"COUNT", b"1",
                ]
            ),
            b"*2\r\n$1\r\na\r\n$1\r\nb\r\n"
        );
        assert_eq!(
            request(
                &mut client,
                &[
                    b"COMMAND",
                    b"GETKEYS",
                    b"GEORADIUS",
                    b"geo",
                    b"0",
                    b"0",
                    b"1",
                    b"km",
                    b"STORE",
                    b"dest",
                ]
            ),
            b"*2\r\n$3\r\ngeo\r\n$4\r\ndest\r\n"
        );
        assert_eq!(
            request(&mut client, &[b"COMMAND", b"GETKEYS", b"KEYS", b"*"]),
            b"-ERR The command has no key arguments\r\n"
        );
        assert_eq!(
            request(&mut client, &[b"COMMAND", b"GETKEYS", b"COMMAND"]),
            b"-ERR The command has no key arguments\r\n"
        );
        assert_eq!(
            request(&mut client, &[b"COMMAND", b"GETKEYS", b"BOGUS"]),
            b"-ERR Invalid command specified\r\n"
        );
        assert_eq!(
            request(&mut client, &[b"COMMAND", b"GETKEYS", b"GET"]),
            b"-ERR Invalid number of arguments specified for command\r\n"
        );
        assert_eq!(
            request(&mut client, &[b"COMMAND", b"GETKEYS"]),
            b"-ERR wrong number of arguments for 'command|getkeys' command\r\n"
        );
        server.shutdown().unwrap();
    }
}

#[test]
fn command_getkeysandflags_reports_acl_access_classes() {
    for mode in [None, Some(1), Some(4)] {
        let server = start(mode);
        let mut client = connect(&server);
        assert_eq!(
            request(&mut client, &[b"COMMAND", b"GETKEYSANDFLAGS", b"GET", b"k"]),
            b"*1\r\n*2\r\n$1\r\nk\r\n*1\r\n+RO\r\n"
        );
        assert_eq!(
            request(
                &mut client,
                &[b"COMMAND", b"GETKEYSANDFLAGS", b"SET", b"k", b"v"]
            ),
            b"*1\r\n*2\r\n$1\r\nk\r\n*1\r\n+RW\r\n"
        );
        assert_eq!(
            request(
                &mut client,
                &[b"COMMAND", b"GETKEYSANDFLAGS", b"MGET", b"a", b"b"]
            ),
            b"*2\r\n*2\r\n$1\r\na\r\n*1\r\n+RO\r\n*2\r\n$1\r\nb\r\n*1\r\n+RO\r\n"
        );
        assert_eq!(
            request(
                &mut client,
                &[
                    b"COMMAND",
                    b"getkeysandflags",
                    b"MSET",
                    b"a",
                    b"1",
                    b"b",
                    b"2",
                ]
            ),
            b"*2\r\n*2\r\n$1\r\na\r\n*1\r\n+RW\r\n*2\r\n$1\r\nb\r\n*1\r\n+RW\r\n"
        );
        assert_eq!(
            request(
                &mut client,
                &[
                    b"COMMAND",
                    b"GETKEYSANDFLAGS",
                    b"SORT",
                    b"src",
                    b"LIMIT",
                    b"0",
                    b"1",
                    b"STORE",
                    b"out",
                ]
            ),
            b"*2\r\n*2\r\n$3\r\nsrc\r\n*1\r\n+RO\r\n*2\r\n$3\r\nout\r\n*1\r\n+RW\r\n"
        );
        assert_eq!(
            request(
                &mut client,
                &[b"COMMAND", b"GETKEYSANDFLAGS", b"COPY", b"src", b"dst"]
            ),
            b"*2\r\n*2\r\n$3\r\nsrc\r\n*1\r\n+RO\r\n*2\r\n$3\r\ndst\r\n*1\r\n+RW\r\n"
        );
        assert_eq!(
            request(
                &mut client,
                &[b"COMMAND", b"GETKEYSANDFLAGS", b"KEYS", b"*"]
            ),
            b"-ERR The command has no key arguments\r\n"
        );
        assert_eq!(
            request(&mut client, &[b"COMMAND", b"GETKEYSANDFLAGS", b"BOGUS"]),
            b"-ERR Invalid command specified\r\n"
        );
        assert_eq!(
            request(&mut client, &[b"COMMAND", b"GETKEYSANDFLAGS", b"GET"]),
            b"-ERR Invalid number of arguments specified for command\r\n"
        );
        assert_eq!(
            request(&mut client, &[b"COMMAND", b"GETKEYSANDFLAGS"]),
            b"-ERR wrong number of arguments for 'command|getkeysandflags' command\r\n"
        );
        assert_eq!(request(&mut client, &[b"PING"]), b"+PONG\r\n");
        server.shutdown().unwrap();
    }
}

#[test]
fn command_list_filters_by_pattern_category_and_module() {
    for mode in [None, Some(1), Some(4)] {
        let server = start(mode);
        let mut client = connect(&server);
        assert_eq!(
            request(
                &mut client,
                &[b"COMMAND", b"LIST", b"FILTERBY", b"PATTERN", b"set*"]
            ),
            b"*5\r\n$3\r\nset\r\n$6\r\nsetbit\r\n$5\r\nsetex\r\n$5\r\nsetnx\r\n$8\r\nsetrange\r\n"
        );
        let read = request(
            &mut client,
            &[b"COMMAND", b"LIST", b"FILTERBY", b"ACLCAT", b"read"],
        );
        assert!(read.starts_with(b"*"));
        assert!(read.windows(4).any(|window| window == b"get\r"));
        assert!(!read.windows(3).any(|window| window == b"set"));
        assert_eq!(
            request(
                &mut client,
                &[b"COMMAND", b"LIST", b"FILTERBY", b"ACLCAT", b"@read"]
            ),
            read
        );
        let connection = String::from_utf8(request(
            &mut client,
            &[b"COMMAND", b"LIST", b"FILTERBY", b"ACLCAT", b"@connection"],
        ))
        .unwrap();
        assert!(connection.contains("client|id\r\n"), "{connection}");
        assert!(connection.contains("command|count\r\n"), "{connection}");
        assert!(connection.contains("client|kill\r\n"), "{connection}");
        let admin = String::from_utf8(request(
            &mut client,
            &[b"COMMAND", b"LIST", b"FILTERBY", b"ACLCAT", b"@admin"],
        ))
        .unwrap();
        assert!(admin.contains("client|kill\r\n"), "{admin}");
        assert!(admin.contains("acl|users\r\n"), "{admin}");
        assert!(!admin.contains("acl|cat\r\n"), "{admin}");
        assert!(admin.contains("config|get\r\n"), "{admin}");
        assert!(admin.contains("latency|latest\r\n"), "{admin}");
        assert!(admin.contains("slowlog|get\r\n"), "{admin}");
        assert!(!admin.contains("latency|help\r\n"), "{admin}");
        assert!(!admin.contains("slowlog|help\r\n"), "{admin}");
        assert!(!admin.contains("pubsub|channels\r\n"), "{admin}");
        let dangerous = String::from_utf8(request(
            &mut client,
            &[b"COMMAND", b"LIST", b"FILTERBY", b"ACLCAT", b"@dangerous"],
        ))
        .unwrap();
        assert!(dangerous.contains("config|resetstat\r\n"), "{dangerous}");
        assert!(dangerous.contains("latency|doctor\r\n"), "{dangerous}");
        assert!(dangerous.contains("slowlog|len\r\n"), "{dangerous}");
        assert!(!dangerous.contains("latency|help\r\n"), "{dangerous}");
        assert!(!dangerous.contains("slowlog|help\r\n"), "{dangerous}");
        let slow = String::from_utf8(request(
            &mut client,
            &[b"COMMAND", b"LIST", b"FILTERBY", b"ACLCAT", b"@slow"],
        ))
        .unwrap();
        assert!(slow.contains("latency|help\r\n"), "{slow}");
        assert!(slow.contains("slowlog|help\r\n"), "{slow}");
        assert!(slow.contains("pubsub|channels\r\n"), "{slow}");
        let pubsub = String::from_utf8(request(
            &mut client,
            &[b"COMMAND", b"LIST", b"FILTERBY", b"ACLCAT", b"@pubsub"],
        ))
        .unwrap();
        assert!(pubsub.contains("pubsub|channels\r\n"), "{pubsub}");
        assert!(pubsub.contains("pubsub|shardnumsub\r\n"), "{pubsub}");
        assert!(!pubsub.contains("pubsub|help\r\n"), "{pubsub}");
        let fast = String::from_utf8(request(
            &mut client,
            &[b"COMMAND", b"LIST", b"FILTERBY", b"ACLCAT", b"@fast"],
        ))
        .unwrap();
        assert!(!fast.contains("config|get\r\n"), "{fast}");
        assert!(!fast.contains("pubsub|channels\r\n"), "{fast}");
        let memory_read = String::from_utf8(request(
            &mut client,
            &[b"COMMAND", b"LIST", b"FILTERBY", b"ACLCAT", b"@read"],
        ))
        .unwrap();
        assert!(memory_read.contains("memory|usage\r\n"), "{memory_read}");
        assert!(memory_read.contains("xinfo|stream\r\n"), "{memory_read}");
        assert!(!memory_read.contains("memory|stats\r\n"), "{memory_read}");
        assert_eq!(
            request(
                &mut client,
                &[b"COMMAND", b"LIST", b"FILTERBY", b"ACLCAT", b"nosuch"]
            ),
            b"*0\r\n"
        );
        assert_eq!(
            request(
                &mut client,
                &[b"COMMAND", b"LIST", b"FILTERBY", b"MODULE", b"missing"]
            ),
            b"*0\r\n"
        );
        assert_eq!(
            request(
                &mut client,
                &[
                    b"COMMAND",
                    b"LIST",
                    b"FILTERBY",
                    b"PATTERN",
                    b"x",
                    b"FILTERBY",
                    b"PATTERN",
                    b"y"
                ]
            ),
            b"-ERR syntax error\r\n"
        );
        server.shutdown().unwrap();
    }
}
