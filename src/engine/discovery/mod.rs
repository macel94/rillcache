//! Bounded Redis-style command and server discovery.

use super::Store;
use crate::protocol::encode_bulk;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::sync::OnceLock;

#[derive(Clone, Copy)]
pub(super) struct Command {
    name: &'static str,
    arity: i64,
    flags: &'static [&'static str],
    first_key: i64,
    last_key: i64,
    key_step: i64,
}

mod bitmaps;
mod connections;
mod geo;
mod hashes;
mod hyperloglog;
mod keyspace;
mod lists;
mod pubsub;
mod server;
mod sets;
mod streams;
mod strings;
mod transactions;
mod zsets;

/// Commands grouped by the feature family that owns their metadata.
///
/// Every registered command is wired to this worktree's Store or server
/// dispatcher. Adding a command means editing its family module, not a shared
/// array. The groups are flattened and sorted once, and every lookup and
/// enumeration goes through [`registry`] so group order is never observable.
const GROUPS: &[&[Command]] = &[
    bitmaps::COMMANDS,
    connections::COMMANDS,
    geo::COMMANDS,
    hashes::COMMANDS,
    hyperloglog::COMMANDS,
    keyspace::COMMANDS,
    lists::COMMANDS,
    pubsub::COMMANDS,
    server::COMMANDS,
    sets::COMMANDS,
    streams::COMMANDS,
    strings::COMMANDS,
    transactions::COMMANDS,
    zsets::COMMANDS,
];

static REGISTRY: OnceLock<Vec<Command>> = OnceLock::new();

/// Flatten the family groups into one strictly name-sorted registry.
fn registry() -> &'static [Command] {
    REGISTRY.get_or_init(|| {
        let mut commands = Vec::with_capacity(GROUPS.iter().map(|group| group.len()).sum());
        for group in GROUPS {
            commands.extend_from_slice(group);
        }
        commands.sort_unstable_by(|left, right| left.name.cmp(right.name));
        commands
    })
}

pub(super) const fn command_count() -> usize {
    let mut count = 0;
    let mut group = 0;
    while group < GROUPS.len() {
        count += GROUPS[group].len();
        group += 1;
    }
    count
}

pub(super) fn command_name_at(index: usize) -> Option<&'static str> {
    registry().get(index).map(|command| command.name)
}

static COMMAND_LOOKUP: OnceLock<HashMap<&'static str, usize>> = OnceLock::new();

/// Registry index for per-command statistics. Names are normalized in a stack
/// buffer and looked up in a one-time map so the command hot path does not scan
/// the whole table.
pub(super) fn command_index(name: &[u8]) -> Option<usize> {
    if name.len() > 32 {
        return None;
    }
    let mut lower = [0u8; 32];
    for (slot, byte) in lower.iter_mut().zip(name) {
        *slot = byte.to_ascii_lowercase();
    }
    let name = std::str::from_utf8(&lower[..name.len()]).ok()?;
    COMMAND_LOOKUP
        .get_or_init(|| {
            registry()
                .iter()
                .enumerate()
                .map(|(index, command)| (command.name, index))
                .collect()
        })
        .get(name)
        .copied()
}

/// Arity from the published command table; `None` means unsupported.
pub(super) fn command_arity(name: &[u8]) -> Option<i64> {
    registry()
        .iter()
        .find(|command| name.eq_ignore_ascii_case(command.name.as_bytes()))
        .map(|command| command.arity)
}

pub(crate) fn is_write_command(name: &[u8]) -> bool {
    registry()
        .iter()
        .find(|command| name.eq_ignore_ascii_case(command.name.as_bytes()))
        .is_some_and(|command| command.flags.contains(&"write"))
}

pub(super) fn canonical_name(name: &[u8]) -> Option<&'static str> {
    registry()
        .iter()
        .find(|command| name.eq_ignore_ascii_case(command.name.as_bytes()))
        .map(|command| command.name)
}

