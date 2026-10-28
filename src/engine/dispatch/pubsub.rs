//! Channel publish commands handled by the store.
//!
//! Connection-scoped subscription commands stay in `session.rs`; only the
//! store-wide fire-and-forget publish operations are dispatched here.

use super::super::{Action, Store, arity, integer};

pub(super) fn execute(
    store: &Store,
    command: &[u8],
    arguments: &[&[u8]],
    output: &mut Vec<u8>,
) -> Option<Action> {
    if command.eq_ignore_ascii_case(b"PUBLISH") {
        if arguments.len() != 3 {
            arity("publish", output);
        } else {
            integer(
                i64::try_from(store.publish(arguments[1], arguments[2])).unwrap_or(i64::MAX),
                output,
            );
        }
        return Some(Action::Continue);
    }
    if command.eq_ignore_ascii_case(b"SPUBLISH") {
        if arguments.len() != 3 {
            arity("spublish", output);
        } else {
            integer(
                i64::try_from(store.publish_shard(arguments[1], arguments[2])).unwrap_or(i64::MAX),
                output,
            );
        }
        return Some(Action::Continue);
    }
    None
}
