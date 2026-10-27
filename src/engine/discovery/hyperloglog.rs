//! `hyperloglog` command metadata.

use super::{Command, cmd};

pub(super) const COMMANDS: &[Command] = &[
    cmd("pfadd", -2, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("pfcount", -2, &["readonly"], 1, -1, 1),
    cmd("pfmerge", -2, &["write", "denyoom"], 1, -1, 1),
];
