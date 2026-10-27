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
    cmd("asking", 1, &["fast"], 0, 0, 0),
    cmd(
        "auth",
        -2,
        &["noscript", "loading", "stale", "fast"],
        0,
        0,
        0,
    ),
    cmd("bgrewriteaof", 1, &["admin", "dangerous", "slow"], 0, 0, 0),
    cmd(
        "bgsave",
        -1,
        &["admin", "noscript", "no_async_loading"],
        0,
        0,
        0,
    ),
    cmd("bitcount", -2, &["readonly"], 1, 1, 1),
    cmd("bitfield", -2, &["write", "denyoom"], 1, 1, 1),
    cmd("bitfield_ro", -2, &["readonly"], 1, 1, 1),
    cmd("bitop", -4, &["write", "denyoom"], 1, -1, 1),
    cmd("bitpos", -3, &["readonly"], 1, 1, 1),
    cmd(
        "blmove",
        6,
        &["write", "denyoom", "noscript", "blocking"],
        1,
        2,
        1,
    ),
    cmd("blmpop", -5, &["write"], 3, -1, 1),
    cmd("blpop", -3, &["write", "noscript", "blocking"], 1, -2, 1),
    cmd("brpop", -3, &["write", "noscript", "blocking"], 1, -2, 1),
    cmd(
        "brpoplpush",
        4,
        &["write", "denyoom", "noscript", "blocking"],
        1,
        2,
        1,
    ),
    cmd("bzmpop", -5, &["write", "noscript", "blocking"], 0, 0, 0),
    cmd("bzpopmax", -3, &["write", "blocking", "fast"], 1, -2, 1),
    cmd("bzpopmin", -3, &["write", "blocking", "fast"], 1, -2, 1),
    cmd(
        "client",
        -2,
        &["admin", "noscript", "loading", "stale"],
        0,
        0,
        0,
    ),
    cmd("cluster", -2, &[], 0, 0, 0),
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
    cmd("failover", -1, &["admin", "noscript", "stale"], 0, 0, 0),
    cmd("flushall", -1, &["write", "slow"], 0, 0, 0),
    cmd("flushdb", -1, &["write", "slow"], 0, 0, 0),
    cmd("function", -2, &["write", "admin"], 0, 0, 0),
    cmd("geoadd", -5, &["write", "denyoom"], 1, 1, 1),
    cmd("geodist", -4, &["readonly"], 1, 1, 1),
    cmd("geohash", -2, &["readonly"], 1, 1, 1),
    cmd("geopos", -2, &["readonly"], 1, 1, 1),
    cmd("georadius", -6, &["write", "denyoom"], 1, 1, 1),
    cmd("georadius_ro", -6, &["readonly"], 1, 1, 1),
    cmd("georadiusbymember", -5, &["write", "denyoom"], 1, 1, 1),
    cmd("georadiusbymember_ro", -5, &["readonly"], 1, 1, 1),
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
    cmd("hexpire", -6, &["write", "fast"], 1, 1, 1),
    cmd("hexpireat", -6, &["write", "fast"], 1, 1, 1),
    cmd("hexpiretime", -5, &["readonly", "fast"], 1, 1, 1),
    cmd("hget", 3, &["readonly", "fast"], 1, 1, 1),
    cmd("hgetall", 2, &["readonly"], 1, 1, 1),
    cmd("hgetdel", -5, &["write", "fast"], 1, 1, 1),
    cmd("hgetex", -5, &["write", "fast"], 1, 1, 1),
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
    cmd("hmset", -4, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("hpersist", -5, &["write", "fast"], 1, 1, 1),
    cmd("hpexpire", -6, &["write", "fast"], 1, 1, 1),
    cmd("hpexpireat", -6, &["write", "fast"], 1, 1, 1),
    cmd("hpexpiretime", -5, &["readonly", "fast"], 1, 1, 1),
    cmd("hpttl", -5, &["readonly", "fast"], 1, 1, 1),
    cmd("hrandfield", -2, &["readonly"], 1, 1, 1),
    cmd("hscan", -3, &["readonly", "slow"], 1, 1, 1),
    cmd("hset", -4, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("hsetex", -6, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("hsetnx", 4, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("hstrlen", 3, &["readonly", "fast"], 1, 1, 1),
    cmd("httl", -5, &["readonly", "fast"], 1, 1, 1),
    cmd("hvals", 2, &["readonly"], 1, 1, 1),
    cmd("incr", 2, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("incrby", 3, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("incrbyfloat", 3, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("info", -1, &["loading", "stale"], 0, 0, 0),
    cmd("keys", 2, &["readonly"], 0, 0, 0),
    cmd("lastsave", 1, &["loading", "stale", "fast"], 0, 0, 0),
    cmd(
        "latency",
        -2,
        &["admin", "noscript", "loading", "stale", "slow"],
        0,
        0,
        0,
    ),
    cmd("lcs", -3, &["readonly"], 1, 2, 1),
    cmd("lindex", 3, &["readonly"], 1, 1, 1),
    cmd("linsert", 5, &["write", "denyoom"], 1, 1, 1),
    cmd("llen", 2, &["readonly", "fast"], 1, 1, 1),
    cmd("lmove", 5, &["write", "denyoom"], 1, 2, 1),
    cmd("lmpop", -4, &["write"], 2, -1, 1),
    cmd("lpop", -2, &["write", "fast"], 1, 1, 1),
    cmd("lpos", -3, &["readonly"], 1, 1, 1),
    cmd("lpush", -3, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("lpushx", -3, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("lrange", 4, &["readonly"], 1, 1, 1),
    cmd("lrem", 4, &["write"], 1, 1, 1),
    cmd("lset", 4, &["write", "denyoom"], 1, 1, 1),
    cmd("ltrim", 4, &["write"], 1, 1, 1),
    cmd("memory", -2, &["readonly", "slow"], 0, 0, 0),
    cmd("mget", -2, &["readonly", "fast"], 1, -1, 1),
    cmd("module", -2, &[], 0, 0, 0),
    cmd(
        "monitor",
        1,
        &["admin", "noscript", "loading", "stale"],
        0,
        0,
        0,
    ),
    cmd("mset", -3, &["write", "denyoom"], 1, -1, 2),
    cmd("msetex", -4, &["write", "denyoom"], 2, -1, 2),
    cmd("move", 3, &["write", "fast"], 1, 1, 1),
    cmd("msetnx", -3, &["write", "denyoom"], 1, -1, 2),
    cmd(
        "multi",
        1,
        &["noscript", "loading", "stale", "fast"],
        0,
        0,
        0,
    ),
    cmd("object", -2, &["readonly"], 2, 2, 1),
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
        "psync",
        -3,
        &["admin", "noscript", "no_async_loading", "no_multi"],
        0,
        0,
        0,
    ),
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
    cmd("readonly", 1, &["fast"], 0, 0, 0),
    cmd("readwrite", 1, &["fast"], 0, 0, 0),
    cmd("rename", 3, &["write"], 1, 2, 1),
    cmd(
        "replicaof",
        3,
        &["admin", "noscript", "stale", "no_async_loading"],
        0,
        0,
        0,
    ),
    cmd(
        "replconf",
        -1,
        &["admin", "noscript", "loading", "stale", "allow_busy"],
        0,
        0,
        0,
    ),
    cmd("renamenx", 3, &["write", "fast"], 1, 2, 1),
    cmd(
        "reset",
        1,
        &["noscript", "loading", "stale", "fast"],
        0,
        0,
        0,
    ),
    cmd(
        "role",
        1,
        &["admin", "noscript", "loading", "stale", "fast"],
        0,
        0,
        0,
    ),
    cmd("rpop", -2, &["write", "fast"], 1, 1, 1),
    cmd("rpoplpush", 3, &["write", "denyoom"], 1, 2, 1),
    cmd("rpush", -3, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("rpushx", -3, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("sadd", -3, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd(
        "save",
        1,
        &["admin", "noscript", "no_async_loading", "no_multi"],
        0,
        0,
        0,
    ),
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
    cmd(
        "shutdown",
        -1,
        &["admin", "noscript", "loading", "stale", "slow"],
        0,
        0,
        0,
    ),
    cmd("sinter", -2, &["readonly"], 1, -1, 1),
    cmd("sintercard", -3, &["readonly"], 2, -1, 1),
    cmd("sinterstore", -3, &["write", "denyoom"], 1, -1, 1),
    cmd("sismember", 3, &["readonly", "fast"], 1, 1, 1),
    cmd(
        "slowlog",
        -2,
        &["admin", "noscript", "loading", "stale", "slow"],
        0,
        0,
        0,
    ),
    cmd(
        "slaveof",
        3,
        &["admin", "noscript", "stale", "no_async_loading"],
        0,
        0,
        0,
    ),
    cmd("smembers", 2, &["readonly"], 1, 1, 1),
    cmd("smismember", -3, &["readonly", "fast"], 1, 1, 1),
    cmd("smove", 4, &["write", "fast"], 1, 2, 1),
    cmd("sort", -2, &["write", "denyoom"], 1, 1, 1),
    cmd("sort_ro", -2, &["readonly"], 1, 1, 1),
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
    cmd("swapdb", 3, &["write", "fast"], 0, 0, 0),
    cmd(
        "sync",
        1,
        &["admin", "noscript", "no_async_loading", "no_multi"],
        0,
        0,
        0,
    ),
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
    cmd("time", 1, &["readonly", "fast"], 0, 0, 0),
    cmd("touch", -2, &["readonly", "fast"], 1, -1, 1),
    cmd("ttl", 2, &["readonly", "fast"], 1, 1, 1),
    cmd("type", 2, &["readonly", "fast"], 1, 1, 1),
    cmd("unlink", -2, &["write", "fast"], 1, -1, 1),
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
    cmd("wait", 3, &["blocking"], 0, 0, 0),
    cmd("waitaof", 4, &["blocking"], 0, 0, 0),
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
    cmd("xsetid", -3, &["write", "denyoom", "fast"], 1, 1, 1),
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
    cmd("zmpop", -4, &["write"], 2, -1, 1),
    cmd("zmscore", -3, &["readonly", "fast"], 1, 1, 1),
    cmd("zpopmax", -2, &["write", "fast"], 1, 1, 1),
    cmd("zpopmin", -2, &["write", "fast"], 1, 1, 1),
    cmd("zrandmember", -2, &["readonly"], 1, 1, 1),
    cmd("zrange", -4, &["readonly"], 1, 1, 1),
    cmd("zrangebylex", -4, &["readonly"], 1, 1, 1),
    cmd("zrangebyscore", -4, &["readonly"], 1, 1, 1),
    cmd("zrangestore", -5, &["write", "denyoom"], 1, 2, 1),
    cmd("zrank", -3, &["readonly", "fast"], 1, 1, 1),
    cmd("zrem", -3, &["write", "fast"], 1, 1, 1),
    cmd("zremrangebylex", 4, &["write"], 1, 1, 1),
    cmd("zremrangebyrank", 4, &["write"], 1, 1, 1),
    cmd("zremrangebyscore", 4, &["write"], 1, 1, 1),
    cmd("zrevrange", -4, &["readonly"], 1, 1, 1),
    cmd("zrevrangebylex", -4, &["readonly"], 1, 1, 1),
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

pub(crate) fn is_write_command(name: &[u8]) -> bool {
    COMMANDS
        .iter()
        .find(|command| name.eq_ignore_ascii_case(command.name.as_bytes()))
        .is_some_and(|command| command.flags.contains(&"write"))
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
    // Legacy GEORADIUS* takes its source key plus an optional last-wins
    // STORE/STOREDIST destination after the radius/unit prefix.
    let legacy_geo = name.eq_ignore_ascii_case(b"GEORADIUS")
        || name.eq_ignore_ascii_case(b"GEORADIUS_RO")
        || name.eq_ignore_ascii_case(b"GEORADIUSBYMEMBER")
        || name.eq_ignore_ascii_case(b"GEORADIUSBYMEMBER_RO");
    if legacy_geo {
        let Some(source) = args.get(1) else {
            return Vec::new();
        };
        let by_member = name.eq_ignore_ascii_case(b"GEORADIUSBYMEMBER")
            || name.eq_ignore_ascii_case(b"GEORADIUSBYMEMBER_RO");
        let mut keys = vec![*source];
        let mut index = if by_member { 5 } else { 6 };
        while index < args.len() {
            if (args[index].eq_ignore_ascii_case(b"STORE")
                || args[index].eq_ignore_ascii_case(b"STOREDIST"))
                && index + 1 < args.len()
            {
                keys.push(args[index + 1]);
                index += 2;
            } else {
                index += 1;
            }
        }
        return keys;
    }
    // SORT has a fixed source plus a STORE destination that may appear anywhere
    // among its options. BY/GET dynamic patterns are intentionally unsupported.
    if name.eq_ignore_ascii_case(b"SORT") || name.eq_ignore_ascii_case(b"SORT_RO") {
        let mut keys = Vec::new();
        if let Some(source) = args.get(1) {
            keys.push(*source);
        }
        let mut index = 2;
        while index < args.len() {
            if args[index].eq_ignore_ascii_case(b"STORE") && index + 1 < args.len() {
                keys.push(args[index + 1]);
                index += 2;
            } else {
                index += 1;
            }
        }
        return keys;
    }
    if name.eq_ignore_ascii_case(b"BLMPOP") {
        let Some(count) = args
            .get(2)
            .and_then(|raw| std::str::from_utf8(raw).ok())
            .and_then(|raw| raw.parse::<usize>().ok())
            .filter(|count| *count > 0)
        else {
            return Vec::new();
        };
        let end = 3usize.saturating_add(count).min(args.len());
        return args[3..end].to_vec();
    }
    // Counted-key sorted-set algebra keeps its key list after `numkeys`, with
    // the destination preceding it for the *STORE variants.
    let counted_zset = name.eq_ignore_ascii_case(b"ZUNION")
        || name.eq_ignore_ascii_case(b"ZINTER")
        || name.eq_ignore_ascii_case(b"ZDIFF")
        || name.eq_ignore_ascii_case(b"ZINTERCARD")
        || name.eq_ignore_ascii_case(b"ZMPOP")
        || name.eq_ignore_ascii_case(b"BZMPOP");
    let store_zset = name.eq_ignore_ascii_case(b"ZUNIONSTORE")
        || name.eq_ignore_ascii_case(b"ZINTERSTORE")
        || name.eq_ignore_ascii_case(b"ZDIFFSTORE");
    if counted_zset || store_zset {
        let numkeys_index = if store_zset || name.eq_ignore_ascii_case(b"BZMPOP") {
            2
        } else {
            1
        };
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
    "generic",
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
            "acl"
                | "config"
                | "flushall"
                | "flushdb"
                | "function"
                | "keys"
                | "latency"
                | "monitor"
                | "role"
                | "script"
                | "shutdown"
                | "slowlog"
                | "sort"
                | "sort_ro"
        ),
        b"connection" => group == Some("connection"),
        b"pubsub" => group == Some("pubsub"),
        b"transaction" => group == Some("transaction"),
        b"generic" => group == Some("generic"),
        b"string" | b"hash" | b"stream" | b"bitmap" | b"hyperloglog" | b"geo" => {
            group == Some(std::str::from_utf8(category).unwrap_or(""))
        }
        b"list" | b"set" | b"zset" => {
            group == Some(std::str::from_utf8(category).unwrap_or(""))
                || matches!(name, "sort" | "sort_ro")
        }
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
        "geoadd"
        | "geodist"
        | "geohash"
        | "geopos"
        | "georadius"
        | "georadius_ro"
        | "georadiusbymember"
        | "georadiusbymember_ro"
        | "geosearch"
        | "geosearchstore" => "geo",
        "pfadd" | "pfcount" | "pfmerge" => "hyperloglog",
        "blmove" | "blpop" | "brpop" | "brpoplpush" | "lindex" | "linsert" | "llen" | "lmove"
        | "lpop" | "lpush" | "lpushx" | "lrange" | "lrem" | "lset" | "ltrim" | "lmpop" | "lpos"
        | "rpop" | "rpoplpush" | "rpush" | "rpushx" => "list",
        "sadd" | "scard" | "sdiff" | "sdiffstore" | "sinter" | "sintercard" | "sinterstore"
        | "sismember" | "smembers" | "smismember" | "smove" | "spop" | "srandmember" | "srem"
        | "sscan" | "sunion" | "sunionstore" => "set",
        "hdel" | "hexists" | "hget" | "hgetall" | "hgetdel" | "hgetex" | "hincrby"
        | "hincrbyfloat" | "hkeys" | "hlen" | "hmget" | "hmset" | "hrandfield" | "hscan"
        | "hset" | "hsetex" | "hsetnx" | "hstrlen" | "hvals" => "hash",
        "bzmpop" | "bzpopmax" | "bzpopmin" | "zadd" | "zcard" | "zcount" | "zincrby"
        | "zlexcount" | "zmscore" | "zpopmax" | "zpopmin" | "zdiff" | "zdiffstore" | "zinter"
        | "zintercard" | "zinterstore" | "zrandmember" | "zrange" | "zrangebylex"
        | "zrangebyscore" | "zrangestore" | "zrank" | "zrem" | "zremrangebylex"
        | "zremrangebyrank" | "zremrangebyscore" | "zrevrange" | "zrevrangebylex"
        | "zrevrangebyscore" | "zrevrank" | "zscan" | "zscore" | "zunion" | "zunionstore" => "zset",
        "xack" | "xadd" | "xdel" | "xgroup" | "xinfo" | "xlen" | "xpending" | "xrange"
        | "xread" | "xreadgroup" | "xrevrange" | "xsetid" | "xtrim" => "stream",
        "asking" | "auth" | "client" | "hello" | "quit" | "readonly" | "readwrite" | "reset"
        | "select" => "connection",
        "discard" | "exec" | "multi" | "unwatch" | "watch" => "transaction",
        "psubscribe" | "publish" | "pubsub" | "punsubscribe" | "spublish" | "ssubscribe"
        | "subscribe" | "sunsubscribe" | "unsubscribe" => "pubsub",
        "object" | "time" | "sort" | "sort_ro" | "bgsave" | "failover" | "lastsave" | "psync"
        | "replicaof" | "replconf" | "save" | "slaveof" | "sync" => "generic",
        "copy" | "dbsize" | "del" | "delex" | "delifeq" | "exists" | "expire" | "expireat"
        | "expiretime" | "flushall" | "flushdb" | "keys" | "persist" | "pexpire" | "pexpireat"
        | "pexpiretime" | "pttl" | "randomkey" | "rename" | "renamenx" | "touch" | "ttl"
        | "type" | "unlink" | "scan" | "move" | "swapdb" => "keyspace",
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
        } else if name.eq_ignore_ascii_case(b"ROLE") {
            if arguments.len() != 1 {
                super::arity("role", output);
            } else {
                // Standalone master with no replication stream or replicas.
                output.extend_from_slice(b"*3\r\n$6\r\nmaster\r\n:0\r\n*0\r\n");
            }
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
            [_] => &[
                b"server",
                b"memory",
                b"keyspace",
                b"stats",
                b"clients",
                b"errorstats",
            ],
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
        let subexpiry_keys = data
            .entries
            .values()
            .filter(|entry| entry.alive(now) && !entry.field_ttl.is_empty())
            .count();
        let (ttl_sum, ttl_count) = data
            .entries
            .values()
            .filter_map(|entry| {
                entry
                    .alive(now)
                    .then_some(entry.expires)
                    .flatten()
                    .map(|deadline| u64::try_from(deadline.saturating_sub(now).max(0)).unwrap_or(0))
            })
            .fold((0u64, 0u64), |(sum, count), ttl| {
                (sum.saturating_add(ttl), count + 1)
            });
        let avg_ttl = ttl_sum.checked_div(ttl_count).unwrap_or(0);
        let mut text = String::new();
        for (index, section) in sections.iter().enumerate() {
            let section = section.to_ascii_lowercase();
            if !matches!(
                section.as_slice(),
                b"server"
                    | b"memory"
                    | b"keyspace"
                    | b"stats"
                    | b"clients"
                    | b"replication"
                    | b"persistence"
                    | b"errorstats"
                    | b"default"
                    | b"all"
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
                    self.payload_limit(),
                    self.eviction_policy().name(),
                    self.notify_flags().as_string()
                );
            }
            if matches!(section.as_slice(), b"keyspace" | b"default" | b"all") {
                let _ = writeln!(
                    text,
                    "# Keyspace\r\ndb0:keys={live_keys},expires={expiring_keys},avg_ttl={avg_ttl},subexpiry={subexpiry_keys}\r",
                );
            }
            if matches!(section.as_slice(), b"clients" | b"default" | b"all") {
                let _ = writeln!(
                    text,
                    "# Clients\r\nconnected_clients:{}\r\ncluster_connections:0\r\nmaxclients:{}\r\nblocked_clients:{}\r",
                    self.connected_clients(),
                    self.max_clients(),
                    self.blocked_clients()
                );
            }
            if matches!(section.as_slice(), b"stats" | b"default" | b"all") {
                let stats = self.stats();
                let _ = writeln!(
                    text,
                    "# Stats\r\ntotal_connections_received:{}\r\ntotal_commands_processed:{}\r\nrejected_connections:{}\r\nexpired_keys:{}\r\nexpired_subkeys:{}\r\nevicted_keys:{}\r\npubsub_channels:{}\r\npubsub_patterns:{}\r",
                    stats.total_connections_received,
                    stats.total_commands_processed,
                    stats.rejected_connections,
                    stats.expired_keys,
                    stats.expired_subkeys,
                    stats.evicted_keys,
                    stats.pubsub_channels,
                    stats.pubsub_patterns
                );
            }
            if matches!(section.as_slice(), b"persistence" | b"all") {
                let persistence = self.persistence_status();
                let _ = write!(
                    text,
                    "# Persistence\r\nloading:0\r\naof_enabled:{}",
                    u8::from(persistence.aof_enabled)
                );
                if persistence.aof_enabled {
                    let _ = write!(
                        text,
                        "\r\naof_rewrite_in_progress:0\r\naof_last_bgrewrite_status:{}\r\naof_last_write_status:{}",
                        if persistence.rewrite_failed {
                            "err"
                        } else {
                            "ok"
                        },
                        if persistence.write_failed {
                            "err"
                        } else {
                            "ok"
                        },
                    );
                    if let Some(size) = persistence.current_size {
                        let _ = write!(text, "\r\naof_current_size:{size}");
                    }
                }
                text.push('\r');
            }
            if matches!(section.as_slice(), b"errorstats" | b"all") {
                text.push_str("# Errorstats\r\n");
                for (code, count) in self.error_stats() {
                    let _ = write!(
                        text,
                        "errorstat_{}:{}\r\n",
                        String::from_utf8_lossy(&code),
                        count
                    );
                }
            }
            if matches!(section.as_slice(), b"replication" | b"all") {
                text.push_str(
                    "# Replication\r\nrole:master\r\nconnected_slaves:0\r\nmaster_failover_state:no-failover\r\nmaster_repl_offset:0\r\n",
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
        let mut names: Vec<&'static str> = COMMANDS.iter().map(|item| item.name).collect();
        if arguments.len() != 2 {
            if arguments.len() != 5 || !arguments[2].eq_ignore_ascii_case(b"FILTERBY") {
                output.extend_from_slice(b"-ERR syntax error\r\n");
                return;
            }
            let filter = arguments[3];
            let value = arguments[4];
            if filter.eq_ignore_ascii_case(b"MODULE") {
                names.clear();
            } else if filter.eq_ignore_ascii_case(b"ACLCAT") {
                let category = if value.starts_with(b"@") {
                    &value[1..]
                } else {
                    value
                };
                names = commands_in_category(category);
            } else if filter.eq_ignore_ascii_case(b"PATTERN") {
                names.retain(|name| super::keyspace::glob_match(value, name.as_bytes()));
            } else {
                output.extend_from_slice(b"-ERR syntax error\r\n");
                return;
            }
        }
        write_array_len(names.len(), output);
        for name in names {
            encode_bulk(Some(name.as_bytes()), output);
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
    } else if sub.eq_ignore_ascii_case(b"GETKEYS") {
        if arguments.len() < 3 {
            output.extend_from_slice(
                b"-ERR wrong number of arguments for 'command|getkeys' command\r\n",
            );
            return;
        }
        let target = &arguments[2..];
        let Some(item) = COMMANDS
            .iter()
            .find(|item| target[0].eq_ignore_ascii_case(item.name.as_bytes()))
        else {
            output.extend_from_slice(b"-ERR Invalid command specified\r\n");
            return;
        };
        let length = target.len() as i64;
        if (item.arity > 0 && length != item.arity) || (item.arity < 0 && length < -item.arity) {
            output.extend_from_slice(b"-ERR Invalid number of arguments specified for command\r\n");
            return;
        }
        let keys = key_arguments(target[0], target);
        if keys.is_empty() {
            if item.first_key <= 0 {
                output.extend_from_slice(b"-ERR The command has no key arguments\r\n");
            } else {
                output.extend_from_slice(b"-ERR Invalid arguments specified for command\r\n");
            }
            return;
        }
        write_array_len(keys.len(), output);
        for key in keys {
            encode_bulk(Some(key), output);
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
    fn role_reports_admin_fast_and_dangerous_categories() {
        for category in [b"admin".as_slice(), b"fast", b"dangerous"] {
            assert!(category_matches(b"role", &[b"ROLE"], category));
        }
        assert!(!category_matches(b"role", &[b"ROLE"], b"write"));
    }

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
            ("bzmpop", "zset"),
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
            ("georadius", "geo"),
            ("georadius_ro", "geo"),
            ("georadiusbymember", "geo"),
            ("georadiusbymember_ro", "geo"),
            ("xsetid", "stream"),
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
        assert_eq!(
            key_arguments(
                b"BZMPOP",
                &[b"BZMPOP", b"0", b"2", b"a", b"b", b"MIN", b"COUNT", b"1"]
            ),
            vec![b"a".as_slice(), b"b".as_slice()]
        );
        assert!(key_arguments(b"BZMPOP", &[b"BZMPOP", b"0", b"0", b"a"]).is_empty());
        assert_eq!(
            key_arguments(b"BZPOPMIN", &[b"BZPOPMIN", b"a", b"b", b"0"]),
            vec![b"a".as_slice(), b"b".as_slice()]
        );
        assert_eq!(
            key_arguments(b"BZPOPMAX", &[b"BZPOPMAX", b"a", b"0.5"]),
            vec![b"a".as_slice()]
        );
        assert_eq!(
            key_arguments(
                b"SORT",
                &[b"SORT", b"src", b"LIMIT", b"0", b"1", b"STORE", b"dest"]
            ),
            vec![b"src".as_slice(), b"dest".as_slice()]
        );
        assert_eq!(
            key_arguments(b"SORT_RO", &[b"SORT_RO", b"src", b"ALPHA"]),
            vec![b"src".as_slice()]
        );
        assert_eq!(
            key_arguments(
                b"GEORADIUS",
                &[
                    b"GEORADIUS",
                    b"geo",
                    b"1",
                    b"2",
                    b"3",
                    b"km",
                    b"STORE",
                    b"dest"
                ]
            ),
            vec![b"geo".as_slice(), b"dest".as_slice()]
        );
        assert_eq!(
            key_arguments(
                b"GEORADIUSBYMEMBER_RO",
                &[b"GEORADIUSBYMEMBER_RO", b"geo", b"m", b"3", b"km", b"ASC"]
            ),
            vec![b"geo".as_slice()]
        );
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
