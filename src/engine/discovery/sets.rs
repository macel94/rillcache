//! `sets` command metadata.

use super::{Command, cmd};

pub(super) const COMMANDS: &[Command] = &[
    cmd("sadd", -3, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("scard", 2, &["readonly", "fast"], 1, 1, 1),
    cmd("sdiff", -2, &["readonly"], 1, -1, 1),
    cmd("sdiffstore", -3, &["write", "denyoom"], 1, -1, 1),
    cmd("sinter", -2, &["readonly"], 1, -1, 1),
    cmd("sintercard", -3, &["readonly"], 2, -1, 1),
    cmd("sinterstore", -3, &["write", "denyoom"], 1, -1, 1),
    cmd("sismember", 3, &["readonly", "fast"], 1, 1, 1),
    cmd("smembers", 2, &["readonly"], 1, 1, 1),
    cmd("smismember", -3, &["readonly", "fast"], 1, 1, 1),
    cmd("smove", 4, &["write", "fast"], 1, 2, 1),
    cmd("spop", -2, &["write", "fast"], 1, 1, 1),
    cmd("srandmember", -2, &["readonly"], 1, 1, 1),
    cmd("srem", -3, &["write", "fast"], 1, 1, 1),
    cmd("sscan", -3, &["readonly", "slow"], 1, 1, 1),
    cmd("sunion", -2, &["readonly"], 1, -1, 1),
    cmd("sunionstore", -3, &["write", "denyoom"], 1, -1, 1),
];
