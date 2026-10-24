#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use rillcache::protocol::encode_request;
use rillcache::{Config, Server};

/// Optional reference testing is intentionally outside the fast, service-free CI
/// loop. `make compare` supplies a disposable, currently pinned reference server.
#[test]
#[ignore = "requires RILLCACHE_REFERENCE_ADDR pointing at a disposable RESP2 server"]
fn supported_subset_matches_current_reference_wire_behavior() {
    let reference: SocketAddr = std::env::var("RILLCACHE_REFERENCE_ADDR")
        .expect("make compare supplies the reference address")
        .parse()
        .unwrap();
    let local = Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        ..Config::default()
    })
    .unwrap()
    .spawn()
    .unwrap();
    let key = b"rillcache:conformance:binary\0\xff";
    let absent = b"rillcache:conformance:missing";
    let cases: Vec<(Vec<&[u8]>, &[u8])> = vec![
        (vec![b"DEL", key, absent], b":0\r\n"),
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
        (vec![b"QUIT"], b"+OK\r\n"),
    ];
    let mut commands = Vec::new();
    let mut expected = Vec::new();
    for (arguments, reply) in cases {
        encode_request(&arguments, &mut commands);
        expected.extend_from_slice(reply);
    }
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
        assert_eq!(actual, expected, "reference mismatch on {address}");
    }
    local.shutdown().unwrap();
}
