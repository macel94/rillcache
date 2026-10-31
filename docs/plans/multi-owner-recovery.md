# Multi-owner recovery handoff

The worktrees directory and tmux server were deleted by an environment reset while
the multi-owner mode was mid-repair. Everything below is what survived and what is
still missing. `main` is unaffected.

## State of this branch

- `cargo check --lib --tests` is clean.
- Compiled and tested:
  - `engine/partition.rs` — 2 tests (balanced 1/64 placement over 1..=64 owners,
    stable owner per binary/empty/large key, seed sensitivity, invalid counts).
  - `engine/mailbox.rs` — 6 tests (oversize rejection, cancelled admission returns
    slot+credit without preparing input, `close()` wakes byte and message waiters,
    fenced data lane never consumes control capacity, `close_data` preserves
    admitted data, cancelled reply returns all credit). Every blocking await is
    bounded so a regression fails instead of hanging.
- Preserved on disk but excluded from the build with `#[cfg(any())]`:
  `engine/owner_actor.rs`, `engine/router.rs` (1,439 lines) and their tests.

## Exact blockers, in order

1. `engine/owner_actor/tests.rs` mixes two reply channels: it passes a
   `ReplySender<usize>` where `Control::Read { reply }` requires
   `Acknowledge = ReplySender<Result<(), OwnerError>>`, and its `ScopedWork`
   closure must end with `Ok(())`. Give the op its own `usize` channel and keep
   the ack channel separate.
2. `bridge.rs` lost `ImmediateExecution` (services handle, `run_request`,
   `journal_sync`, `purge_expired`, `auto_rewrite_if_due`) and `ready_result`
   (single-poll adapter that panics rather than blocking when a future is
   pending). `router.rs` imports both.
3. `router.rs` was written against an async `Execution` boundary in
   `session/*` (`async fn execute/watch_snapshot/execute_transaction/blocking_*`).
   Published `main` has the synchronous trait. Either restore the async boundary
   (session + reactor + tokio path) or adapt the router to the sync trait; the
   design requires the local fast path to stay hop-free either way.
4. Then: `serve_multi` wiring plus the `--storage-owners` flag (the merge commit
   `13948a1` shows the intended shape), journal/pubsub/admin behaviour across
   owners, `make ci`, the wire probes at several owner counts, and both protected
   gates on a real PR head.

## Evidence to reuse

- External correctness probe: `/tmp/rillcache-architecture-wire-acceptance.py`
  (7 cases; passes on published `main`). Earlier multi-owner runs passed at
  owners 1, 2, 3, 5, 8 and 16 before the reset.
- Verified partitioning before the reset: `--storage-owners N` spawned exactly N
  `rillcache-owner-*` threads.
- Design: `docs/plans/thread-per-core-target.md`, `docs/plans/multi-owner-execution.md`.

No feature is claimed by this branch and no PR exists for it.
