//! `lists` command metadata.

use super::{Command, cmd};

pub(super) const COMMANDS: &[Command] = &[
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
    cmd("rpop", -2, &["write", "fast"], 1, 1, 1),
    cmd("rpoplpush", 3, &["write", "denyoom"], 1, 2, 1),
    cmd("rpush", -3, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("rpushx", -3, &["write", "denyoom", "fast"], 1, 1, 1),
];
