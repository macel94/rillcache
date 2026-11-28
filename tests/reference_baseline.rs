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

/// Stream trim/read semantics (C9): exact `XTRIM ... MAXLEN =`/`MINID =` behaviour,
/// range reads with COUNT, XDEL and the basic group read/ack flow. Deterministic
/// ids keep the corpus clock-free; approximate (`~`) trimming is deliberately not
/// asserted because its retained size is implementation-defined.
fn run_stream_trim_read_corpus(client: &mut Client, suffix: &[u8]) {
    let stream = key(b"rillcache:baseline:stream-trim:", suffix);
    assert_command(client, &[b"DEL", &stream], Resp::Integer(0));

    let entry = |id: &[u8], field: &[u8], value: &[u8]| {
        Resp::Array(Some(vec![
            Resp::Bulk(Some(id.to_vec())),
            Resp::Array(Some(vec![
                Resp::Bulk(Some(field.to_vec())),
                Resp::Bulk(Some(value.to_vec())),
            ])),
        ]))
    };

    for (id, field) in [
        (b"1-1".as_slice(), b"a".as_slice()),
        (b"2-2", b"b"),
        (b"3-3", b"c"),
    ] {
        assert_command(
            client,
            &[b"XADD", &stream, id, field, b"v"],
            Resp::Bulk(Some(id.to_vec())),
        );
    }
    assert_command(client, &[b"XLEN", &stream], Resp::Integer(3));
    assert_command(
        client,
        &[b"XRANGE", &stream, b"-", b"+", b"COUNT", b"2"],
        Resp::Array(Some(vec![
            entry(b"1-1", b"a", b"v"),
            entry(b"2-2", b"b", b"v"),
        ])),
    );
    assert_command(
        client,
        &[b"XREVRANGE", &stream, b"+", b"-", b"COUNT", b"1"],
        Resp::Array(Some(vec![entry(b"3-3", b"c", b"v")])),
    );

    // Exact trimming removes the oldest entries and keeps the range monotonically.
    assert_command(
        client,
        &[b"XTRIM", &stream, b"MAXLEN", b"=", b"2"],
        Resp::Integer(1),
    );
    assert_command(client, &[b"XLEN", &stream], Resp::Integer(2));
    assert_command(
        client,
        &[b"XRANGE", &stream, b"-", b"+"],
        Resp::Array(Some(vec![
            entry(b"2-2", b"b", b"v"),
            entry(b"3-3", b"c", b"v"),
        ])),
    );
    assert_command(
        client,
        &[b"XTRIM", &stream, b"MINID", b"=", b"3"],
        Resp::Integer(1),
    );
    assert_command(client, &[b"XLEN", &stream], Resp::Integer(1));
    assert_command(client, &[b"XDEL", &stream, b"3-3"], Resp::Integer(1));
    assert_command(client, &[b"XLEN", &stream], Resp::Integer(0));

    // Reads are ordered and bounded; a requested id already seen yields no entries.
    assert_command(
        client,
        &[b"XADD", &stream, b"4-4", b"k", b"v1"],
        Resp::Bulk(Some(b"4-4".to_vec())),
    );
    assert_command(
        client,
        &[b"XADD", &stream, b"5-5", b"k", b"v2"],
        Resp::Bulk(Some(b"5-5".to_vec())),
    );
    assert_command(
        client,
        &[b"XREAD", b"COUNT", b"1", b"STREAMS", &stream, b"0"],
        Resp::Array(Some(vec![Resp::Array(Some(vec![
            Resp::Bulk(Some(stream.clone())),
            Resp::Array(Some(vec![entry(b"4-4", b"k", b"v1")])),
        ]))])),
    );
    assert_command(
        client,
        &[b"XREAD", b"STREAMS", &stream, b"4-4"],
        Resp::Array(Some(vec![Resp::Array(Some(vec![
            Resp::Bulk(Some(stream.clone())),
            Resp::Array(Some(vec![entry(b"5-5", b"k", b"v2")])),
        ]))])),
    );
    assert_command(
        client,
        &[b"XREAD", b"STREAMS", &stream, b"5-5"],
        Resp::Array(None),
    );

    // Group flow: create, deliver one entry to a new consumer, acknowledge it once.
    let group = b"g1".as_slice();
    let consumer = b"c1".as_slice();
    assert_command(
        client,
        &[b"XGROUP", b"CREATE", &stream, group, b"0"],
        Resp::Simple(b"OK".to_vec()),
    );
    assert_command(
        client,
        &[
            b"XREADGROUP",
            b"GROUP",
            group,
            consumer,
            b"COUNT",
            b"1",
            b"STREAMS",
            &stream,
            b">",
        ],
        Resp::Array(Some(vec![Resp::Array(Some(vec![
            Resp::Bulk(Some(stream.clone())),
            Resp::Array(Some(vec![entry(b"4-4", b"k", b"v1")])),
        ]))])),
    );
    assert_command(client, &[b"XACK", &stream, group, b"4-4"], Resp::Integer(1));
    assert_command(client, &[b"XACK", &stream, group, b"4-4"], Resp::Integer(0));
    assert_command(
        client,
        &[
            b"XREADGROUP",
            b"GROUP",
            group,
            consumer,
            b"STREAMS",
            &stream,
            b">",
        ],
        Resp::Array(Some(vec![Resp::Array(Some(vec![
            Resp::Bulk(Some(stream.clone())),
            Resp::Array(Some(vec![entry(b"5-5", b"k", b"v2")])),
        ]))])),
    );
    assert_command(
        client,
        &[
            b"XREADGROUP",
            b"GROUP",
            group,
            consumer,
            b"STREAMS",
            &stream,
            b">",
        ],
        Resp::Array(None),
    );
    assert_command(client, &[b"DEL", &stream], Resp::Integer(1));
}

