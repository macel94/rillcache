#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use rillcache::protocol::encode_request;
use rillcache::{Config, Server};

#[test]
#[ignore = "requires RILLCACHE_REFERENCE_ADDR and RILLCACHE_REFERENCE_KIND"]
fn getex_missing_invalid_ttl_exposes_the_redis_valkey_contract_difference() {
    let reference: SocketAddr = std::env::var("RILLCACHE_REFERENCE_ADDR")
        .unwrap()
        .parse()
        .unwrap();
    let kind =
        std::env::var("RILLCACHE_REFERENCE_KIND").expect("make compare declares redis or valkey");
    assert!(["redis", "valkey"].contains(&kind.as_str()));
    for worker_threads in [None, Some(1), Some(4)] {
        let local = Server::bind(Config {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            worker_threads,
            ..Config::default()
        })
        .unwrap()
        .spawn()
        .unwrap();
        for (address, valkey) in [(local.local_addr(), false), (reference, kind == "valkey")] {
            let mut stream = TcpStream::connect(address).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut command = Vec::new();
            encode_request(
                &[
                    b"GETEX",
                    b"rillcache:conformance:absent-getex",
                    b"EX",
                    b"invalid",
                ],
                &mut command,
            );
            encode_request(&[b"QUIT"], &mut command);
            stream.write_all(&command).unwrap();
            let mut actual = Vec::new();
            stream.read_to_end(&mut actual).unwrap();
            let expected = if valkey {
                b"-ERR value is not an integer or out of range\r\n+OK\r\n".as_slice()
            } else {
                b"$-1\r\n+OK\r\n"
            };
            assert_eq!(actual, expected, "GETEX contract differs for {address}");
        }
        local.shutdown().unwrap();
    }
}

