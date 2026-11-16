#![forbid(unsafe_code)]

//! Bounded RESP2 conformance checks for the command subset supported on main.
use std::io::{self, BufRead, BufReader, Cursor, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rillcache::protocol::encode_request;
use rillcache::{Config, Server, ServerHandle};

const IO_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_REPLY_BYTES: usize = 2 * 1024 * 1024;
const MAX_COLLECTION_ITEMS: usize = 4096;
const MAX_NESTING: usize = 16;

#[derive(Debug, PartialEq, Eq)]
enum Resp {
    Simple(Vec<u8>),
    Error(Vec<u8>),
    Integer(i64),
    Bulk(Option<Vec<u8>>),
    Array(Option<Vec<Resp>>),
}

struct RespReader<R> {
    input: R,
    bytes: usize,
    items: usize,
}

impl<R: BufRead> RespReader<R> {
    fn new(input: R) -> Self {
        Self {
            input,
            bytes: 0,
            items: 0,
        }
    }

    fn next(&mut self) -> io::Result<Resp> {
        self.bytes = 0;
        self.items = 0;
        self.value(0)
    }

    fn line(&mut self) -> io::Result<Vec<u8>> {
        let mut line = Vec::new();
        loop {
            let available = self.input.fill_buf()?;
            if available.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "truncated RESP line",
                ));
            }
            let count = available
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(available.len(), |index| index + 1);
            if line
                .len()
                .checked_add(count)
                .filter(|length| *length <= MAX_REPLY_BYTES)
                .is_none()
            {
                return Err(invalid("RESP line exceeds byte limit"));
            }
            let complete = available[count - 1] == b'\n';
            line.extend_from_slice(&available[..count]);
            self.input.consume(count);
            self.account(count)?;
            if complete {
                break;
            }
        }
        if line.len() < 2 || !line.ends_with(b"\r\n") {
            return Err(invalid("RESP line is missing CRLF"));
        }
        line.truncate(line.len() - 2);
        Ok(line)
    }

    fn account(&mut self, count: usize) -> io::Result<()> {
        self.bytes = self
            .bytes
            .checked_add(count)
            .filter(|length| *length <= MAX_REPLY_BYTES)
            .ok_or_else(|| invalid("RESP reply exceeds byte limit"))?;
        Ok(())
    }

    fn decimal(&mut self) -> io::Result<i64> {
        std::str::from_utf8(&self.line()?)
            .map_err(invalid)?
            .parse::<i64>()
            .map_err(invalid)
    }

    fn value(&mut self, depth: usize) -> io::Result<Resp> {
        if depth > MAX_NESTING {
            return Err(invalid("RESP nesting limit exceeded"));
        }
        self.items += 1;
        if self.items > MAX_COLLECTION_ITEMS {
            return Err(invalid("RESP item limit exceeded"));
        }
        let mut prefix = [0; 1];
        self.input.read_exact(&mut prefix)?;
        self.account(1)?;
        match prefix[0] {
            b'+' => Ok(Resp::Simple(self.line()?)),
            b'-' => Ok(Resp::Error(self.line()?)),
            b':' => Ok(Resp::Integer(self.decimal()?)),
            b'$' => {
                let length = self.decimal()?;
                if length == -1 {
                    return Ok(Resp::Bulk(None));
                }
                let length = usize::try_from(length).map_err(invalid)?;
                let payload = self.payload(length)?;
                Ok(Resp::Bulk(Some(payload)))
            }
            b'*' => {
                let length = self.decimal()?;
                if length == -1 {
                    return Ok(Resp::Array(None));
                }
                let length = usize::try_from(length).map_err(invalid)?;
                if length > MAX_COLLECTION_ITEMS {
                    return Err(invalid("RESP collection limit exceeded"));
                }
                let mut values = Vec::with_capacity(length.min(256));
                for _ in 0..length {
                    values.push(self.value(depth + 1)?);
                }
                Ok(Resp::Array(Some(values)))
            }
            other => Err(invalid(format!("unsupported RESP2 reply prefix {other:?}"))),
        }
    }

    fn payload(&mut self, length: usize) -> io::Result<Vec<u8>> {
        let needed = length
            .checked_add(2)
            .ok_or_else(|| invalid("RESP bulk length overflow"))?;
        self.account(needed)?;
        let mut bytes = vec![0; length];
        self.input.read_exact(&mut bytes)?;
        let mut ending = [0; 2];
        self.input.read_exact(&mut ending)?;
        if ending != *b"\r\n" {
            return Err(invalid("RESP bulk is missing CRLF"));
        }
        Ok(bytes)
    }

    fn read_trailing_bounded(&mut self) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        self.input
            .by_ref()
            .take((MAX_REPLY_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_REPLY_BYTES {
            return Err(invalid("trailing data exceeds byte limit"));
        }
        Ok(bytes)
    }
}

