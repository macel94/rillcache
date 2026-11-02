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

## Mailbox progress under control traffic

CONTROL keeps independent capacity and burst priority, but on an unfenced owner
at most eight consecutive CONTROL deliveries may precede ready admitted DATA.
One DATA delivery restores CONTROL priority immediately; lane FIFO and admission
credits are unchanged. An empty DATA lane never holds back CONTROL. Idle or
cancelled receives do not restart the burst. While a reservation fence is held,
DATA remains disabled regardless of the burst count, so release/abort/stop can
still make progress. The bound counts mailbox deliveries, not elapsed time or
OS scheduling; it is a starvation fix, not a throughput or latency SLO claim.

## Deferred owner batches

The original multi-owner socket adapter blocked on one mailbox round trip per
remote command. The readiness reactor now keeps each connection's requested
commands as slots and submits one ordered batch to every participating owner:
same-owner commands in a parsed pipeline share a mailbox message, and replies
are emitted strictly in request order. A connection has at most one active run,
so backpressure, command boundaries and the existing command/byte quanta are
unchanged.

A refused or full admission is retryable and retains the original commands; it
never partially applies a batch. Cross-owner atomic commands, keyless shared
commands and blocking/pub-sub/session commands act as barriers and keep the
synchronous reservation path. A run's deferred replies use a **separate** bounded
reply budget from synchronous control/data replies, so maintenance and a
cross-owner reservation cannot wait behind batches the reactor has not drained.
Slow commands are recorded from the original shared argument storage when their
reply is emitted, so SLOWLOG keeps working without disabling batching; latency
monitor and commandstats keep the synchronous path.

## Measured: before and after deferred batches

The original evidence, measured on the development container at 8 clients,
pipeline 16, independent keys and four allowed CPUs (`rillcache-bench`, release
build), showed the socket thread blocking on each remote hop:

| Owners | SET ops/s | GET ops/s |
| ---: | ---: | ---: |
| 1 (local only) | 51,835 | 97,840 |
| 2 | 1,912 | 2,137 |
| 4 | 1,344 | 2,014 |

An in-process measurement attributed that cost to wakeup latency rather than
router logic: a bare Tokio unbounded-channel plus oneshot round trip between two
threads on the same container cost about 206 us/op, and one router hop about
368 us/op.

After deferred batches, on the same throttled container but a later, noisier
sample (8 clients, pipeline 16, 60k requests, two alternating repetitions,
advisory only):

| Owners | Baseline SET | Deferred SET | Baseline GET | Deferred GET |
| ---: | ---: | ---: | ---: | ---: |
| 1 (local only) | 183,645 | 241,287 | 353,755 | 372,189 |
| 2 | 3,482 | 54,874 | 3,010 | 36,540 |
| 4 | 3,479 | 44,924 | 3,459 | 38,121 |

Deferred batching removes most of the blocking penalty, but on this host it still
does not beat the zero-hop one-owner path at pipeline 16. A bare cross-thread
round trip and thread wake cost tens to hundreds of microseconds here, while the
one-owner path executes the whole pipeline locally. Absolute rates and the
two-owner/four-owner ordering move substantially with host load, so these are
direction-only numbers; no CI or gate claim is made from them. `--storage-owners
1` therefore remains the default and the protected gates still measure that
default path.

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
