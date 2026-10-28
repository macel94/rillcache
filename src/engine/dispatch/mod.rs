//! Command-family routing for `Store::execute_locked`.
//!
//! `engine.rs` keeps only the direct PING fast path; every other store command
//! is handled by one family module below. Each family owns the command-name
//! match for its commands and returns the existing [`Action`], so a feature adds
//! its arm to its family instead of growing one central match.
//!
//! Family command sets are disjoint (the only intentional overlap, `ZADD` /
//! `ZRANGE`, stays ordered inside [`collections`]). Routing therefore cannot
//! change which handler replies.
//!
//! Transaction markers (`MULTI`, `EXEC`, `DISCARD`, `WATCH`, `UNWATCH`) are
//! connection state in `session.rs`. `EXEC` replays its queue through
//! `execute_locked`, so queued store commands reach this dispatcher normally;
//! the markers themselves never do. Blocking variants likewise live in
//! `session.rs` because they need per-connection wait state.

use super::{Action, Store};

mod admin;
mod collections;
mod keyspace;
mod pubsub;
mod streams;
mod strings;

/// Direct GET/SET/ECHO entry used by the engine hot path. Keeping one
/// implementation here avoids duplicating SET option parsing in `engine.rs`.
#[inline(always)]
pub(super) fn hot_string(
    store: &Store,
    command: &[u8],
    arguments: &[&[u8]],
    output: &mut Vec<u8>,
) -> Option<Action> {
    strings::execute_data(store, command, arguments, output)
}

/// Route one non-hot store command. Empty input and unknown commands retain
/// their historical replies.
#[inline]
pub(super) fn route(store: &Store, arguments: &[&[u8]], output: &mut Vec<u8>) -> Action {
    let Some(command) = arguments.first() else {
        output.extend_from_slice(b"-ERR empty command\r\n");
        return Action::Continue;
    };
    if let Some(action) = strings::execute_other(store, command, arguments, output) {
        return action;
    }
    if let Some(action) = keyspace::execute(store, command, arguments, output) {
        return action;
    }
    if let Some(action) = collections::execute(store, command, arguments, output) {
        return action;
    }
    if let Some(action) = streams::execute(store, command, arguments, output) {
        return action;
    }
    if let Some(action) = pubsub::execute(store, command, arguments, output) {
        return action;
    }
    if let Some(action) = admin::execute(store, command, arguments, output) {
        return action;
    }
    output.extend_from_slice(b"-ERR unknown command\r\n");
    Action::Continue
}