/// Optional reference testing is intentionally outside the fast, service-free CI
/// loop. `make compare` supplies a disposable, currently pinned reference server.
#[test]
#[ignore = "requires RILLCACHE_REFERENCE_ADDR pointing at a disposable RESP2 server"]
fn supported_subset_matches_current_reference_wire_behavior() {
    let reference: SocketAddr = std::env::var("RILLCACHE_REFERENCE_ADDR")
        .expect("make compare supplies the reference address")
        .parse()
        .unwrap();
    let key = b"rillcache:conformance:binary\0\xff";
    let absent = b"rillcache:conformance:missing";
    let cases: Vec<(Vec<&[u8]>, &[u8])> = vec![
        (vec![b"DEL", key, absent], b":0\r\n"),
        (vec![b"DEL", b"multi:a", b"multi:b", b"multi:c"], b":0\r\n"),
        (
            vec![b"MSET", b"multi:a", b"one", b"multi:b", b""],
            b"+OK\r\n",
        ),
        (
            vec![b"MGET", b"multi:b", absent, b"multi:a", b"multi:a"],
            b"*4\r\n$0\r\n\r\n$-1\r\n$3\r\none\r\n$3\r\none\r\n",
        ),
        (
            vec![b"MSETNX", b"multi:a", b"no", b"multi:c", b"not-written"],
            b":0\r\n",
        ),
        (vec![b"GET", b"multi:c"], b"$-1\r\n"),
        (
            vec![b"MSETNX", b"multi:c", b"first", b"multi:c", b"last"],
            b":1\r\n",
        ),
        (
            vec![b"MGET", b"multi:c", b"multi:c"],
            b"*2\r\n$4\r\nlast\r\n$4\r\nlast\r\n",
        ),
        (vec![b"DEL", b"multi:a", b"multi:b", b"multi:c"], b":3\r\n"),
        (vec![b"pInG"], b"+PONG\r\n"),
        (vec![b"PING", b"\0\xff\r\n"], b"$4\r\n\0\xff\r\n\r\n"),
        (vec![b"ECHO", b""], b"$0\r\n\r\n"),
        (vec![b"ECHO", b"\xff\0"], b"$2\r\n\xff\0\r\n"),
        (vec![b"GET", absent], b"$-1\r\n"),
        (vec![b"sEt", key, b"\0\xff\r\n"], b"+OK\r\n"),
        (vec![b"GET", key], b"$4\r\n\0\xff\r\n\r\n"),
        (vec![b"EXISTS", key, key, absent], b":2\r\n"),
        (vec![b"SET", key, b""], b"+OK\r\n"),
        (vec![b"GET", key], b"$0\r\n\r\n"),
        (vec![b"DEL", key, key, absent], b":1\r\n"),
        (vec![b"GET", key], b"$-1\r\n"),
        (vec![b"SET", key, b"no", b"XX"], b"$-1\r\n"),
        (vec![b"SET", key, b"\0\xff", b"NX", b"nx"], b"+OK\r\n"),
        (vec![b"SET", key, b"no", b"NX"], b"$-1\r\n"),
        (vec![b"GET", key], b"$2\r\n\0\xff\r\n"),
        (
            vec![b"SET", key, b"no", b"NX", b"XX"],
            b"-ERR syntax error\r\n",
        ),
        (
            vec![b"SET", key, b"no", b"XX", b"NX"],
            b"-ERR syntax error\r\n",
        ),
        (vec![b"SET", key, b"", b"xx", b"XX"], b"+OK\r\n"),
        (vec![b"GET", key], b"$0\r\n\r\n"),
        (vec![b"DEL", key], b":1\r\n"),
        (
            vec![b"GET"],
            b"-ERR wrong number of arguments for 'get' command\r\n",
        ),
        (
            vec![b"SET", key],
            b"-ERR wrong number of arguments for 'set' command\r\n",
        ),
        (
            vec![b"PING", b"a", b"b"],
            b"-ERR wrong number of arguments for 'ping' command\r\n",
        ),
        (
            vec![b"SET", key, b"old", b"PXAT", b"4102444800123"],
            b"+OK\r\n",
        ),
        (vec![b"PEXPIRETIME", key], b":4102444800123\r\n"),
        (vec![b"SET", key, b"wrong", b"NX", b"GET"], b"$3\r\nold\r\n"),
        (
            vec![b"SET", key, b"new", b"XX", b"GET", b"KEEPTTL"],
            b"$3\r\nold\r\n",
        ),
        (vec![b"PEXPIRETIME", key], b":4102444800123\r\n"),
        (vec![b"GETEX", key, b"PERSIST"], b"$3\r\nnew\r\n"),
        (vec![b"PTTL", key], b":-1\r\n"),
        (vec![b"SETEX", key, b"3600", b"old"], b"+OK\r\n"),
        (vec![b"GETSET", key, b"0"], b"$3\r\nold\r\n"),
        (vec![b"PTTL", key], b":-1\r\n"),
        (vec![b"PEXPIREAT", key, b"4102444800123", b"NX"], b":1\r\n"),
        (
            vec![b"PEXPIREAT", key, b"4102444800122", b"XX", b"LT"],
            b":1\r\n",
        ),
        (
            vec![b"PEXPIREAT", key, b"4102444800123", b"XX", b"GT"],
            b":1\r\n",
        ),
        (vec![b"INCR", key], b":1\r\n"),
        (vec![b"DECR", key], b":0\r\n"),
        (
            vec![b"INCRBY", key, b"9223372036854775807"],
            b":9223372036854775807\r\n",
        ),
        (
            vec![b"INCR", key],
            b"-ERR increment or decrement would overflow\r\n",
        ),
        (
            vec![b"DECRBY", key, b"-9223372036854775808"],
            b"-ERR decrement would overflow\r\n",
        ),
        (vec![b"PEXPIRETIME", key], b":4102444800123\r\n"),
        (vec![b"SET", key, b"00", b"KEEPTTL"], b"+OK\r\n"),
        (
            vec![b"INCR", key],
            b"-ERR value is not an integer or out of range\r\n",
        ),
        (vec![b"PEXPIRETIME", key], b":4102444800123\r\n"),
        (
            vec![b"SET", key, b"bad", b"NX", b"PX", b"0"],
            b"-ERR invalid expire time in 'set' command\r\n",
        ),
        (
            vec![b"SET", key, b"last", b"EX", b"ignored", b"EX", b"3600"],
            b"+OK\r\n",
        ),
        (vec![b"GETDEL", key], b"$4\r\nlast\r\n"),
        (vec![b"GETEX", key, b"EX", b"1"], b"$-1\r\n"),
        (vec![b"SET", key, b"v", b"EXAT", b"1"], b"+OK\r\n"),
        (vec![b"GET", key], b"$-1\r\n"),
        (vec![b"PSETEX", key, b"3600000", b"v"], b"+OK\r\n"),
        (vec![b"EXPIRE", key, b"0"], b":1\r\n"),
        (vec![b"PTTL", key], b":-2\r\n"),
        (vec![b"SETNX", key, b"v"], b":1\r\n"),
        (vec![b"SETNX", key, b"no"], b":0\r\n"),
        (vec![b"GETDEL", key], b"$1\r\nv\r\n"),
        (vec![b"QUIT"], b"+OK\r\n"),
    ];
    let mut commands = Vec::new();
    let mut expected = Vec::new();
    for (arguments, reply) in cases {
        encode_request(&arguments, &mut commands);
        expected.extend_from_slice(reply);
    }
    for worker_threads in [None, Some(1), Some(4)] {
        let local = Server::bind(Config {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            worker_threads,
            ..Config::default()
        })
        .unwrap()
        .spawn()
        .unwrap();
        for address in [local.local_addr(), reference] {
            let mut stream = TcpStream::connect(address).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            stream.write_all(&commands).unwrap();
            let mut actual = Vec::new();
            stream.read_to_end(&mut actual).unwrap();
            assert_eq!(
                actual, expected,
                "reference mismatch on {address}, workers {worker_threads:?}"
            );
        }
        local.shutdown().unwrap();
    }
}