fn invalid(message: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}

struct Client {
    socket: TcpStream,
    replies: RespReader<BufReader<TcpStream>>,
}

impl Client {
    fn connect(address: SocketAddr) -> io::Result<Self> {
        let socket = TcpStream::connect_timeout(&address, IO_TIMEOUT)?;
        socket.set_read_timeout(Some(IO_TIMEOUT))?;
        socket.set_write_timeout(Some(IO_TIMEOUT))?;
        socket.set_nodelay(true)?;
        let reader = socket.try_clone()?;
        reader.set_read_timeout(Some(IO_TIMEOUT))?;
        Ok(Self {
            socket,
            replies: RespReader::new(BufReader::new(reader)),
        })
    }

    fn command(&mut self, args: &[&[u8]]) -> io::Result<Resp> {
        let mut request = Vec::new();
        encode_request(args, &mut request);
        self.socket.write_all(&request)?;
        self.replies.next()
    }

    fn pipeline(&mut self, commands: &[&[&[u8]]]) -> io::Result<Vec<Resp>> {
        let mut request = Vec::new();
        for args in commands {
            encode_request(args, &mut request);
        }
        self.socket.write_all(&request)?;
        commands.iter().map(|_| self.replies.next()).collect()
    }

    fn fragmented(&mut self, args: &[&[u8]]) -> io::Result<Resp> {
        let mut request = Vec::new();
        encode_request(args, &mut request);
        for byte in request {
            self.socket.write_all(&[byte])?;
        }
        self.replies.next()
    }

    fn quit_and_read_rest(mut self, following: &[&[&[u8]]]) -> io::Result<(Resp, Vec<u8>)> {
        let mut request = Vec::new();
        encode_request(&[b"QUIT"], &mut request);
        for command in following {
            encode_request(command, &mut request);
        }
        self.socket.write_all(&request)?;
        let reply = self.replies.next()?;
        let rest = self.replies.read_trailing_bounded()?;
        Ok((reply, rest))
    }
}

fn start(worker_threads: Option<usize>) -> ServerHandle {
    Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        worker_threads,
        ..Config::default()
    })
    .unwrap()
    .spawn()
    .unwrap()
}

fn key(prefix: &[u8], suffix: &[u8]) -> Vec<u8> {
    [prefix, suffix].concat()
}

fn assert_command(client: &mut Client, args: &[&[u8]], expected: Resp) {
    assert_eq!(client.command(args).unwrap(), expected, "command {args:?}");
}

fn assert_deadline(client: &mut Client, key: &[u8], deadline: i64) {
    assert_command(client, &[b"PEXPIRETIME", key], Resp::Integer(deadline));
    assert_command(
        client,
        &[b"EXPIRETIME", key],
        Resp::Integer(if deadline < 0 {
            deadline
        } else {
            deadline / 1000
        }),
    );
    // Only compare stable sentinel values; positive remaining TTLs are clock-dependent.
    if deadline < 0 {
        assert_command(client, &[b"TTL", key], Resp::Integer(deadline));
        assert_command(client, &[b"PTTL", key], Resp::Integer(deadline));
    }
}