/// Command-surface differential cases (V2): deterministic replies for the
/// string/bit, hash, list, set, sorted-set, HLL and generic families that the
/// coverage accounting previously listed as uncovered. Values stay small and
/// clock-free so the replies are exact and comparable with the pinned servers.
fn run_command_surface_corpus(client: &mut Client, suffix: &[u8]) {
    let bulk = |value: &[u8]| Resp::Bulk(Some(value.to_vec()));
    let array = |items: Vec<Resp>| Resp::Array(Some(items));

    let string = key(b"rillcache:baseline:surface-string:", suffix);
    assert_command(client, &[b"SETNX", &string, b"abc"], Resp::Integer(1));
    assert_command(client, &[b"SETNX", &string, b"x"], Resp::Integer(0));
    assert_command(client, &[b"STRLEN", &string], Resp::Integer(3));
    assert_command(client, &[b"GETRANGE", &string, b"1", b"-1"], bulk(b"bc"));
    assert_command(
        client,
        &[b"SETRANGE", &string, b"1", b"Z"],
        Resp::Integer(3),
    );
    assert_command(client, &[b"GET", &string], bulk(b"aZc"));
    assert_command(client, &[b"GETDEL", &string], bulk(b"aZc"));
    assert_command(client, &[b"EXISTS", &string], Resp::Integer(0));
    assert_command(
        client,
        &[b"SETEX", &string, b"100", b"v"],
        Resp::Simple(b"OK".to_vec()),
    );
    assert_command(client, &[b"GETSET", &string, b"w"], bulk(b"v"));
    assert_command(
        client,
        &[b"TYPE", &string],
        Resp::Simple(b"string".to_vec()),
    );

    let bits = key(b"rillcache:baseline:surface-bits:", suffix);
    assert_command(client, &[b"SETBIT", &bits, b"0", b"1"], Resp::Integer(0));
    assert_command(client, &[b"SETBIT", &bits, b"7", b"1"], Resp::Integer(0));
    assert_command(client, &[b"GETBIT", &bits, b"0"], Resp::Integer(1));
    assert_command(client, &[b"BITCOUNT", &bits], Resp::Integer(2));
    assert_command(client, &[b"BITPOS", &bits, b"1"], Resp::Integer(0));

    let hash = key(b"rillcache:baseline:surface-hash:", suffix);
    assert_command(
        client,
        &[b"HSET", &hash, b"a", b"1", b"b", b"2"],
        Resp::Integer(2),
    );
    assert_command(
        client,
        &[b"HGETALL", &hash],
        array(vec![bulk(b"a"), bulk(b"1"), bulk(b"b"), bulk(b"2")]),
    );
    assert_command(
        client,
        &[b"HKEYS", &hash],
        array(vec![bulk(b"a"), bulk(b"b")]),
    );
    assert_command(
        client,
        &[b"HVALS", &hash],
        array(vec![bulk(b"1"), bulk(b"2")]),
    );
    assert_command(client, &[b"HEXISTS", &hash, b"a"], Resp::Integer(1));
    assert_command(client, &[b"HSTRLEN", &hash, b"a"], Resp::Integer(1));
    assert_command(client, &[b"HINCRBY", &hash, b"a", b"4"], Resp::Integer(5));
    assert_command(client, &[b"HDEL", &hash, b"b"], Resp::Integer(1));
    assert_command(client, &[b"HLEN", &hash], Resp::Integer(1));

    let list = key(b"rillcache:baseline:surface-list:", suffix);
    assert_command(
        client,
        &[b"RPUSH", &list, b"1", b"2", b"3"],
        Resp::Integer(3),
    );
    assert_command(client, &[b"LLEN", &list], Resp::Integer(3));
    assert_command(client, &[b"LINDEX", &list, b"1"], bulk(b"2"));
    assert_command(
        client,
        &[b"LSET", &list, b"1", b"9"],
        Resp::Simple(b"OK".to_vec()),
    );
    assert_command(
        client,
        &[b"LTRIM", &list, b"0", b"1"],
        Resp::Simple(b"OK".to_vec()),
    );
    assert_command(
        client,
        &[b"LRANGE", &list, b"0", b"-1"],
        array(vec![bulk(b"1"), bulk(b"9")]),
    );
    assert_command(client, &[b"LPOP", &list], bulk(b"1"));
    assert_command(client, &[b"RPOP", &list], bulk(b"9"));
    assert_command(client, &[b"EXISTS", &list], Resp::Integer(0));
    assert_command(client, &[b"RPUSH", &list, b"a"], Resp::Integer(1));
    assert_command(client, &[b"RPUSHX", &list, b"b"], Resp::Integer(2));
    assert_command(client, &[b"LPUSHX", &list, b"c"], Resp::Integer(3));
    assert_command(
        client,
        &[b"LMOVE", &list, &list, b"LEFT", b"RIGHT"],
        bulk(b"c"),
    );
    assert_command(
        client,
        &[b"LINSERT", &list, b"BEFORE", b"a", b"z"],
        Resp::Integer(4),
    );
    assert_command(
        client,
        &[b"LMPOP", b"1", &list, b"LEFT", b"COUNT", b"2"],
        array(vec![bulk(&list), array(vec![bulk(b"z"), bulk(b"a")])]),
    );

    let set = key(b"rillcache:baseline:surface-set:", suffix);
    assert_command(client, &[b"SADD", &set, b"a", b"b"], Resp::Integer(2));
    assert_command(
        client,
        &[b"SMISMEMBER", &set, b"a", b"c"],
        array(vec![Resp::Integer(1), Resp::Integer(0)]),
    );
    assert_command(client, &[b"SREM", &set, b"b"], Resp::Integer(1));
    assert_command(client, &[b"SCARD", &set], Resp::Integer(1));
    let set2 = key(b"rillcache:baseline:surface-set2:", suffix);
    assert_command(client, &[b"SADD", &set2, b"a", b"d"], Resp::Integer(2));
    assert_command(client, &[b"SINTER", &set, &set2], array(vec![bulk(b"a")]));
    assert_command(
        client,
        &[b"SUNION", &set, &set2],
        array(vec![bulk(b"a"), bulk(b"d")].into_iter().collect::<Vec<_>>()),
    );
    assert_command(client, &[b"SDIFF", &set2, &set], array(vec![bulk(b"d")]));

    let zset = key(b"rillcache:baseline:surface-zset:", suffix);
    assert_command(
        client,
        &[b"ZADD", &zset, b"1", b"a", b"2", b"b", b"3", b"c"],
        Resp::Integer(3),
    );
    assert_command(client, &[b"ZCOUNT", &zset, b"1", b"2"], Resp::Integer(2));
    assert_command(
        client,
        &[b"ZMSCORE", &zset, b"a", b"c"],
        array(vec![bulk(b"1"), bulk(b"3")]),
    );
    assert_command(client, &[b"ZREVRANK", &zset, b"a"], Resp::Integer(2));
    assert_command(client, &[b"ZINCRBY", &zset, b"10", b"a"], bulk(b"11"));
    assert_command(client, &[b"ZREM", &zset, b"c"], Resp::Integer(1));
    assert_command(client, &[b"ZCARD", &zset], Resp::Integer(2));
    assert_command(
        client,
        &[b"ZPOPMIN", &zset],
        array(vec![bulk(b"b"), bulk(b"2")]),
    );

    let hll = key(b"rillcache:baseline:surface-hll:", suffix);
    assert_command(client, &[b"PFADD", &hll, b"x", b"y"], Resp::Integer(1));
    assert_command(client, &[b"PFCOUNT", &hll], Resp::Integer(2));
    assert_command(
        client,
        &[b"PFMERGE", &hll, &hll],
        Resp::Simple(b"OK".to_vec()),
    );

    assert_command(client, &[b"ECHO", b"hi"], bulk(b"hi"));
    let counter = key(b"rillcache:baseline:surface-counter:", suffix);
    assert_command(
        client,
        &[b"SET", &counter, b"5"],
        Resp::Simple(b"OK".to_vec()),
    );
    assert_command(client, &[b"DECR", &counter], Resp::Integer(4));
}
/// Collection and generic surface cases (V2): set algebra with stores/cards,
/// sorted-set range/removal/algebra families, and the remaining string and
/// generic commands. Exact replies only; nondeterministic commands (SCAN/KEYS
/// ordering, RANDOMKEY, ZRANDMEMBER) are deliberately excluded.
fn run_collection_surface_corpus(client: &mut Client, suffix: &[u8]) {
    let bulk = |value: &[u8]| Resp::Bulk(Some(value.to_vec()));
    let array = |items: Vec<Resp>| Resp::Array(Some(items));

    let s1 = key(b"rillcache:baseline:cs-s1:", suffix);
    let s2 = key(b"rillcache:baseline:cs-s2:", suffix);
    let dst = key(b"rillcache:baseline:cs-dst:", suffix);
    assert_command(client, &[b"SADD", &s1, b"a", b"b"], Resp::Integer(2));
    assert_command(client, &[b"SADD", &s2, b"b", b"c"], Resp::Integer(2));
    assert_command(client, &[b"SDIFFSTORE", &dst, &s1, &s2], Resp::Integer(1));
    assert_command(client, &[b"SISMEMBER", &dst, b"a"], Resp::Integer(1));
    assert_command(client, &[b"SDIFFCARD", b"2", &s1, &s2], Resp::Integer(1));
    assert_command(client, &[b"SINTERSTORE", &dst, &s1, &s2], Resp::Integer(1));
    assert_command(client, &[b"SISMEMBER", &dst, b"b"], Resp::Integer(1));
    assert_command(client, &[b"SINTERCARD", b"2", &s1, &s2], Resp::Integer(1));
    assert_command(client, &[b"SUNIONSTORE", &dst, &s1, &s2], Resp::Integer(3));
    assert_command(client, &[b"SCARD", &dst], Resp::Integer(3));
    assert_command(client, &[b"SUNIONCARD", b"2", &s1, &s2], Resp::Integer(3));
    assert_command(client, &[b"SMOVE", &s1, &s2, b"a"], Resp::Integer(1));
    assert_command(
        client,
        &[b"SMISMEMBER", &s2, b"a"],
        array(vec![Resp::Integer(1)]),
    );

    let z = key(b"rillcache:baseline:cs-z:", suffix);
    assert_command(
        client,
        &[b"ZADD", &z, b"1", b"a", b"2", b"b", b"3", b"c"],
        Resp::Integer(3),
    );
    assert_command(
        client,
        &[b"ZRANGEBYSCORE", &z, b"-inf", b"+inf"],
        array(vec![bulk(b"a"), bulk(b"b"), bulk(b"c")]),
    );
    assert_command(
        client,
        &[b"ZRANGEBYSCORE", &z, b"2", b"3"],
        array(vec![bulk(b"b"), bulk(b"c")]),
    );
    assert_command(
        client,
        &[b"ZREVRANGEBYSCORE", &z, b"+inf", b"-inf"],
        array(vec![bulk(b"c"), bulk(b"b"), bulk(b"a")]),
    );
    assert_command(
        client,
        &[b"ZREVRANGEBYSCORE", &z, b"3", b"2"],
        array(vec![bulk(b"c"), bulk(b"b")]),
    );

    let zlex = key(b"rillcache:baseline:cs-zlex:", suffix);
    assert_command(
        client,
        &[b"ZADD", &zlex, b"0", b"a", b"0", b"b", b"0", b"c"],
        Resp::Integer(3),
    );
    assert_command(
        client,
        &[b"ZRANGEBYLEX", &zlex, b"-", b"+"],
        array(vec![bulk(b"a"), bulk(b"b"), bulk(b"c")]),
    );
    assert_command(
        client,
        &[b"ZREVRANGEBYLEX", &zlex, b"+", b"-"],
        array(vec![bulk(b"c"), bulk(b"b"), bulk(b"a")]),
    );
    assert_command(client, &[b"ZLEXCOUNT", &zlex, b"-", b"+"], Resp::Integer(3));
    assert_command(
        client,
        &[b"ZREMRANGEBYLEX", &zlex, b"(a", b"(c"],
        Resp::Integer(1),
    );
    assert_command(client, &[b"ZCARD", &zlex], Resp::Integer(2));
    assert_command(
        client,
        &[b"ZREMRANGEBYRANK", &z, b"0", b"0"],
        Resp::Integer(1),
    );
    assert_command(
        client,
        &[b"ZREMRANGEBYSCORE", &z, b"3", b"3"],
        Resp::Integer(1),
    );
    assert_command(client, &[b"ZCARD", &z], Resp::Integer(1));

    let za = key(b"rillcache:baseline:cs-za:", suffix);
    let zb = key(b"rillcache:baseline:cs-zb:", suffix);
    assert_command(
        client,
        &[b"ZADD", &za, b"1", b"a", b"2", b"b"],
        Resp::Integer(2),
    );
    assert_command(client, &[b"ZADD", &zb, b"3", b"b"], Resp::Integer(1));
    assert_command(client, &[b"ZDIFF", b"2", &za, &zb], array(vec![bulk(b"a")]));
    assert_command(
        client,
        &[b"ZINTER", b"2", &za, &zb],
        array(vec![bulk(b"b")]),
    );
    assert_command(
        client,
        &[b"ZUNION", b"2", &za, &zb],
        array(vec![bulk(b"a"), bulk(b"b")]),
    );
    assert_command(
        client,
        &[b"ZDIFFSTORE", &dst, b"2", &za, &zb],
        Resp::Integer(1),
    );
    assert_command(
        client,
        &[b"ZINTERSTORE", &dst, b"2", &za, &zb],
        Resp::Integer(1),
    );
    assert_command(
        client,
        &[b"ZUNIONSTORE", &dst, b"2", &za, &zb],
        Resp::Integer(2),
    );
    assert_command(client, &[b"ZINTERCARD", b"2", &za, &zb], Resp::Integer(1));
    assert_command(
        client,
        &[b"ZMPOP", b"1", &za, b"MIN"],
        array(vec![
            bulk(&za),
            array(vec![array(vec![bulk(b"a"), bulk(b"1")])]),
        ]),
    );

    let k1 = key(b"rillcache:baseline:cs-k1:", suffix);
    let k2 = key(b"rillcache:baseline:cs-k2:", suffix);
    assert_command(
        client,
        &[b"MSET", &k1, b"abc", &k2, b"abd"],
        Resp::Simple(b"OK".to_vec()),
    );
    assert_command(client, &[b"SUBSTR", &k1, b"0", b"0"], bulk(b"a"));
    assert_command(client, &[b"LCS", &k1, &k2, b"LEN"], Resp::Integer(2));
    assert_command(client, &[b"EXPIRE", &k1, b"100"], Resp::Integer(1));
    assert_command(client, &[b"PERSIST", &k1], Resp::Integer(1));
    assert_command(client, &[b"PERSIST", &k1], Resp::Integer(0));
    assert_command(client, &[b"TOUCH", &k2], Resp::Integer(1));
    assert_command(client, &[b"TOUCH", &k1], Resp::Integer(1));
    assert_command(client, &[b"RENAMENX", &k1, &k2], Resp::Integer(0));
    assert_command(client, &[b"RENAME", &k1, &k1], Resp::Simple(b"OK".to_vec()));
    assert_command(client, &[b"UNLINK", &k1, &k2], Resp::Integer(2));

    let h = key(b"rillcache:baseline:cs-hash:", suffix);
    assert_command(
        client,
        &[b"HMSET", &h, b"f", b"v"],
        Resp::Simple(b"OK".to_vec()),
    );
    assert_command(
        client,
        &[b"HSCAN", &h, b"0"],
        array(vec![bulk(b"0"), array(vec![bulk(b"f"), bulk(b"v")])]),
    );
}
/// Advanced surface cases (V2): geospatial, hash field expiry, immediately
/// satisfied blocking list/zset commands, and the deterministic administrative
/// and extension replies. Nondeterministic or state-order-sensitive commands are
/// still excluded.
fn run_advanced_surface_corpus(client: &mut Client, suffix: &[u8]) {
    let bulk = |value: &[u8]| Resp::Bulk(Some(value.to_vec()));
    let array = |items: Vec<Resp>| Resp::Array(Some(items));

    let geo = key(b"rillcache:baseline:adv-geo:", suffix);
    let geo_dst = key(b"rillcache:baseline:adv-geo-dst:", suffix);
    assert_command(
        client,
        &[
            b"GEOADD",
            &geo,
            b"13.361389",
            b"38.115556",
            b"Palermo",
            b"15.087269",
            b"37.502669",
            b"Catania",
        ],
        Resp::Integer(2),
    );
    assert_command(
        client,
        &[b"GEOHASH", &geo, b"Palermo", b"Catania"],
        array(vec![bulk(b"sqc8b49rny0"), bulk(b"sqdtr74hyu0")]),
    );
    assert_command(
        client,
        &[
            b"GEOSEARCH",
            &geo,
            b"FROMMEMBER",
            b"Palermo",
            b"BYRADIUS",
            b"200",
            b"km",
            b"ASC",
        ],
        array(vec![bulk(b"Palermo"), bulk(b"Catania")]),
    );
    assert_command(
        client,
        &[
            b"GEOSEARCHSTORE",
            &geo_dst,
            &geo,
            b"FROMMEMBER",
            b"Palermo",
            b"BYRADIUS",
            b"200",
            b"km",
            b"ASC",
        ],
        Resp::Integer(2),
    );
    assert_command(client, &[b"ZCARD", &geo_dst], Resp::Integer(2));

    let hash = key(b"rillcache:baseline:adv-hash:", suffix);
    assert_command(
        client,
        &[b"HSET", &hash, b"f", b"v", b"n", b"1"],
        Resp::Integer(2),
    );
    assert_command(
        client,
        &[b"HEXPIRE", &hash, b"100", b"FIELDS", b"1", b"f"],
        array(vec![Resp::Integer(1)]),
    );
    assert_command(
        client,
        &[b"HTTL", &hash, b"FIELDS", b"1", b"f"],
        array(vec![Resp::Integer(100)]),
    );
    assert_command(
        client,
        &[b"HPERSIST", &hash, b"FIELDS", b"1", b"f"],
        array(vec![Resp::Integer(1)]),
    );
    assert_command(
        client,
        &[b"HPTTL", &hash, b"FIELDS", b"1", b"f"],
        array(vec![Resp::Integer(-1)]),
    );
    assert_command(
        client,
        &[b"HINCRBYFLOAT", &hash, b"n", b"1.5"],
        bulk(b"2.5"),
    );

    let psetex = key(b"rillcache:baseline:adv-psetex:", suffix);
    assert_command(
        client,
        &[b"PSETEX", &psetex, b"100000", b"v"],
        Resp::Simple(b"OK".to_vec()),
    );
    assert_command(client, &[b"GET", &psetex], bulk(b"v"));
    let float = key(b"rillcache:baseline:adv-float:", suffix);
    assert_command(
        client,
        &[b"SET", &float, b"1"],
        Resp::Simple(b"OK".to_vec()),
    );
    assert_command(client, &[b"INCRBYFLOAT", &float, b"1.5"], bulk(b"2.5"));

    let list = key(b"rillcache:baseline:adv-list:", suffix);
    let list2 = key(b"rillcache:baseline:adv-list2:", suffix);
    assert_command(client, &[b"RPUSH", &list, b"b", b"a"], Resp::Integer(2));
    assert_command(client, &[b"LPUSH", &list, b"c"], Resp::Integer(3));
    assert_command(client, &[b"RPOPLPUSH", &list, &list2], bulk(b"a"));
    assert_command(
        client,
        &[b"LRANGE", &list, b"0", b"-1"],
        array(vec![bulk(b"c"), bulk(b"b")]),
    );
    assert_command(
        client,
        &[b"LRANGE", &list2, b"0", b"-1"],
        array(vec![bulk(b"a")]),
    );

    // Immediately satisfiable blocking forms return the ordinary pop replies.
    let pop1 = key(b"rillcache:baseline:adv-pop1:", suffix);
    let pop2 = key(b"rillcache:baseline:adv-pop2:", suffix);
    let move_src = key(b"rillcache:baseline:adv-move-src:", suffix);
    let move_dst = key(b"rillcache:baseline:adv-move-dst:", suffix);
    let mpop = key(b"rillcache:baseline:adv-mpop:", suffix);
    let brpoplpush_src = key(b"rillcache:baseline:adv-brpl-src:", suffix);
    let brpoplpush_dst = key(b"rillcache:baseline:adv-brpl-dst:", suffix);
    assert_command(client, &[b"RPUSH", &pop1, b"x"], Resp::Integer(1));
    assert_command(
        client,
        &[b"BLPOP", &pop1, b"0"],
        array(vec![bulk(&pop1), bulk(b"x")]),
    );
    assert_command(client, &[b"RPUSH", &pop2, b"y"], Resp::Integer(1));
    assert_command(
        client,
        &[b"BRPOP", &pop2, b"0"],
        array(vec![bulk(&pop2), bulk(b"y")]),
    );
    assert_command(client, &[b"RPUSH", &move_src, b"m"], Resp::Integer(1));
    assert_command(
        client,
        &[b"BLMOVE", &move_src, &move_dst, b"LEFT", b"RIGHT", b"0"],
        bulk(b"m"),
    );
    assert_command(client, &[b"RPUSH", &mpop, b"p", b"q"], Resp::Integer(2));
    assert_command(
        client,
        &[b"BLMPOP", b"0", b"1", &mpop, b"LEFT"],
        array(vec![bulk(&mpop), array(vec![bulk(b"p")])]),
    );
    assert_command(client, &[b"RPUSH", &brpoplpush_src, b"z"], Resp::Integer(1));
    assert_command(
        client,
        &[b"BRPOPLPUSH", &brpoplpush_src, &brpoplpush_dst, b"0"],
        bulk(b"z"),
    );

    let zset = key(b"rillcache:baseline:adv-zset:", suffix);
    let zset_dst = key(b"rillcache:baseline:adv-zset-dst:", suffix);
    assert_command(
        client,
        &[b"ZADD", &zset, b"1", b"a", b"2", b"b"],
        Resp::Integer(2),
    );
    assert_command(
        client,
        &[b"ZPOPMAX", &zset],
        array(vec![bulk(b"b"), bulk(b"2")]),
    );
    assert_command(
        client,
        &[b"ZREVRANGE", &zset, b"0", b"-1"],
        array(vec![bulk(b"a")]),
    );
    assert_command(client, &[b"ZADD", &zset, b"3", b"c"], Resp::Integer(1));
    assert_command(
        client,
        &[b"ZRANGESTORE", &zset_dst, &zset, b"0", b"-1"],
        Resp::Integer(2),
    );
    assert_command(
        client,
        &[b"ZRANGE", &zset_dst, b"0", b"-1"],
        array(vec![bulk(b"a"), bulk(b"c")]),
    );
    let bz1 = key(b"rillcache:baseline:adv-bz1:", suffix);
    let bz2 = key(b"rillcache:baseline:adv-bz2:", suffix);
    assert_command(client, &[b"ZADD", &bz1, b"1", b"a"], Resp::Integer(1));
    assert_command(
        client,
        &[b"BZPOPMIN", &bz1, b"0"],
        array(vec![bulk(&bz1), bulk(b"a"), bulk(b"1")]),
    );
    assert_command(client, &[b"ZADD", &bz2, b"2", b"b"], Resp::Integer(1));
    assert_command(
        client,
        &[b"BZPOPMAX", &bz2, b"0"],
        array(vec![bulk(&bz2), bulk(b"b"), bulk(b"2")]),
    );

    assert_command(
        client,
        &[b"ROLE"],
        array(vec![bulk(b"master"), Resp::Integer(0), array(vec![])]),
    );
    assert_command(client, &[b"MODULE", b"LIST"], array(vec![]));
    assert_command(client, &[b"ACL", b"WHOAMI"], bulk(b"default"));
    assert_command(
        client,
        &[b"CONFIG", b"GET", b"maxmemory"],
        array(vec![bulk(b"maxmemory"), bulk(b"67108864")]),
    );
    assert_command(
        client,
        &[b"CLUSTER", b"INFO"],
        Resp::Error(b"ERR This instance has cluster support disabled".to_vec()),
    );
    assert_command(client, &[b"WAIT", b"0", b"0"], Resp::Integer(0));
    assert_command(client, &[b"UNWATCH"], Resp::Simple(b"OK".to_vec()));
    assert_command(client, &[b"RESET"], Resp::Simple(b"RESET".to_vec()));

    // DELEX/DELIFEQ/DIGEST are published extensions absent from both pinned
    // references, so they are covered by the Rust tests rather than this corpus,
    // which is replayed against the references by `make compare`.

    assert_command(client, &[b"FLUSHDB"], Resp::Simple(b"OK".to_vec()));
    assert_command(client, &[b"FLUSHALL"], Resp::Simple(b"OK".to_vec()));
}
/// Extended surface cases (V2): deterministic replies for the bit, field-expiry,
/// sorting, geo-radius-read and simple administrative commands that remained
/// pending after the first three surface blocks.
fn run_extended_surface_corpus(client: &mut Client, suffix: &[u8]) {
    let bulk = |value: &[u8]| Resp::Bulk(Some(value.to_vec()));
    let array = |items: Vec<Resp>| Resp::Array(Some(items));

    // The preceding block flushes the keyspace, so the count is deterministic.
    assert_command(client, &[b"DBSIZE"], Resp::Integer(0));

    let bits = key(b"rillcache:baseline:ext-bits:", suffix);
    assert_command(
        client,
        &[b"BITFIELD", &bits, b"SET", b"u8", b"0", b"255"],
        array(vec![Resp::Integer(0)]),
    );
    assert_command(
        client,
        &[b"BITFIELD_RO", &bits, b"GET", b"u8", b"0"],
        array(vec![Resp::Integer(255)]),
    );
    let bits2 = key(b"rillcache:baseline:ext-bits2:", suffix);
    assert_command(
        client,
        &[b"BITFIELD", &bits2, b"SET", b"u8", b"0", b"15"],
        array(vec![Resp::Integer(0)]),
    );
    let bits3 = key(b"rillcache:baseline:ext-bits3:", suffix);
    assert_command(
        client,
        &[b"BITOP", b"AND", &bits3, &bits, &bits2],
        Resp::Integer(1),
    );
    assert_command(client, &[b"GET", &bits3], bulk(&[15]));

    let hash = key(b"rillcache:baseline:ext-hash:", suffix);
    assert_command(
        client,
        &[b"HSETEX", &hash, b"EX", b"100", b"FIELDS", b"1", b"f", b"v"],
        Resp::Integer(1),
    );
    assert_command(
        client,
        &[b"HGETEX", &hash, b"FIELDS", b"1", b"f"],
        array(vec![bulk(b"v")]),
    );
    assert_command(
        client,
        &[b"HGETDEL", &hash, b"FIELDS", b"1", b"f"],
        array(vec![bulk(b"v")]),
    );
    assert_command(
        client,
        &[b"HSETEX", &hash, b"FIELDS", b"1", b"f", b"v"],
        Resp::Integer(1),
    );
    let deadline = b"4102444800";
    assert_command(
        client,
        &[b"HEXPIREAT", &hash, deadline, b"FIELDS", b"1", b"f"],
        array(vec![Resp::Integer(1)]),
    );
    assert_command(
        client,
        &[b"HEXPIRETIME", &hash, b"FIELDS", b"1", b"f"],
        array(vec![Resp::Integer(4_102_444_800)]),
    );
    assert_command(
        client,
        &[b"HPEXPIRE", &hash, b"100000", b"FIELDS", b"1", b"f"],
        array(vec![Resp::Integer(1)]),
    );
    assert_command(
        client,
        &[
            b"HPEXPIREAT",
            &hash,
            b"4102444800000",
            b"FIELDS",
            b"1",
            b"f",
        ],
        array(vec![Resp::Integer(1)]),
    );
    assert_command(
        client,
        &[b"HPEXPIRETIME", &hash, b"FIELDS", b"1", b"f"],
        array(vec![Resp::Integer(4_102_444_800_000)]),
    );

    let list = key(b"rillcache:baseline:ext-sort:", suffix);
    assert_command(
        client,
        &[b"RPUSH", &list, b"c", b"a", b"b"],
        Resp::Integer(3),
    );
    assert_command(
        client,
        &[b"SORT", &list, b"ALPHA"],
        array(vec![bulk(b"a"), bulk(b"b"), bulk(b"c")]),
    );
    assert_command(
        client,
        &[b"SORT_RO", &list, b"ALPHA"],
        array(vec![bulk(b"a"), bulk(b"b"), bulk(b"c")]),
    );

    assert_command(
        client,
        &[b"PUBLISH", b"rillcache:baseline:ext-ch", b"m"],
        Resp::Integer(0),
    );
    assert_command(
        client,
        &[b"SPUBLISH", b"rillcache:baseline:ext-ch", b"m"],
        Resp::Integer(0),
    );

    let zset = key(b"rillcache:baseline:ext-zset:", suffix);
    assert_command(client, &[b"ZADD", &zset, b"1", b"a"], Resp::Integer(1));
    assert_command(
        client,
        &[b"BZMPOP", b"0", b"1", &zset, b"MIN"],
        array(vec![
            bulk(&zset),
            array(vec![array(vec![bulk(b"a"), bulk(b"1")])]),
        ]),
    );

    let stream = key(b"rillcache:baseline:ext-stream:", suffix);
    assert_command(
        client,
        &[b"XADD", &stream, b"1-1", b"f", b"v"],
        Resp::Bulk(Some(b"1-1".to_vec())),
    );
    assert_command(
        client,
        &[b"XSETID", &stream, b"9-9"],
        Resp::Simple(b"OK".to_vec()),
    );
    assert_command(client, &[b"DEL", &stream], Resp::Integer(1));

    let geo = key(b"rillcache:baseline:ext-geo:", suffix);
    assert_command(
        client,
        &[
            b"GEOADD",
            &geo,
            b"13.361389",
            b"38.115556",
            b"Palermo",
            b"15.087269",
            b"37.502669",
            b"Catania",
        ],
        Resp::Integer(2),
    );
    assert_command(
        client,
        &[b"GEORADIUS_RO", &geo, b"15", b"37", b"200", b"km", b"ASC"],
        array(vec![bulk(b"Catania"), bulk(b"Palermo")]),
    );
    assert_command(
        client,
        &[
            b"GEORADIUSBYMEMBER_RO",
            &geo,
            b"Palermo",
            b"200",
            b"km",
            b"ASC",
        ],
        array(vec![bulk(b"Palermo"), bulk(b"Catania")]),
    );
    assert_command(
        client,
        &[b"GEORADIUS", &geo, b"15", b"37", b"200", b"km", b"ASC"],
        array(vec![bulk(b"Catania"), bulk(b"Palermo")]),
    );
    assert_command(
        client,
        &[
            b"GEORADIUSBYMEMBER",
            &geo,
            b"Palermo",
            b"200",
            b"km",
            b"ASC",
        ],
        array(vec![bulk(b"Palermo"), bulk(b"Catania")]),
    );

    let msetex_a = key(b"rillcache:baseline:ext-msetex-a:", suffix);
    let msetex_b = key(b"rillcache:baseline:ext-msetex-b:", suffix);
    assert_command(
        client,
        &[
            b"MSETEX", b"2", &msetex_a, b"1", &msetex_b, b"2", b"EX", b"100",
        ],
        Resp::Integer(1),
    );
    assert_command(client, &[b"GET", &msetex_a], bulk(b"1"));

    assert_command(
        client,
        &[b"WAITAOF", b"0", b"1", b"10"],
        array(vec![Resp::Integer(0), Resp::Integer(0)]),
    );
    assert_command(
        client,
        &[b"BGREWRITEAOF"],
        Resp::Error(b"ERR append-only journal is not enabled or cannot be rewritten".to_vec()),
    );

    let group_stream = key(b"rillcache:baseline:ext-group:", suffix);
    assert_command(
        client,
        &[b"XADD", &group_stream, b"1-1", b"f", b"v"],
        Resp::Bulk(Some(b"1-1".to_vec())),
    );
    assert_command(
        client,
        &[b"XGROUP", b"CREATE", &group_stream, b"g1", b"0"],
        Resp::Simple(b"OK".to_vec()),
    );
    assert_command(
        client,
        &[
            b"XREADGROUP",
            b"GROUP",
            b"g1",
            b"c1",
            b"COUNT",
            b"1",
            b"STREAMS",
            &group_stream,
            b">",
        ],
        array(vec![array(vec![
            bulk(&group_stream),
            array(vec![array(vec![
                bulk(b"1-1"),
                array(vec![bulk(b"f"), bulk(b"v")]),
            ])]),
        ])]),
    );
    assert_command(
        client,
        &[b"XCLAIM", &group_stream, b"g1", b"c2", b"0", b"1-1"],
        array(vec![array(vec![
            bulk(b"1-1"),
            array(vec![bulk(b"f"), bulk(b"v")]),
        ])]),
    );
    assert_command(
        client,
        &[b"XAUTOCLAIM", &group_stream, b"g1", b"c3", b"0", b"0-0"],
        array(vec![
            bulk(b"0-0"),
            array(vec![array(vec![
                bulk(b"1-1"),
                array(vec![bulk(b"f"), bulk(b"v")]),
            ])]),
            array(vec![]),
        ]),
    );

    // Wrong-type edges: the exact vendor-shared error for a list, string and
    // sorted-set mismatch, and that the healthy key is unchanged afterwards.
    let wrong_string = key(b"rillcache:baseline:ext-wrong-string:", suffix);
    let wrong_list = key(b"rillcache:baseline:ext-wrong-list:", suffix);
    let wrong_zset = key(b"rillcache:baseline:ext-wrong-zset:", suffix);
    assert_command(
        client,
        &[b"SET", &wrong_string, b"v"],
        Resp::Simple(b"OK".to_vec()),
    );
    assert_command(
        client,
        &[b"LPUSH", &wrong_string, b"x"],
        Resp::Error(b"WRONGTYPE Operation against a key holding the wrong kind of value".to_vec()),
    );
    assert_command(client, &[b"RPUSH", &wrong_list, b"x"], Resp::Integer(1));
    assert_command(
        client,
        &[b"GET", &wrong_list],
        Resp::Error(b"WRONGTYPE Operation against a key holding the wrong kind of value".to_vec()),
    );
    assert_command(
        client,
        &[b"ZADD", &wrong_zset, b"1", b"a"],
        Resp::Integer(1),
    );
    assert_command(
        client,
        &[b"LPOP", &wrong_zset],
        Resp::Error(b"WRONGTYPE Operation against a key holding the wrong kind of value".to_vec()),
    );
    assert_command(client, &[b"GET", &wrong_string], bulk(b"v"));
}
/// Pub/Sub surface cases on a dedicated connection: subscribing changes the
/// connection mode, so the shared corpus client cannot be reused. The frames are
/// deterministic counts and are asserted before the connection is dropped.
fn run_pubsub_surface_corpus(address: SocketAddr) {
    let mut client = Client::connect(address).unwrap();
    let bulk = |value: &[u8]| Resp::Bulk(Some(value.to_vec()));
    let array = |items: Vec<Resp>| Resp::Array(Some(items));
    assert_command(
        &mut client,
        &[b"SUBSCRIBE", b"rillcache:baseline:ps-channel"],
        array(vec![
            bulk(b"subscribe"),
            bulk(b"rillcache:baseline:ps-channel"),
            Resp::Integer(1),
        ]),
    );
    assert_command(
        &mut client,
        &[b"UNSUBSCRIBE", b"rillcache:baseline:ps-channel"],
        array(vec![
            bulk(b"unsubscribe"),
            bulk(b"rillcache:baseline:ps-channel"),
            Resp::Integer(0),
        ]),
    );
    assert_command(
        &mut client,
        &[b"SSUBSCRIBE", b"rillcache:baseline:ps-shard"],
        array(vec![
            bulk(b"ssubscribe"),
            bulk(b"rillcache:baseline:ps-shard"),
            Resp::Integer(1),
        ]),
    );
    assert_command(
        &mut client,
        &[b"SUNSUBSCRIBE", b"rillcache:baseline:ps-shard"],
        array(vec![
            bulk(b"sunsubscribe"),
            bulk(b"rillcache:baseline:ps-shard"),
            Resp::Integer(0),
        ]),
    );
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

fn assert_pattern_count(client: &mut Client, count: i64) {
    assert_command(client, &[b"PUBSUB", b"NUMPAT"], Resp::Integer(count));
    let Resp::Bulk(Some(stats)) = client.command(&[b"INFO", b"STATS"]).unwrap() else {
        panic!("INFO STATS did not return a bulk string");
    };
    let expected = format!("pubsub_patterns:{count}");
    assert!(
        stats
            .split(|byte| *byte == b'\n')
            .any(|line| { line.strip_suffix(b"\r") == Some(expected.as_bytes()) }),
        "expected {expected} in INFO STATS"
    );
}

fn run_pubsub_pattern_corpus(address: SocketAddr, suffix: &[u8]) {
    let mut observer = Client::connect(address).unwrap();
    let mut first = Client::connect(address).unwrap();
    let mut second = Client::connect(address).unwrap();
    let shared = key(b"rillcache:baseline:\0shared:*", suffix);
    let other = key(b"rillcache:baseline:\xffother:*", suffix);
    let ack = |verb: &[u8], pattern: &[u8], count| {
        Resp::Array(Some(vec![
            Resp::Bulk(Some(verb.to_vec())),
            Resp::Bulk(Some(pattern.to_vec())),
            Resp::Integer(count),
        ]))
    };

    assert_pattern_count(&mut observer, 0);
    assert_command(
        &mut first,
        &[b"PSUBSCRIBE", &shared],
        ack(b"psubscribe", &shared, 1),
    );
    for _ in 0..2 {
        assert_command(
            &mut second,
            &[b"PSUBSCRIBE", &shared],
            ack(b"psubscribe", &shared, 1),
        );
    }
    assert_pattern_count(&mut observer, 1);
    assert_command(
        &mut first,
        &[b"PSUBSCRIBE", &other],
        ack(b"psubscribe", &other, 2),
    );
    assert_pattern_count(&mut observer, 2);
    assert_command(
        &mut first,
        &[b"PUNSUBSCRIBE", &shared],
        ack(b"punsubscribe", &shared, 1),
    );
    assert_pattern_count(&mut observer, 2);
    assert_command(
        &mut second,
        &[b"PUNSUBSCRIBE", &shared],
        ack(b"punsubscribe", &shared, 0),
    );
    assert_pattern_count(&mut observer, 1);
    assert_command(
        &mut first,
        &[b"PUNSUBSCRIBE", &other],
        ack(b"punsubscribe", &other, 0),
    );
    assert_pattern_count(&mut observer, 0);
}

fn run_lpos_rank_corpus(address: SocketAddr, suffix: &[u8]) {
    let mut client = Client::connect(address).unwrap();
    let list = key(b"rillcache:baseline:list-rank:", suffix);
    let missing = key(b"rillcache:baseline:list-missing:", suffix);
    let value = b"\0\xff";
    assert_command(
        &mut client,
        &[b"RPUSH", &list, value, b"gap", value],
        Resp::Integer(3),
    );
    for key in [&list, &missing] {
        assert_command(
            &mut client,
            &[b"LPOS", key, value, b"RANK", b"-9223372036854775808"],
            Resp::Error(b"ERR value is out of range, value must between -9223372036854775807 and 9223372036854775807".to_vec()),
        );
    }
    for rank in [b"-9223372036854775807".as_slice(), b"9223372036854775807"] {
        assert_command(
            &mut client,
            &[b"LPOS", &list, value, b"RANK", rank],
            Resp::Bulk(None),
        );
    }
    assert_command(
        &mut client,
        &[b"LPOS", &list, value, b"RANK", b"-1"],
        Resp::Integer(2),
    );
    assert_command(&mut client, &[b"DEL", &list], Resp::Integer(1));
}

fn run_lrem_count_corpus(address: SocketAddr, suffix: &[u8]) {
    let mut client = Client::connect(address).unwrap();
    let list = key(b"rillcache:baseline:list-remove:", suffix);
    let missing = key(b"rillcache:baseline:list-remove-missing:", suffix);
    let value = b"\0\xff";
    assert_command(
        &mut client,
        &[b"RPUSH", &list, value, b"keep", value],
        Resp::Integer(3),
    );
    for key in [&list, &missing] {
        assert_command(
            &mut client,
            &[b"LREM", key, b"-9223372036854775808", value],
            Resp::Error(b"ERR value is out of range, value must between -9223372036854775807 and 9223372036854775807".to_vec()),
        );
    }
    assert_command(
        &mut client,
        &[b"LRANGE", &list, b"0", b"-1"],
        Resp::Array(Some(
            [value.as_slice(), b"keep", value]
                .map(|item| Resp::Bulk(Some(item.to_vec())))
                .into(),
        )),
    );
    assert_command(
        &mut client,
        &[b"LREM", &list, b"-9223372036854775807", value],
        Resp::Integer(2),
    );
    assert_command(
        &mut client,
        &[b"LINDEX", &list, b"0"],
        Resp::Bulk(Some(b"keep".to_vec())),
    );
    assert_command(&mut client, &[b"DEL", &list], Resp::Integer(1));
}

fn run_conditional_write_corpus(client: &mut Client, suffix: &[u8]) {
    let existing = key(b"rillcache:baseline:nx-existing:\0", suffix);
    let absent = key(b"rillcache:baseline:nx-absent:", suffix);
    let hash = key(b"rillcache:baseline:nx-hash:", suffix);
    let field = b"\0field\xff";
    let value = b"a\0\xff";
    let bulk = || Resp::Bulk(Some(value.to_vec()));
    let deadline = 4_102_444_800_123;

    assert_command(
        client,
        &[b"SET", &existing, value, b"PXAT", b"4102444800123"],
        Resp::Simple(b"OK".to_vec()),
    );
    // Put the absent key first to catch a partial insertion before rejection.
    assert_command(
        client,
        &[b"MSETNX", &absent, b"new", &existing, b"overwrite"],
        Resp::Integer(0),
    );
    assert_command(client, &[b"GET", &absent], Resp::Bulk(None));
    assert_command(client, &[b"GET", &existing], bulk());
    assert_deadline(client, &existing, deadline);

    assert_command(
        client,
        &[b"HSETNX", &existing, field, b"wrong"],
        Resp::Error(b"WRONGTYPE Operation against a key holding the wrong kind of value".to_vec()),
    );
    assert_command(client, &[b"GET", &existing], bulk());
    assert_deadline(client, &existing, deadline);
    assert_command(client, &[b"HSETNX", &hash, field, value], Resp::Integer(1));
    assert_command(
        client,
        &[b"PEXPIREAT", &hash, b"4102444800123"],
        Resp::Integer(1),
    );
    assert_command(
        client,
        &[b"HSETNX", &hash, field, b"overwrite"],
        Resp::Integer(0),
    );
    assert_command(client, &[b"HGET", &hash, field], bulk());
    assert_deadline(client, &hash, deadline);
    assert_command(client, &[b"HLEN", &hash], Resp::Integer(1));
    let empty_field = b"\0empty\xff";
    assert_command(
        client,
        &[b"HSETNX", &hash, empty_field, b""],
        Resp::Integer(1),
    );
    assert_command(
        client,
        &[b"HSETNX", &hash, empty_field, b"overwrite"],
        Resp::Integer(0),
    );
    assert_command(
        client,
        &[b"HGET", &hash, empty_field],
        Resp::Bulk(Some(Vec::new())),
    );
    assert_deadline(client, &hash, deadline);
    assert_command(client, &[b"HLEN", &hash], Resp::Integer(2));

    assert_command(client, &[b"DEL", &existing, &hash], Resp::Integer(2));
    assert_command(
        client,
        &[b"MSETNX", &existing, value, &absent, b""],
        Resp::Integer(1),
    );
    assert_command(
        client,
        &[b"MGET", &existing, &absent],
        Resp::Array(Some(vec![bulk(), Resp::Bulk(Some(Vec::new()))])),
    );
    assert_deadline(client, &existing, -1);
    assert_deadline(client, &absent, -1);
    assert_command(client, &[b"DEL", &existing, &absent], Resp::Integer(2));
}

fn run_supported_corpus(address: SocketAddr, suffix: &[u8], kind: &str) {
    run_pubsub_pattern_corpus(address, suffix);
    run_lpos_rank_corpus(address, suffix);
    run_lrem_count_corpus(address, suffix);
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
    run_conditional_write_corpus(&mut client, suffix);
    run_transaction_corpus(address, suffix);
    run_stream_trim_read_corpus(&mut client, suffix);
    run_command_surface_corpus(&mut client, suffix);
    run_collection_surface_corpus(&mut client, suffix);
    run_advanced_surface_corpus(&mut client, suffix);
    run_extended_surface_corpus(&mut client, suffix);
    run_pubsub_surface_corpus(address);

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
