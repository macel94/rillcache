---
type: boundary
title: Compatibility and production boundaries
description: What Rillcache verifies against Redis, what it deliberately does not implement, and which evidence is still missing before any production claim.
tags: [compatibility, unsupported, production-readiness, redis, boundaries]
sources:
  - id: openwiki-source-1e8733fb319d9fc0ba9c1e2f
    resource: repo://docs/ci-benchmark-review.md
  - id: openwiki-source-491734283c0612f21b1858f0
    resource: repo://docs/compatibility.md
  - id: openwiki-source-0117ac68a72ed7ce495ae997
    resource: repo://docs/persistence.md
  - id: openwiki-source-c23acd0ff2503357d054416d
    resource: repo://docs/production-readiness-gaps.md
  - id: openwiki-source-c6698ebe740262f12ab4ce11
    resource: repo://docs/reference-conformance.md
  - id: openwiki-source-eb9d302b52d2c490966a1913
    resource: repo://docs/requirements/compatibility.md
  - id: openwiki-source-86a0414cf00af5910d8bf8e2
    resource: repo://docs/unsupported.md
  - id: openwiki-source-3989644b876c3ddd6e4b127e
    resource: repo://docs/upstream-ci.md
  - id: openwiki-source-48c84a451bc541a1bb256a33
    resource: repo://src/engine/discovery/mod.rs
  - id: openwiki-source-f9ac37fcb58515328264e06d
    resource: repo://src/engine/unsupported.rs
  - id: openwiki-source-bf6cda76234f60dd7eecaa7f
    resource: repo://src/session/mod.rs
  - id: openwiki-source-c70ddd1cc4a17418b8fc2bf1
    resource: repo://tests/discovery.rs
  - id: openwiki-source-fdf89b772c3fe634921300db
    resource: repo://tests/sessions.rs
generated: { by: "opencode", at: "2026-11-11T20:57:56.397Z" }
verified:
  - by: openwiki/0.9.0
    at: 2026-11-11T20:57:56.397Z
---

# Compatibility and production boundaries

Rillcache aims at Redis compatibility; it does not currently claim to be a
drop-in Redis replacement or a production service. This page records the boundary
so a passing command test is not mistaken for a compatibility or durability
guarantee.

## What is implemented

The command registry contains **243 commands**. The supported surface includes:

- RESP2 arrays of non-null bulk strings and Redis-style inline commands, including
  binary keys and values and ordered mixed-protocol pipelines;
- strings and counters, including `SET` conditions and expiry options, `GETEX`,
  `GETDEL`, append/range operations and 64-bit overflow checks;
- bitmaps, `BITFIELD`, `LCS`, HyperLogLog and multi-key string operations;
- lists, sets, hashes and sorted sets, including field TTLs, range algebra,
  random selection, multi-pop and blocking pops;
- geospatial sorted sets, streams with consumer groups, pending-entry inspection,
  persisted consumer/PEL state and consumer last-activity, bounded
  `SCAN`/`HSCAN`/`SSCAN`/`ZSCAN`, and `SORT`;
- `MULTI`/`EXEC`/`DISCARD`/`WATCH`/`UNWATCH` with a bounded queue;
- channel, pattern and sharded-channel pub/sub plus keyspace notifications;
- password and ACL-file authentication, ACL selectors, denial logging and native
  TLS 1.3;
- an opt-in append-only journal with restart replay, CRC32 checksums, optional
  HMAC-SHA256 authentication, cooperative compaction and offline backup/restore;
- payload-budget eviction and a documented observation surface.

The complete command-level detail lives in
[data types and command families](../features/data-types-and-commands.md) and the
repository's area guides. This page does not reproduce every option.

## What is deliberately absent

Several capabilities return explicit unsupported replies rather than pretending
to work:

- `CLUSTER` is disabled and never returns `MOVED`/`ASK` redirects.
- `MODULE LIST` is empty; `MODULE LOAD`, `LOADEX` and `UNLOAD` are rejected.
- `SAVE` and `BGSAVE` report RDB unsupported; there is no RDB import/export.
- Replication commands report a standalone server; there are no replicas.
- Only database 0 exists; `SELECT` rejects nonzero indices.
- Lua `EVAL`, script execution and Redis Functions are absent.
- RESP3 is limited to the simple strings, errors, integers, bulk strings, arrays
  and nulls emitted by the current command set. Pushes, attributes, tracking and
  map/set/boolean data replies are not a complete RESP3 implementation.
- RDB/AOF interchange is not supported: the journal is a private RESP command
  stream, not Redis multipart AOF.

These are capability boundaries, not transient bugs. A client that needs cluster
routing, replicas, modules, scripting or multiple databases cannot use Rillcache
as a substitute today.