fn run_expiry_corpus(client: &mut Client, suffix: &[u8]) {
    let source = key(b"rillcache:baseline:expiry:\0", suffix);
    let destination = key(b"rillcache:baseline:expiry-copy:", suffix);
    let early = b"4102444800000".as_slice();
    let later = b"4102444900000".as_slice();
    let value = b"expiry\0\xff";
    assert_command(client, &[b"DEL", &source, &destination], Resp::Integer(0));
    assert_deadline(client, &source, -2);
    assert_command(client, &[b"PERSIST", &source], Resp::Integer(0));
    assert_command(
        client,
        &[b"SET", &source, value],
        Resp::Simple(b"OK".to_vec()),
    );
    assert_deadline(client, &source, -1);

    // A persistent key has an infinite deadline for GT/LT. Rejected conditions
    // must leave both the value and the absolute deadline unchanged.
    for (deadline, option, changed, expected) in [
        (early, b"GT".as_slice(), 0, -1),
        (early, b"XX".as_slice(), 0, -1),
        (early, b"LT".as_slice(), 1, 4_102_444_800_000),
        (later, b"NX".as_slice(), 0, 4_102_444_800_000),
        (later, b"XX".as_slice(), 1, 4_102_444_900_000),
        (early, b"GT".as_slice(), 0, 4_102_444_900_000),
        (later, b"GT".as_slice(), 0, 4_102_444_900_000),
        (later, b"LT".as_slice(), 0, 4_102_444_900_000),
        (early, b"LT".as_slice(), 1, 4_102_444_800_000),
        (later, b"GT".as_slice(), 1, 4_102_444_900_000),
    ] {
        assert_command(
            client,
            &[b"PEXPIREAT", &source, deadline, option],
            Resp::Integer(changed),
        );
        assert_deadline(client, &source, expected);
        assert_command(client, &[b"GET", &source], Resp::Bulk(Some(value.to_vec())));
    }
    assert_command(client, &[b"PERSIST", &source], Resp::Integer(1));
    assert_command(client, &[b"PERSIST", &source], Resp::Integer(0));
    assert_deadline(client, &source, -1);
    assert_command(
        client,
        &[b"EXPIREAT", &source, b"4102444800", b"NX"],
        Resp::Integer(1),
    );
    assert_deadline(client, &source, 4_102_444_800_000);
    for command in [
        &[b"SET".as_slice(), &source, value, b"KEEPTTL"][..],
        &[b"APPEND".as_slice(), &source, b""][..],
    ] {
        let expected = if command[0] == b"SET" {
            Resp::Simple(b"OK".to_vec())
        } else {
            Resp::Integer(value.len() as i64)
        };
        assert_command(client, command, expected);
        assert_deadline(client, &source, 4_102_444_800_000);
    }
    assert_command(client, &[b"COPY", &source, &destination], Resp::Integer(1));
    assert_deadline(client, &destination, 4_102_444_800_000);
    assert_command(
        client,
        &[b"GETEX", &source, b"PERSIST"],
        Resp::Bulk(Some(value.to_vec())),
    );
    assert_deadline(client, &source, -1);
    assert_deadline(client, &destination, 4_102_444_800_000);
    assert_command(
        client,
        &[b"RENAME", &destination, &source],
        Resp::Simple(b"OK".to_vec()),
    );
    assert_deadline(client, &source, 4_102_444_800_000);
    assert_deadline(client, &destination, -2);
    assert_command(
        client,
        &[b"GETEX", &source, b"PXAT", b"1"],
        Resp::Bulk(Some(value.to_vec())),
    );
    assert_deadline(client, &source, -2);
    assert_command(client, &[b"GET", &source], Resp::Bulk(None));
    assert_command(
        client,
        &[b"SET", &source, value],
        Resp::Simple(b"OK".to_vec()),
    );
    assert_command(client, &[b"PEXPIRE", &source, b"0"], Resp::Integer(1));
    assert_deadline(client, &source, -2);
    assert_command(client, &[b"DEL", &source, &destination], Resp::Integer(0));
}

