# Production compatibility profile (draft)

**Status: DRAFT — the project is NOT production ready.**

This document defines the only deployment shape a future production claim may
cover. It is an input to [C1](production-readiness-gaps.md#1-compatibility-and-command-surface),
not a readiness claim, and it does not close any P0 item. A row becomes supported
only when the listed verification exists on the tested head and the remaining
P0 gates are closed or explicitly excluded by a reviewed profile decision.

## Profile identity

| Field | Value |
| --- | --- |
| Profile | `rillcache-standalone-0.1-draft` |
| Application version | `0.1.0` (unreleased) |
| Reference servers | Redis `8.10.2`, Valkey `9.1.2` (pinned in `benchmarks/targets.mk`) |
| Topology | One standalone process, one owner-local data path |
| Logical databases | Database `0` only |
| Persistence mode | Optional private append-only journal; RDB excluded |
| Transport | Plain TCP, optional native TLS 1.3 |
| Platforms | Linux x86_64 (development/CI); other platforms unverified |

## Supported surface

- The command names returned by `COMMAND LIST` on the tested head. The
  [compatibility guide](compatibility.md) describes command-family behavior and
  the [area references](README.md) carry option-level limits. This profile does
  not silently add commands that `COMMAND LIST` does not report.
- Strings, bitmaps, lists, sets, hashes, sorted sets, HyperLogLog (private
  representation), geospatial commands over sorted sets, and streams as described
  in [collections](collections.md), [string algorithms](string-algorithms.md),
  [HyperLogLog](hyperloglog.md), [geospatial](geospatial.md) and
  [streams](streams.md).
- `MULTI`/`EXEC`/`DISCARD`/`WATCH`/`UNWATCH` with the documented standalone
  contract in [transactions](transactions.md).
- Pub/Sub and keyspace notifications with the bounded, at-most-once,
  process-local delivery described in [pub/sub](pubsub.md) and
  [notifications](notifications.md).
- Password or ACL-file authentication, the bounded command/key/channel ACL
  model, optional audit logging, and TLS 1.3 as described in
  [security](security.md) and [TLS](tls.md).
- A private append-only journal with `always`/`everysec`/`no` fsync policies,
  cooperative rewrite, bounded startup replay, optional CRC32/HMAC framing, and
  the offline `rillcache-backup` archive described in [persistence](persistence.md).
- Bounded startup files through `--config` (where available on the tested head).

## Explicit exclusions

The following are **not** part of this draft profile and must not be inferred from
a command returning a generic error:

- Scripting, `EVAL`/`EVALSHA`, functions and `FCALL` (C2).
- Loadable modules (C3); `MODULE LIST` is empty and loading is unsupported.
- Multiple logical databases, `SELECT` other than `0`, `MOVE`, `SWAPDB` (C4).
- RDB, `SAVE`/`BGSAVE`, `DUMP`/`RESTORE`/`MIGRATE`, online backup and PITR
  (C5, D1, D5).
- Replication, Sentinel, cluster routing, failover and multi-node topologies
  (A1–A5).
- `CLIENT TRACKING`/client-side caching, `OBJECT IDLETIME`/`FREQ`, full RESP3
  attributes/push tracking and `LATENCY GRAPH|HISTOGRAM` (C5, C6, O2, O3).
- Redis-compatible HyperLogLog payload interchange and diagnostics (C10).
- A full `redis.conf` language, `CONFIG REWRITE`, includes and live reload (C13,
  O5).
- `redis-py`, node-redis, go-redis and other language-client conformance (C7).
- Any durability RPO/RTO, independent security review, true RSS enforcement,
  cross-process observability or production SLO (D1–D11, S1–S9, O1–O9).

## Persistence contract

The private journal is **best-effort**, not an interoperable AOF. With
`appendfsync always`, an acknowledged write cannot precede the durable append;
with `everysec`, crash recovery yields a contiguous prefix and a bounded loss
window; with `no`, the operating system decides. A torn final transaction is
discarded to the last verified boundary. RDB, online backup, point-in-time
recovery, encryption at rest and multi-process journal ownership are excluded.
Operators must use local storage and the documented backup/restore tool; shared
or network filesystems are unsupported.

## Security contract

Authentication and ACL files, TLS 1.3, audit logs and the existing denial
throttles are implemented, but the profile remains **partial**: there is no
independent security review, no full Redis ACL surface, no global/cross-process
rate limiting, no tenant isolation, no live session revocation beyond the
existing connection controls, and no audited collector federation. Secrets are
operator-managed files or arguments; `requirepass` on the command line is
visible in process arguments.

## Client verification matrix

| Client | Status |
| --- | --- |
| Pinned `redis-cli` RESP2/RESP3/AUTH/metadata | Tested in CI |
| `valkey-benchmark` official generator | Used by the performance gates |
| `redis-py` / node-redis / go-redis handshake, AUTH/ACL, pipeline, pub/sub, TLS, reconnect | **Unverified** — excluded from this profile |
| Arbitrary clients issuing unsupported discovery commands | Not covered |

## Limits

The default limits are `max_clients=128`, 2 MiB request frames, 1 MiB bulk
arguments, a 64 MiB key/value payload budget, 512 MiB strings and one million
elements per collection. These are logical/admission limits, not an RSS cap or
an allocator bound. The documented platform/CI limits and their scopes are in
[getting started](getting-started.md), [collections](collections.md) and
[observability](observability.md).

## Verification required for a production row

A profile row is supported only when all of the following exist for the exact
tested head:

1. The differential/upstream command evidence described in
   [reference conformance](reference-conformance.md) and the C1 matrix.
2. The all-mode correctness matrix and both protected performance gates.
3. The deployment profile's TLS/ACL/audit checks and an independent security
   review.
4. The profile's durability, backup/restore and failure drills.
5. A supported language-client matrix and a declared support window.

Until then, the project remains **NOT PRODUCTION READY**. See the canonical
[production readiness gaps](production-readiness-gaps.md) for every open item.
