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
| Clients requiring several databases | Excluded; startup with `db` other than `0`, `SELECT` other than `0`, `MOVE`, `SWAPDB` and `COPY ... DB n` are rejected |
| Persistence mode | Optional private append-only journal; RDB excluded |
| Transport | Plain TCP, optional native TLS 1.3 |
| Deployment-wide rate limiting | Not implemented in the server; process-local budgets only. Multi-process or replicated deployments require an authoritative external control plane |
| Reliable delivery | Not available in Pub/Sub; stream consumer groups provide acknowledged redelivery across client disconnects, and an external durable transport is required beyond the process |
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

The [original upstream Tcl hash diagnostic](upstream-ci.md#compatibility-contract-and-exclusions)
compares the pinned native Redis and Valkey controls with Rillcache. Its result
is evidence for the selected files and revision, not approval of the hash row.

## Audit custody boundary and compensating controls

The audit path is **best-effort** and is not a custody chain: denial lines are
emitted to an optional local file and optional UDP syslog sink, with no sender
authentication, transport encryption, federation, delivery guarantee,
backpressure, retry or aggregation. The collector never blocks command
execution; a closed or unread sink is silently best-effort, and the standalone
collector exits nonzero with a redacted count summary when its output fails.
Per-line HMAC and a bounded keyring (first key signs, all verify) provide
tamper evidence for file lines, not confidentiality or delivery.

Because the server cannot provide custody, deployments that need audit evidence
must supply the compensating controls below; without them the audit path is
diagnostic only:

- ship to a collector bound to loopback or a private interface, then forward
  from the collector to independently administered, append-only storage with
  its own retention and access control;
- protect the HMAC keyring as a secret, keep at least the previous key during
  rotation so old lines still verify, and restrict the audit file with
  `--secure-secret-files` where the platform supports it;
- alert on the collector's nonzero exit and redacted count summary, and on
  missing expected traffic, since absence of lines is not evidence of absence;
- treat rillcache audit records as secondary evidence and rely on durable
  primary sources (the journal with its fsync policy and external access logs)
  for incident custody.

`tests/audit_syslog.rs` proves the boundary: with an unread sink and after the
sink is dropped, 128 denied commands plus allowed commands still complete
normally. Signing, keyring verification and collector failure reporting are
covered by `tests/audit_hmac.rs`, `tests/audit_keyring.rs` and
`tests/audit_collector.rs`.

## Deployment-wide rate limiting

The server enforces authentication budgets **inside one process only**:
`--auth-failure-limit` bounds consecutive failures per connection (default 10),
and the opt-in `--auth-source-limit` / `--auth-global-limit` add per-peer-IP and
process-wide lockouts over `--auth-source-window-ms` (default 60 s). These
counters are never shared with another server process, so they do not compose
across replicas, rolling deployments or a shared NAT address; one process
locking a source out leaves every other process unaffected.

The supported topology is therefore a single owner process, and any deployment
that exposes more than one process must put an authoritative rate limiter in
front of it. The required external behaviour is:

- admission control runs **before** AUTH and applies to connection and
  credential attempts, so a locked client never reaches the server;
- it fails **closed** when the limiter is unavailable, and its refusals are
  visible to the client as a connection or command rejection rather than a
  silent success;
- it is configured from the same source of truth for every replica, since the
  server's own budgets remain defence in depth and cannot see peer traffic.

`tests/auth_process_isolation.rs` proves the isolation that makes this
requirement necessary: two real server processes with identical limits do not
share lockout state.

## Migration and interchange

There is **no RDB or native multipart-AOF interchange**: `SAVE`/`BGSAVE` return
an RDB-unsupported error, `LASTSAVE` reports `0`, and `DUMP`/`RESTORE`/`MIGRATE`
return one explicit capability error that names the supported alternatives. Do
not point a Redis/Valkey `DUMP` payload or an RDB file at this profile.

The supported migration path is a bounded **offline** conversion of an exported
DB 0 string dataset, followed by a verified checkpoint before first start:

1. export one RESP `SET key value` (or `SET key value PXAT unix-ms`) record per
   value, with absolute expirations and binary bulk strings;
2. run `scripts/import_string_journal.py snapshot.resp /isolated/appendonly.aof`
   on a stopped, empty, trusted staging directory (limits: 16 MiB total, 2 MiB
   per frame, 1 MiB per bulk argument; only `SET` records; an existing journal is
   never overwritten and a rejected record publishes nothing);
3. run `rillcache-backup create --dir ... --output ...` and
   `rillcache-backup verify --input ...`, then start the server on the verified
   journal and compare strings, expirations and counts with the source.

`tests/migration_path.rs` executes that documented sequence with the real tools
and boots the resulting journal, asserting binary values, absolute expiry and
key count, plus fail-closed rejection of a non-`SET` record and an occupied
target. Live cutover from a running source, native AOF/RDB conversion, non-string
types and other databases are not supported.

## Explicit exclusions

The following are **not** part of this draft profile and must not be inferred from
a command returning a generic error:

- Scripting, `EVAL`/`EVALSHA` (including the `_RO` variants), functions and
  `FCALL`/`FCALL_RO` (C2). No Lua runtime is linked. The exclusion is explicit
  rather than silent: absent execution commands return the unknown-command error,
  every unsupported `SCRIPT`/`FUNCTION` subcommand returns a deliberate
  unsupported-subcommand error, a missing subcommand returns the arity error, and
  only `SCRIPT FLUSH`/`FUNCTION FLUSH` acknowledge an empty registry. Clients that
  require script or function execution are incompatible and must not be deployed
  against this profile.
- Loadable modules (C3); `MODULE LIST` is empty and loading is unsupported.
- Multiple logical databases, `SELECT` other than `0`, `MOVE`, `SWAPDB` (C4).
  Clients that configure a nonzero database cannot use this profile: a pinned
  redis-py client started with `db=1` fails during connection setup with
  `DB index is out of range` before its first command, and a DB 0 client keeps
  its data. Addressed databases must be chosen at the application layer, not by
  `SELECT`. `DBSIZE` and `INFO keyspace` describe the single `db0` keyspace; no
  per-database statistics or `FLUSHDB`/`FLUSHALL` selection exists. See
  [compatibility](compatibility.md#important-boundaries) and the
  [database-zero evidence](production-readiness-gaps.md#1-compatibility-and-command-surface).
- Deployment-wide or cross-process rate limiting (S4). The server's budgets are
  per process; use the external control plane described above.
- RDB, `SAVE`/`BGSAVE`, `DUMP`/`RESTORE`/`MIGRATE`, online backup and PITR
  (C5, D1, D5). Migration means the bounded offline import in
  [migration and interchange](#migration-and-interchange), not interchange
  formats.
- Replication, Sentinel, cluster routing, failover and multi-node topologies
  (A1–A5). **Cluster clients are explicitly excluded:** every `CLUSTER`
  subcommand except `HELP` returns `-ERR This instance has cluster support
  disabled`, no slot map exists, no `MOVED`/`ASK` redirect is ever issued, no
  `CROSSSLOT` rejection exists for multi-key commands, braces in keys are
  ordinary bytes rather than hash tags, and sharded Pub/Sub stays inside this
  single process. Clients that need slot discovery, redirects, topology refresh
  or cross-node routing cannot operate against this profile.
- `CLIENT TRACKING`/client-side caching, `OBJECT IDLETIME`/`FREQ`, full RESP3
  attributes/push tracking and `LATENCY GRAPH` (C5, C6, O2, O3). `LATENCY
  HISTOGRAM` exists only as the documented bounded 32-bucket approximation.
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
throttles are implemented, but the profile remains **partial**: the bounded
per-IP throttle uses a shared overflow budget under table pressure and an
opt-in process-wide failed-auth limit is available, but there is no independent
security review, no full Redis ACL surface, no cross-process/distributed rate
limiting, no tenant isolation, no live session revocation beyond the existing
connection controls, and no audited collector federation. Secrets are
operator-managed files or arguments; `requirepass` on the command line is
visible in process arguments.

## Client verification matrix

| Client | Status |
| --- | --- |
| Pinned `redis-cli` RESP2/RESP3/AUTH/metadata | Tested in CI |
| `valkey-benchmark` official generator | Used by the performance gates |
| redis-py 5.2.1 RESP2/RESP3, named ACL, binary strings, pipeline/transaction, Pub/Sub, reconnect | **Partial, optional local harness** — three synchronous modes; TLS and error paths not tested; not a support commitment |
| node-redis / go-redis; full client TLS, error and version matrix | **Unverified** — excluded from this profile |
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