fn run_transaction_corpus(address: SocketAddr, suffix: &[u8]) {
    let key = key(b"rillcache:baseline:transaction:\0", suffix);
    let mut client = Client::connect(address).unwrap();
    let mut observer = Client::connect(address).unwrap();
    let ok = || Resp::Simple(b"OK".to_vec());
    let queued = || Resp::Simple(b"QUEUED".to_vec());
    let bulk = |value: &[u8]| Resp::Bulk(Some(value.to_vec()));
    let original = b"before\0\xff";
    let replacement = b"after\0\xff";
    assert_command(&mut client, &[b"DEL", &key], Resp::Integer(0));
    assert_command(&mut client, &[b"SET", &key, original], ok());
    assert_eq!(
        client
            .pipeline(&[
                &[b"MULTI"],
                &[b"SET", &key, replacement],
                &[b"GET", &key],
                &[b"INCR", &key],
                &[b"SET", &key, b"continued"],
            ])
            .unwrap(),
        vec![ok(), queued(), queued(), queued(), queued()]
    );
    // A second connection must not observe queued writes. A runtime error is
    // one EXEC element, not a rollback or a reason to omit following commands.
    assert_command(&mut observer, &[b"GET", &key], bulk(original));
    assert_command(
        &mut client,
        &[b"EXEC"],
        Resp::Array(Some(vec![
            ok(),
            bulk(replacement),
            Resp::Error(b"ERR value is not an integer or out of range".to_vec()),
            ok(),
        ])),
    );
    assert_command(&mut observer, &[b"GET", &key], bulk(b"continued"));
    assert_eq!(
        client
            .pipeline(&[&[b"MULTI"], &[b"SET", &key, b"discarded"], &[b"DISCARD"]])
            .unwrap(),
        vec![ok(), queued(), ok()]
    );
    assert_command(&mut observer, &[b"GET", &key], bulk(b"continued"));

    // A queue-time arity error poisons EXEC and discards even valid queued writes.
    assert_eq!(
        client
            .pipeline(&[
                &[b"MULTI"],
                &[b"SET", &key, b"must-not-run"],
                &[b"SET", &key],
                &[b"EXEC"],
            ])
            .unwrap(),
        vec![
            ok(),
            queued(),
            Resp::Error(b"ERR wrong number of arguments for 'set' command".to_vec()),
            Resp::Error(b"EXECABORT Transaction discarded because of previous errors.".to_vec()),
        ]
    );
    assert_command(&mut observer, &[b"GET", &key], bulk(b"continued"));
    assert_command(&mut client, &[b"WATCH", &key], ok());
    assert_command(&mut observer, &[b"SET", &key, b"outside"], ok());
    assert_eq!(
        client
            .pipeline(&[&[b"MULTI"], &[b"SET", &key, b"conflicted"], &[b"EXEC"]])
            .unwrap(),
        vec![ok(), queued(), Resp::Array(None)]
    );
    assert_command(&mut observer, &[b"GET", &key], bulk(b"outside"));

    // An aborted EXEC clears WATCH: a new transaction may commit normally.
    assert_command(&mut observer, &[b"SET", &key, b"new-outside"], ok());
    assert_eq!(
        client
            .pipeline(&[&[b"MULTI"], &[b"SET", &key, replacement], &[b"EXEC"]])
            .unwrap(),
        vec![ok(), queued(), Resp::Array(Some(vec![ok()]))]
    );
    assert_command(&mut observer, &[b"GET", &key], bulk(replacement));
    assert_command(&mut observer, &[b"DEL", &key], Resp::Integer(1));
    assert_command(&mut client, &[b"WATCH", &key], ok());
    assert_command(&mut observer, &[b"SET", &key, b"created"], ok());
    assert_eq!(
        client
            .pipeline(&[&[b"MULTI"], &[b"DEL", &key], &[b"EXEC"]])
            .unwrap(),
        vec![ok(), queued(), Resp::Array(None)]
    );
    assert_command(&mut observer, &[b"GET", &key], bulk(b"created"));
    assert_command(&mut observer, &[b"DEL", &key], Resp::Integer(1));
}