/// The access a command needs for one declared key argument. Redis key specs
/// distinguish read-only, write-only and read-write keys; ACL `%R~`/`%W~`
/// patterns must grant the access the command actually performs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum KeyAccess {
    Read,
    Write,
    ReadWrite,
}

impl KeyAccess {
    /// Whether a pattern with `self` access satisfies a required access.
    pub(super) fn grants(self, required: Self) -> bool {
        self == required || self == Self::ReadWrite
    }
}

/// Declared key arguments for ACL key-pattern enforcement. Commands with no
/// static key spec (`KEYS`, `DBSIZE`, `FLUSH*`) return no keys. Dynamic
/// `XREAD`/`XREADGROUP` stream keys are extracted from their STREAMS blocks.
pub(super) fn key_arguments<'a>(name: &[u8], args: &'a [&'a [u8]]) -> Vec<&'a [u8]> {
    let Some(command) = registry()
        .iter()
        .find(|command| name.eq_ignore_ascii_case(command.name.as_bytes()))
    else {
        return Vec::new();
    };
    // BITOP's first argument is the operation, then the destination key and
    // source keys; the static first-key index would mistake the operation for a
    // key and misattribute the destination.
    if name.eq_ignore_ascii_case(b"BITOP") {
        if args.len() < 4 {
            return Vec::new();
        }
        let mut keys = Vec::with_capacity(args.len() - 2);
        keys.push(args[2]);
        keys.extend_from_slice(&args[3..]);
        return keys;
    }
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
    // XREAD and XREADGROUP declare stream keys dynamically in the STREAMS
    // block. Only the key half of that block is a key list; IDs are not keys.
    if name.eq_ignore_ascii_case(b"XREAD") || name.eq_ignore_ascii_case(b"XREADGROUP") {
        let minimum = if name.eq_ignore_ascii_case(b"XREADGROUP") {
            4 // command, GROUP, group name, consumer name
        } else {
            1
        };
        let Some(streams) = args
            .iter()
            .enumerate()
            .skip(minimum)
            .find_map(|(index, arg)| arg.eq_ignore_ascii_case(b"STREAMS").then_some(index))
        else {
            return Vec::new();
        };
        let values = &args[streams + 1..];
        if values.is_empty() || !values.len().is_multiple_of(2) {
            return Vec::new();
        }
        return values[..values.len() / 2].to_vec();
    }
    // Legacy GEORADIUS* takes its source key plus the effective last-wins
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
        let mut destination = None;
        let mut index = if by_member { 5 } else { 6 };
        while index < args.len() {
            if (args[index].eq_ignore_ascii_case(b"STORE")
                || args[index].eq_ignore_ascii_case(b"STOREDIST"))
                && index + 1 < args.len()
            {
                destination = Some(args[index + 1]);
                index += 2;
            } else {
                index += 1;
            }
        }
        let mut keys = vec![*source];
        keys.extend(destination);
        return keys;
    }
    // SORT has a fixed source plus a STORE destination that may appear anywhere
    // among its options. BY/GET dynamic patterns are intentionally unsupported.
    if name.eq_ignore_ascii_case(b"SORT") || name.eq_ignore_ascii_case(b"SORT_RO") {
        let mut keys = Vec::new();
        if let Some(source) = args.get(1) {
            keys.push(*source);
        }
        let mut destination = None;
        let mut index = 2;
        while index < args.len() {
            if args[index].eq_ignore_ascii_case(b"STORE") && index + 1 < args.len() {
                destination = Some(args[index + 1]);
                index += 2;
            } else {
                index += 1;
            }
        }
        keys.extend(destination);
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

/// Declared key arguments paired with the access each key requires, following
/// Redis 7 key-spec RO/OW/RW flags rather than treating command-level WRITE as
/// a key permission. Dynamic keys are extracted by [`key_arguments`].
pub(super) fn key_accesses<'a>(name: &[u8], args: &'a [&'a [u8]]) -> Vec<(&'a [u8], KeyAccess)> {
    let keys = key_arguments(name, args);
    if keys.is_empty() {
        return Vec::new();
    }
    let Some(command) = registry()
        .iter()
        .find(|command| name.eq_ignore_ascii_case(command.name.as_bytes()))
    else {
        return Vec::new();
    };
    let readonly = command.flags.contains(&"readonly");
    keys.into_iter()
        .enumerate()
        .map(|(position, key)| {
            let access = if (readonly || name.eq_ignore_ascii_case(b"XREADGROUP"))
                && !name.eq_ignore_ascii_case(b"PFCOUNT")
            {
                KeyAccess::Read
            } else {
                write_key_access(command.name, args, position)
            };
            (key, access)
        })
        .collect()
}

fn write_key_access(name: &str, args: &[&[u8]], position: usize) -> KeyAccess {
    let name = name.as_bytes();
    let eq = |literal: &[u8]| name.eq_ignore_ascii_case(literal);
    // Commands whose key is created or replaced without reading its value.
    if eq(b"MSET")
        || eq(b"MSETNX")
        || eq(b"MSETEX")
        || eq(b"PSETEX")
        || eq(b"SETEX")
        || eq(b"SETNX")
        || eq(b"DEL")
        || eq(b"UNLINK")
    {
        return KeyAccess::Write;
    }
    // Redis marks SET's key spec RW/ACCESS with variable flags: plain SET is
    // overwrite-only, while GET also accesses the previous value. Skip option
    // operands so e.g. `SET k v EX GET` does not mistake an expiry value for GET.
    if eq(b"SET") {
        let mut index = 3;
        while index < args.len() {
            let option = args[index];
            if option.eq_ignore_ascii_case(b"GET") {
                return KeyAccess::ReadWrite;
            }
            index += if option.eq_ignore_ascii_case(b"EX")
                || option.eq_ignore_ascii_case(b"PX")
                || option.eq_ignore_ascii_case(b"EXAT")
                || option.eq_ignore_ascii_case(b"PXAT")
                || option.eq_ignore_ascii_case(b"IFEQ")
                || option.eq_ignore_ascii_case(b"IFNE")
                || option.eq_ignore_ascii_case(b"IFDEQ")
                || option.eq_ignore_ascii_case(b"IFDNE")
            {
                2
            } else {
                1
            };
        }
        return KeyAccess::Write;
    }
    // `BITFIELD` reads for `GET` and reads-modifies-writes for `SET`/`INCRBY`.
    if eq(b"BITFIELD") {
        let writes = args.get(2..).unwrap_or_default().iter().any(|argument| {
            argument.eq_ignore_ascii_case(b"SET") || argument.eq_ignore_ascii_case(b"INCRBY")
        });
        return if writes {
            KeyAccess::ReadWrite
        } else {
            KeyAccess::Read
        };
    }
    // Multi-key commands that mix read sources with written destinations. Key
    // ordering here matches `key_arguments`.
    if eq(b"BITOP") {
        return if position == 0 {
            KeyAccess::Write
        } else {
            KeyAccess::Read
        };
    }
    if eq(b"COPY") {
        return if position == 0 {
            KeyAccess::Read
        } else {
            KeyAccess::Write
        };
    }
    if eq(b"GEORADIUS") || eq(b"GEORADIUSBYMEMBER") {
        return if position == 0 {
            KeyAccess::Read
        } else {
            KeyAccess::Write
        };
    }
    if eq(b"GEOSEARCHSTORE")
        || eq(b"SDIFFSTORE")
        || eq(b"SINTERSTORE")
        || eq(b"SUNIONSTORE")
        || eq(b"ZDIFFSTORE")
        || eq(b"ZINTERSTORE")
        || eq(b"ZUNIONSTORE")
        || eq(b"ZRANGESTORE")
    {
        return if position == 0 {
            KeyAccess::Write
        } else {
            KeyAccess::Read
        };
    }
    if eq(b"PFMERGE") {
        return if position == 0 {
            KeyAccess::ReadWrite
        } else {
            KeyAccess::Read
        };
    }
    if eq(b"RENAME") || eq(b"RENAMENX") {
        return if position == 0 {
            KeyAccess::ReadWrite
        } else {
            KeyAccess::Write
        };
    }
    if eq(b"SORT") {
        return if position == 0 {
            KeyAccess::Read
        } else {
            KeyAccess::Write
        };
    }
    // Everything else that mutates a key inspects it first, as Redis key specs
    // mark with RW.
    KeyAccess::ReadWrite
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
pub(super) fn category_matches(name: &[u8], args: &[&[u8]], category: &[u8]) -> bool {
    let Some(command) = registry()
        .iter()
        .find(|command| name.eq_ignore_ascii_case(command.name.as_bytes()))
    else {
        return false;
    };
    let has = |flag: &str| command.flags.contains(&flag);
    let name = command.name;
    let group = command_group(name);
    if category.eq_ignore_ascii_case(b"all") {
        return true;
    }
    if let Some(subcommand) = args.get(1)
        && let Some(matched) = subcommand_category(name, subcommand, category)
    {
        return matched;
    }
    match category {
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

/// Bounded Redis 8 subcommand-category overrides for the commands whose parent
/// flags over- or under-grant the real subcommand categories. Unknown
/// subcommands fall back to the command-level approximation.
fn subcommand_category(name: &str, subcommand: &[u8], category: &[u8]) -> Option<bool> {
    let is = |candidates: &[&[u8]]| {
        candidates
            .iter()
            .any(|candidate| subcommand.eq_ignore_ascii_case(candidate))
    };
    match name {
        "client" => {
            let recognized = is(&[
                b"id",
                b"getname",
                b"setname",
                b"setinfo",
                b"info",
                b"tracking",
                b"caching",
                b"reply",
                b"getredir",
                b"trackinginfo",
                b"unblock",
                b"help",
                b"kill",
                b"list",
                b"no-evict",
                b"pause",
                b"unpause",
            ]);
            if !recognized {
                return None;
            }
            if category.eq_ignore_ascii_case(b"admin")
                || category.eq_ignore_ascii_case(b"dangerous")
            {
                Some(is(&[
                    b"kill",
                    b"list",
                    b"no-evict",
                    b"pause",
                    b"unblock",
                    b"unpause",
                ]))
            } else {
                None
            }
        }
        "memory" => {
            let usage = subcommand.eq_ignore_ascii_case(b"usage");
            let admin = is(&[b"stats", b"doctor", b"purge", b"malloc-stats"]);
            let recognized = usage || admin || subcommand.eq_ignore_ascii_case(b"help");
            if !recognized {
                return None;
            }
            if category.eq_ignore_ascii_case(b"read") {
                Some(usage)
            } else if category.eq_ignore_ascii_case(b"admin") {
                Some(admin)
            } else if category.eq_ignore_ascii_case(b"dangerous") {
                Some(false)
            } else {
                None
            }
        }
        "object" => {
            let help = subcommand.eq_ignore_ascii_case(b"help");
            let recognized = help || is(&[b"encoding", b"refcount", b"idletime", b"freq"]);
            if !recognized {
                return None;
            }
            if category.eq_ignore_ascii_case(b"keyspace") {
                Some(true)
            } else if category.eq_ignore_ascii_case(b"read") {
                Some(!help)
            } else if category.eq_ignore_ascii_case(b"admin")
                || category.eq_ignore_ascii_case(b"dangerous")
            {
                Some(false)
            } else {
                None
            }
        }
        "acl" => {
            let admin = is(&[
                b"deluser", b"dryrun", b"getuser", b"list", b"load", b"log", b"save", b"setuser",
                b"users",
            ]);
            let recognized = admin || is(&[b"cat", b"genpass", b"whoami", b"help"]);
            if !recognized {
                return None;
            }
            if category.eq_ignore_ascii_case(b"admin")
                || category.eq_ignore_ascii_case(b"dangerous")
            {
                Some(admin)
            } else {
                None
            }
        }
        "command" => {
            let recognized = is(&[
                b"count",
                b"list",
                b"info",
                b"getkeys",
                b"docs",
                b"help",
                b"getkeysandflags",
            ]);
            if !recognized {
                return None;
            }
            if category.eq_ignore_ascii_case(b"connection") {
                Some(true)
            } else if category.eq_ignore_ascii_case(b"admin")
                || category.eq_ignore_ascii_case(b"dangerous")
                || category.eq_ignore_ascii_case(b"read")
                || category.eq_ignore_ascii_case(b"write")
                || category.eq_ignore_ascii_case(b"keyspace")
            {
                Some(false)
            } else {
                None
            }
        }
        "xgroup" => {
            let help = subcommand.eq_ignore_ascii_case(b"help");
            let recognized = help
                || is(&[
                    b"create",
                    b"createconsumer",
                    b"delconsumer",
                    b"destroy",
                    b"setid",
                ]);
            if !recognized {
                return None;
            }
            if category.eq_ignore_ascii_case(b"write") {
                Some(!help)
            } else if category.eq_ignore_ascii_case(b"read")
                || category.eq_ignore_ascii_case(b"admin")
                || category.eq_ignore_ascii_case(b"dangerous")
                || category.eq_ignore_ascii_case(b"connection")
                || category.eq_ignore_ascii_case(b"keyspace")
            {
                Some(false)
            } else {
                None
            }
        }
        "xinfo" => {
            let help = subcommand.eq_ignore_ascii_case(b"help");
            let recognized = help || is(&[b"stream", b"groups", b"consumers"]);
            if !recognized {
                return None;
            }
            if category.eq_ignore_ascii_case(b"read") {
                Some(!help)
            } else if category.eq_ignore_ascii_case(b"write")
                || category.eq_ignore_ascii_case(b"admin")
                || category.eq_ignore_ascii_case(b"dangerous")
                || category.eq_ignore_ascii_case(b"connection")
                || category.eq_ignore_ascii_case(b"keyspace")
            {
                Some(false)
            } else {
                None
            }
        }
        _ => None,
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
    registry()
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
                    | b"commandstats"
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
                    "# Clients\r\nconnected_clients:{}\r\ncluster_connections:0\r\nmaxclients:{}\r\nauth_failure_limit:{}\r\nauth_source_limit:{}\r\nblocked_clients:{}\r",
                    self.connected_clients(),
                    self.max_clients(),
                    self.auth_failure_limit(),
                    self.auth_source_limit(),
                    self.blocked_clients()
                );
            }
            if matches!(section.as_slice(), b"stats" | b"default" | b"all") {
                let stats = self.stats();
                let _ = writeln!(
                    text,
                    "# Stats\r\ntotal_connections_received:{}\r\ntotal_commands_processed:{}\r\nrejected_connections:{}\r\nexpired_keys:{}\r\nexpired_subkeys:{}\r\nevicted_keys:{}\r\npubsub_channels:{}\r\npubsub_patterns:{}\r\naudit_log_write_failures:{}\r\naudit_syslog_write_failures:{}\r",
                    stats.total_connections_received,
                    stats.total_commands_processed,
                    stats.rejected_connections,
                    stats.expired_keys,
                    stats.expired_subkeys,
                    stats.evicted_keys,
                    stats.pubsub_channels,
                    stats.pubsub_patterns,
                    stats.audit_log_write_failures,
                    stats.audit_syslog_write_failures
                );
            }
            if matches!(section.as_slice(), b"commandstats" | b"all") {
                text.push_str("# Commandstats\r\n");
                for stat in self.command_statistics() {
                    let _ = writeln!(
                        text,
                        "cmdstat_{}:calls={},usec={},usec_per_call={},rejected_calls={},failed_calls={}\r",
                        stat.name,
                        stat.calls,
                        stat.usec,
                        stat.usec / stat.calls,
                        stat.rejected_calls,
                        stat.failed_calls
                    );
                }
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
        write_array_len(registry().len(), output);
        for item in registry() {
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
            write!(output, ":{}\r\n", registry().len()).expect("Vec write");
        }
    } else if sub.eq_ignore_ascii_case(b"LIST") {
        let mut names: Vec<&'static str> = registry().iter().map(|item| item.name).collect();
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
            write_array_len(registry().len(), output);
            for item in registry() {
                metadata(item, output);
            }
        } else {
            write_array_len(arguments.len() - 2, output);
            for name in &arguments[2..] {
                if let Some(item) = registry()
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
        let Some(item) = registry()
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
    fn family_groups_flatten_to_sorted_unique_registry() {
        let flattened: Vec<&'static str> = GROUPS
            .iter()
            .flat_map(|group| group.iter().map(|command| command.name))
            .collect();
        assert_eq!(flattened.len(), command_count());
        assert_eq!(registry().len(), command_count());

        let names: Vec<&str> = registry().iter().map(|command| command.name).collect();
        assert!(
            names.windows(2).all(|pair| pair[0] < pair[1]),
            "flattened registry must be strictly name-sorted and unique"
        );

        for group in GROUPS {
            assert!(
                group.windows(2).all(|pair| pair[0].name < pair[1].name),
                "each family group must be strictly name-sorted"
            );
        }

        let mut unique = flattened;
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(
            unique.len(),
            command_count(),
            "command names must be unique across family groups"
        );

        for name in [
            "get",
            "setbit",
            "hset",
            "lpush",
            "sadd",
            "zadd",
            "xadd",
            "subscribe",
            "hello",
            "info",
            "expire",
            "watch",
            "geoadd",
            "pfadd",
        ] {
            assert!(
                command_index(name.as_bytes()).is_some(),
                "representative command {name} is missing"
            );
        }
    }

    #[test]
    fn command_registry_lookup_is_case_insensitive() {
        assert_eq!(command_index(b"GET"), command_index(b"get"));
        assert!(command_index(b"GET").is_some());
        assert!(command_index(b"not-a-command").is_none());
        assert!(command_index(&[b'x'; 33]).is_none());
        let index = command_index(b"ping").unwrap();
        assert_eq!(command_name_at(index), Some("ping"));
        assert!(command_count() > index);
    }

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
    fn key_access_modes_match_read_write_key_specs() {
        assert_eq!(
            key_accesses(b"GET", &[b"GET", b"k"]),
            vec![(b"k".as_slice(), KeyAccess::Read)]
        );
        assert_eq!(
            key_accesses(b"HSET", &[b"HSET", b"h", b"f", b"v"]),
            vec![(b"h".as_slice(), KeyAccess::ReadWrite)]
        );
        assert_eq!(
            key_accesses(b"DEL", &[b"DEL", b"a", b"b"]),
            vec![
                (b"a".as_slice(), KeyAccess::Write),
                (b"b".as_slice(), KeyAccess::Write)
            ]
        );
        assert_eq!(
            key_accesses(b"SET", &[b"SET", b"k", b"v"]),
            vec![(b"k".as_slice(), KeyAccess::Write)]
        );
        assert_eq!(
            key_accesses(b"SET", &[b"SET", b"k", b"v", b"GET"]),
            vec![(b"k".as_slice(), KeyAccess::ReadWrite)]
        );
        assert_eq!(
            key_accesses(b"SET", &[b"SET", b"k", b"v", b"EX", b"GET"]),
            vec![(b"k".as_slice(), KeyAccess::Write)]
        );
        assert_eq!(
            key_accesses(b"BITFIELD", &[b"BITFIELD", b"b", b"GET", b"u8", b"0"]),
            vec![(b"b".as_slice(), KeyAccess::Read)]
        );
        assert_eq!(
            key_accesses(
                b"BITFIELD",
                &[
                    b"BITFIELD",
                    b"b",
                    b"GET",
                    b"u8",
                    b"0",
                    b"SET",
                    b"u8",
                    b"0",
                    b"1"
                ]
            ),
            vec![(b"b".as_slice(), KeyAccess::ReadWrite)]
        );
        assert_eq!(
            key_accesses(b"BITOP", &[b"BITOP", b"AND", b"dest", b"a", b"b"]),
            vec![
                (b"dest".as_slice(), KeyAccess::Write),
                (b"a".as_slice(), KeyAccess::Read),
                (b"b".as_slice(), KeyAccess::Read)
            ]
        );
        assert_eq!(
            key_accesses(b"COPY", &[b"COPY", b"src", b"dest"]),
            vec![
                (b"src".as_slice(), KeyAccess::Read),
                (b"dest".as_slice(), KeyAccess::Write)
            ]
        );
        assert_eq!(
            key_accesses(b"RENAME", &[b"RENAME", b"src", b"dest"]),
            vec![
                (b"src".as_slice(), KeyAccess::ReadWrite),
                (b"dest".as_slice(), KeyAccess::Write)
            ]
        );
        assert_eq!(
            key_accesses(
                b"SORT",
                &[b"SORT", b"src", b"LIMIT", b"0", b"1", b"STORE", b"dest"]
            ),
            vec![
                (b"src".as_slice(), KeyAccess::Read),
                (b"dest".as_slice(), KeyAccess::Write)
            ]
        );
        assert_eq!(
            key_accesses(b"ZINTERSTORE", &[b"ZINTERSTORE", b"dest", b"2", b"a", b"b"]),
            vec![
                (b"dest".as_slice(), KeyAccess::Write),
                (b"a".as_slice(), KeyAccess::Read),
                (b"b".as_slice(), KeyAccess::Read)
            ]
        );
        // Redis 7 marks PFCOUNT RW because it may promote a compact sketch,
        // despite its command-level READONLY flag.
        assert_eq!(
            key_accesses(b"PFCOUNT", &[b"PFCOUNT", b"hll"]),
            vec![(b"hll".as_slice(), KeyAccess::ReadWrite)]
        );
        assert_eq!(
            key_accesses(
                b"XREAD",
                &[
                    b"XREAD", b"COUNT", b"2", b"STREAMS", b"s1", b"s2", b"0-0", b"0-0"
                ]
            ),
            vec![
                (b"s1".as_slice(), KeyAccess::Read),
                (b"s2".as_slice(), KeyAccess::Read)
            ]
        );
        assert_eq!(
            key_accesses(
                b"XREADGROUP",
                &[b"XREADGROUP", b"GROUP", b"g", b"c", b"STREAMS", b"s", b">"]
            ),
            vec![(b"s".as_slice(), KeyAccess::Read)]
        );
        assert_eq!(
            key_arguments(
                b"SORT",
                &[b"SORT", b"src", b"STORE", b"first", b"STORE", b"last"]
            ),
            vec![b"src".as_slice(), b"last".as_slice()]
        );
        assert_eq!(
            key_accesses(
                b"GEORADIUS",
                &[
                    b"GEORADIUS",
                    b"geo",
                    b"1",
                    b"2",
                    b"3",
                    b"km",
                    b"STORE",
                    b"a",
                    b"STOREDIST",
                    b"b"
                ]
            ),
            vec![
                (b"geo".as_slice(), KeyAccess::Read),
                (b"b".as_slice(), KeyAccess::Write)
            ]
        );
        assert!(
            key_arguments(b"XREAD", &[b"XREAD", b"STREAMS", b"s", b"0-0", b"extra"]).is_empty()
        );
        assert!(key_accesses(b"PING", &[b"PING"]).is_empty());
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
