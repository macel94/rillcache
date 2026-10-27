//! `bitmaps` command metadata.

use super::{Command, cmd};

pub(super) const COMMANDS: &[Command] = &[
    cmd("bitcount", -2, &["readonly"], 1, 1, 1),
    cmd("bitfield", -2, &["write", "denyoom"], 1, 1, 1),
    cmd("bitfield_ro", -2, &["readonly"], 1, 1, 1),
    cmd("bitop", -4, &["write", "denyoom"], 1, -1, 1),
    cmd("bitpos", -3, &["readonly"], 1, 1, 1),
    cmd("getbit", 3, &["readonly", "fast"], 1, 1, 1),
    cmd("setbit", 4, &["write", "denyoom"], 1, 1, 1),
];
