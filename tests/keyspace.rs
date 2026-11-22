#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

fn start(workers: Option<usize>) -> ServerHandle {
    let config = Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        worker_threads: workers,
        ..Config::default()
    };
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
    stream
}

fn command(stream: &mut TcpStream, args: &[&[u8]], expected: &[u8]) {
    let mut request = Vec::new();
    encode_request(args, &mut request);
    stream.write_all(&request).unwrap();
    let mut reply = vec![0; expected.len()];
    stream.read_exact(&mut reply).unwrap();
    assert_eq!(reply, expected, "{args:?}");
}

#[test]
fn database_zero_rejections_leave_the_shared_keyspace_unchanged() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut default_client = connect(&server);
        let mut selecting_client = connect(&server);
        let source = b"src\0";
        let destination = b"dst\xff";

        command(&mut selecting_client, &[b"SELECT", b"0"], b"+OK\r\n");
        command(&mut default_client, &[b"SET", source, b"v\0"], b"+OK\r\n");
        command(
            &mut default_client,
            &[b"SET", destination, b"old"],
            b"+OK\r\n",
        );

        let cases: &[(&[&[u8]], &[u8])] = &[
            (&[b"SELECT", b"1"], b"-ERR DB index is out of range\r\n"),
            (&[b"SELECT", b"0\0"], b"-ERR invalid DB index\r\n"),
            (
                &[b"COPY", source, destination, b"DB", b"1", b"REPLACE"],
                b"-ERR DB index is out of range\r\n",
            ),
            (
                &[b"MOVE", source, b"1"],
                b"-ERR this server supports only database 0\r\n",
            ),
            (
                &[b"SWAPDB", b"0", b"1"],
                b"-ERR this server supports only database 0\r\n",
            ),
        ];
        for &(args, expected) in cases {
            command(&mut selecting_client, args, expected);
            for client in [&mut selecting_client, &mut default_client] {
                command(client, &[b"GET", source], b"$2\r\nv\0\r\n");
                command(client, &[b"GET", destination], b"$3\r\nold\r\n");
                command(client, &[b"DBSIZE"], b":2\r\n");
            }
        }

        command(
            &mut selecting_client,
            &[b"COPY", source, destination, b"DB", b"0", b"REPLACE"],
            b":1\r\n",
        );
        command(
            &mut default_client,
            &[b"GET", destination],
            b"$2\r\nv\0\r\n",
        );
        command(&mut selecting_client, &[b"FLUSHDB", b"SYNC"], b"+OK\r\n");
        command(&mut default_client, &[b"DBSIZE"], b":0\r\n");
        server.shutdown().unwrap();
    }
}

#[test]
fn copy_replace_budget_refusal_preserves_values_and_expiry_over_tcp() {
    for workers in [None, Some(1), Some(4)] {
        let server = Server::bind(Config {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            worker_threads: workers,
            max_data_bytes: 20,
            ..Config::default()
        })
        .unwrap()
        .spawn()
        .unwrap();
        let mut writer = connect(&server);
        let mut observer = connect(&server);
        let source = b"src\0";
        let destination = b"dst\xff";
        command(
            &mut writer,
            &[b"SET", source, b"source\0\xff", b"PXAT", b"4102444800000"],
            b"+OK\r\n",
        );
        command(
            &mut writer,
            &[b"SET", destination, b"old\0", b"PXAT", b"4102444900000"],
            b"+OK\r\n",
        );
        command(
            &mut writer,
            &[b"COPY", source, destination, b"REPLACE"],
            b"-OOM configured payload limit exceeded\r\n",
        );
        command(&mut observer, &[b"GET", source], b"$8\r\nsource\0\xff\r\n");
        command(&mut observer, &[b"GET", destination], b"$4\r\nold\0\r\n");
        command(
            &mut observer,
            &[b"PEXPIRETIME", source],
            b":4102444800000\r\n",
        );
        command(
            &mut observer,
            &[b"PEXPIRETIME", destination],
            b":4102444900000\r\n",
        );
        command(&mut observer, &[b"DBSIZE"], b":2\r\n");

        // Two four-byte keys and two eight-byte values fit the new budget exactly.
        command(
            &mut writer,
            &[b"CONFIG", b"SET", b"maxmemory", b"24"],
            b"+OK\r\n",
        );
        command(
            &mut writer,
            &[b"COPY", source, destination, b"REPLACE"],
            b":1\r\n",
        );
        for key in [source, destination] {
            command(&mut observer, &[b"GET", key], b"$8\r\nsource\0\xff\r\n");
            command(&mut observer, &[b"PEXPIRETIME", key], b":4102444800000\r\n");
        }
        command(&mut observer, &[b"DBSIZE"], b":2\r\n");
        server.shutdown().unwrap();
    }
}