fn run_supported_corpus(address: SocketAddr, suffix: &[u8], kind: &str) {
    let binary_key = key(b"rillcache:baseline:binary:", suffix);
    let empty_key = key(b"rillcache:baseline:empty:", suffix);
    let condition_key = key(b"rillcache:baseline:condition:", suffix);
    let counter_key = key(b"rillcache:baseline:counter:", suffix);
    let invalid_counter_key = key(b"rillcache:baseline:invalid-counter:", suffix);
    let absent_key = key(b"rillcache:baseline:absent:", suffix);
    let deadline = b"4102444800123";
    let binary = b"\0$-1\r\n\xff";
    let mut client = Client::connect(address).unwrap();
    run_expiry_corpus(&mut client, suffix);
    run_transaction_corpus(address, suffix);

    assert_command(
        &mut client,
        &[
            b"DEL",
            &binary_key,
            &empty_key,
            &condition_key,
            &counter_key,
            &invalid_counter_key,
            &absent_key,
        ],
        Resp::Integer(0),
    );
    assert_command(
        &mut client,
        &[b"SET", &binary_key, binary, b"PXAT", deadline],
        Resp::Simple(b"OK".to_vec()),
    );
    assert_command(
        &mut client,
        &[b"GET", &binary_key],
        Resp::Bulk(Some(binary.to_vec())),
    );
    assert_command(
        &mut client,
        &[b"EXISTS", &binary_key, &binary_key, &absent_key],
        Resp::Integer(2),
    );
    assert_command(
        &mut client,
        &[b"DEL", &binary_key, &binary_key, &absent_key],
        Resp::Integer(1),
    );
    assert_command(
        &mut client,
        &[b"SET", &empty_key, b""],
        Resp::Simple(b"OK".to_vec()),
    );
    assert_command(
        &mut client,
        &[b"GET", &empty_key],
        Resp::Bulk(Some(Vec::new())),
    );
    assert_command(&mut client, &[b"GET", &absent_key], Resp::Bulk(None));

    assert_command(
        &mut client,
        &[b"SET", &condition_key, b"original", b"PXAT", deadline],
        Resp::Simple(b"OK".to_vec()),
    );
    assert_command(
        &mut client,
        &[b"SET", &condition_key, b"rejected", b"NX", b"GET"],
        Resp::Bulk(Some(b"original".to_vec())),
    );
    assert_command(
        &mut client,
        &[b"SET", &condition_key, b"bad", b"NX", b"XX"],
        Resp::Error(b"ERR syntax error".to_vec()),
    );
    assert_command(
        &mut client,
        &[b"SET", &condition_key, b"bad", b"NX", b"PX", b"0"],
        Resp::Error(b"ERR invalid expire time in 'set' command".to_vec()),
    );
    assert_command(
        &mut client,
        &[b"GET", &condition_key],
        Resp::Bulk(Some(b"original".to_vec())),
    );
    assert_command(
        &mut client,
        &[b"PEXPIRETIME", &condition_key],
        Resp::Integer(4_102_444_800_123),
    );

    assert_command(
        &mut client,
        &[b"SET", &counter_key, b"40", b"PXAT", deadline],
        Resp::Simple(b"OK".to_vec()),
    );
    assert_command(
        &mut client,
        &[b"INCRBY", &counter_key, b"2"],
        Resp::Integer(42),
    );
    assert_command(
        &mut client,
        &[b"DECRBY", &counter_key, b"-1"],
        Resp::Integer(43),
    );
    assert_command(
        &mut client,
        &[b"PEXPIRETIME", &counter_key],
        Resp::Integer(4_102_444_800_123),
    );
    assert_command(
        &mut client,
        &[
            b"SET",
            &invalid_counter_key,
            b"9223372036854775807",
            b"PXAT",
            deadline,
        ],
        Resp::Simple(b"OK".to_vec()),
    );
    assert_command(
        &mut client,
        &[b"INCR", &invalid_counter_key],
        Resp::Error(b"ERR increment or decrement would overflow".to_vec()),
    );
    assert_command(
        &mut client,
        &[b"GET", &invalid_counter_key],
        Resp::Bulk(Some(b"9223372036854775807".to_vec())),
    );
    assert_command(
        &mut client,
        &[b"PEXPIRETIME", &invalid_counter_key],
        Resp::Integer(4_102_444_800_123),
    );
    assert_command(
        &mut client,
        &[b"PEXPIRE", &counter_key, b"not-an-integer"],
        Resp::Error(b"ERR value is not an integer or out of range".to_vec()),
    );
    assert_command(
        &mut client,
        &[b"PEXPIRETIME", &counter_key],
        Resp::Integer(4_102_444_800_123),
    );

    // Redis 8.10.2 returns null for a missing key before validating this GETEX TTL;
    // Valkey 9.1.2 reports the invalid TTL. The project deliberately follows Redis.
    let getex = client
        .command(&[b"GETEX", &absent_key, b"EX", b"invalid"])
        .unwrap();
    if kind == "valkey" {
        assert_eq!(
            getex,
            Resp::Error(b"ERR value is not an integer or out of range".to_vec())
        );
    } else {
        assert_eq!(getex, Resp::Bulk(None));
    }

    // Ordered pipelining returns one framed response per request.
    let replies = client
        .pipeline(&[
            &[b"PING"],
            &[b"SET", &binary_key, binary],
            &[b"GET", &binary_key],
            &[b"DEL", &binary_key, &binary_key],
        ])
        .unwrap();
    assert_eq!(
        replies,
        vec![
            Resp::Simple(b"PONG".to_vec()),
            Resp::Simple(b"OK".to_vec()),
            Resp::Bulk(Some(binary.to_vec())),
            Resp::Integer(1),
        ]
    );

    // Byte-at-a-time request writes exercise framing without readiness sleeps.
    assert_eq!(
        client.fragmented(&[b"ECHO", b"fragment\0\xff"]).unwrap(),
        Resp::Bulk(Some(b"fragment\0\xff".to_vec()))
    );

    assert_command(
        &mut client,
        &[
            b"DEL",
            &binary_key,
            &empty_key,
            &condition_key,
            &counter_key,
            &invalid_counter_key,
        ],
        Resp::Integer(4),
    );
    // Collections and streams: exact replies shared with pinned Redis/Valkey.
    let hash_key = key(b"rillcache:baseline:hash:", suffix);
    let set_key = key(b"rillcache:baseline:set:", suffix);
    let zset_key = key(b"rillcache:baseline:zset:", suffix);
    let stream_key = key(b"rillcache:baseline:stream:", suffix);
    assert_command(
        &mut client,
        &[b"HSET", &hash_key, b"f", b"v"],
        Resp::Integer(1),
    );
    assert_command(
        &mut client,
        &[b"HGET", &hash_key, b"f"],
        Resp::Bulk(Some(b"v".to_vec())),
    );
    assert_command(&mut client, &[b"HLEN", &hash_key], Resp::Integer(1));
    assert_command(
        &mut client,
        &[b"HMGET", &hash_key, b"f", b"absent"],
        Resp::Array(Some(vec![
            Resp::Bulk(Some(b"v".to_vec())),
            Resp::Bulk(None),
        ])),
    );
    assert_command(&mut client, &[b"SADD", &set_key, b"a"], Resp::Integer(1));
    assert_command(&mut client, &[b"SCARD", &set_key], Resp::Integer(1));
    assert_command(
        &mut client,
        &[b"SISMEMBER", &set_key, b"a"],
        Resp::Integer(1),
    );
    assert_command(
        &mut client,
        &[b"SMEMBERS", &set_key],
        Resp::Array(Some(vec![Resp::Bulk(Some(b"a".to_vec()))])),
    );
    assert_command(
        &mut client,
        &[b"ZADD", &zset_key, b"1", b"a", b"2", b"b"],
        Resp::Integer(2),
    );
    assert_command(&mut client, &[b"ZCARD", &zset_key], Resp::Integer(2));
    assert_command(
        &mut client,
        &[b"ZSCORE", &zset_key, b"b"],
        Resp::Bulk(Some(b"2".to_vec())),
    );
    assert_command(
        &mut client,
        &[b"ZRANGE", &zset_key, b"0", b"-1"],
        Resp::Array(Some(vec![
            Resp::Bulk(Some(b"a".to_vec())),
            Resp::Bulk(Some(b"b".to_vec())),
        ])),
    );
    assert_command(&mut client, &[b"ZRANK", &zset_key, b"b"], Resp::Integer(1));
    assert_command(
        &mut client,
        &[b"XADD", &stream_key, b"1-1", b"f", b"v"],
        Resp::Bulk(Some(b"1-1".to_vec())),
    );
    assert_command(&mut client, &[b"XLEN", &stream_key], Resp::Integer(1));
    assert_command(
        &mut client,
        &[b"XRANGE", &stream_key, b"-", b"+"],
        Resp::Array(Some(vec![Resp::Array(Some(vec![
            Resp::Bulk(Some(b"1-1".to_vec())),
            Resp::Array(Some(vec![
                Resp::Bulk(Some(b"f".to_vec())),
                Resp::Bulk(Some(b"v".to_vec())),
            ])),
        ]))])),
    );
    assert_command(
        &mut client,
        &[b"DEL", &hash_key, &set_key, &zset_key, &stream_key],
        Resp::Integer(4),
    );
    let after_quit_key = key(b"rillcache:baseline:after-quit:", suffix);
    let client = Client::connect(address).unwrap();
    let following: &[&[u8]] = &[b"SET", after_quit_key.as_slice(), b"must-not-run"];
    let (reply, rest) = client.quit_and_read_rest(&[following]).unwrap();
    assert_eq!(reply, Resp::Simple(b"OK".to_vec()));
    assert!(
        rest.is_empty(),
        "QUIT flushed unexpected trailing replies: {rest:?}"
    );
    let mut check = Client::connect(address).unwrap();
    assert_eq!(
        check.command(&[b"GET", &after_quit_key]).unwrap(),
        Resp::Bulk(None)
    );
}

