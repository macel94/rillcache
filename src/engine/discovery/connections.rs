//! `connections` command metadata.

use super::{Command, cmd};

pub(super) const COMMANDS: &[Command] = &[
    cmd("asking", 1, &["fast"], 0, 0, 0),
    cmd(
        "auth",
        -2,
        &["noscript", "loading", "stale", "fast"],
        0,
        0,
        0,
    ),
    cmd(
        "client",
        -2,
        &["admin", "noscript", "loading", "stale"],
        0,
        0,
        0,
    ),
    cmd("echo", 2, &["fast"], 0, 0, 0),
    cmd(
        "hello",
        -1,
        &["noscript", "loading", "stale", "fast"],
        0,
        0,
        0,
    ),
    cmd("ping", -1, &["fast"], 0, 0, 0),
    cmd("quit", -1, &["fast"], 0, 0, 0),
    cmd("readonly", 1, &["fast"], 0, 0, 0),
    cmd("readwrite", 1, &["fast"], 0, 0, 0),
    cmd(
        "reset",
        1,
        &["noscript", "loading", "stale", "fast"],
        0,
        0,
        0,
    ),
    cmd("select", 2, &["loading", "stale", "fast"], 0, 0, 0),
];