#[test]
fn keyspace_commands_work_over_tcp_in_all_execution_modes() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers);
        let mut stream = connect(&server);

        command(&mut stream, &[b"TYPE", b"absent"], b"+none\r\n");
        command(&mut stream, &[b"RANDOMKEY"], b"$-1\r\n");
        command(
            &mut stream,
            &[b"CONFIG", b"GET", b"list-max-listpack-size"],
            b"*2\r\n$22\r\nlist-max-listpack-size\r\n$2\r\n-2\r\n",
        );
        command(
            &mut stream,
            &[b"CONFIG", b"SET", b"hash-max-ziplist-entries", b"0"],
            b"+OK\r\n",
        );
        command(
            &mut stream,
            &[b"CONFIG", b"GET", b"hash-max-listpack-entries"],
            b"*2\r\n$25\r\nhash-max-listpack-entries\r\n$1\r\n0\r\n",
        );
        command(
            &mut stream,
            &[b"CONFIG", b"SET", b"hash-max-listpack-entries", b"512"],
            b"+OK\r\n",
        );
        command(&mut stream, &[b"SET", b"a\0\xff", b"v\0"], b"+OK\r\n");
        command(&mut stream, &[b"TYPE", b"a\0\xff"], b"+string\r\n");
        command(
            &mut stream,
            &[b"TOUCH", b"a\0\xff", b"a\0\xff", b"missing"],
            b":2\r\n",
        );
        command(&mut stream, &[b"RENAME", b"a\0\xff", b"b\xff"], b"+OK\r\n");
        command(&mut stream, &[b"GET", b"b\xff"], b"$2\r\nv\0\r\n");
        command(&mut stream, &[b"COPY", b"b\xff", b"binary-copy"], b":1\r\n");
        command(&mut stream, &[b"SET", b"b\xff", b"changed"], b"+OK\r\n");
        command(&mut stream, &[b"GET", b"binary-copy"], b"$2\r\nv\0\r\n");
        command(&mut stream, &[b"SET", b"glob:a", b"1"], b"+OK\r\n");
        command(&mut stream, &[b"SET", b"glob:b", b"2"], b"+OK\r\n");
        command(
            &mut stream,
            &[b"KEYS", b"glob:?"],
            b"*2\r\n$6\r\nglob:a\r\n$6\r\nglob:b\r\n",
        );
        command(&mut stream, &[b"RENAMENX", b"b\xff", b"c"], b":1\r\n");
        command(&mut stream, &[b"SET", b"b\xff", b"occupied"], b"+OK\r\n");
        command(&mut stream, &[b"RENAMENX", b"c", b"b\xff"], b":0\r\n");
        command(&mut stream, &[b"DBSIZE"], b":5\r\n");

        command(&mut stream, &[b"LPUSH", b"list", b"one", b"two"], b":2\r\n");
        command(&mut stream, &[b"TYPE", b"list"], b"+list\r\n");
        command(&mut stream, &[b"COPY", b"list", b"list-copy"], b":1\r\n");
        command(&mut stream, &[b"LPOP", b"list-copy"], b"$3\r\ntwo\r\n");
        command(&mut stream, &[b"LLEN", b"list"], b":2\r\n");
        command(
            &mut stream,
            &[b"LRANGE", b"list", b"0", b"-1"],
            b"*2\r\n$3\r\ntwo\r\n$3\r\none\r\n",
        );
        command(&mut stream, &[b"RPOP", b"list"], b"$3\r\none\r\n");
        command(&mut stream, &[b"DEL", b"list"], b":1\r\n");

        command(&mut stream, &[b"SADD", b"set", b"b", b"a", b"b"], b":2\r\n");
        command(&mut stream, &[b"SISMEMBER", b"set", b"a"], b":1\r\n");
        command(
            &mut stream,
            &[b"HSET", b"hash", b"field", b"value"],
            b":1\r\n",
        );
        command(
            &mut stream,
            &[b"HGET", b"hash", b"field"],
            b"$5\r\nvalue\r\n",
        );
        command(
            &mut stream,
            &[b"ZADD", b"zset", b"2", b"b", b"1", b"a"],
            b":2\r\n",
        );
        command(
            &mut stream,
            &[b"ZRANGE", b"zset", b"0", b"-1"],
            b"*2\r\n$1\r\na\r\n$1\r\nb\r\n",
        );

        command(
            &mut stream,
            &[b"XADD", b"stream", b"1", b"f", b"v"],
            b"$3\r\n1-0\r\n",
        );
        command(&mut stream, &[b"TYPE", b"stream"], b"+stream\r\n");
        command(
            &mut stream,
            &[b"COPY", b"stream", b"stream-copy"],
            b":1\r\n",
        );
        command(&mut stream, &[b"XDEL", b"stream", b"1-0"], b":1\r\n");
        command(&mut stream, &[b"XLEN", b"stream-copy"], b":1\r\n");
        command(
            &mut stream,
            &[b"XINFO", b"STREAM", b"stream-copy", b"FULL"],
            b"*10\r\n$6\r\nlength\r\n:1\r\n$7\r\nentries\r\n*1\r\n*2\r\n$3\r\n1-0\r\n*2\r\n$1\r\nf\r\n$1\r\nv\r\n$6\r\ngroups\r\n*0\r\n$20\r\nmax-deleted-entry-id\r\n$3\r\n0-0\r\n$13\r\nentries-added\r\n:1\r\n",
        );
        command(&mut stream, &[b"PING"], b"+PONG\r\n");
        command(
            &mut stream,
            &[b"MGET", b"set", b"hash", b"zset", b"stream-copy"],
            b"*4\r\n$-1\r\n$-1\r\n$-1\r\n$-1\r\n",
        );
        command(
            &mut stream,
            &[b"SETBIT", b"set", b"0", b"1"],
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        );
        command(&mut stream, &[b"CONFIG", b"RESETSTAT"], b"+OK\r\n");
        command(&mut stream, &[b"FLUSHALL", b"SYNC"], b"+OK\r\n");
        command(&mut stream, &[b"DBSIZE"], b":0\r\n");
        command(&mut stream, &[b"RANDOMKEY"], b"$-1\r\n");
        server.shutdown().unwrap();
    }
}
