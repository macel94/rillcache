# Production Redis compatibility: delivery and acceptance plan

Production drop-in compatibility is the goal, **not the current release status**.
A command subset and passing microbenchmarks are insufficient to make that claim.
This plan keeps the README's missing capabilities visible until independently
verified. No unsafe code, FFI, custom allocator or arbitrary external Rust
dependency is permitted, including in tests. The sole exception is rustls and the
transitive crates required to build it, vendored for offline builds. Optional
external interoperability tools remain outside the application dependency graph.

## Current phase scope: durability deferred

Durability completeness is **explicitly deferred** for this phase. The existing
append-only journal, fsync policies, restart replay, compaction and `SHUTDOWN`
flush remain implemented and covered, but they are best-effort. There is no RDB,
background/incremental rewrite, journal checksum or encryption, ENOSPC/crash/soak
validation, backup/restore drill, point-in-time recovery or signal-driven drain.
Do not treat this server as a durable system of record or as fulfilling the
durability gates below. This is a scope decision, not a production-readiness
claim.

## Required compatibility and operational gates

Each capability now lives in its own area file so a feature PR edits one gate:

| Capability | Area file |
| --- | --- |
| Multithreaded execution | [multithreaded-execution.md](production-readiness/multithreaded-execution.md) |
| String expiration and SET options; integer/string command coverage | [string-and-integer-coverage.md](production-readiness/string-and-integer-coverage.md) |
| Client protocol and discovery | [client-protocol-and-discovery.md](production-readiness/client-protocol-and-discovery.md) |
| Authentication and authorization | [authentication-and-authorization.md](production-readiness/authentication-and-authorization.md) |
| Transport encryption | [transport-encryption.md](production-readiness/transport-encryption.md) |
| Durability | [durability.md](production-readiness/durability.md) |
| Data types and command surface | [collections](production-readiness/data-types-collections.md), [streams](production-readiness/data-types-streams.md), [observability/operations](production-readiness/data-types-observability.md) |
| Transactions and programmability | [transactions-and-programmability.md](production-readiness/transactions-and-programmability.md) |
| Availability and topology | [availability-and-topology.md](production-readiness/availability-and-topology.md) |
| Container distribution | [container-distribution.md](production-readiness/container-distribution.md) |
| Resource/operational safety | [resource-and-operational-safety.md](production-readiness/resource-and-operational-safety.md) |
| Production evidence | [production-evidence.md](production-readiness/production-evidence.md) |

## Incremental delivery order

1. Explicit worker modes, local/GitHub correctness matrix and mode benchmarks (S11).
2. Expiration/SET option semantics with deterministic clock and reference tests.
3. Atomic integer operations with shared-key contention and reference tests.
4. Client protocol/session compatibility, then security, richer types,
   transactions and topology in separately testable INVEST stories with EARS
   acceptance criteria. Durability completeness is deferred for this phase and is
   not an implied near-term increment. Do not conflate scaffolding or a roadmap entry with a
   completed production gate.

Every increment must pass offline `make ci`, release tests and relevant pinned
Redis/Valkey comparisons before publication. Preserve data and protocol limits,
error atomicity, binary safety and clean shutdown in every execution mode.
Production claims remain blocked until the selected deployment profile's gates
are actually met; unresolved scope, the durability deferral and the
TLS-hardening constraint must be stated.

References: [Redis commands](https://redis.io/docs/latest/commands/),
[Redis TLS](https://redis.io/docs/latest/operate/oss_and_stack/management/security/encryption/),
[Rust TCP API](https://doc.rust-lang.org/std/net/struct.TcpStream.html).
