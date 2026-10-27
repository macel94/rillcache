//! `pubsub` command metadata.

use super::{Command, cmd};

pub(super) const COMMANDS: &[Command] = &[
    cmd(
        "psubscribe",
        -2,
        &["pubsub", "noscript", "loading", "stale"],
        0,
        0,
        0,
    ),
    cmd(
        "publish",
        3,
        &["pubsub", "loading", "stale", "fast"],
        0,
        0,
        0,
    ),
    cmd(
        "pubsub",
        -2,
        &["pubsub", "loading", "stale", "fast"],
        0,
        0,
        0,
    ),
    cmd(
        "punsubscribe",
        -1,
        &["pubsub", "noscript", "loading", "stale"],
        0,
        0,
        0,
    ),
    cmd(
        "spublish",
        3,
        &["pubsub", "loading", "stale", "fast"],
        0,
        0,
        0,
    ),
    cmd(
        "ssubscribe",
        -2,
        &["pubsub", "noscript", "loading", "stale"],
        0,
        0,
        0,
    ),
    cmd(
        "subscribe",
        -2,
        &["pubsub", "noscript", "loading", "stale"],
        0,
        0,
        0,
    ),
    cmd(
        "sunsubscribe",
        -1,
        &["pubsub", "noscript", "loading", "stale"],
        0,
        0,
        0,
    ),
    cmd(
        "unsubscribe",
        -1,
        &["pubsub", "noscript", "loading", "stale"],
        0,
        0,
        0,
    ),
];