fn unique_suffix() -> Vec<u8> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}-{nanos}", std::process::id()).into_bytes()
}

#[test]
fn supported_baseline_corpus_passes_in_all_local_modes() {
    let suffix = unique_suffix();
    for (mode, workers) in [
        ("per-client", None),
        ("one-worker", Some(1)),
        ("four-workers", Some(4)),
    ] {
        let server = start(workers);
        run_supported_corpus(
            server.local_addr(),
            &[suffix.as_slice(), mode.as_bytes()].concat(),
            "redis",
        );
        server.shutdown().unwrap();
    }
}

/// Optional native comparison. `RILLCACHE_REFERENCE_KIND` is `redis` or `valkey`;
/// the reference must be disposable because this corpus writes temporary keys.
#[test]
#[ignore = "requires disposable RILLCACHE_REFERENCE_ADDR and RILLCACHE_REFERENCE_KIND"]
fn supported_baseline_matches_optional_native_reference() {
    let address: SocketAddr = std::env::var("RILLCACHE_REFERENCE_ADDR")
        .expect("set RILLCACHE_REFERENCE_ADDR")
        .parse()
        .expect("reference address must be a socket address");
    let kind = std::env::var("RILLCACHE_REFERENCE_KIND").expect("set redis or valkey");
    assert!(
        kind == "redis" || kind == "valkey",
        "kind must be redis or valkey"
    );
    let suffix = unique_suffix();
    for (mode, workers) in [
        ("per-client", None),
        ("one-worker", Some(1)),
        ("four-workers", Some(4)),
    ] {
        let server = start(workers);
        let unique = [suffix.as_slice(), mode.as_bytes()].concat();
        run_supported_corpus(server.local_addr(), &unique, "redis");
        run_supported_corpus(address, &unique, &kind);
        server.shutdown().unwrap();
    }
}

