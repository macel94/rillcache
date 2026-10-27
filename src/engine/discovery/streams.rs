//! `streams` command metadata.

use super::{Command, cmd};

pub(super) const COMMANDS: &[Command] = &[
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
];
