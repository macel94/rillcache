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
| Authentication and authorization | Partial implementation (S27–S28): single-password mode plus Redis-style multi-user ACL files, SHA-256 password hashes, ordered command/category/command rules, declared-key patterns, live `NOPERM`/`NOAUTH` denial, `ACL SETUSER/DELUSER/WHOAMI/USERS/LIST/GETUSER/CAT/SAVE/LOAD`, atomic fsynced ACL persistence and non-secret diagnostics. All-mode wire tests cover restrictions, revocation, file round-trips and startup failures. Still required: selectors, subcommand categories, channel enforcement (needs pub/sub), persistent/centralized audit and rate limiting, independent security review and a transport-protection boundary. ACL rules over plaintext TCP are not a complete security design. |
| Transport encryption | Authenticated TLS, certificate/hostname validation, rotation, failure/interop tests. **Rust std exposes TCP, not TLS.** No OpenSSL FFI/dependency exception and no improvised cryptography. An externally operated TLS terminator is a possible explicitly documented deployment boundary, not equivalent native Redis TLS support. Native TLS needs an explicit resolution of the current dependency constraint before implementation. |
| Durability | Partial implementation (S29): opt-in RESP-command append-only journal, mutation-only final-state records with absolute TTLs, `always`/`everysec`/`no` fsync policies, startup replay, partial-tail truncation, visible mid-file corruption failure, joined-shutdown sync, and all-mode restart tests across strings/collections/streams/groups. Still required: RDB import/export, background/incremental and automatic AOF rewrite, checksums/encryption, multi-process ownership, stream pending/consumer durability, disk-full/ENOSPC injection, backup/restore drills, crash/soak testing, point-in-time recovery and signal-driven drain. The journal is not byte-compatible multipart AOF or an RDB claim. |
| Data types and command surface | Implemented S37/S39/S40/S42/S44/S45: binary-safe lists, sets, hashes and sorted sets with mutation, removal, set algebra/store commands, score ranges/pops and WRONGTYPE/TTL/OOM semantics; bitmap access/count/search/operations and arbitrary-width signed/unsigned bitfields (`SETBIT`/`GETBIT`/`BITCOUNT`/`BITPOS`/`BITOP`/`BITFIELD`/`BITFIELD_RO`); geospatial sorted sets (`GEOADD`/`GEOPOS`/`GEODIST`/`GEOHASH`/`GEOSEARCH`/`GEOSEARCHSTORE`) with linear-scan radius/box searches; bounded cursor iteration; blocking `BLPOP`/`BRPOP` (S35); PFADD/PFCOUNT/PFMERGE approximate cardinality; keyspace/keyevent notifications (S40); and a bounded stream subset. Channel/pattern pub/sub (S31), connection RESET (S33) and process statistics (S34) are implemented separately. Still required: blocking stream commands, legacy `GEORADIUS*` aliases, expired/evicted notification classes and durable notification delivery, sharded pub/sub, broader stream consumer-group operations and explicit unsupported module/Redis-8 extension behavior. |
| Transactions and programmability | Partial implementation (S26): bounded `MULTI`/`EXEC`/`DISCARD`/`WATCH`/`UNWATCH` over supported data commands, store-gate batch isolation, expiry-aware optimistic invalidation and RESP2/RESP3 replies. Still required: scripts/functions with failure/resource semantics, connection-local commands at EXEC time, and documented absence of rollback, replication and durable commit. A sequence of individually atomic commands is not a transaction. |
| Availability and topology | Replication/PSYNC, failover/Sentinel, Redis Cluster slots/redirections/migration, partition/rejoin/recovery tests and mixed-version interoperability. Multiple independent servers do not imply replication or clustering. |
| Container distribution | S14/S15: scratch/non-root static images, shell-free PING probe, native amd64/arm64 mode tests, separate GHCR publisher and anonymous-access gate, restricted Docker/Kubernetes examples. See [delivery and first-package visibility](containers.md). Distribution alone closes no authentication, durability, topology or production-validation gate. |
| Resource/operational safety | Partial: bounded payload/client/request limits, a bounded client registry with CLIENT LIST/KILL and INFO clients, joined shutdown, persistence-aware final sync and honest `INFO stats` counters for connections/commands/rejections/expiration/pub-sub with `CONFIG RESETSTAT`. Payload-budget eviction policies now cover noeviction/allkeys/volatile random/LRU/volatile-TTL with honest mutation-LRU and stats; still required: true RSS/allocator accounting, LFU, background eviction and notification events (payload is not RSS), slow-client limits, portable graceful termination, configuration compatibility/reload, per-command/latency/client-list observability, upgrade/rollback and long-running soak tests. |
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
