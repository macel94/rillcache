//! Bounded Redis-style command and server discovery.

use super::Store;
use crate::protocol::encode_bulk;
use std::fmt::Write as _;
use std::io::Write as _;

#[derive(Clone, Copy)]
struct Command {
    name: &'static str,
    arity: i64,
    flags: &'static [&'static str],
    first_key: i64,
    last_key: i64,
    key_step: i64,
}

// Every registered command is wired to this worktree's Store or server dispatcher.
const COMMANDS: &[Command] = &[
    cmd("acl", -2, &["admin", "dangerous", "slow"], 0, 0, 0),
    cmd("append", 3, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd(
        "auth",
        -2,
        &["noscript", "loading", "stale", "fast"],
        0,
        0,
        0,
    ),
    cmd("bgrewriteaof", 1, &["admin", "dangerous", "slow"], 0, 0, 0),
    cmd("bitcount", -2, &["readonly"], 1, 1, 1),
    cmd("bitfield", -2, &["write", "denyoom"], 1, 1, 1),
    cmd("bitfield_ro", -2, &["readonly"], 1, 1, 1),
    cmd("bitop", -4, &["write", "denyoom"], 1, -1, 1),
    cmd("bitpos", -3, &["readonly"], 1, 1, 1),
    cmd("blpop", -3, &["write", "noscript", "blocking"], 1, -2, 1),
    cmd("brpop", -3, &["write", "noscript", "blocking"], 1, -2, 1),
    cmd(
        "client",
        -2,
        &["admin", "noscript", "loading", "stale"],
        0,
        0,
        0,
    ),
    cmd("command", -1, &["loading", "stale"], 0, 0, 0),
    cmd("config", -2, &["admin", "noscript"], 0, 0, 0),
    cmd("copy", -3, &["write", "denyoom"], 1, 2, 1),
    cmd("dbsize", 1, &["readonly", "fast"], 0, 0, 0),
    cmd("decr", 2, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("decrby", 3, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("del", -2, &["write", "fast"], 1, -1, 1),
    cmd("delex", -2, &["write", "fast"], 1, 1, 1),
    cmd("delifeq", 3, &["write", "fast"], 1, 1, 1),
    cmd("digest", 2, &["readonly", "fast"], 1, 1, 1),
    cmd(
        "discard",
        1,
        &["noscript", "loading", "stale", "fast"],
        0,
        0,
        0,
    ),
    cmd("echo", 2, &["fast"], 0, 0, 0),
    cmd(
        "exec",
        1,
        &[
            "noscript",
            "loading",
            "stale",
            "skip_monitor",
            "skip_slowlog",
        ],
        0,
        0,
        0,
    ),
    cmd("exists", -2, &["readonly", "fast"], 1, -1, 1),
    cmd("expire", -3, &["write", "fast"], 1, 1, 1),
    cmd("expireat", -3, &["write", "fast"], 1, 1, 1),
    cmd("expiretime", 2, &["readonly", "fast"], 1, 1, 1),
    cmd("flushall", -1, &["write", "slow"], 0, 0, 0),
    cmd("flushdb", -1, &["write", "slow"], 0, 0, 0),
    cmd("function", -2, &["write", "admin"], 0, 0, 0),
    cmd("geoadd", -5, &["write", "denyoom"], 1, 1, 1),
    cmd("geodist", -4, &["readonly"], 1, 1, 1),
    cmd("geohash", -2, &["readonly"], 1, 1, 1),
    cmd("geopos", -2, &["readonly"], 1, 1, 1),
    cmd("geosearch", -7, &["readonly"], 1, 1, 1),
    cmd("geosearchstore", -8, &["write", "denyoom"], 1, 2, 1),
    cmd("get", 2, &["readonly", "fast"], 1, 1, 1),
    cmd("getbit", 3, &["readonly", "fast"], 1, 1, 1),
    cmd("getdel", 2, &["write", "fast"], 1, 1, 1),
    cmd("getex", -2, &["write", "fast"], 1, 1, 1),
    cmd("getrange", 4, &["readonly"], 1, 1, 1),
    cmd("getset", 3, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("hdel", -3, &["write", "fast"], 1, 1, 1),
    cmd("hexists", 3, &["readonly", "fast"], 1, 1, 1),
    cmd("hget", 3, &["readonly", "fast"], 1, 1, 1),
    cmd("hgetall", 2, &["readonly"], 1, 1, 1),
    cmd(
        "hello",
        -1,
        &["noscript", "loading", "stale", "fast"],
        0,
        0,
        0,
    ),
    cmd("hincrby", 4, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("hincrbyfloat", 4, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("hkeys", 2, &["readonly"], 1, 1, 1),
    cmd("hlen", 2, &["readonly", "fast"], 1, 1, 1),
    cmd("hmget", -3, &["readonly", "fast"], 1, 1, 1),
    cmd("hscan", -3, &["readonly", "slow"], 1, 1, 1),
    cmd("hset", -4, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("hsetnx", 4, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("hstrlen", 3, &["readonly", "fast"], 1, 1, 1),
    cmd("hvals", 2, &["readonly"], 1, 1, 1),
    cmd("incr", 2, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("incrby", 3, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("incrbyfloat", 3, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("info", -1, &["loading", "stale"], 0, 0, 0),
    cmd("keys", 2, &["readonly"], 0, 0, 0),
    cmd("lcs", -3, &["readonly"], 1, 2, 1),
    cmd("lindex", 3, &["readonly"], 1, 1, 1),
    cmd("linsert", 5, &["write", "denyoom"], 1, 1, 1),
    cmd("llen", 2, &["readonly", "fast"], 1, 1, 1),
    cmd("lpop", -2, &["write", "fast"], 1, 1, 1),
    cmd("lpush", -3, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("lpushx", -3, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("lrange", 4, &["readonly"], 1, 1, 1),
    cmd("lrem", 4, &["write"], 1, 1, 1),
    cmd("lset", 4, &["write", "denyoom"], 1, 1, 1),
    cmd("ltrim", 4, &["write"], 1, 1, 1),
    cmd("memory", -2, &["readonly", "slow"], 0, 0, 0),
    cmd("mget", -2, &["readonly", "fast"], 1, -1, 1),
    cmd("mset", -3, &["write", "denyoom"], 1, -1, 2),
    cmd("msetex", -4, &["write", "denyoom"], 2, -1, 2),
    cmd("msetnx", -3, &["write", "denyoom"], 1, -1, 2),
    cmd(
        "multi",
        1,
        &["noscript", "loading", "stale", "fast"],
        0,
        0,
        0,
    ),
    cmd("persist", 2, &["write", "fast"], 1, 1, 1),
    cmd("pexpire", -3, &["write", "fast"], 1, 1, 1),
    cmd("pexpireat", -3, &["write", "fast"], 1, 1, 1),
    cmd("pexpiretime", 2, &["readonly", "fast"], 1, 1, 1),
    cmd("pfadd", -2, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("pfcount", -2, &["readonly"], 1, -1, 1),
    cmd("pfmerge", -2, &["write", "denyoom"], 1, -1, 1),
    cmd("ping", -1, &["fast"], 0, 0, 0),
    cmd("psetex", 4, &["write", "denyoom"], 1, 1, 1),
    cmd(
        "psubscribe",
        -2,
        &["pubsub", "noscript", "loading", "stale"],
        0,
        0,
        0,
    ),
    cmd("pttl", 2, &["readonly", "fast"], 1, 1, 1),
    cmd(
        "publish",
        3,
        &["pubsub", "loading", "stale", "fast"],
        0,
        0,
        0,
    ),
    cmd(
        "pubsub",
        -2,
        &["pubsub", "loading", "stale", "fast"],
        0,
        0,
        0,
    ),
    cmd(
        "punsubscribe",
        -1,
        &["pubsub", "noscript", "loading", "stale"],
        0,
        0,
        0,
    ),
    cmd("quit", -1, &["fast"], 0, 0, 0),
    cmd("randomkey", 1, &["readonly"], 0, 0, 0),
    cmd("rename", 3, &["write"], 1, 2, 1),
    cmd("renamenx", 3, &["write", "fast"], 1, 2, 1),
    cmd(
        "reset",
        1,
        &["noscript", "loading", "stale", "fast"],
        0,
        0,
        0,
    ),
    cmd("rpop", -2, &["write", "fast"], 1, 1, 1),
    cmd("rpush", -3, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("rpushx", -3, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("sadd", -3, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("scan", -2, &["readonly", "slow"], 0, 0, 0),
    cmd("scard", 2, &["readonly", "fast"], 1, 1, 1),
    cmd("script", -2, &["admin"], 0, 0, 0),
    cmd("sdiff", -2, &["readonly"], 1, -1, 1),
    cmd("sdiffstore", -3, &["write", "denyoom"], 1, -1, 1),
    cmd("select", 2, &["loading", "stale", "fast"], 0, 0, 0),
    cmd("set", -3, &["write", "denyoom"], 1, 1, 1),
    cmd("setbit", 4, &["write", "denyoom"], 1, 1, 1),
    cmd("setex", 4, &["write", "denyoom"], 1, 1, 1),
    cmd("setnx", 3, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("setrange", 4, &["write", "denyoom"], 1, 1, 1),
    cmd("sinter", -2, &["readonly"], 1, -1, 1),
    cmd("sintercard", -3, &["readonly"], 2, -1, 1),
    cmd("sinterstore", -3, &["write", "denyoom"], 1, -1, 1),
    cmd("sismember", 3, &["readonly", "fast"], 1, 1, 1),
    cmd("smembers", 2, &["readonly"], 1, 1, 1),
    cmd("smismember", -3, &["readonly", "fast"], 1, 1, 1),
    cmd("smove", 4, &["write", "fast"], 1, 2, 1),
    cmd("spop", -2, &["write", "fast"], 1, 1, 1),
    cmd(
        "spublish",
        3,
        &["pubsub", "loading", "stale", "fast"],
        0,
        0,
        0,
    ),
    cmd("srandmember", -2, &["readonly"], 1, 1, 1),
    cmd("srem", -3, &["write", "fast"], 1, 1, 1),
    cmd("sscan", -3, &["readonly", "slow"], 1, 1, 1),
    cmd(
        "ssubscribe",
        -2,
        &["pubsub", "noscript", "loading", "stale"],
        0,
        0,
        0,
    ),
    cmd("strlen", 2, &["readonly", "fast"], 1, 1, 1),
    cmd(
        "subscribe",
        -2,
        &["pubsub", "noscript", "loading", "stale"],
        0,
        0,
        0,
    ),
    cmd("substr", 4, &["readonly"], 1, 1, 1),
    cmd("sunion", -2, &["readonly"], 1, -1, 1),
    cmd("sunionstore", -3, &["write", "denyoom"], 1, -1, 1),
    cmd(
        "sunsubscribe",
        -1,
        &["pubsub", "noscript", "loading", "stale"],
        0,
        0,
        0,
    ),
    cmd("touch", -2, &["readonly", "fast"], 1, -1, 1),
    cmd("ttl", 2, &["readonly", "fast"], 1, 1, 1),
    cmd("type", 2, &["readonly", "fast"], 1, 1, 1),
    cmd(
        "unsubscribe",
        -1,
        &["pubsub", "noscript", "loading", "stale"],
        0,
        0,
        0,
    ),
    cmd(
        "unwatch",
        1,
        &["noscript", "loading", "stale", "fast"],
        0,
        0,
        0,
    ),
    cmd(
        "watch",
        -2,
        &["noscript", "loading", "stale", "fast"],
        1,
        -1,
        1,
    ),
    cmd("xack", -4, &["write", "fast"], 1, 1, 1),
    cmd("xadd", -5, &["write", "denyoom"], 1, 1, 1),
    cmd("xautoclaim", -6, &["write", "fast"], 1, 1, 1),
    cmd("xclaim", -6, &["write", "fast"], 1, 1, 1),
    cmd("xdel", -3, &["write"], 1, 1, 1),
    cmd("xgroup", -2, &["write"], 2, 2, 1),
    cmd("xinfo", -2, &["readonly"], 2, 2, 1),
    cmd("xlen", 2, &["readonly", "fast"], 1, 1, 1),
    cmd("xpending", -3, &["readonly", "slow"], 1, 1, 1),
    cmd("xrange", -4, &["readonly", "slow"], 1, 1, 1),
    cmd("xread", -4, &["readonly", "blocking"], 0, 0, 0),
    cmd("xreadgroup", -7, &["write"], 0, 0, 0),
    cmd("xrevrange", -4, &["readonly", "slow"], 1, 1, 1),
    cmd("xtrim", -4, &["write"], 1, 1, 1),
    cmd("zadd", -4, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("zcard", 2, &["readonly", "fast"], 1, 1, 1),
    cmd("zcount", 4, &["readonly", "fast"], 1, 1, 1),
    cmd("zdiff", -3, &["readonly"], 0, 0, 0),
    cmd("zdiffstore", -4, &["write", "denyoom"], 1, 1, 1),
    cmd("zincrby", 4, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("zinter", -3, &["readonly"], 0, 0, 0),
    cmd("zintercard", -3, &["readonly"], 0, 0, 0),
    cmd("zinterstore", -4, &["write", "denyoom"], 1, 1, 1),
    cmd("zlexcount", 4, &["readonly", "fast"], 1, 1, 1),
    cmd("zpopmax", -2, &["write", "fast"], 1, 1, 1),
    cmd("zpopmin", -2, &["write", "fast"], 1, 1, 1),
    cmd("zrandmember", -2, &["readonly"], 1, 1, 1),
    cmd("zrange", -4, &["readonly"], 1, 1, 1),
    cmd("zrangebyscore", -4, &["readonly"], 1, 1, 1),
    cmd("zrangestore", -5, &["write", "denyoom"], 1, 2, 1),
    cmd("zrank", -3, &["readonly", "fast"], 1, 1, 1),
    cmd("zrem", -3, &["write", "fast"], 1, 1, 1),
    cmd("zremrangebylex", 4, &["write"], 1, 1, 1),
    cmd("zremrangebyrank", 4, &["write"], 1, 1, 1),
    cmd("zremrangebyscore", 4, &["write"], 1, 1, 1),
    cmd("zrevrange", -4, &["readonly"], 1, 1, 1),
    cmd("zrevrangebyscore", -4, &["readonly"], 1, 1, 1),
    cmd("zrevrank", -3, &["readonly", "fast"], 1, 1, 1),
    cmd("zscan", -3, &["readonly", "slow"], 1, 1, 1),
    cmd("zscore", 3, &["readonly", "fast"], 1, 1, 1),
    cmd("zunion", -3, &["readonly"], 0, 0, 0),
    cmd("zunionstore", -4, &["write", "denyoom"], 1, 1, 1),
];

/// Arity from the published command table; `None` means unsupported.
pub(super) fn command_arity(name: &[u8]) -> Option<i64> {
    COMMANDS
        .iter()
        .find(|command| name.eq_ignore_ascii_case(command.name.as_bytes()))
        .map(|command| command.arity)
}

pub(super) fn canonical_name(name: &[u8]) -> Option<&'static str> {
    COMMANDS
        .iter()
        .find(|command| name.eq_ignore_ascii_case(command.name.as_bytes()))
        .map(|command| command.name)
}

/// Declared key arguments for ACL key-pattern enforcement. Commands with no
/// static key spec (`KEYS`, `DBSIZE`, `FLUSH*`, `XREADGROUP`) return no keys;
/// this matches Redis's key-spec behavior and is documented as not a sandbox.
pub(super) fn key_arguments<'a>(name: &[u8], args: &'a [&'a [u8]]) -> Vec<&'a [u8]> {
    let Some(command) = COMMANDS
        .iter()
        .find(|command| name.eq_ignore_ascii_case(command.name.as_bytes()))
    else {
        return Vec::new();
    };
    // SINTERCARD has a counted key list followed by optional LIMIT arguments.
    // Its static `last_key` would otherwise include the LIMIT tokens as keys.
    if name.eq_ignore_ascii_case(b"SINTERCARD") {
        let Some(count) = args
            .get(1)
            .and_then(|raw| std::str::from_utf8(raw).ok())
            .and_then(|raw| raw.parse::<usize>().ok())
            .filter(|count| *count > 0)
        else {
            return Vec::new();
        };
        let end = 2usize.saturating_add(count).min(args.len());
        return args[2..end].to_vec();
    }
    // Counted-key sorted-set algebra keeps its key list after `numkeys`, with
    // the destination preceding it for the *STORE variants.
    let counted_zset = name.eq_ignore_ascii_case(b"ZUNION")
        || name.eq_ignore_ascii_case(b"ZINTER")
        || name.eq_ignore_ascii_case(b"ZDIFF")
        || name.eq_ignore_ascii_case(b"ZINTERCARD");
    let store_zset = name.eq_ignore_ascii_case(b"ZUNIONSTORE")
        || name.eq_ignore_ascii_case(b"ZINTERSTORE")
        || name.eq_ignore_ascii_case(b"ZDIFFSTORE");
    if counted_zset || store_zset {
        let numkeys_index = if store_zset { 2 } else { 1 };
        let Some(count) = args
            .get(numkeys_index)
            .and_then(|raw| std::str::from_utf8(raw).ok())
            .and_then(|raw| raw.parse::<usize>().ok())
            .filter(|count| *count > 0)
        else {
            return Vec::new();
        };
        let start = numkeys_index + 1;
        let end = start.saturating_add(count).min(args.len());
        let mut keys = Vec::with_capacity(end - start + usize::from(store_zset));
        if store_zset && let Some(destination) = args.get(1) {
            keys.push(*destination);
        }
        keys.extend_from_slice(&args[start..end]);
        return keys;
    }
    if command.first_key <= 0 {
        return Vec::new();
    }
    let last = if command.last_key < 0 {
        args.len() as i64 + command.last_key
    } else {
        command.last_key
    };
    let step = command.key_step.max(1);
    let mut keys = Vec::new();
    let mut index = command.first_key;
    while index <= last && index < args.len() as i64 {
        keys.push(args[index as usize]);
        index += step;
    }
    keys
}

pub(super) const CATEGORIES: &[&str] = &[
    "admin",
    "all",
    "bitmap",
    "connection",
    "dangerous",
    "fast",
    "geo",
    "hash",
    "keyspace",
    "list",
    "pubsub",
    "read",
    "set",
    "slow",
    "stream",
    "string",
    "transaction",
    "write",
    "zset",
];

/// Redis-style ACL category approximation over the published command table.
/// `@slow` covers commands without the `fast` flag; script/module categories do
/// not exist because neither capability does.
pub(super) fn category_matches(name: &[u8], _args: &[&[u8]], category: &[u8]) -> bool {
    let Some(command) = COMMANDS
        .iter()
        .find(|command| name.eq_ignore_ascii_case(command.name.as_bytes()))
    else {
        return false;
    };
    let has = |flag: &str| command.flags.contains(&flag);
    let name = command.name;
    let group = command_group(name);
    match category {
        b"all" => true,
        b"read" => has("readonly"),
        b"write" => has("write"),
        b"admin" => has("admin"),
        b"fast" => has("fast"),
        b"slow" => !has("fast"),
        b"dangerous" => matches!(
            name,
            "acl" | "config" | "flushall" | "flushdb" | "function" | "keys" | "script"
        ),
        b"connection" => group == Some("connection"),
        b"pubsub" => group == Some("pubsub"),
        b"transaction" => group == Some("transaction"),
        b"keyspace" | b"string" | b"list" | b"set" | b"hash" | b"zset" | b"stream" | b"bitmap"
        | b"hyperloglog" | b"geo" => group == Some(std::str::from_utf8(category).unwrap_or("")),
        _ => false,
    }
}

fn command_group(name: &str) -> Option<&'static str> {
    Some(match name {
        "append" | "decr" | "decrby" | "digest" | "get" | "getdel" | "getex" | "getrange"
        | "getset" | "incr" | "incrby" | "incrbyfloat" | "lcs" | "mget" | "mset" | "msetex"
        | "msetnx" | "psetex" | "set" | "setex" | "setnx" | "setrange" | "strlen" | "substr" => {
            "string"
        }
        "bitcount" | "bitfield" | "bitfield_ro" | "bitop" | "bitpos" | "getbit" | "setbit" => {
            "bitmap"
        }
        "geoadd" | "geodist" | "geohash" | "geopos" | "geosearch" | "geosearchstore" => "geo",
        "pfadd" | "pfcount" | "pfmerge" => "hyperloglog",
        "blpop" | "brpop" | "lindex" | "linsert" | "llen" | "lpop" | "lpush" | "lpushx"
        | "lrange" | "lrem" | "lset" | "ltrim" | "rpop" | "rpush" | "rpushx" => "list",
        "sadd" | "scard" | "sdiff" | "sdiffstore" | "sinter" | "sintercard" | "sinterstore"
        | "sismember" | "smembers" | "smismember" | "smove" | "spop" | "srandmember" | "srem"
        | "sscan" | "sunion" | "sunionstore" => "set",
        "hdel" | "hexists" | "hget" | "hgetall" | "hincrby" | "hincrbyfloat" | "hkeys" | "hlen"
        | "hmget" | "hscan" | "hset" | "hsetnx" | "hstrlen" | "hvals" => "hash",
        "zadd" | "zcard" | "zcount" | "zincrby" | "zlexcount" | "zpopmax" | "zpopmin" | "zdiff"
        | "zdiffstore" | "zinter" | "zintercard" | "zinterstore" | "zrandmember" | "zrange"
        | "zrangebyscore" | "zrangestore" | "zrank" | "zrem" | "zremrangebylex"
        | "zremrangebyrank" | "zremrangebyscore" | "zrevrange" | "zrevrangebyscore"
        | "zrevrank" | "zscan" | "zscore" | "zunion" | "zunionstore" => "zset",
        "xack" | "xadd" | "xdel" | "xgroup" | "xinfo" | "xlen" | "xpending" | "xrange"
        | "xread" | "xreadgroup" | "xrevrange" => "stream",
        "auth" | "client" | "hello" | "quit" | "reset" | "select" => "connection",
        "discard" | "exec" | "multi" | "unwatch" | "watch" => "transaction",
        "psubscribe" | "publish" | "pubsub" | "punsubscribe" | "spublish" | "ssubscribe"
        | "subscribe" | "sunsubscribe" | "unsubscribe" => "pubsub",
        "copy" | "dbsize" | "del" | "delex" | "delifeq" | "exists" | "expire" | "expireat"
        | "expiretime" | "flushall" | "flushdb" | "keys" | "persist" | "pexpire" | "pexpireat"
        | "pexpiretime" | "pttl" | "randomkey" | "rename" | "renamenx" | "touch" | "ttl"
        | "type" | "scan" => "keyspace",
        _ => return None,
    })
}

pub(super) fn commands_in_category(category: &[u8]) -> Vec<&'static str> {
    COMMANDS
        .iter()
        .filter(|command| {
            let name = command.name.as_bytes();
            category_matches(name, &[name], category)
        })
        .map(|command| command.name)
        .collect()
}

const fn cmd(
    name: &'static str,
    arity: i64,
    flags: &'static [&'static str],
    first_key: i64,
    last_key: i64,
    key_step: i64,
) -> Command {
    Command {
        name,
        arity,
        flags,
        first_key,
        last_key,
        key_step,
    }
}

impl Store {
    /// Returns false without touching output for commands outside this module.
    pub(super) fn execute_discovery(&self, arguments: &[&[u8]], output: &mut Vec<u8>) -> bool {
        let Some(name) = arguments.first() else {
            return false;
        };
        if name.eq_ignore_ascii_case(b"COMMAND") {
            command(arguments, output);
            true
        } else if name.eq_ignore_ascii_case(b"INFO") {
            self.info(arguments, output);
            true
        } else {
            false
        }
    }

    fn info(&self, arguments: &[&[u8]], output: &mut Vec<u8>) {
        let sections: &[&[u8]] = match arguments {
            [_] => &[b"server", b"memory", b"keyspace", b"stats", b"clients"],
            [_, section] => &[*section],
            _ => {
                output.extend_from_slice(b"-ERR wrong number of arguments for 'info' command\r\n");
                return;
            }
        };
        let Ok(data) = self.data.read() else {
            output.extend_from_slice(b"-ERR internal store error\r\n");
            return;
        };
        let now = self.now(&data);
        let live_keys = data
            .entries
            .values()
            .filter(|entry| entry.alive(now))
            .count();
        let expiring_keys = data
            .entries
            .values()
            .filter(|entry| entry.alive(now) && entry.expires.is_some())
            .count();
        let mut text = String::new();
        for (index, section) in sections.iter().enumerate() {
            let section = section.to_ascii_lowercase();
            if !matches!(
                section.as_slice(),
                b"server" | b"memory" | b"keyspace" | b"stats" | b"clients" | b"default" | b"all"
            ) {
                output.extend_from_slice(b"-ERR unsupported INFO section\r\n");
                return;
            }
            if index > 0 {
                text.push_str("\r\n");
            }
            if matches!(section.as_slice(), b"server" | b"default" | b"all") {
                let _ = writeln!(
                    text,
                    "# Server\r\nrillcache_version:{}\r\nrillcache_mode:standalone\rarch_bits:{}\rmem_allocator:system\r",
                    env!("CARGO_PKG_VERSION"),
                    usize::BITS
                );
            }
            if matches!(section.as_slice(), b"memory" | b"default" | b"all") {
                let _ = writeln!(
                    text,
                    "# Memory\r\nrillcache_payload_bytes:{}\r\nrillcache_payload_limit_bytes:{}\r\nrillcache_payload_scope:key_and_value_bytes_not_RSS\r\nmaxmemory_policy:{}\r\nnotify_keyspace_events:{}\r",
                    data.payload_bytes,
                    self.max_payload_bytes,
                    self.eviction_policy().name(),
                    self.notify_flags().as_string()
                );
            }
            if matches!(section.as_slice(), b"keyspace" | b"default" | b"all") {
                let _ = writeln!(
                    text,
                    "# Keyspace\r\ndb0:keys={live_keys},expires={expiring_keys}\r"
                );
            }
            if matches!(section.as_slice(), b"clients" | b"default" | b"all") {
                let _ = writeln!(
                    text,
                    "# Clients\r\nconnected_clients:{}\r\ncluster_connections:0\r",
                    self.connected_clients()
                );
            }
            if matches!(section.as_slice(), b"stats" | b"default" | b"all") {
                let stats = self.stats();
                let _ = writeln!(
                    text,
                    "# Stats\r\ntotal_connections_received:{}\r\ntotal_commands_processed:{}\r\nrejected_connections:{}\r\nexpired_keys:{}\r\nevicted_keys:{}\r\npubsub_channels:{}\r\npubsub_patterns:{}\r",
                    stats.total_connections_received,
                    stats.total_commands_processed,
                    stats.rejected_connections,
                    stats.expired_keys,
                    stats.evicted_keys,
                    stats.pubsub_channels,
                    stats.pubsub_patterns
                );
            }
        }
        encode_bulk(Some(text.as_bytes()), output);
    }
}

fn command(arguments: &[&[u8]], output: &mut Vec<u8>) {
    if arguments.len() == 1 {
        write_array_len(COMMANDS.len(), output);
        for item in COMMANDS {
            metadata(item, output);
        }
        return;
    }
    let sub = arguments[1];
    if sub.eq_ignore_ascii_case(b"COUNT") {
        if arguments.len() != 2 {
            output.extend_from_slice(
                b"-ERR wrong number of arguments for 'command|count' command\r\n",
            );
        } else {
            write!(output, ":{}\r\n", COMMANDS.len()).expect("Vec write");
        }
    } else if sub.eq_ignore_ascii_case(b"LIST") {
        if arguments.len() != 2 {
            output.extend_from_slice(b"-ERR unsupported COMMAND LIST options\r\n");
        } else {
            write_array_len(COMMANDS.len(), output);
            for item in COMMANDS {
                encode_bulk(Some(item.name.as_bytes()), output);
            }
        }
    } else if sub.eq_ignore_ascii_case(b"INFO") {
        if arguments.len() == 2 {
            write_array_len(COMMANDS.len(), output);
            for item in COMMANDS {
                metadata(item, output);
            }
        } else {
            write_array_len(arguments.len() - 2, output);
            for name in &arguments[2..] {
                if let Some(item) = COMMANDS
                    .iter()
                    .find(|item| name.eq_ignore_ascii_case(item.name.as_bytes()))
                {
                    metadata(item, output);
                } else {
                    output.extend_from_slice(b"*-1\r\n");
                }
            }
        }
    } else {
        output.extend_from_slice(b"-ERR unsupported COMMAND subcommand\r\n");
    }
}

fn metadata(item: &Command, output: &mut Vec<u8>) {
    write_array_len(6, output);
    encode_bulk(Some(item.name.as_bytes()), output);
    write!(output, ":{}\r\n", item.arity).expect("Vec write");
    write_array_len(item.flags.len(), output);
    for flag in item.flags {
        output.extend_from_slice(b"+");
        output.extend_from_slice(flag.as_bytes());
        output.extend_from_slice(b"\r\n");
    }
    write!(
        output,
        ":{}\r\n:{}\r\n:{}\r\n",
        item.first_key, item.last_key, item.key_step
    )
    .expect("Vec write");
}

fn write_array_len(length: usize, output: &mut Vec<u8>) {
    write!(output, "*{length}\r\n").expect("Vec write");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_mutation_commands_have_real_acl_categories() {
        for (name, category) in [
            ("hincrby", "hash"),
            ("hsetnx", "hash"),
            ("linsert", "list"),
            ("ltrim", "list"),
            ("sintercard", "set"),
            ("smove", "set"),
            ("zrangebyscore", "zset"),
            ("zrangestore", "zset"),
            ("zremrangebylex", "zset"),
            ("zpopmin", "zset"),
            ("zunion", "zset"),
            ("zunionstore", "zset"),
            ("zinter", "zset"),
            ("zinterstore", "zset"),
            ("zdiff", "zset"),
            ("zdiffstore", "zset"),
            ("zintercard", "zset"),
            ("pfadd", "hyperloglog"),
            ("pfcount", "hyperloglog"),
            ("pfmerge", "hyperloglog"),
            ("bitcount", "bitmap"),
            ("bitfield", "bitmap"),
            ("bitfield_ro", "bitmap"),
            ("bitop", "bitmap"),
            ("geoadd", "geo"),
            ("geodist", "geo"),
            ("geohash", "geo"),
            ("geopos", "geo"),
            ("geosearch", "geo"),
            ("geosearchstore", "geo"),
            ("bitpos", "bitmap"),
            ("spublish", "pubsub"),
            ("ssubscribe", "pubsub"),
            ("sunsubscribe", "pubsub"),
        ] {
            assert_eq!(command_group(name), Some(category));
        }
    }

    #[test]
    fn counted_key_lists_exclude_options_from_acl_key_enforcement() {
        assert_eq!(
            key_arguments(
                b"SINTERCARD",
                &[b"SINTERCARD", b"2", b"a", b"b", b"LIMIT", b"1"]
            ),
            vec![b"a".as_slice(), b"b".as_slice()]
        );
        assert_eq!(
            key_arguments(b"SINTERCARD", &[b"SINTERCARD", b"1", b"a"]),
            vec![b"a".as_slice()]
        );
        assert!(key_arguments(b"SINTERCARD", &[b"SINTERCARD", b"x", b"a"]).is_empty());
        assert_eq!(
            key_arguments(
                b"ZUNION",
                &[b"ZUNION", b"2", b"a", b"b", b"WEIGHTS", b"1", b"2"]
            ),
            vec![b"a".as_slice(), b"b".as_slice()]
        );
        assert_eq!(
            key_arguments(
                b"ZINTERSTORE",
                &[
                    b"ZINTERSTORE",
                    b"dest",
                    b"2",
                    b"a",
                    b"b",
                    b"AGGREGATE",
                    b"MIN"
                ]
            ),
            vec![b"dest".as_slice(), b"a".as_slice(), b"b".as_slice()]
        );
        assert_eq!(
            key_arguments(
                b"ZINTERCARD",
                &[b"ZINTERCARD", b"2", b"a", b"b", b"LIMIT", b"1"]
            ),
            vec![b"a".as_slice(), b"b".as_slice()]
        );
        assert!(key_arguments(b"ZUNION", &[b"ZUNION", b"0", b"a"]).is_empty());
        assert!(key_arguments(b"ZUNIONSTORE", &[b"ZUNIONSTORE", b"d", b"x", b"a"]).is_empty());
    }

    #[test]
    fn unknown_dispatch_is_false_and_leaves_output_untouched() {
        let store = Store::new(1024);
        let mut output = b"existing".to_vec();
        assert!(!store.execute_discovery(&[b"NO-SUCH-COMMAND", b"\0"], &mut output));
        assert_eq!(output, b"existing");
        assert!(!store.execute_discovery(&[], &mut output));
        assert_eq!(output, b"existing");
    }
}
