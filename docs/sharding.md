# Shared-nothing keyspace migration (increment 1)

Status: **routing seam only**. The server still owns one keyspace shard guarded
by `RwLock`. This increment adds the routing contract and the `Keyspace`
facade; it does not claim a sharded or lock-free server, and it does not change
wire behavior.

## Enforced constraints

The migration must stay inside the repository's existing rules:

- `#![forbid(unsafe_code)]` on every target. No raw pointers, FFI, allocator or
  atomic-pointer tricks.
- The only external crate exception is `rustls` and its transitive set, enforced
  by `tests/policy.rs`. `core_affinity`, `tokio`, `bytes`, `hashbrown`, `flume`
  and `crossbeam` are therefore unavailable.
- `std::collections::HashMap` is the safe per-shard map; `std::sync::mpsc`'s
  bounded `sync_channel` is the owner-thread mailbox and the oneshot reply.
- Borrowed `&[u8]` parsing remains the zero-copy boundary; the protocol already
  dispatches small argument lists without allocating.
- Core pinning is **external**: run under `taskset -c 0-3 …` or a cgroup cpuset.
  `std` exposes no affinity API, and adding a crate would require relaxing the
  dependency policy. The process does not claim to pin itself.

## Current seam

`src/engine/shard.rs` provides:

- `ShardRouter`: power-of-two shard counts in `1..=64`, a process-random seed,
  and `xxh3` plus a SplitMix64 finalizer for the masked index.
- `Keyspace`: owns the current `RwLock<Data>` and exposes `read`, `write` and
  `try_write`, so the existing command modules keep compiling unchanged.
- `Store::shard_count()` and `Store::shard_for(key)`: public introspection of
  the active layout.

With one shard the mask is zero, so every key routes to shard `0`; the routing
hash is still computed and tested so the contract does not change when the
owner-thread step lands. The seed is stable for the process lifetime so every
worker agrees on ownership, but random across restarts so client-controlled keys
cannot deliberately target one shard. Each shard's `HashMap` keeps its own
randomized hasher.

## Why this order

A message hop per single-key command is a real cost, and the mandatory Valkey
gate requires strict wins rather than neutral refactors. The seam is therefore
separated from the execution change:

1. **Routing and facade (this increment).** No behavior change.
2. **One owner thread.** Move `Data` and the transaction gate behind a bounded
   mailbox; benchmark before accepting.
3. **Multiple shards, single-key commands.** Fan out `MGET`/`DEL`/`EXISTS`;
   keep atomic multi-key commands on a coordinator.
4. **Atomic multi-key.** Prepare/commit in deterministic shard order, then
   `MULTI`/`EXEC`/`WATCH`, blocking commands and `SCAN` cursors.
5. **Cross-cutting ownership.** Journal sequencer, global eviction budget,
   pub/sub and `MONITOR` routing, joined shutdown.

## Hazards that remain

- `MSET`/`MSETEX`, `RENAME`, `COPY`, transactions and `WATCH` need an explicit
  commit protocol once keys live in different owners. Independent messages are
  not atomicity.
- The append-only journal has one global order; per-shard writers need a
  sequencer or replay can diverge.
- Blocking list/zset/stream waiters must be shard-owned, and the session loop
  must poll both its socket and the reply channel without `tokio`.
- `SCAN` cursors must encode a shard id; expiry, `maxmemory` eviction accounting
  and keyspace notifications are currently global.
- The existing `transaction_gate` read lock is taken by every command; moving
  only `Data` would leave that contention in place.

## EARS acceptance for this increment

- **Ubiquitous:** While the layout has one shard, the server shall route every
  key, including empty, binary and multi-kilobyte keys, to shard `0`.
- **Event-driven:** When a router is built with a count in `1..=64`, the count
  shall be a power of two and the router shall report exactly that count.
- **Unwanted behavior:** If the count is zero, not a power of two, or above 64,
  then router construction shall return a typed error instead of panicking.
- **State-driven:** While a process is running, the same key and seed shall map
  to the same shard; a different seed shall be able to change ownership.
- **Ubiquitous:** Routing shall use only safe, allocation-free computation over
  the borrowed key bytes.

## Verification

```sh
cargo test --locked --offline engine::shard
cargo clippy --locked --offline --all-targets --all-features -- -D warnings
cargo test --locked --offline
```

The dependency-policy test and every existing wire/behavior test remain
unchanged.
