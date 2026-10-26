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

// Only commands dispatched by the base Store implementation are listed here.
const COMMANDS: &[Command] = &[
    cmd("command", -1, &["loading", "stale"], 0, 0, 0),
    cmd("config", -2, &["admin", "noscript"], 0, 0, 0),
    cmd("copy", -3, &["write", "denyoom"], 1, 1, 1),
    cmd("decr", 2, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("decrby", 3, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("del", -2, &["write", "fast"], 1, -1, 1),
    cmd("echo", 2, &["fast"], 0, 0, 0),
    cmd("exists", -2, &["readonly", "fast"], 1, -1, 1),
    cmd("expire", -3, &["write", "fast"], 1, 1, 1),
    cmd("expireat", -3, &["write", "fast"], 1, 1, 1),
    cmd("expiretime", 2, &["readonly", "fast"], 1, 1, 1),
    cmd("flushall", -1, &["write", "slow"], 0, 0, 0),
    cmd("flushdb", -1, &["write", "slow"], 0, 0, 0),
    cmd("function", -2, &["write", "admin"], 0, 0, 0),
    cmd("get", 2, &["readonly", "fast"], 1, 1, 1),
    cmd("getdel", 2, &["write", "fast"], 1, 1, 1),
    cmd("getex", -2, &["write", "fast"], 1, 1, 1),
    cmd("getset", 3, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("incr", 2, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("incrby", 3, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("info", -1, &["loading", "stale"], 0, 0, 0),
    cmd("memory", -2, &["readonly", "slow"], 0, 0, 0),
    cmd("persist", 2, &["write", "fast"], 1, 1, 1),
    cmd("pexpire", -3, &["write", "fast"], 1, 1, 1),
    cmd("pexpireat", -3, &["write", "fast"], 1, 1, 1),
    cmd("pexpiretime", 2, &["readonly", "fast"], 1, 1, 1),
    cmd("ping", -1, &["fast"], 0, 0, 0),
    cmd("psetex", 4, &["write", "denyoom"], 1, 1, 1),
    cmd("pttl", 2, &["readonly", "fast"], 1, 1, 1),
    cmd("quit", -1, &["fast"], 0, 0, 0),
    cmd("script", -2, &["admin"], 0, 0, 0),
    cmd("set", -3, &["write", "denyoom"], 1, 1, 1),
    cmd("setex", 4, &["write", "denyoom"], 1, 1, 1),
    cmd("setnx", 3, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("ttl", 2, &["readonly", "fast"], 1, 1, 1),
    cmd("xadd", -5, &["write", "denyoom"], 1, 1, 1),
    cmd("xdel", -3, &["write"], 1, 1, 1),
    cmd("xgroup", -2, &["write"], 2, 2, 1),
    cmd("xinfo", -2, &["readonly"], 2, 2, 1),
    cmd("xlen", 2, &["readonly", "fast"], 1, 1, 1),
    cmd("xreadgroup", -7, &["write"], 0, 0, 0),
];

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
            [_] => &[b"server", b"memory", b"keyspace"],
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
                b"server" | b"memory" | b"keyspace" | b"default" | b"all"
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
                    "# Memory\r\nrillcache_payload_bytes:{}\r\nrillcache_payload_limit_bytes:{}\r\nrillcache_payload_scope:key_and_value_bytes_not_RSS\r",
                    data.payload_bytes, self.max_payload_bytes
                );
            }
            if matches!(section.as_slice(), b"keyspace" | b"default" | b"all") {
                let _ = writeln!(
                    text,
                    "# Keyspace\r\ndb0:keys={live_keys},expires={expiring_keys}\r"
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
    fn unknown_dispatch_is_false_and_leaves_output_untouched() {
        let store = Store::new(1024);
        let mut output = b"existing".to_vec();
        assert!(!store.execute_discovery(&[b"NO-SUCH-COMMAND", b"\0"], &mut output));
        assert_eq!(output, b"existing");
        assert!(!store.execute_discovery(&[], &mut output));
        assert_eq!(output, b"existing");
    }
}
