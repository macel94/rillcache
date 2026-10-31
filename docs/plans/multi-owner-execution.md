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
