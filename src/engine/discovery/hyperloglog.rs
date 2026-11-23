//! `hyperloglog` command metadata.

use super::{Command, cmd};

pub(super) const COMMANDS: &[Command] = &[
    cmd("pfadd", -2, &["write", "denyoom", "fast"], 1, 1, 1),
    cmd("pfcount", -2, &["readonly"], 1, -1, 1),
    // PFDEBUG <subcommand> <key>: the key is the third token.
    cmd("pfdebug", -3, &["readonly", "admin", "dangerous"], 2, 2, 1),
    cmd("pfmerge", -2, &["write", "denyoom"], 1, -1, 1),
    cmd(
        "pfselftest",
        1,
        &["readonly", "admin", "dangerous"],
        0,
        0,
        0,
    ),
];
