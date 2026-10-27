//! `geo` command metadata.

use super::{Command, cmd};

pub(super) const COMMANDS: &[Command] = &[
    cmd("geoadd", -5, &["write", "denyoom"], 1, 1, 1),
    cmd("geodist", -4, &["readonly"], 1, 1, 1),
    cmd("geohash", -2, &["readonly"], 1, 1, 1),
    cmd("geopos", -2, &["readonly"], 1, 1, 1),
    cmd("georadius", -6, &["write", "denyoom"], 1, 1, 1),
    cmd("georadius_ro", -6, &["readonly"], 1, 1, 1),
    cmd("georadiusbymember", -5, &["write", "denyoom"], 1, 1, 1),
    cmd("georadiusbymember_ro", -5, &["readonly"], 1, 1, 1),
    cmd("geosearch", -7, &["readonly"], 1, 1, 1),
    cmd("geosearchstore", -8, &["write", "denyoom"], 1, 2, 1),
];
