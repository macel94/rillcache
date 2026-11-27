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
| Topology | One standalone process, one owner-local data path; replication is excluded by the reviewed [A1 decision](replication.md#a1-profile-decision-replication-excluded) with its [single-node availability statement](replication.md#single-node-availability-and-data-loss-statement) |
| Logical databases | Database `0` only |
| Clients requiring several databases | Excluded; startup with `db` other than `0`, `SELECT` other than `0`, `MOVE`, `SWAPDB` and `COPY ... DB n` are rejected |
| Persistence mode | Optional private append-only journal; RDB excluded |
| Transport | Plain TCP, optional native TLS 1.3 |
| Deployment-wide rate limiting | Not implemented in the server; process-local budgets only. Multi-process or replicated deployments require an authoritative external control plane |
| Reliable delivery | Not available in Pub/Sub; stream consumer groups provide acknowledged redelivery across client disconnects, and an external durable transport is required beyond the process |
| Platforms | Linux `x86_64` glibc only, tested in CI; everything else excluded — see the [platform matrix](platforms.md) |

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

## Fixed-worker scheduling and latency

The default Unix mode is readiness-based (Mio, owner-local reactor). Opting into
`--threads N` switches to N cooperative workers that poll instead of blocking on
OS readiness: a 1 ms idle wait, a bounded active window per pass and cooperative
yields between clients; blocking commands are polled about every 10 ms.

The tradeoff is deliberate and is **not** a real-time guarantee:

- Idle command latency is bounded by the poll interval (about 1 ms) rather than
  by an OS wakeup; under sustained load the active window bounds how long one
  client can hold a worker before others get a turn.
- Polling trades syscalls and wakeups for simplicity: the default readiness mode
  is the lower-idle-CPU choice, and the worker modes have no CPU or latency SLO.
- Clients beyond the worker count queue and are served in subsequent passes;
  `max-clients` still bounds admission, and a single expensive command can delay
  others (no fair-share or preemption).
- `CLIENT PAUSE`, blocked commands and slow readers keep their own documented
  bounds; none of them turns polling into an OS readiness wakeup.

`tests/fixed_worker_latency.rs` pins the practical contract for `--threads 1` and
`--threads 4`: a worker answers after 300 ms idle, a second client's `GET`
completes while another client floods 2,000 `PING`s, and 8× the worker count in
concurrent clients are all served within the test bound. CPU cost under idle and
contention is measured by the benchmark studies, not asserted here.

## Slow-client and queue policy

Every admitted connection has a bounded queue and bounded reply work; a slow
reader cannot block the owner, the publisher or other clients.

- **Pub/Sub:** each subscriber owns a 1,024-frame queue. Publishing is
  non-blocking; when the queue is full the subscriber is removed and the attempt
  is counted in `INFO stats` as `pubsub_queue_full_drops` (process lifetime,
  excludes disconnected queues and socket write failures). Other subscribers
  keep receiving.
- **Per-connection output:** replies are written in bounded chunks from a
  per-connection buffer and are never written by another thread, so a client
  that stops reading affects only its own connection; its queue drains or the
  connection is removed by the socket policy, not by unbounded buffering.
- **Large multi-key replies:** `MGET` is refused with `ERR MGET reply limit
  exceeded` when the reply would exceed 2 MiB, preventing duplicate-key
  amplification; `max-bulk-bytes`/`max-frame-bytes` bound requests and
  `max-clients` bounds admissions (see the capacity envelope).
- **Blocked clients:** `BLPOP`-class waiters are bounded by `max-clients` and
  `CLIENT UNBLOCK`/`CLIENT KILL`/pause; they hold no reply buffer while waiting.

This profile does **not** provide configurable slow-client length/rate
thresholds, a per-client kill policy, fair-share guarantees between clients, or
per-tenant quotas; a single client can still consume its share of owner CPU
while admitted. Operators that need those controls must place admission and
rate limiting in front of the server.

`tests/queue_policy.rs` pins the behaviour in default, fixed-one and
four-worker modes: a non-reading subscriber is dropped and counted once its
queue fills while the publisher and a healthy subscriber continue, and an 8×512
KiB `MGET` is refused at the documented bound with the value intact.

## Resident-memory bound and headroom guidance

`--max-data-bytes` bounds logical key/value bytes, not the resident set. Operators
who need a declared memory bound start the server with `--memory-bound-bytes N`
and `--memory-bound-headroom-bytes H`. Startup fails closed when only one of the
pair is given, when `H` is zero or not smaller than `N`, when `N` exceeds a
readable cgroup limit, or when the baseline resident set measured at startup
leaves no payload budget. The derived payload budget is
`min(--max-data-bytes, N - H - baseline_rss)` and is enforced by the existing
payload-admission path: a write that would cross it is refused with
`-OOM configured payload limit exceeded` before it is applied, and the process
keeps serving.

The declared bound is a *bounded RSS target*, not a hard cap. Enforcement is on
the payload budget; thread stacks, allocator metadata, socket buffers and
fragmentation are covered by the declared headroom, while the operator's cgroup
limit remains the kernel-enforced backstop. Declare the bound at or below the
container/cgroup limit.

| Reserved in the headroom | Guideline | Why |
| --- | --- | --- |
| I/O workers and stacks | 1 MiB per worker, minimum 32 MiB | fixed workers and TLS sessions |
| Allocator metadata and maps | 10% of the bound | per-key and per-connection overhead |
| Socket buffers | 4 KiB per admitted client per direction | 128 clients are about 1 MiB |
| Fragmentation and journal staging | 15% of the bound | rewrite/backup staging and fragmentation |

Recommended: `--memory-bound-bytes` equal to the container memory limit and a
headroom of at least 25% of the bound, never below 64 MiB. `INFO memory` reports
`rillcache_memory_bound_bytes`, `rillcache_memory_bound_headroom_bytes`,
`rillcache_memory_bound_baseline_rss_bytes`,
`rillcache_memory_bound_cgroup_limit_bytes`,
`rillcache_memory_bound_payload_limit_bytes`, the explicit
`rillcache_memory_bound_scope` (`rss_bound_derived_payload_limit_not_hard_rss_cap`)
and `rillcache_memory_bound_enforcement`
(`payload_admission_refuses_writes_before_the_bound`).

OOM behavior is tested, not assumed. With the bound enabled the process refuses
writes and survives an exhausted address space; without it, an exhausted address
space aborts the process and restart either replays every acknowledged write or
fails closed on a torn journal tail. `tests/memory_bound.rs` runs both drills
against real child binaries bounded with `ulimit -v`, plus the in-process
refusal and `INFO` evidence. Allocator-level accounting is excluded because it
would require an allocator swap the dependency policy forbids, and kernel
OOM-killer behaviour on dedicated hardware remains a controlled-hardware drill.

## Eviction and access-statistics policy

The selected eviction policies are `noeviction` (default), `allkeys-lru`,
`allkeys-random`, `volatile-lru`, `volatile-random` and `volatile-ttl`, selectable
at startup and at runtime through `CONFIG SET maxmemory-policy`. In this profile
"LRU" is a **mutation-order approximation**: the server does not record per-key
read time, so recency reflects writes rather than accesses. The selected policy
is observable through `CONFIG GET maxmemory-policy` and
`INFO memory` (`maxmemory_policy:`), and eviction counts through
`INFO stats` (`evicted_keys`). `maxmemory` bounds logical key/value bytes, not
RSS (see O1).

LFU and read-idle statistics are **excluded**: `OBJECT FREQ` returns
`ERR An LFU maxmemory policy is not selected, access frequency not tracked. ...`
and `OBJECT IDLETIME` returns `ERR OBJECT IDLETIME is not supported by this
bounded server: read-idle time is not tracked`; `allkeys-lfu`/`volatile-lfu` are
rejected as `ERR invalid maxmemory-policy` and no frequency or idle value is
invented. Clients that need true LRU/LFU semantics, `OBJECT FREQ`/`IDLETIME`, or
frequency-based eviction are incompatible with this profile.

`tests/access_policy.rs` pins the boundary in default, fixed-one and four-worker
modes: both `OBJECT` exclusions, the `noeviction` default, a runtime switch to
`allkeys-lru` reflected in `CONFIG GET` and `INFO memory`, and the rejection of
`allkeys-lfu` with the previous policy retained.

## Security acceptance gate for excluded features

Scripting/functions, replication, failover/Sentinel, cluster routing and modules
are excluded, and they are gated rather than merely documented: any future
implementation must ship a threat model, its authentication/authorization path,
resource bounds, negative tests, audit events and (where required) independent
review before it becomes reachable. `tests/security_gate.rs` fails if any of the
current entry points starts succeeding, so the gate cannot be bypassed silently.
See [the security gate](security.md#security-acceptance-gate-for-new-features).

## Tenant isolation boundary

Multi-tenant deployment is **explicitly excluded**. One process is one trust
domain: all ACL users share the keyspace, the logical payload budget, the
connection limit, the CPU/owner threads, the Pub/Sub and keyspace-notification
namespace, the authentication budgets and the process lifetime. ACL users are
*authorization* inside that domain, not tenants.

What ACL does and does not provide:

- Key patterns deny direct data access outside the granted pattern, and
  keyspace-wide commands (`DBSIZE`, `FLUSHDB`, `KEYS`-class operations) require
  an all-key grant, so an ordinary scoped user cannot enumerate or flush another
  user's keys.
- ACL creates no separate databases, quotas, memory budgets or network
  identities. A user that consumes the payload budget or the client limit
  degrades every other user, `maxmemory` eviction is global, and a restart,
  crash or OOM affects everyone.
- `PUBSUB` introspection is governed by the `@pubsub` category rather than channel
  patterns, so a user holding that category can enumerate channel names used by
  other users (Redis behaves the same way). Do not grant `@pubsub` across
  trust domains.
- Delivery, notifications and client metadata are process-global, and there is
  one event loop/owner per process, so timing and load side channels exist.

The required compensating control for shared infrastructure is one process per
tenant with its own port, credentials, network policy, resource limits and
lifecycle, or an external isolation layer. Running mutually untrusted users in
one Rillcache process is out of profile.

`tests/tenant_boundary.rs` pins the boundary in one process: a scoped tenant is
denied another tenant's key and denied keyspace-wide commands, the owner still
reads the value, a `+@pubsub`-only user sees the other tenant's channel name via
`PUBSUB CHANNELS`, and `INFO clients` reports the process-wide limit.

## HyperLogLog representation and interchange

`PFADD`/`PFCOUNT`/`PFMERGE` are supported over a **private** representation: an
exact hash set up to 4,096 distinct elements, then a 2^14-register sketch with a
cache header. The standard error of that sketch is about 0.81 %
(`1.04 / sqrt(16384)`); cardinalities below the exact limit are returned exactly.
Redis/Valkey sparse encoding, register layout and the sparse-threshold
configuration are not implemented.

Interchange is explicitly excluded: the HLL payload is not a Redis/Valkey
payload, and there is no conversion path. `DUMP`, `RESTORE` and `MIGRATE` return
the bounded capability error, `SAVE`/`BGSAVE` return the RDB-unsupported error,
and the offline migration path accepts only DB 0 `SET` records, so an HLL value
cannot be imported or exported. `PFDEBUG ENCODING|GETREG` and `PFSELFTEST` expose
the private representation and its self-test; they are diagnostics, not an
interoperability contract. Clients that only call `PFADD`/`PFCOUNT`/`PFMERGE`
work; clients that need to move HLL payloads between Redis/Valkey and this
profile are incompatible, and the 0.81 % error must be acceptable to the caller.

`tests/hll_interchange.rs` pins the boundary in default, fixed-one and
four-worker modes: exact encoding and count below the limit,
`*16384` registers, the switch to `dense`, `PFSELFTEST` success, an estimate
within 2 % of 6,000 distinct members, and the explicit `DUMP`/`RESTORE`/`SAVE`
refusals.

## RESP3 type and session subset used by the supported clients

The pinned client matrix ([language clients](language-clients.md)) runs
`HELLO 3` followed by PING, binary SET/GET, INCR, HSET/HGETALL, pipelines,
MULTI/EXEC, Pub/Sub, reconnect and TLS. That traffic uses the following RESP3
surface, and nothing beyond it is claimed:

| RESP3 type or session feature | Behaviour in this profile | Evidence |
| --- | --- | --- |
| Map (`%`) | `HELLO` handshake map | `tests/resp3_client_subset.rs`, `tests/session*.rs` |
| Push (`>`) | Pub/Sub subscribe/message frames after `HELLO 3` | `tests/pubsub.rs`, `tests/resp3_client_subset.rs` |
| Simple string, error, integer, bulk string, array, null | Unchanged from RESP2 for the supported commands | same tests |
| Array where Redis returns a map | `HGETALL` stays a flat array under RESP3; pinned clients accept it | `tests/resp3_client_subset.rs` |
| Double (`,`), boolean (`#`), big number (`(`), verbatim (`=`), set (`~`), attributes (`|`), streamed strings (`;`) | **Not emitted.** No supported command produces them, and no attribute frame is attached to any reply | `tests/resp3_client_subset.rs` asserts the emitted types |
| Client tracking / client-side caching | **Excluded.** `CLIENT TRACKING` returns `ERR unsupported CLIENT subcommand or invalid arguments`, `CLIENT GETREDIR` returns `-1`, `CLIENT TRACKINGINFO` reports `flags off` | `tests/resp3_client_subset.rs`, `tests/clients.rs` |
| `HELLO` protocol negotiation | `HELLO 2` and `HELLO 3` are supported; any other version returns `NOPROTO unsupported protocol version` | `tests/resp3_client_subset.rs` |
| Multiple databases | Excluded (see C4) | `tests/keyspace.rs` |

Request parsing is framing-agnostic: clients send RESP2 arrays even after
`HELLO 3`, and the parser accepts them at every fragmentation boundary with
bounded limits and deterministic errors (`tests/protocol_fuzz.rs`, and the
fragmented handshake case above). RESP3 attributes, streamed strings and push
tracking are not implemented, so clients that require them are out of profile.

## Single-owner capacity envelope

The supported data path is **one owner**: data commands are executed by a single
owner thread, so command-execution parallelism is one CPU regardless of pipeline
depth or client count. `--storage-owners N` (2..=64) is an opt-in experimental
mode, not a capacity claim: its own measurements put it below the one-owner path
at pipeline 16 on the test host, and no protected benchmark gates multi-owner
mode. The default `Config` therefore keeps `storage_owners` unset.

Because there is no measured throughput SLO, the declared capacity limit is the
enforced admission envelope rather than a requests-per-second number:

| Bound | Default | Enforced by |
| --- | --- | --- |
| Data-execution owners | 1 | `--storage-owners` (opt-in experimental only) |
| Simultaneous admitted clients | 128 | `--max-clients`; further connections receive `ERR max number of clients reached` and are closed |
| Logical key/value payload | 64 MiB | `--max-data-bytes`; writes beyond the budget are refused without partial application |
| Request argument / frame | 1 MiB / 2 MiB | `--max-bulk-bytes` / `--max-frame-bytes` |

Capacity in practice is bounded by whichever of those limits is reached first,
plus CPU and memory overhead outside the logical payload budget (metadata,
buffers, sockets and allocator fragmentation are not covered by
`--max-data-bytes`). Scaling vertically increases the limits, not the number of
data owners. Multi-owner scaling, a throughput SLO and cross-owner capacity
sharing are not claimed and are not gated.

`tests/owner_capacity.rs` binds the declaration: the default config is
single-owner with 128 clients, and in default, fixed-one and four-worker modes the
third client beyond `max_clients = 2` receives the explicit refusal while
existing clients keep working and closing one client releases capacity.
`tests/maxclients_info.rs` and `tests/storage_ownership_tests` cover
`INFO clients` reporting and owner-thread isolation.

## Runtime configuration changes

Startup options are read once. Only a declared mutable subset can change while
running, and `CONFIG SET` is the reload path: every name/value pair in one
command is validated against a candidate generation before anything is
published, so a batch either applies completely or leaves the previous
generation untouched.

- **Mutable at runtime:** the listpack/intset thresholds
  (`list-max-listpack-size`, `hash-max-listpack-entries`,
  `hash-max-listpack-value`, `set-max-intset-entries`,
  `set-max-listpack-entries`, `set-max-listpack-value`,
  `zset-max-listpack-entries`, `zset-max-listpack-value`), `maxclients`,
  `maxmemory`, `maxmemory-policy`, `appendfsync`, `requirepass`,
  `notify-keyspace-events`, `slowlog-log-slower-than`, `slowlog-max-len`,
  `commandstats-enabled`, `latency-monitor-threshold`,
  `auto-aof-rewrite-percentage` and `auto-aof-rewrite-min-size`.
- **Startup-only, rejected at runtime:** listeners and TLS material, worker and
  socket topology, `dir`, `appendfilename`/`appendonly`, ACL-file path, audit
  destinations, secret/`requirepass` files, request/frame limits, shutdown
  deadline and the `config-version` marker. `CONFIG SET` returns an explicit
  error such as `appendonly and appendfilename are startup-only; restart with
  the desired options`, and `CONFIG REWRITE` explains that the startup file is
  read-only. `CONFIG GET` does not invent values for unsupported names.
- **Failure retention:** an invalid value, an unknown name or a startup-only name
  rejects the entire batch; previously applied values stay in effect.

`tests/runtime_config.rs` binds this contract in default, fixed-one and
four-worker modes: a two-setting batch applies together, a batch mixing a valid
value with `appendfilename` is rejected with the prior values intact, a batch
with a negative listpack value is rejected with the prior values intact, and
`CONFIG REWRITE` fails explicitly. TLS keeps its separately documented atomic
generation reload.

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
  This is not a temporary gap: loading native code would require an FFI/unsafe
  boundary and an in-process sandbox that the reviewable dependency policy
  forbids, so module-dependent clients are incompatible with this profile.
  `MODULE LIST` returns an empty array, `MODULE HELP` returns one bounded row,
  `MODULE LOAD`/`LOADEX`/`UNLOAD` return `MODULE loading is not supported by
  this server`, and any other subcommand returns `unknown MODULE subcommand`.
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
- The async Tokio transport (`--io-mode tokio`) is an opt-in experimental option
  outside this profile (R2): the default Mio/fixed-worker paths are the supported
  execution modes, and no async gate result substitutes for them. See the
  [async transport review record](async-transport.md#r2-review-record-opt-in-experimental-transport-outside-the-production-profile).
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

The supported recovery path is a quiescent copy only: an online copy is refused
by `rillcache-backup create`, there is no point-in-time recovery, and the
documented procedure restores into an isolated instance and validates exact
data, absolute TTLs and accounting before traffic. The isolated drill records
RPO/RTO components and the retention rule (keep the newest verified archives,
pin copies externally); see [the drill](persistence.md#isolated-restore-drill-rporto-and-retention).

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
