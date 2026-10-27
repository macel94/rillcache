//! `hashes` command metadata.

use super::{Command, cmd};

pub(super) const COMMANDS: &[Command] = &[
    cmd("hdel", -3, &["write", "fast"], 1, 1, 1),
    cmd("hexists", 3, &["readonly", "fast"], 1, 1, 1),
    cmd("hexpire", -6, &["write", "fast"], 1, 1, 1),
    cmd("hexpireat", -6, &["write", "fast"], 1, 1, 1),
    cmd("hexpiretime", -5, &["readonly", "fast"], 1, 1, 1),
    cmd("hget", 3, &["readonly", "fast"], 1, 1, 1),
    cmd("hgetall", 2, &["readonly"], 1, 1, 1),
    cmd("hgetdel", -5, &["write", "fast"], 1, 1, 1),
    cmd("hgetex", -5, &["write", "fast"], 1, 1, 1),
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
];
