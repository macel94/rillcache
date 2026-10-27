//! `transactions` command metadata.

use super::{Command, cmd};

pub(super) const COMMANDS: &[Command] = &[
    cmd(
        "discard",
        1,
        &["noscript", "loading", "stale", "fast"],
        0,
        0,
        0,
    ),
    cmd(
        "exec",
        1,
        &[
            "noscript",
            "loading",
            "stale",
            "skip_monitor",
            "skip_slowlog",
        ],
        0,
        0,
        0,
    ),
    cmd(
        "multi",
        1,
        &["noscript", "loading", "stale", "fast"],
        0,
        0,
        0,
    ),
    cmd(
        "unwatch",
        1,
        &["noscript", "loading", "stale", "fast"],
        0,
        0,
        0,
    ),
    cmd(
        "watch",
        -2,
        &["noscript", "loading", "stale", "fast"],
        1,
        -1,
        1,
    ),
];