## What the tests actually prove

There are three different kinds of evidence, and they prove different things:

1. **Rust tests** exercise protocol framing, command semantics, expiry, accounting,
   transactions, persistence, TLS and all three worker modes. They are strong
   regression evidence for implemented behavior.
2. **A dependency-free reference corpus** (`tests/reference_baseline.rs`) checks a
   focused RESP2 subset against Rillcache and, optionally, a disposable Redis or
   Valkey instance. It covers binary safety, null versus empty values, duplicate
   keys, atomic failure, overflow, fragmentation and pipelining. It is explicitly
   not a full compatibility suite.
3. **Three original upstream Tcl files** — `unit/type/string`,
   `unit/type/incr` and `unit/keyspace` — run against pinned Redis 8.10.2 and
   Valkey 9.1.2 in the upstream workflow. The native control runs first and the
   candidate must pass the same unchanged file. The scope uses `--singledb`,
   ignores encoding/digest checks and excludes `needs:debug`, `needs:repl` and
   `slow` tests. Other upstream units are not run.

The repository therefore has real differential evidence for a narrow slice, not a
command-by-command matrix for the whole registry. Python, Node.js and Go client
libraries are not in the required CI matrix. A pinned `redis-cli` interoperability
test exists, but arbitrary clients can still issue discovery or session commands
that Rillcache does not implement.

## Performance evidence is not compatibility evidence

The protected Valkey gates require all 20 original scoped medians and all 10
official four-CPU medians to beat Valkey on the exact PR head. Those gates are
performance acceptance, not proof of feature equivalence, realistic mixed-traffic
behaviour, tail latency, memory efficiency or production stability.

Published studies preserve regressions as well as wins. The four-CPU work moved
from failed cohorts to passing ones through source changes; it did not relax the
gate, omit losing rows or reuse historical passes. Treat a green performance run
as evidence only for the workloads and hardware it recorded.

## Durability is best-effort

The append-only journal supports `always`, `everysec` and `no`, startup replay,
partial-tail truncation, visible mid-file corruption and cooperative
`BGREWRITEAOF`. CRC32 checksums detect accidental transaction corruption, and an
optional HMAC-SHA256 chain authenticates transaction content and order; neither
provides encryption at rest or anti-rollback protection. It is still not a durable
system of record:

- no RDB import/export or interoperable multipart AOF, and no encryption at rest;
- no fork-based or copy-on-write rewrite; `BGREWRITEAOF` materialises the key list
  and advances in bounded slices between commands;
- no crash, ENOSPC, power-loss or disk-fault injection corpus;
- no online backup, point-in-time recovery or retention policy; offline quiescent
  backup, verification and restore exist;
- no portable SIGTERM/SIGINT drain in the standalone binary;
- no shared-filesystem writer coordination; journal ownership is a local advisory
  lock;
- stream consumer last-activity is restored when the keyspace tracks wall-clock
  time (it has an expiry or field TTL); without tracked time the clock shortcut
  stores zero, and history for deleted or renamed consumers is not retained.
  Consumer objects and PEL owners, delivery counts and timestamps persist,
  including deleted-id tombstones.

An `always` success is the strongest current acknowledgement guarantee. An
`everysec` crash can lose roughly the last second of writes, and `no` relies on the
operating system or an orderly shutdown. The full contract is in
[persistence and recovery](../storage/persistence-and-recovery.md).

## Other production blockers

The repository also lacks or only partially provides:

- a published, versioned compatibility profile and real client-library matrix;
- replication, failover, Sentinel, cluster or an explicit single-node availability
  contract;
- an independent security review, full TLS hardening (SNI, hot reload, OCSP) and
  global/cross-process rate limiting;
- RSS/allocator accounting, process-wide network memory limits and a measured
  overload contract;
- realistic mixed-workload, tail-latency, persistence and failure-mode SLOs on
  controlled hardware;
- configuration-file compatibility, `CONFIG REWRITE` and the complete runtime
  configuration surface;
- image signing, SBOM, a stable support policy and a migration policy.

Multi-owner execution is opt-in and currently slower than the default one-owner
path in its own measurements. It is a correctness and scaling seam, not evidence
that the default server scales linearly.

## How to use this boundary

When adding a command or feature:

1. implement the behavioral surface, including invalid, empty, binary and
   concurrent cases;
2. add deterministic tests and, where relevant, wire/reference coverage;
3. update the area requirement and compatibility boundary;
4. run the focused checks and then the complete offline suite;
5. do not convert a plan, stub or single local run into a production claim.

For the contributor workflow, see
[testing and delivery](../development/testing-and-delivery.md). For the journal
and recovery details, see
[persistence and recovery](../storage/persistence-and-recovery.md).
