# Requirements and incremental delivery

The README is the product charter. This document turns it into small, testable
increments; it is not a claim of full Redis compatibility.

## Non-negotiable constraints

- **Safe Rust only:** every project target forbids `unsafe_code`. No FFI, raw
  pointer manipulation, custom allocator, native extension or build script.
  Safe standard-library APIs are allowed; the standard library/runtime/OS have
  internals outside this project's code and are not claimed to contain no unsafe.
- **One external Rust dependency exception:** rustls and the transitive crates
  required to build it, vendored for offline builds. No other build, development,
  test or benchmark dependency is permitted. The dependency-policy test inspects
  all targets/features and rejects anything outside the rustls allowlist.
- All project code is AI-generated. Human review, product judgment and test-based
  validation remain central. Automation does not replace that validation.
- Compatibility is incremental, byte-oriented and demonstrated by tests.
- Rust alone is not evidence of speed. Performance claims require comparable,
  repeated end-to-end measurements against current Redis and Valkey releases.
- All future main commits must come from PRs satisfying the [mandatory test and
  Valkey gate](pr-quality-gate.md), including docs/reports. No direct/admin/bot bypass.

## Definition of done

Definition of done for each future story: EARS acceptance cases and limits agreed,
unit and wire tests added, dependency/safety policy intact, `make ci` passing,
relevant benchmark captured, compatibility documentation updated, human validation.

## INVEST stories: implemented increments

Each story provides a separately testable outcome. Estimates are relative scope
(XS/S), not delivery promises. Internal design remains negotiable; the constraints
above do not. Shared RESP plumbing is an explicit technical prerequisite, not a
reason to couple all future commands into a single large story.

Each story table is split by area so independent feature increments edit only their own file:

- [Core transport, strings and runtime stories](requirements/stories-core.md) — protocol, strings, expiry, counters, transactions, workers and storage/dispatch hot paths.
- [Collection and keyspace stories](requirements/stories-collections.md) — bitmaps, lists, sets, hashes, sorted sets, geospatial, HyperLogLog, sorting and keyspace deletion.
- [Stream stories](requirements/stories-streams.md) — append/retention, groups, pending claims, ranges, blocking reads and metadata.
- [Security and persistence stories](requirements/stories-security-persistence.md) — authentication, ACLs, the journal, compaction, persistence status and TLS.
- [Observability, delivery and performance stories](requirements/stories-observability-delivery.md) — INFO/statistics, client administration, pub/sub, notifications, benchmark evidence, containers and PR delivery.

## EARS acceptance requirements

Each EARS section lives in the area file below; requirement IDs remain unique across the split:

- [Policy and transport](requirements/policy-and-transport.md) — protocol, worker modes, connection lifecycle, limits, shutdown and client administration.
- [Data and strings](requirements/data-and-strings.md) — strings, expiry, integer counters, transactions, bitmaps and bitfields.
- [Collections](requirements/collections.md) — lists, sets, hashes, sorted sets, geospatial, HyperLogLog and sorting.
- [Streams](requirements/streams.md) — stream retention, groups, pending entries, ranges and blocking reads.
- [Security](requirements/security.md) — password authentication, ACLs and the TLS 1.3 sidecar.
- [Persistence](requirements/persistence.md) — journal durability, compaction, fsync policy, persistence status and RDB/replication boundaries.
- [Observability](requirements/observability.md) — statistics, discovery, slow log, latency, MONITOR, pub/sub and notifications.
- [Delivery and performance](requirements/delivery.md) — CI, containers, benchmark evidence, scheduling experiments and the PR merge gate.
- [Compatibility boundary](requirements/compatibility.md) — the supported v0.1 surface and documented gaps.
- [Next independently deliverable stories](requirements/next-stories.md) — remaining work and its definition of done.

## Sources

- [INVEST](https://agilealliance.org/glossary/invest/)
- [EARS patterns](https://alistairmavin.com/ears/)
- [RESP specification](https://redis.io/docs/latest/develop/reference/protocol-spec/)
- [Redis command reference](https://redis.io/docs/latest/commands/)
- [Safe/unsafe boundary](https://doc.rust-lang.org/nomicon/safe-unsafe-meaning.html)
- [Cargo lint policy](https://doc.rust-lang.org/cargo/reference/manifest.html#the-lints-section)
