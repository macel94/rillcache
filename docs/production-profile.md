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
| Deployment-wide rate limiting | Not implemented in the server; process-local budgets only. Multi-process or replicated deployments require an authoritative external control plane |
| Reliable delivery | Not available in Pub/Sub; stream consumer groups provide acknowledged redelivery across client disconnects, and an external durable transport is required beyond the process |
| Platforms | Linux x86_64 (development/CI); other platforms unverified |

## Supported surface

- The command names returned by `COMMAND LIST` on the tested head, enumerated
  with arity and flags in the
  [command matrix](production-readiness/command-matrix.md). The
  [compatibility guide](compatibility.md) describes command-family behavior and
  the [area references](README.md) carry option-level limits. This profile does
  not silently add commands that `COMMAND LIST` does not report.
- The reverse direction is enumerated as well: every command and subcommand name
  reported by the pinned references and not advertised by the head is listed in
  the [command matrix](production-readiness/command-matrix.md) with its exact
  wire reply. Those names are not part of this profile. A top-level command the
  head does not advertise is never accepted silently: it returns either the
  standard `ERR unknown command` or the named capability error recorded in
  [unsupported surfaces](unsupported.md), so a client that needs Redis 8
  vector-set (`V*`) or array (`ar*`) commands, Valkey 9
  `BACKUP`/`COMMANDLOG`/`HOTKEYS`/`HIMPORT`/`TRIMSLOTS`, `DEBUG`, `LOLWUT` or
  scripting entry points fails on the first command rather than at a later
  semantic divergence. A subcommand of an advertised container is answered or
  rejected by that container; the matrix records which, and the area documents
  define how far that answer goes.
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

## Executed command rows

The [supported surface](#supported-surface) above is the name set; the rows below
are the behavior evidence for it, one row per family in the manifest.

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

The following are **not** part of this draft profile and must not be inferred
from a command returning a generic error. The
[command matrix](production-readiness/command-matrix.md) is the exact,
machine-checked enumeration of every excluded name and its reply; this list names
the capability families. The `exclusions` block of
[`ci/upstream/profile-v1.json`](../ci/upstream/profile-v1.json) executes a
representative fail-closed check per family on the candidate and both pinned
references, so an excluded capability cannot silently start succeeding.

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
- Redis 8 vector-set commands (`VADD`, `VCARD`, `VDIM`, `VEMB`, `VGETATTR`,
  `VINFO`, `VISMEMBER`, `VLINKS`, `VRANDMEMBER`, `VRANGE`, `VREM`, `VSETATTR`,
  `VSIM`), Redis 8 array commands (`ar*`) and the Valkey 9 `BACKUP`,
  `COMMANDLOG`, `HOTKEYS`, `HIMPORT`, `TRIMSLOTS` and stream extension commands
  (`XACKDEL`, `XCFGSET`, `XDELEX`, `XIDMPRECORD`, `XNACK`) (C5). `DEBUG` and
  `LOLWUT` are also absent. All of them return `ERR unknown command`; none of the
  advertised commands depends on them.
- A full `redis.conf` language, `CONFIG REWRITE`, includes and live reload (C13,
  O5).
- Language-client conformance beyond the three pinned clients and the declared case set (C7).
- Any durability RPO/RTO, independent security review, true RSS enforcement,
  cross-process observability or production SLO (D1–D11, S1–S9, O1–O9).

## Persistence contract

The private journal is **best-effort**, not an interoperable AOF. With
`appendfsync always`, an acknowledged write cannot precede the durable append;
with `everysec`, crash recovery yields a contiguous prefix and a bounded loss
window; with `no`, the operating system decides. A torn final transaction is
discarded to the last verified boundary. RDB, online backup, point-in-time
recovery and multi-process journal ownership are excluded. Operators must use
local storage and the documented backup/restore tool; shared or network
filesystems are unsupported.

Storage-fault behavior is measured on a bounded tmpfs: a real `ENOSPC` append
replies with an error, latches `aof_last_write_status:err`, keeps every
acknowledged write across a `SIGKILL` restart and drops only the unacknowledged
mutation; a kernel-truncated record is discarded on restart; a rewrite that runs
out of space keeps the live journal; and an unwritable data directory fails
startup. Device-level `EIO`, power loss and filesystem corruption remain
unverified. See [storage faults](persistence.md#storage-faults-and-acknowledgment).

Integrity is layered: CRC32 catches accidental corruption, the optional
HMAC-SHA256 chain authenticates transaction content and order, and the optional
external rollback anchor (created with `rillcache-backup anchor`, verified from
`--aof-anchor-file`) detects a whole-journal rollback or a removed durable
suffix. The anchor must be pinned outside the data volume; an attacker who can
replace both the journal and the anchor defeats it. Application-level encryption
at rest is an **explicit exclusion**: confidentiality depends on platform
disk/volume encryption and owner-only permissions, the HMAC key is an
operator-managed file with no in-process keyring or live rotation, and rotation
is an offline verified rewrite followed by a new anchor. The full contract,
custody and threat model are in [persistence](persistence.md#rollback-and-truncation-anchor)
and [key management](persistence.md#encryption-at-rest-and-key-management).

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
| `redis-py` | `5.2.1` | Tested (C7) | [Pinned language-client matrix](language-clients.md): 14 declared cases including AUTH/ACL, RESP2/RESP3, pipeline, transaction, Pub/Sub, TLS and error paths |
| `node-redis` | `6.4.0` | Tested (C7) | [Pinned language-client matrix](language-clients.md): the same 14 declared cases |
| `go-redis` | `v9.14.0` | Tested (C7) | [Pinned language-client matrix](language-clients.md): the same 14 declared cases |
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
