# Multi-owner execution: opt-in whole-owner routing and transactions

Status: **implemented and wire-tested opt-in mode; not the default, and not a
performance or scaling claim.** The proven one-owner path remains the default and
is unchanged. This document owns the behavior, acceptance evidence and limits of
this increment; it implements sections 3-8, 10 and 11 of the reviewed target
design (SHA256 `326db47d02465156f8011f4b7a557ba5c2daa7018074963f574e5d0b950a9add`)
only to the extent stated under **Not claimed**.

## Configuration

`--storage-owners N` (`1..=64`) opts into multi-owner data execution. `1` or an
absent flag keeps the existing single-owner implementation exactly as shipped.
Untrusted or unsupported combinations are rejected before binding:

- multi-owner (`N > 1`) requires the Unix readiness reactor, and therefore
  rejects `--threads` and `--tls-bind`;
- the count must be valid, and startup reports the owner and virtual-partition
  counts on stderr.

The default stays single-owner until this mode's own evidence justifies promotion.

## Ownership and execution

- Each owner is a dedicated thread running a current-thread Tokio runtime and a
  `LocalSet` that drives one `OwnerActor`. Its `ShardExecutor` owns `Data`,
  creation and drop on that thread; it is `!Send`/`!Sync` and never travels in a
  request, reply or guard.
- `PartitionLayout` fixes one immutable layout for the process. `N == 1` uses one
  partition; `N > 1` uses 64 virtual partitions assigned round-robin over owners.
  Owner counts need not be powers of two; every owner receives at least one
  partition and counts differ by at most one. No runtime migration exists.
- Each owner has separate bounded DATA and CONTROL lanes, bounded retained input
  bytes and a bounded reply budget. Byte credit precedes a message slot, so a
  caller waiting for a slot already owns its bytes and every slot holder can make
  progress. CONTROL work is still serviced while DATA admission is fenced.
- A single-owner command is one owned request/reply hop: arguments are copied into
  an owned frame after admission, the reply owns its bytes, and no `Data` lease
  exists. In the default single-owner mode the socket thread still executes
  directly with no hop.

## Multi-key atomicity and EXEC

Commands whose keys span owners (and every whole-keyspace command) take
**whole-owner reservations**:

1. A bounded acquisition dispatcher serializes only reservation acquisition. An
   executing transaction never holds it, so a fenced owner always makes progress.
2. Reservations are granted in increasing owner-ID order after the owner's current
   turn. Contention with an already-fenced owner waits and retries; it is not a
   failure, so no fence leaks and the socket is not closed.
3. Under the fence, one operation time is chosen. Watched-key generations, TTLs
   and conditions are validated once. The command executes against a scratch
   `ShardExecutor` built from cloned entries for exactly the keys it may touch.
4. Only keys the command **actually wrote** are transplanted back to their owners.
   A same-value write still advances the `WATCH` version; a delete removes the key;
   a rejected command (validation, type, or payload budget) publishes nothing.
   `EXEC` runtime errors remain array elements and do not roll back earlier
   commands, matching the existing contract.
5. Journal records and keyspace notifications are produced by the deterministic
   execution turn under the fence, then every participant is released.

The coordinator is a socket-thread call: it blocks on bounded replies while it
holds no other lock. There is no cancellation window that can strand a fence, and
worker failure after a partial apply is fail-stop, never continued partial-state
service.

`WATCH` records the owning worker and that owner's epoch per key. `EXEC` probes
every participating owner under one reservation, so a same-value mutation or a
flush on any owner aborts with a null array.

## Blocking, expiry, budgets and durable effects

- Blocking commands gather their candidate keys, check and mutate on the first
  non-empty key in command order under reservations, then release. Waiting stays
  in the session; disconnect and `CLIENT UNBLOCK` reclaim it without an owner
  registration to leak. Wakeups re-enter the same protocol.
