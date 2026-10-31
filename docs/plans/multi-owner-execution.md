# Multi-owner data execution

Status: **implemented and opt-in**. `--storage-owners 1` (the default) keeps the
proven zero-hop single-owner path; `--storage-owners N` (`2..=64`) runs data
execution on N owned workers.

## What it is

Each owner is a dedicated thread with its own `!Send` `ShardExecutor` and its own
`Data`: entries, TTL indexes, mutation versions, eviction state and payload
accounting. Data is constructed on its owner and never leaves it. The socket loop
drives a router, which owns the workers and implements the same execution
boundary as the single-owner path, so sessions, authorization, observers,
pub/sub, pause and shutdown behave identically.

Keys map to one of 64 virtual partitions, and partitions map to owners, so any
owner count in `1..=64` works — including non-powers-of-two. Placement is fixed
for the process lifetime and seeded per process.

## Atomicity

A single-owner command on the local owner executes directly. A command that
touches several owners takes whole-owner reservations in increasing owner order,
reads or applies on every participant while fenced, and releases afterwards.
That covers `MSET`/`MGET`, `RENAME`, `COPY`, duplicate-key ordering, `MULTI`/
`EXEC` with `WATCH` validation, and blocking pops across owners. `EXEC` runtime
errors still do not roll back earlier commands, exactly as in the single-owner
path. A cancelled reservation releases its fences; a partial apply is fail-stop
rather than continued partial service.

## Budget and observability

Payload accounting stays process-wide: owners publish their contribution into a
shared credit total, admission uses the shared limit, and `INFO`/`MEMORY`/`STATS`/
`DBSIZE` read one coherent cross-owner aggregate for the duration of the command.

## Measured: the remote hop is the scaling limit

A remote command costs one blocking mailbox round trip on the socket thread.
Measured on the development container at 8 clients, pipeline 16, independent
keys, four allowed CPUs (`rillcache-bench`, release build):

| Owners | SET ops/s | GET ops/s |
| ---: | ---: | ---: |
| 1 (local only) | 51,835 | 97,840 |
| 2 | 1,912 | 2,137 |
| 4 | 1,344 | 2,014 |

An in-process measurement attributes this to wakeup cost rather than router
logic: a bare Tokio unbounded-channel plus oneshot round trip between two
threads on the same container costs about 206 us/op, and one router hop costs
about 368 us/op. This is a heavily throttled shared host (single-owner runs also
measure roughly an order of magnitude below CI), so the absolute numbers are not
a performance claim. The ratio is still structural: the socket thread blocks for
a full cross-thread round trip per remote command, while the single-owner path
never leaves its thread.

Consequences, in order of expected value:

1. Keep `--storage-owners 1` as the default; it is the fastest path for every
   workload the protected gates measure and for typical mixed traffic.
2. The next real improvement is removing the block, not tuning it: dispatch
   remote work without occupying the socket thread (deferred replies on the
   existing reactor, or the opt-in Tokio transport awaiting the owner reply),
   and batch same-owner commands from one buffered pipeline into a single hop so
   a 16-deep pipeline pays one round trip rather than sixteen.

## Limits

- No live resharding, migration or rebalancing; the layout is fixed at startup.
- No cross-owner parallelism inside one atomic command: participants are fenced,
  so a wide multi-key command serializes its participants rather than overlapping
  them.
- Persistence keeps one global journal order; the append-only path is not
  parallelised across owners.
- `--io-mode tokio` and multi-owner mode are independent opt-ins; neither is the
  default, and the protected gates measure the default mode.
- This is not a clustering or replication feature, and it makes no universal
  performance claim.