#[cfg(test)]
mod parser_tests {
    use super::*;

    #[test]
    fn parses_binary_empty_null_and_nested_resp2_values() {
        let wire = b"*4\r\n+OK\r\n:7\r\n$0\r\n\r\n$-1\r\n";
        let mut reader = RespReader::new(BufReader::new(Cursor::new(wire)));
        assert_eq!(
            reader.next().unwrap(),
            Resp::Array(Some(vec![
                Resp::Simple(b"OK".to_vec()),
                Resp::Integer(7),
                Resp::Bulk(Some(Vec::new())),
                Resp::Bulk(None),
            ]))
        );
        let binary = b"$4\r\n\0\xff\r\n\r\n";
        let mut reader = RespReader::new(BufReader::new(Cursor::new(binary)));
        assert_eq!(
            reader.next().unwrap(),
            Resp::Bulk(Some(b"\0\xff\r\n".to_vec()))
        );
    }

    #[test]
    fn rejects_oversized_truncated_and_malformed_frames() {
        let oversized = format!("${}\r\n", MAX_REPLY_BYTES + 1);
        let mut reader = RespReader::new(BufReader::new(Cursor::new(oversized)));
        assert_eq!(
            reader.next().unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );

        let mut reader = RespReader::new(BufReader::new(Cursor::new(b"$4\r\nab")));
        assert_eq!(
            reader.next().unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );

        let mut reader = RespReader::new(BufReader::new(Cursor::new(b"$1\r\nx!!")));
        assert_eq!(
            reader.next().unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
}