- Lazy expiry happens at the reservation's single operation time. Background
  reclamation fans out per owner. Expired notifications keep the existing classes.
- `DBSIZE`, `KEYS`, `RANDOMKEY`, `SCAN`, `FLUSHALL` and `FLUSHDB` reserve every
  owner. `SCAN` encodes the owner in the decimal cursor and walks owners until the
  cursor returns to zero, preserving weak iteration. `INFO`, `MEMORY` and `STATS`
  publish one coherent aggregate dataset for the duration of the command.
- Payload accounting uses one shared budget. A single-owner write admits against
  `own payload + free global credit`, so two owners cannot each spend the whole
  budget. A multi-owner write checks the exact global total and, under an eviction
  policy, reclaims headroom from the largest fenced owner before applying; under
  `noeviction` it rejects the whole command without a partial write.
- The append-only journal keeps one global order. A mutation is recorded by the
  turn that performs it under the fence. Replay at startup is seeded into the
  owning workers before any frontend starts, and a rewrite builds one snapshot
  from every owner under a barrier.
- Pub/sub, ACL/auth, pause, client administration and observation remain the
  shared `ServerServices` they already were; delivery queues stay bounded.
- Shutdown closes admission, rejects new work, stops owners, joins every owner
  thread and then returns. Repeated shutdown is a no-op.

## Acceptance evidence

Repository tests (`make dev-check`, `make test`):

- `engine::router::tests` (15): layout stability for every owner count, binary and
  empty keys, single-owner hops, atomic cross-owner `MSET`/`MGET` with no partial
  commit, cross-owner `RENAME`, duplicate-key order/counts, same-value `WATCH`
  conflict, `EXEC` no-rollback, cross-owner blocking wakeup, global commands,
  aggregated `INFO`/`MEMORY`, finite-budget rejection and reclaim,
  one-notification-per-written-key, owner-encoded `SCAN`, joined idempotent
  shutdown, and concurrent opposite-order writers with a non-atomic-reader probe.
- `server::multi_owner_tests` (4): configuration rejection, socket-level atomicity
  and transactions, blocking wakeup/disconnect, and append-only state replayed
  into every owner across a real restart.

Independent external probe `/tmp/rillcache-architecture-wire-acceptance.py`
(SHA256 recorded in the reports under `benchmarks/studies/multi-owner/`) passes
all 7 cases at `--storage-owners 1, 2, 4, 8, 16` and for the unchanged default.

## Not claimed

- The socket adapters are still the synchronous Mio reactor. A remote hop blocks
  the calling socket thread on a bounded reply; there are no per-owner Tokio
  socket tasks or `LocalSet` connection tasks. The router API is asynchronous, but
  this increment does not claim asynchronous socket execution.
- No zero-hop local dispatch **on a socket thread** in multi-owner mode. Zero-hop
  local execution is retained by the default single-owner mode.
- TLS and fixed `--threads` frontends are rejected with multi-owner rather than
  routed through an unverified bridge. They keep their existing single-owner
  behavior.
- The scratch transplant copies the touched key set (bounded at 32 MiB); a larger
  transaction fails with a clear error instead of unbounded memory. It is a
  correctness-first stand-in for key-level intent locks and VLL-style scheduling.
- Whole-owner reservations serialize more than necessary. There is no key-level
  concurrency, batching, speculative execution or lock-free data path.
- A single-owner write that exhausts shared credit can return OOM without first
  evicting keys owned by another worker; reclaim is guaranteed for multi-owner
  writes. A runtime `CONFIG` payload-limit reduction blocks further growth and is
  reclaimed on the next multi-owner write that needs headroom, not at the instant
  of configuration.
- No CPU affinity/pinning, NUMA placement, runtime resharding, replication or
  clustering claim. Seeded XXH3 routing is not a cryptographic HashDoS defense.
- No performance or scaling claim, and no promotion to default. Both unchanged
  required Valkey gates continue to measure the default single-owner path.
