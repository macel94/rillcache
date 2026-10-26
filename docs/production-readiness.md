# Production Redis compatibility: delivery and acceptance plan

Production drop-in compatibility is the goal, **not the current release status**.
A command subset and passing microbenchmarks are insufficient to make that claim.
This plan keeps the README's missing capabilities visible until independently
verified. No unsafe code, FFI, custom allocator or external Rust dependency is
permitted, including in tests. Optional external interoperability tools remain
outside the application dependency graph.

## Required compatibility and operational gates

| Capability | Current boundary / evidence required before closing |
| --- | --- |
| Multithreaded execution | Managed per-client workers plus opt-in fixed 1..64 I/O/execution lanes; test socket behavior and benchmarks in per-client/1/4-worker modes, actual worker counts, idle clients, backpressure, fairness, limits and joined shutdown. No linear-scaling claim. |
| String expiration and SET options | Implemented S12: injectable clock; EX/PX/EXAT/PXAT, GET/KEEPTTL, TTL/PTTL and expiration commands; option/error/overflow semantics, atomic conditional writes and bounded expiry metadata/reclamation, with deterministic and wire/reference tests. Newer digest-based SET conditions require a separate compatibility decision/test corpus. |
| Integer/string command coverage | Implemented S13: INCR/DECR/INCRBY/DECRBY, canonical signed-64-bit parsing, overflow rejection, TTL retention and concurrent unit/socket tests. Implemented S25: MGET/MSET/MSETNX with atomic payload admission, duplicate-key semantics, expiry behavior and a 2 MiB bounded MGET reply; other string/bit commands need their own wire corpus. |
| Client protocol and discovery | Partial implementation: HELLO 2/3, bounded CLIENT ID/name/library metadata, compact CLIENT INFO, SELECT 0, frame-aware RESP3 null conversion, all-mode ownership tests, and bounded COMMAND/INFO discovery for implemented commands. Still required: real pinned redis-cli/Python/Node/Go client handshake/interoperability tests and any separately justified richer protocol/database support. No forged Redis version or successful no-op for unsupported behavior. |
| Authentication and authorization | Partial implementation (S27): one operator-configured `default` password via flag/file, constant-time SHA-256 verification, `NOAUTH` rejection before data/admin work, `AUTH`/`HELLO AUTH`, authenticated runtime rotation and non-secret `CONFIG GET`; all-mode wire tests and adversarial error/atomicity tests. Still required: multi-user ACLs, command/key/channel permissions, brute-force controls, audit logging, live deauthentication, independent security review and a transport-protection boundary. A single password over plaintext TCP is not a complete security design. |
| Transport encryption | Authenticated TLS, certificate/hostname validation, rotation, failure/interop tests. **Rust std exposes TCP, not TLS.** No OpenSSL FFI/dependency exception and no improvised cryptography. An externally operated TLS terminator is a possible explicitly documented deployment boundary, not equivalent native Redis TLS support. Native TLS needs an explicit resolution of the current dependency constraint before implementation. |
| Durability | Redis-compatible RDB/AOF behavior and import/export, fsync policy, crash/replay/corruption handling, atomic replacement, disk-full/I/O error tests, backup/restore drills and documented acknowledgment guarantees. In-memory data is currently lost on exit. |
| Data types and command surface | Hashes, lists/blocking commands, sets, sorted sets, bitmaps, HyperLogLog, geospatial, streams/consumer groups and pub/sub/patterns; WRONGTYPE, ordering, numeric and binary semantics; publish supported commands and unsupported module/Redis-8 extensions explicitly. |
| Transactions and programmability | Partial implementation (S26): bounded `MULTI`/`EXEC`/`DISCARD`/`WATCH`/`UNWATCH` over supported data commands, store-gate batch isolation, expiry-aware optimistic invalidation and RESP2/RESP3 replies. Still required: scripts/functions with failure/resource semantics, connection-local commands at EXEC time, and documented absence of rollback, replication and durable commit. A sequence of individually atomic commands is not a transaction. |
| Availability and topology | Replication/PSYNC, failover/Sentinel, Redis Cluster slots/redirections/migration, partition/rejoin/recovery tests and mixed-version interoperability. Multiple independent servers do not imply replication or clustering. |
| Container distribution | S14/S15: scratch/non-root static images, shell-free PING probe, native amd64/arm64 mode tests, separate GHCR publisher and anonymous-access gate, restricted Docker/Kubernetes examples. See [delivery and first-package visibility](containers.md). Distribution alone closes no authentication, durability, topology or production-validation gate. |
| Resource/operational safety | True memory accounting/eviction policies (payload budget is not RSS), slow-client limits, graceful process termination, configuration compatibility/reload, observability/INFO, persistence-aware shutdown, upgrade/rollback and long-running soak tests. |
| Production evidence | Defined deployment/client/version profile, upstream compatibility suite, crash/fault/security/soak tests, realistic controlled-hardware workload/memory/durability comparisons, documented SLOs, backups and independent human/security validation. |

## Incremental delivery order

1. Explicit worker modes, local/GitHub correctness matrix and mode benchmarks (S11).
2. Expiration/SET option semantics with deterministic clock and reference tests.
3. Atomic integer operations with shared-key contention and reference tests.
4. Client protocol/session compatibility, then security, durability, richer types,
   transactions and topology in separately testable INVEST stories with EARS
   acceptance criteria. Do not conflate scaffolding or a roadmap entry with a
   completed production gate.

Every increment must pass offline `make ci`, release tests and relevant pinned
Redis/Valkey comparisons before publication. Preserve data and protocol limits,
error atomicity, binary safety and clean shutdown in every execution mode.
Production claims remain blocked until the selected deployment profile's gates
are actually met; unresolved scope or the native-TLS constraint must be stated.

References: [Redis commands](https://redis.io/docs/latest/commands/),
[Redis TLS](https://redis.io/docs/latest/operate/oss_and_stack/management/security/encryption/),
[Rust TCP API](https://doc.rust-lang.org/std/net/struct.TcpStream.html).
