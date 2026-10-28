//! Redis Streams commands (`X*`).

use super::super::{Action, Store};

pub(super) fn execute(
    store: &Store,
    _command: &[u8],
    arguments: &[&[u8]],
    output: &mut Vec<u8>,
) -> Option<Action> {
    if store.execute_streams(arguments, output) {
        return Some(Action::Continue);
    }
    None
}
