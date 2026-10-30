//! Sorted/hashed/list/set collections, scans, GEO and SORT.
//!
//! `execute_collection_commands` must stay ahead of `execute_collections`:
//! their shared `ZADD` / `ZRANGE` arms intentionally resolve to the extended
//! handler first, matching the pre-split central dispatch.

use super::super::{Action, ShardExecutor};

pub(super) fn execute(
    store: &ShardExecutor,
    _command: &[u8],
    arguments: &[&[u8]],
    output: &mut Vec<u8>,
) -> Option<Action> {
    if store.execute_scan(arguments, output)
        || store.execute_geo(arguments, output)
        || store.execute_sort(arguments, output)
        || store.execute_collection_commands(arguments, output)
        || store.execute_collections(arguments, output)
    {
        return Some(Action::Continue);
    }
    None
}
