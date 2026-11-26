# Production compatibility profile (draft)

**Status: DRAFT — the project is NOT production ready.**

This document is the human-readable rendering of the only deployment shape a
future production claim may cover. Its machine-readable, versioned definition is
[`ci/upstream/profile-v1.json`](../ci/upstream/profile-v1.json) (profile
`rillcache-standalone-0.1`, version `1`). The manifest is authoritative: every
row, exclusion and client entry below is declared there, and
`scripts/upstream/profile_matrix.py` executes each of them against the pinned
native Redis and Valkey builds.

The profile is an input to [C1](production-readiness-gaps.md#1-compatibility-and-command-surface),
not a readiness claim. A row becomes supported only when the recorded matrix
result exists for the exact tested head and the remaining P0 gates are closed or
explicitly excluded by a reviewed profile decision. A green matrix run is
per-row evidence for that head; it is never release acceptance.

## Profile identity

| Field | Value |
| --- | --- |
| Profile | `rillcache-standalone-0.1` version `1` |
| Manifest | [`ci/upstream/profile-v1.json`](../ci/upstream/profile-v1.json) |
| Status | `draft` — not a support commitment or compatibility approval |
| Application version | `0.1.0` (unreleased) |
| Pinned references | Redis `8.10.2` (sha256 `541a374b…273e8`), Valkey `9.1.2` (sha256 `19c23908…c983`), also pinned in [`ci/upstream/pins.json`](../ci/upstream/pins.json) and `benchmarks/targets.mk` |
| Topology | One standalone process, one owner-local data path |
| Logical databases | Database `0` only |
| Clients requiring several databases | Excluded; startup with `db` other than `0`, `SELECT` other than `0`, `MOVE`, `SWAPDB` and `COPY ... DB n` are rejected |
| Persistence mode | Optional private append-only journal; RDB excluded |
| Transport | Plain TCP, optional native TLS 1.3 |
| Reliable delivery | Not available in Pub/Sub; stream consumer groups provide acknowledged redelivery across client disconnects, and an external durable transport is required beyond the process |
| Platforms | Linux x86_64 (development/CI); other platforms unverified |

## How the profile is executed

`scripts/upstream/profile_matrix.py` is the C1 matrix runner. For the exact
candidate binary it:

1. Validates the manifest: schema, unique row ids, pinned reference versions and
   SHA-256 values equal to `ci/upstream/pins.json` and `benchmarks/targets.mk`,
   every exclusion bound to an existing canonical gap id, and this document
   naming the manifest path and profile id. Validation is offline and also runs
   as `--check` in the lightweight harness tests.
2. Starts one candidate and one pinned native reference per reference row set and
   runs the identical command rows on both, recording every reply.
3. Compares the `COMMAND LIST` surface by command presence and arity.
4. Executes the declared exclusions and asserts that the candidate fails closed
   (or acknowledges only the declared empty-registry no-op) while the pinned
   references accept the same commands.
5. Executes the pinned `redis-cli` client rows against both servers.
6. Writes `result.json` and `summary.md` and fails the job on any undeclared row,
   surface, exclusion or client difference. Artifacts are retained even on
   failure.

The hosted job is `command-matrix` in
[`upstream-compatibility.yml`](../.github/workflows/upstream-compatibility.yml).
It consumes the same pinned build bundle as the Tcl suites and is a dependency of
the required `Tcl redis`/`Tcl valkey` checks, so a failed matrix blocks the PR.
Local reproduction requires the pinned native builds:

```bash
python3 scripts/upstream/prepare.py --bundle   # GitHub Actions only
python3 scripts/upstream/profile_matrix.py --check
python3 scripts/upstream/profile_matrix.py --output target/upstream/evidence/profile-matrix
```

## Command surface

The supported surface is exactly the command names returned by `COMMAND LIST` on
the tested head, with the matrix below as behavior evidence. The
[compatibility guide](compatibility.md) describes command-family behavior and the
[area references](README.md) carry option-level limits. This profile does not
silently add commands that `COMMAND LIST` does not report.

| Family | Executed rows (declared in the manifest) |
| --- | --- |
| Strings | set/get, binary payloads, append/range, mset/mget, counters, float counters, conditional set, msetnx, getdel/getex, setex/psetex, keepttl, LCS |
| Bitmaps | setbit/getbit/bitcount, bitop, bitpos/bitcount ranges, bitfield |
| Lists | push/range/index/lset/linsert/lrem/ltrim, blocking-free pops, lmove/rpoplpush |
| Sets | membership/cardinality, inter/union/diff (+cardinality, +store), smove |
| Hashes | field CRUD, hgetall, hsetnx/counters, hrandfield, hscan |
| Sorted sets | basic scores/ranks, score and lex ranges, pop/random/mpop, store and cardinality |
| Geospatial | geoadd/geopos/geodist/geohash/geosearch, geosearchstore, georadius (_RO) |
| HyperLogLog | pfadd/pfcount/pfmerge (private representation; payload interchange excluded) |
| Streams | xadd/xlen/xrange/xrevrange/xdel, xtrim, consumer groups and pending entries, xinfo |
| Transactions | MULTI/EXEC, DISCARD, empty-queue EXEC |
| Keyspace | exists/type/rename/copy/touch/unlink, expiry and expiry options, keys, scan |
| Sort / errors / connection | SORT and SORT_RO (BY/GET), wrong-type/arity/syntax/integer/unknown-command errors, PING/ECHO/SELECT 0/QUIT |

The matrix compares replies after discarding only clock values (a live TTL within
its declared bound) and non-deterministic ordering (explicitly declared set,
random-member and cursor-tail sorts). No other normalization exists; a differing
byte, integer, array order or error message is a failed row.

### Declared command-surface divergences

The matrix compares command presence and arity for every `COMMAND LIST` name.
Only these reviewed divergences may differ; anything else fails the job.

| Command | Field | Divergence | Reason |
| --- | --- | --- | --- |
| `PFDEBUG` | arity | candidate `-3`, both references `3` | Bounded variable-argument form; PFDEBUG payload interchange stays excluded (C10) |
| `DIGEST` | presence | absent from Valkey | Private keyspace digest following the Redis 8.10.2 name |
| `DELIFEQ` | presence | absent from both references | Conditional delete added after both pinned revisions; published extension |
| `DELEX` | presence | absent from Valkey | Redis 8.10.2 conditional delete variant |
| `SDIFFCARD` | presence | absent from Valkey | Redis 8.10.2 set-cardinality command |
| `SUNIONCARD` | presence | absent from Valkey | Redis 8.10.2 set-cardinality command |

`SLAVEOF NO ONE` is a declared no-op: replication is excluded (A1), so the legacy
no-replica form returns `+OK` without changing topology while `REPLICAOF` and
`FAILOVER` return explicit unsupported errors. Clients must not treat that `+OK`
as a topology change.

## Explicit exclusions

The following are **not** part of this draft profile. Each exclusion is executed
by the matrix as a fail-closed check, so it can be re-verified rather than
inferred from a generic error:

| Exclusion | Gap | Executed check |
| --- | --- | --- |
| Scripting execution: `EVAL`/`EVALSHA`, `SCRIPT LOAD`, `FCALL`, `FUNCTION LOAD` | C2 | Every execution entry point errors; only `SCRIPT FLUSH`/`FUNCTION FLUSH` acknowledge an empty registry |
| Loadable modules, `MODULE LOAD` | C3 | Loading errors; `MODULE LIST` is empty |
| Multiple logical databases: `SELECT n≠0`, `MOVE`, `SWAPDB`, `COPY ... DB n` | C4 | Rejected without mutating the DB 0 value or creating the destination |
| RDB snapshots: `SAVE`, `BGSAVE` | C5 | Explicit unsupported-persistence error |
| `DUMP`/`RESTORE`/`MIGRATE` payloads, online backup, PITR | D1, D5 | Explicit unsupported-serialization error; no private blob is emitted |
| Replication, Sentinel, cluster routing, failover (A1–A5) | A1 | `REPLICAOF`/`FAILOVER` error; the `SLAVEOF NO ONE` no-op is declared above |
| `CLIENT TRACKING`/client-side caching | C6 | Rejected |
| `OBJECT IDLETIME`/`OBJECT FREQ` | O2 | Rejected instead of fabricating values |
| `LATENCY GRAPH` (`HISTOGRAM` remains a bounded 32-bucket approximation) | C5, O3 | Rejected; `LATENCY HISTORY` is the supported diagnostic |
| `redis.conf` language, `CONFIG REWRITE`, includes, live reload | C13, O5 | Startup configuration is read-only |
| Redis-compatible HyperLogLog payload interchange and diagnostics | C10 | Private representation only |
| Language-client conformance beyond the matrix below | C7 | Declared unsupported |
| Any durability RPO/RTO, independent security review, true RSS enforcement, cross-process observability or production SLO | D1–D11, S1–S9, O1–O9 | Outside this profile |

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

| Client | Declared version | Status | Evidence |
| --- | --- | --- | --- |
| `redis-cli` | `8.10.2` pinned build | Tested | RESP2/RESP3 client rows in the C1 matrix plus the pinned Tcl matrix |
| `valkey-benchmark` | `9.1.2` pinned build | Tested | Official workload generator used by the protected performance gates |
| `redis-py` | `5.2.1` | Partial (C7) | Optional local/hosted diagnostic: RESP2/RESP3, named ACL, binary values, pipeline, transaction, Pub/Sub, reconnect against the candidate only; TLS and error paths uncovered; not a commitment |
| node-redis, go-redis | none pinned | Excluded (C7) | No pinned harness; excluded from this draft profile |
| Arbitrary clients | none pinned | Excluded (C7) | Clients depending on unlisted commands or RESP3 attributes are unsupported |

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
