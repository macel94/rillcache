//! Server administration, configuration, discovery and unsupported-command
//! responses.
//!
//! `execute_collection_config` runs before the general administration handler
//! so CONFIG GET/SET keep their specialized implementation instead of the
//! generic CONFIG fallback.

use super::super::{Action, ShardExecutor, arity};

pub(super) fn execute(
    store: &ShardExecutor,
    command: &[u8],
    arguments: &[&[u8]],
    output: &mut Vec<u8>,
) -> Option<Action> {
    if store.execute_collection_config(arguments, output)
        || store.execute_admin(arguments, output)
        || store.execute_discovery(arguments, output)
    {
        return Some(Action::Continue);
    }
    if command.eq_ignore_ascii_case(b"QUIT") {
        if arguments.len() == 1 {
            output.extend_from_slice(b"+OK\r\n");
            return Some(Action::Close);
        }
        arity("quit", output);
        return Some(Action::Continue);
    }
    if store.execute_unsupported(arguments, output) {
        return Some(Action::Continue);
    }
    None
}
