# Multi-owner recovery note (superseded)

This file recorded the state after an environment reset destroyed the uncommitted
worktree. The mode has since been reconciled, wired and validated; see
[multi-owner execution](multi-owner-execution.md) for the current behaviour and
limits.

Kept for history: the recovery replayed the pi session transcript (9 writes,
3 edits, 15 file-mutating scripts), merged published main, recreated
`partition.rs` and the mailbox tests, split the owner ack/result channels, gave
each executor its construction thread, reconciled the router with the published
synchronous execution boundary, and fixed a real bug it exposed — `touched_keys`
skipped deleted keys, so a cross-owner `RENAME` left the source behind.
