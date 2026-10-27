//! `strings` command metadata.

use super::{Command, cmd};

pub(super) const COMMANDS: &[Command] = &[
    cmd("append", 3, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("decr", 2, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("decrby", 3, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("digest", 2, &["readonly", "fast"], 1, 1, 1),
    cmd("get", 2, &["readonly", "fast"], 1, 1, 1),
    cmd("getdel", 2, &["write", "fast"], 1, 1, 1),
    cmd("getex", -2, &["write", "fast"], 1, 1, 1),
    cmd("getrange", 4, &["readonly"], 1, 1, 1),
    cmd("getset", 3, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("incr", 2, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("incrby", 3, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("incrbyfloat", 3, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("lcs", -3, &["readonly"], 1, 2, 1),
    cmd("mget", -2, &["readonly", "fast"], 1, -1, 1),
    cmd("mset", -3, &["write", "denyoom"], 1, -1, 2),
    cmd("msetex", -4, &["write", "denyoom"], 2, -1, 2),
    cmd("msetnx", -3, &["write", "denyoom"], 1, -1, 2),
    cmd("psetex", 4, &["write", "denyoom"], 1, 1, 1),
    cmd("set", -3, &["write", "denyoom"], 1, 1, 1),
    cmd("setex", 4, &["write", "denyoom"], 1, 1, 1),
    cmd("setnx", 3, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("setrange", 4, &["write", "denyoom"], 1, 1, 1),
    cmd("strlen", 2, &["readonly", "fast"], 1, 1, 1),
    cmd("substr", 4, &["readonly"], 1, 1, 1),
];
