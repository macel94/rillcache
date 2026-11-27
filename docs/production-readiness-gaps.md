# Production readiness gaps

**Status: NOT PRODUCTION READY.**

This is the single authoritative list of what is still missing before Rillcache
can be considered production ready. It is a checklist, not a roadmap promise and
not a compatibility claim.

- **Baseline:** `main` at `1aff4a7` (2026-11-02).
- **Owner:** the project maintainer. Security, durability and compatibility items
  require the review described in each exit criterion.
- **Canonical:** add, remove or change open items here. Do not maintain a second
  open-gap list in `README.md`, `docs/production-readiness.md`, area documents or
  `docs/requirements/next-stories.md`. Those documents describe current behavior
  and boundaries; this file owns the remaining-work checklist.
- **Evidence rule:** if an area document and executable tests disagree, the tests
  are the evidence and this file must be corrected. Do not mark an item closed
  from a plan, a stub, a no-op reply or a single local run.

## What "production ready" means here

For this project, production ready means all of the following for a declared
deployment profile:

1. A published Redis/Valkey compatibility profile passes a differential corpus
   and real client-library tests for the supported command and protocol surface.
2. Data survives process crash, power loss, restart and disk faults within a
   stated RPO/RTO, with tested backup and restore.
3. Replication, failover or cluster topology is either implemented and validated
   or explicitly excluded from the profile with an honest single-node availability
   statement.
4. Authentication, authorization, transport encryption and the audit path have an
   independent security review and operational hardening.
5. Memory, CPU, queues and connection admission are bounded by measured limits,
   not only by a payload counter; overload and slow clients fail predictably.
6. Operators can configure, observe, upgrade, roll back, back up and recover the
   service using documented runbooks, not source knowledge.
7. Performance evidence covers realistic mixed workloads on controlled hardware,
   including tails, memory, persistence and failure modes, with stated SLOs.
8. Distribution is legally clear, versioned, reproducible and verifiable.

Until every **P0** item is closed, or explicitly removed from the declared profile
by a reviewed decision recorded here, no production or drop-in claim is valid.

## Priority and status

- **P0:** blocks every production claim. Close before any `1.0`/production label.
- **P1:** required for a normal production deployment. May be closed or explicitly
  accepted for a narrower profile with an owner and an expiry date.
- **P2:** scale, efficiency or hardening that can follow an initial supported
  profile but must not be presented as solved.
- **Missing:** no implementation.
- **Partial:** a bounded implementation exists; the listed remainder is absent.
- **Evidence gap:** behavior may exist, but the required proof is absent.
- **Deferred:** deliberately postponed by a scope decision; still blocks a
  production claim unless excluded from the profile.

## Gate summary

| Gate | State | Worst gap |
| --- | --- | --- |
| Compatibility and command surface | Partial | RESP3/session features (C6), notification coverage (C8) |
| Durability and recovery | Partial / deferred | No RDB or interoperable AOF, no device-level fault or power-loss evidence, durability performance and cross-platform recovery unmeasured |
| Availability and topology | Missing | No replication, failover, Sentinel or cluster |
| Security and transport | Partial | No independent review, full ACLs, cross-process rate limiting, TLS hardening or audited collector federation |
| Observability and operations | Partial | Payload-only memory reporting, no RSS/allocator accounting, no metrics/log/SLO contract |
| Resource safety and scalability | Partial | One-owner default, opt-in multi-owner is slower in its own measurements, no RSS bound |
| Performance evidence | Evidence gap | Narrow shared-CI microbenchmarks, no realistic workload, controlled hardware or SLOs |
| Distribution, release and legal | Partial | No project license, stable support policy, image signing/SBOM or migration policy |
| Verification and documentation | Partial | Only five upstream Tcl files, no fuzz/soak, stale/contradictory public claims |

---

## 1. Compatibility and command surface

- [x] **C1. The production compatibility profile is undefined.** [P0]
  **Complete (2026-11-26) with a versioned profile and an executed matrix.**
  [`ci/upstream/profile-v1.json`](../ci/upstream/profile-v1.json) declares
  profile `rillcache-standalone-0.1` version `1` (`draft`): exact Redis `8.10.2`
  and Valkey `9.1.2` pins with SHA-256, standalone topology, transport,
  persistence and rate-limiting boundaries, 56 command rows over 12 families, 12
  executable exclusions each bound to a canonical gap id (C2, C3, C4, C5, C6,
  C13, D1, O2, A1, S4), the client matrix and six command-surface divergences.
  The [production profile](production-profile.md) renders the same manifest.
  `scripts/upstream/profile_matrix.py` validates the manifest against
  `ci/upstream/pins.json`, `benchmarks/targets.mk`, this checklist and the
  profile document, then executes every row, exclusion and pinned `redis-cli`
  client row on the candidate and on **both** pinned native references.
  Hosted evidence for head `ab7da80c`: `Declared compatibility profile matrix`
  job 140211666492 in run
  [45908462103](https://github.com/macel94/rillcache/actions/runs/45908462103)
  reports 111 row executions (105 passed, 6 declared divergences, 0 failed),
  24 exclusion checks (24 enforced, 0 violated), 28 client rows passed, and no
  undeclared command-surface mismatch against either reference. The six required
  Tcl contexts depend on that job, so a failed matrix blocks the PR. Four genuine
  reply differences stay visible as manifest-frozen sha256 declarations with the
  exact payloads retained in `result.json`: XPENDING consumer-count and
  empty-summary types, `XINFO GROUPS entries-read` reporting `0` where both
  references report null, and the bounded `ERR unknown command` text that does
  not echo client bytes. This closes profile definition and per-row command
  evidence only: language-client conformance (C7), durability RPO/RTO, security
  review and SLOs remain open, so no production, drop-in or support claim follows.

- [x] **C2. Scripting and functions are missing.** [P0]
  **Complete (2026-11-26) by the explicit-exclusion branch, with no Lua runtime
  linked.** `EVAL`, `EVALSHA`, `EVAL_RO`, `EVALSHA_RO`, `FCALL` and `FCALL_RO`
  are absent from the registry and return the unknown-command error;
  `SCRIPT DEBUG|KILL|LOAD|EXISTS` and `FUNCTION
  LOAD|DELETE|LIST|STATS|DUMP|RESTORE|KEYS` return a deliberate
  unsupported-subcommand error; a missing subcommand returns the arity error; and
  an invalid `FLUSH` mode is rejected. Only `SCRIPT FLUSH`/`FUNCTION FLUSH`
  acknowledge an empty registry. Native TCP evidence runs in default, fixed-one
  and four-worker modes: `tests/discovery.rs`
  `script_and_function_flush_do_not_imply_execution_support` covers the no-ops,
  `COMMAND INFO` absence and representative rejections, and
  `every_named_scripting_entry_point_fails_explicitly` covers every remaining
  named entry point plus unchanged data (`DBSIZE`/`GET`) after the attempts.
  The profile and compatibility boundary state that clients requiring script or
  function execution are incompatible. This does not claim script execution,
  determinism, atomicity or a Lua decision beyond exclusion.

- [x] **C3. Module loading is missing.** [P1]
  **Complete (2026-11-27) by the exclusion branch: the profile, compatibility
  matrix and unsupported-surface reference all exclude loadable modules, and the
  boundary is tested.** No module ABI, loader or in-process sandbox exists, and
  loading native code would require an FFI/unsafe boundary the reviewed
  dependency policy forbids, so module-dependent clients are incompatible.
  `MODULE LIST` returns an empty array, `MODULE HELP` returns one bounded row,
  `MODULE LOAD`/`LOADEX`/`UNLOAD` return `MODULE loading is not supported by this
  server`, and other subcommands return `unknown MODULE subcommand`; a bare
  `MODULE` returns the arity error. Evidence: `tests/unsupported_surface.rs`
  exercises every one of those cases in default, fixed-one and four-worker
  modes, and `docs/unsupported.md` documents the same contract. Not claimed:
  module loading, an ABI, or isolation for native code.- [x] **C4. Multiple logical databases are missing.** [P0]
  **Complete (2026-11-26) as an explicit, tested exclusion.** Only database 0
  exists, and the profile now states the client-facing consequence rather than
  leaving it to a generic error. `SELECT` accepts `0`; nonzero or malformed
  indices, `MOVE`, `SWAPDB` and `COPY ... DB n` are rejected without changing
  binary source/destination data or the key count. `DBSIZE` and `INFO keyspace`
  report only `db0`, so no per-database observability or isolation boundary is
  implied. Wire coverage: `tests/keyspace.rs`
  `database_zero_rejections_leave_the_shared_keyspace_unchanged` runs in default,
  fixed-one and four-worker modes, and `tests/discovery.rs` pins the `db0`
  keyspace section. Pinned-client evidence on exact head `f72b023c` (merged as
  `cfbe0c02`, PR #620): the redis-py 5.2.1 startup check observed
  `DB index is out of range` for `db=1` under RESP2 and RESP3 in Mio, fixed-one,
  Tokio and native TLS, with the DB 0 sentinel unchanged; artifact
  `upstream-redis-py-tls-45791950526-1`. This closes the exclusion branch of the
  criterion; it is not multi-database support, and clients that need several
  logical databases remain unsupported. See
  [profile exclusions](production-profile.md#explicit-exclusions) and
  [compatibility boundaries](compatibility.md#important-boundaries).

- [x] **C5. The command and subcommand surface is incomplete.** [P0]
  **Complete (2026-11-26) by a pinned command-by-subcommand differential
  matrix; this evidence adds no command and implements no missing surface.**
  [The command matrix](production-readiness/command-matrix.md) records all 463
  names reported by the pinned Redis `8.10.2` and Valkey `9.1.2` `COMMAND`
  replies. It lists the 247 names the tested head advertises with their arity and
  flags, and for each of the 216 names it does not advertise the exact wire reply
  to a minimal-arity request. Every absent top-level command fails explicitly:
  54 return the standard `ERR unknown command` and `DUMP`/`RESTORE`/`MIGRATE`
  return the named serialization capability error documented in
  [unsupported surfaces](unsupported.md). The 159 absent subcommands are recorded
  the same way: 20 belong to containers the head does not advertise at all, and
  of the 139 subcommands of advertised containers 53 answer the container's
  bounded behavior, 42 are rejected with a named capability error and 44 with
  their recorded argument or subcommand error. Named capability errors must also
  be recorded in [unsupported surfaces](unsupported.md) or the
  [profile](production-profile.md).
  Machine checks: `tests/command_matrix.rs` runs the check in default, fixed-one
  and four-worker modes and fails when the advertised set differs from `COMMAND`,
  when an absent command answers successfully, when `COMMAND INFO` describes an
  absent name, when a named capability error is undocumented, when the probe pass
  leaves keyspace data or an unusable session, or when the committed matrix does
  not match the tested head byte for byte. The upstream inventory is generated
  from the checksum-verified archives pinned in `benchmarks/targets.mk` by
  `scripts/upstream_command_inventory.py`; the check itself is offline.
  Limits that remain open elsewhere: the 57 absent top-level commands (Redis 8
  vector-set and array commands, Valkey 9 `BACKUP`/`COMMANDLOG`/`HOTKEYS`/
  `HIMPORT`/`TRIMSLOTS`/stream extensions, `DEBUG`, `LOLWUT`, scripting
  entry points, `DUMP`/`RESTORE`/`MIGRATE`) are excluded from the draft profile
  rather than implemented, the 53 answered subcommands are bounded by their area
  documents, and option-level, protocol-version and language-client coverage stay
  with C1, C6 and C7. **Closes when (criterion):** a command-by-command
  differential matrix exists for supported data types and admin surfaces; every
  absent command is either implemented or explicitly excluded with a
  client-facing error and profile entry.

- [x] **C6. RESP3 and session behavior are a bounded subset.** [P1]
  **Complete (2026-11-27) for the RESP3 surface the pinned client matrix
  actually uses, with explicit exclusions.** The profile documents the surface
  used by `redis-py` 5.2.1, `node-redis` 6.4.0 and `go-redis` v9.14.0 after
  `HELLO 3`: the handshake map, Pub/Sub push frames, and RESP2-compatible simple
  strings, errors, integers, bulk strings, arrays and nulls for the supported
  commands, with `HGETALL` documented as a flat array rather than a map. Double,
  boolean, big-number, verbatim, set, attribute and streamed-string types are
  not emitted; client tracking/caching is excluded with explicit replies
  (`CLIENT TRACKING` -> `ERR unsupported CLIENT subcommand or invalid
  arguments`, `GETREDIR` -> `-1`, `TRACKINGINFO` -> `flags off`); `HELLO`
  rejects versions other than 2 and 3 with `NOPROTO`. Evidence:
  `tests/resp3_client_subset.rs` pins those types, exclusions and a fragmented
  `HELLO 3` handshake in default, fixed-one and four-worker modes, alongside
  the existing `tests/pubsub.rs` push coverage and `tests/protocol_fuzz.rs`
  fragmentation/limit corpus. Not claimed: attributes, streamed strings, push
  tracking or full RESP3 type parity.- [x] **C7. Real client-library compatibility is incomplete.** [P0]
  **Complete (2026-11-27) with a versioned, pinned language-client matrix.**
  [`ci/upstream/clients-v1.json`](../ci/upstream/clients-v1.json) (matrix
  `rillcache-language-clients` v1) pins `redis-py` 5.2.1, `node-redis` 6.4.0 and
  `go-redis` v9.14.0 and declares fourteen cases: handshake, named-user
  AUTH/ACL allowance and denial, RESP2 and RESP3 round-trips (binary values,
  counters, hash reads), pipeline, transaction, pub/sub, wrong-type and
  unknown-command errors, an unauthenticated connection, reconnect, and TLS
  positive plus wrong-identity rejection. `scripts/upstream/client_matrix.py`
  validates the manifest against each pinned lock file, verifies the version each
  client reports at runtime, and fails on a missing, failed, duplicated or
  undeclared case. Hosted evidence for head `e0807d9e`: `Pinned language-client
  matrix` job 140227942914 in run
  [45913387009](https://github.com/macel94/rillcache/actions/runs/45913387009)
  reports 14/14 cases for each of the three clients with no failures; the six
  required Tcl contexts depend on that job. The RESP3 cases record that the
  bounded RESP3 subset emits arrays rather than maps (C6), asserted and frozen
  per client instead of hidden. This is compatibility evidence for the declared
  case set and pinned versions: it is not a support window, does not cover other
  client versions, pool-under-load, cluster routing, client-side caching, RESP3
  push tracking or mutual TLS, and does not close C6.

- [ ] **C8. Keyspace notification coverage is incomplete.** [P1]
  `keymiss` and new-key events are implemented for the documented read/write
  command set, but `FLUSH*` event classes remain unsupported; delivery is
  at-most-once and process-local.
  **Partial (2026-11-14): hash lifecycle and TTL notifications.** A lazy
  field-expiry pass publishes one `hexpired` event per affected key and an
  additional generic `del` when the empty hash is removed, including when the
  observing command is rejected. `HEXPIRE`/`HPEXPIRE`/`HEXPIREAT`/`HPEXPIREAT`
  publish `hexpire` and `HPERSIST` publishes `hpersist` when at least one field
  actually changed, with no event or `WATCH` bump for condition failures.
  `HDEL`/`HGETDEL` publish `hdel` only for a real removal and append `del` when
  that removal emptied the hash; no-op deletes and commands that only observed
  lazy expiry emit no delete event. Tests cover publication order and the no-op
  cases, and the event matrix records the lazy-only detection point.
  `HSETEX` now publishes `hset` plus `hexpire`, or `hdel`/`del` for past
  deadlines, with no-op conditions emitting nothing. `HGETEX` publishes
  `hexpire`/`hpersist` for future deadlines and successful persists, never a
  synthetic write. With an already-due `PXAT`/`EXAT` deadline, it returns selected
  pre-expiry values and publishes `hexpired`, followed by `del` only when the hash
  becomes empty; all three worker modes now have a regression test for this
  Valkey-verified ordering.
  **Partial (2026-11-14): key-miss notifications.** The `m` class reports `GET`,
  each missing `MGET` occurrence and missing `EXISTS` arguments; `TYPE`/`TTL`
  family misses; missing `HGET`/`HGETALL`/`HLEN`/`HEXISTS`/`HMGET`/`HKEYS`/
  `HVALS`/`HSTRLEN` hashes; `STRLEN`/`GETRANGE`/`GETBIT` strings; missing
  `LLEN`/`LINDEX`/`LRANGE`/`LPOS` lists; missing `SCARD`/`SISMEMBER`/
  `SMISMEMBER`/`SMEMBERS` sets; missing `ZCARD`/`ZSCORE`/`ZMSCORE`/`ZRANGE`/`ZRANK`/`ZREVRANK`/
  `ZREVRANGE`/`ZCOUNT`/`ZRANGEBYSCORE`/`ZREVRANGEBYSCORE`/`ZLEXCOUNT` sorted
  sets; `SRANDMEMBER`/`ZRANDMEMBER`/`HRANDFIELD` random reads; `XLEN`/`XRANGE`/
  `XREVRANGE` streams; `HSCAN`/`SSCAN`/`ZSCAN` collection scans; set/sorted-set
  algebra sources including store variants, with one miss per missing source
  key; stream `XACK`/`XPENDING`/`XCLAIM`/`XAUTOCLAIM`/`XINFO` metadata reads;
  `SORT`/`SORT_RO` sources including `STORE` and `BY`/`GET` pattern lookups,
  and `XREAD`/`XREADGROUP` initial stream lookups. **Partial (2026-11-18):**
  `BITOP` emits one key-miss event for each missing source occurrence; existing
  strings and wrong-type source keys do not create false misses. `TOUCH` reports
  each missing operand while hits, including wrong-type keys, remain non-misses.
  `COPY` reports a missing source lookup but not a hit, including when the
  destination already exists and the command returns zero.
  `PFCOUNT` reports each missing HLL source lookup, including repeated sources. `PFMERGE` also reports the missing destination read
  and each missing source occurrence; existing HLL and wrong-type keys do not
  create false misses. **Partial (2026-11-18):** Geospatial read commands now
  report missing sorted-set source lookups for `GEODIST`, `GEOHASH`, `GEOPOS`,
  `GEOSEARCH`/`GEOSEARCHSTORE` and legacy `GEORADIUS*` variants. Missing members
  in existing sets and existing wrong-type keys remain non-misses.
  These events use the normal keyspace/keyevent channels. Hits, absent
  fields/elements/members and existing wrong-type keys do not. Other read
  commands remain open. `A` intentionally
  excludes `m`, as in Redis.
  **Partial (2026-11-14): new-key notifications.** The `n` class reports the
  first creation of a key by supported mutation paths, including absent
  COPY/RENAME destinations, while overwrites, extra stream entries and reads do
  not emit. `A` excludes `n`, as in Redis. `FLUSH*`/module classes and key-miss
  coverage for other read commands remain open. Delivery is at-most-once and
  process-local.
  **Closes when:** the event matrix matches the declared Redis/Valkey version for
  supported mutations, with ordering, duplicate/loss and reconnect tests; durable
  delivery remains explicitly out of scope unless required by the profile.

- [ ] **C9. Streams remain a bounded subset with volatile delivery state.** [P1]
  Entries, last IDs, group cursors, consumers and pending records survive restart
  (D6 complete), but `XINFO` still diverges (`nacked-count` is zero), and
  approximate `XTRIM` follows a different granularity.
  **Partial (2026-11-14): known `entries-read` counts.** Groups created at `$` or
  `0` now report `entries-read` and increment it on new `XREADGROUP ... >`
  deliveries; `XGROUP CREATE|SETID ... ENTRIESREAD n` sets the counter and journal
  rewrite/replay preserves it. Without an explicit counter, explicit starts and
  replay reconstruct retained entries up to the cursor (an undercount if history
  was deleted/trimmed). Tests cover `$`, `0`, SETID, ENTRIESREAD, delivery
  increments and replay. `lag` now reports `entries_added - entries_read` for
  known counters, stays stable across `XDEL`/`XTRIM`, and is null for
  `ENTRIESREAD -1` or an unknown counter. Consumer `idle` now uses last
  attempted interaction and `inactive` uses last successful delivery/claim,
  with both timestamps persisted across rewrite/replay.
  **Closes when:** metadata fields match the declared version or are explicitly
  excluded, and trim/read semantics pass a differential corpus.

- [x] **C10. HyperLogLog is not Redis-interoperable.** [P1]
  **Complete (2026-11-27) by the criterion's exclusion branch: HLL
  persistence/interchange is explicitly excluded and the representation and
  accuracy differences are documented.** `PFADD`/`PFCOUNT`/`PFMERGE` use a
  private representation — an exact set up to 4,096 distinct members, then a
  2^14-register sketch with a cache header and a documented ~0.81 % standard
  error; Redis/Valkey sparse encoding, register layout and sparse thresholds are
  absent. The profile states that the payload is not interchangeable: `DUMP`,
  `RESTORE` and `MIGRATE` return the bounded capability error, `SAVE`/`BGSAVE`
  return the RDB-unsupported error, and the supported offline migration imports
  only DB 0 `SET` records, so HLL values cannot be moved in or out;
  `PFDEBUG ENCODING|GETREG` and `PFSELFTEST` are private diagnostics. Evidence:
  `tests/hll_interchange.rs` runs in default, fixed-one and four-worker modes and
  pins exact encoding/count below the limit, the `*16384` register array, the
  switch to `dense`, `PFSELFTEST`, an estimate within 2 % of 6,000 distinct
  members, and the explicit refusals. Not claimed: Redis-compatible payloads,
  sparse encoding/thresholds or payload conversion.- [ ] **C11. Geospatial search is linear.** [P2]
  `GEOSEARCH` and the legacy radius aliases scan the source sorted set linearly;
  there is no native geohash neighbour index. This is correct for small sets but
  is not evidence for large production keyspaces.
  **Closes when:** complexity and memory are documented and benchmarked at the
  target scale, with a native index or an accepted latency limit.

- [x] **C12. Pub/Sub is deliberately non-durable.** [P1]
  **Complete (2026-11-26) as an explicit boundary with a validated alternative.**
  Pub/Sub delivery remains in-process, at-most-once, unordered across
  publishers, bounded by per-client queues and neither persisted nor
  replicated; sharded channels stay a standalone namespace with no cluster
  hash-slot routing. The profile now states that boundary in client terms and
  names the supported reliable alternative: a stream consumer group
  (`XREADGROUP`/`XPENDING`/`XCLAIM`/`XAUTOCLAIM`/`XACK`), whose pending state
  survives a client disconnect and is recorded in the journal but is not
  validated across a server restart, or an external durable transport for
  recovery beyond the process or host. Evidence: `tests/stream_claim.rs`
  `stream_group_redelivers_pending_entries_after_consumer_disconnect` closes the
  delivering connection, then redelivers the exact binary payload to fresh
  connections through `XCLAIM` and `XAUTOCLAIM` before `XACK` clears the set, in
  default, fixed-one and four-worker modes; the merged RESP2
  `disconnected_subscriber_does_not_receive_gap_messages_after_resubscribing`
  keeps the Pub/Sub half of the boundary honest (publish in the gap returns
  zero and is never replayed). This is at-least-once redelivery with duplicates,
  not exactly-once, not replication, not a restart-recovery claim and not a
  durability claim for host loss.
  See [Pub/Sub](pubsub.md), [streams](streams.md) and
  [compatibility boundaries](compatibility.md#important-boundaries).

- [ ] **C13. Configuration compatibility is missing.** [P1]
  There is no full `redis.conf` parser, `CONFIG REWRITE` or complete runtime
  configuration surface. Startup historically used only a CLI subset; selected
  `CONFIG GET/SET` options exist at runtime.
  **Partial (2026-11-14): bounded startup file.** `--config PATH` (or `-c`)
  reads a UTF-8 file of one directive/value pair per line, with comments,
  quoting, `maxmemory`/`maxclients` aliases and the same validation as CLI
  flags. It is not a general `redis.conf`: includes, multi-value directives,
  `save`, replication, modules and other unsupported language fail closed before
  bind; later CLI options override earlier file values. See
  [startup configuration](configuration.md). `--check-config` checks merged
  startup options without starting the server; it does not test resource opening.
  `config-version 1` can explicitly declare the startup file format; unmarked
  files retain version 1 behavior. A parser test checks the documented example.
  Startup config reads stop at the 1 MiB limit plus one byte; this is a per-file
  bound and does not set an I/O timeout or limit the number of files.
  **Remaining:** a complete defaults inventory, `CONFIG REWRITE`, live
  reload/rollback and the complete runtime-mutable surface are absent, and the
  upstream configuration language is not accepted.
  **Closes when:** the supported configuration schema is versioned, validated
  before bind, documented with defaults/precedence, and runtime-mutable settings
  are atomic and complete for the profile.

- [x] **C14. Transaction semantics diverge from Redis.** [P1]
  **Complete (2026-11-14) for the standalone transaction contract.**
  Connection-local commands are rejected at queue time rather than executed at
  `EXEC` time. There is no rollback, replication of transactions or durable
  distributed commit; the single-process owner is the only isolation boundary.
  [Transactions](transactions.md) now records the divergence explicitly and
  names the tests: the session control-flow/runtime-error suite and the all-mode
  `tests/blocking.rs` queue-time rejection cases. Replication, durable commit
  and failover fencing remain under the availability/durability items and are
  not implied by this contract.

---

## 2. Durability and recovery

- [x] **D1. RDB, DUMP/RESTORE and migration interoperability are missing.** [P0]
  **Complete (2026-11-26) by the exclusion branch: the profile excludes RDB/AOF
  interchange and specifies a supported migration tool, which is now exercised
  end-to-end.** `SAVE`/`BGSAVE` return explicit RDB-unsupported errors,
  `LASTSAVE` is `0`, and `DUMP`/`RESTORE`/`MIGRATE` return one capability error
  naming the journal/backup alternatives; these are asserted in default,
  fixed-one and four-worker modes by `tests/boundaries.rs` and
  `tests/unsupported_surface.rs`. The supported migration path is the bounded
  offline DB 0 string import documented in
  [persistence](persistence.md#import-a-bounded-offline-string-journal) and
  [the profile](production-profile.md#migration-and-interchange):
  `scripts/import_string_journal.py`, then `rillcache-backup create`/`verify`,
  then a controlled first start. Evidence: `tests/migration_path.rs`
  `documented_import_verify_and_start_path_preserves_strings_and_expiry` runs
  that exact sequence with the real tools and asserts a binary value, an absolute
  expiry and the key count on the booted server, while
  `unsupported_records_and_existing_targets_fail_closed` proves a non-`SET`
  record and an occupied target are refused without publishing. Not claimed:
  native RDB/multipart-AOF conversion, live cutover, non-string or non-DB-0
  data.

- [x] **D2. AOF rewrite is synchronous and blocks command service.** [P0]
  **Complete (2026-11-26) by the accepted start-pause SLO and the concurrent
  pause measurement; the cooperative rewrite itself landed earlier.**
  `BGREWRITEAOF` replies `+Background append only file rewriting started` after
  replies `+Background append only file rewriting started` after materialising the
  key list, and the maintenance loop advances one bounded slice (≤512 keys or
  1 MiB) per tick between client commands. Changed keys are re-encoded as whole
  values, so reconciliation cannot duplicate collection members; the bulk fsync
  runs outside the gate and the final residual plus rename plus directory sync run
  under a short gate. `INFO persistence` reports `aof_rewrite_in_progress`, and a
  kill mid-rewrite keeps every acknowledged write (`tests/durability_crash.rs`).
  **Partial (2026-11-14): bounded rewrite progress.** `INFO persistence` also
  reports `aof_rewrite_phase` (`snapshot`, `reconcile`, `sync`, `commit`, `none`
  or `unavailable`) and stable snapshot progress as
  `aof_rewrite_keys_done`/`aof_rewrite_keys_total`. Reconciliation may revisit a
  key, so progress stops at the snapshot total instead of counting duplicates.
  `aof_rewrite_bytes_written` reports actual bytes written to the temporary
  file, including framing and reconciliation, without claiming durability.
  `aof_last_rewrite_start_usec` reports the latest successful setup interval;
  it is not a client latency measurement or an independent start-pause bound.
  A staging setup failure reports a rewrite error without marking the live
  journal unwritable; an isolated fixture verifies retry and replay.
  **Measured (host-local):** old synchronous pass 130 ms full stall and 129 ms
  worst PING for 20,000 keys (2.5 MB); cooperative start pause 9–20 ms
  (key-list materialisation), 1.6–2.4 ms per 512-key slice, commit under 1 ms plus
  the residual fsync. Completion time still scales with the dataset.
  **Accepted SLO and measurement (2026-11-26).**
  `tests/journal_rewrite_pause.rs` starts a journaling server, writes 5,000 then
  40,000 keys, issues `BGREWRITEAOF`, keeps a second client writing during the
  snapshot and a third measuring `PING` latency, and fails when the start pause,
  the worst concurrent command stall or either progress counter breaks the
  budget. Measured on the shared development host: start pause **3.9–5.6 ms** at
  5,000 keys and **25.3–32.2 ms** at 40,000 keys, worst concurrent stall
  **12–48 ms** and **64–130 ms**, with `aof_rewrite_keys_done` reaching both
  snapshot totals and `aof_rewrite_bytes_written` growing to 708,000 and
  5,738,000 bytes across `snapshot`, `reconcile`, `sync` and `commit`. Because the
  start pause materialises one key name per live key it is linear in key count,
  so the reviewed SLO recorded in
  [persistence](../persistence.md#compaction-with-bgrewriteaof) accepts **≤1 s
  start pause and ≤1 s worst concurrent stall for keyspaces up to 1,000,000
  keys**; larger keyspaces are explicitly unmeasured and must not be assumed.
  Limits that remain: no fork or copy-on-write page sharing, so pause and memory
  scale with the live key count; completion time still scales with the dataset; a
  sustained write storm lengthens reconciliation rounds, so byte progress has no
  fixed total and cannot report a percentage. RDB, online backup and
  point-in-time recovery remain D1, D5 and D10 items.
  **Closes when (criterion):** the start pause is bounded independently of key
  count or an accepted SLO is recorded, and progress reporting covers bytes/keys
  completed.

- [x] **D3. Journal integrity and encryption at rest are Partial.** [P0]
  **Complete (2026-11-27) with an external rollback anchor and a reviewed
  integrity/encryption contract.** Evidence for head `2963471c`:
  `Tests and offline CI` job 140250092595 in run
  [45920581977](https://github.com/macel94/rillcache/actions/runs/45920581977)
  exercises `tests/aof_anchor.rs` and the `rollback_anchor_*` engine tests. CRC32 checksums and the optional HMAC-SHA256
  chain are unchanged. `rillcache-backup anchor` now re-verifies an authenticated
  journal through the offline replay path and publishes a bounded (512-byte)
  key-authenticated anchor committing to the exact verified byte length and
  chain tag; it refuses to overwrite an existing anchor and writes owner-only
  permissions. `--aof-anchor-file` makes startup require that boundary and tag,
  so a shorter journal (whole-file rollback, restored older copy, removed
  durable suffix) or a replaced/rewritten chain fails closed with an explicit
  error, while normal growth and a crash-torn tail after the boundary still
  replay. The anchor is read-only server input and never rewritten by the
  server. Evidence: `tests/aof_anchor.rs` (tool writes the anchor, anchored
  restart, growth accepted, truncation/rollback rejected, foreign-key anchor
  rejected, malformed/empty/unanchored configurations fail closed) and
  `src/engine/journal_tests.rs` `rollback_anchor_*` (extension, rollback,
  truncation, empty journal, tampered/oversized/trailing anchors, MAC under a
  foreign key, and rewrite invalidation). The reviewed contract is documented in
  [persistence](persistence.md#rollback-and-truncation-anchor) and
  [encryption/key management](persistence.md#encryption-at-rest-and-key-management):
  application-level encryption at rest is an explicit exclusion delegated to
  platform disk/volume encryption, key custody is an operator-managed file with
  owner-only permissions and no in-process keyring, and rotation is an offline
  verified rewrite plus a new anchor. This does not add timestamps, replay
  counters, KMS integration, revocation or protection against an attacker who can
  also replace the pinned anchor; those remain documented limits, not claims.

- [x] **D4. Crash, ENOSPC and disk-fault behavior is unverified.** [P0]
  **Complete (2026-11-27) for real filesystem faults at every exercised
  boundary, merged on `main` as `402aad9c`.** `tests/durability_crash.rs` keeps
  the deterministic `SIGKILL`, `WAITAOF` and torn-tail drills.
  `tests/durability_faults.rs` adds real kernel faults on a size-bounded tmpfs:
  an `ENOSPC` append replies `-ERR append-only write failed: No space left on
  device`, latches `aof_last_write_status:err`, refuses later mutations with the
  failed-state error, recovers exactly the acknowledged prefix after `SIGKILL`
  and drops the unacknowledged mutation; a kernel-truncated (short) write is
  discarded on restart while the acknowledged prefix and later writes survive;
  an `ENOSPC` rewrite latches `aof_last_bgrewrite_status:err`, leaves the live
  journal byte-identical and still serving, and a retry after freeing space
  compacts; and an unwritable data directory fails startup closed. Evidence:
  `Real storage-fault drill` job 140275582381 in run
  [45928548371](https://github.com/macel94/rillcache/actions/runs/45928548371)
  for head `33e8a46b`; the protected `Tests and offline CI` check needs that job,
  so a mount failure or fault regression fails closed instead of skipping. The
  fault matrix, loss bounds and the applied-but-unacknowledged in-memory state
  are documented in
  [storage faults](persistence.md#storage-faults-and-acknowledgment). Not covered
  and not claimed: device-level `EIO`, power loss, block rotation, filesystem
  corruption, per-user quota and separate `fsync`-failure injection; these need
  `dm-error`, hardware or a reviewed FUSE harness. This entry corrects the
  checklist record for #458: the evidence merge landed but an authoring script
  bug dropped the checkbox change from the squash.

- [x] **D5. Backup, restore and point-in-time recovery are Partial.** [P0]
  **Complete (2026-11-27) with a measured quiescent-copy drill and an explicit
  online-backup/PITR exclusion, merged on `main` as `7ffc294c`.**
  `scripts/restore_drill.py` runs the real server and `rillcache-backup`:
  it loads a deterministic mixed dataset (binary strings, collections, streams
  with a consumer group, counters, absolute `PEXPIREAT` deadlines), snapshots the
  exact state, asserts that an **online** `create` is refused and publishes no
  archive, takes and verifies a quiescent archive, restores it into a separate
  directory (asserting only the restored journal exists and the source journal is
  byte-identical), starts a second instance from the restored journal and
  compares the key set, per-key type/value bytes, absolute expirations, db0
  `keys`/`expires`/`subexpiry` and `MEMORY STATS` `keys.count`/`dataset.bytes`
  exactly (the clock-derived `avg_ttl` only inside a bounded elapsed window),
  then repeats for three recovery points, applies the keep-newest-verified
  retention rule and restores **each** retained archive into its own isolated
  instance to reproduce its point. Measured on the hosted job for head
  `568e81e2`: 2000-key dataset, `create` 27.839 ms, `verify` 26.068 ms,
  `restore` 25.814 ms, isolated startup 17.213 ms, restore-to-serving 43.027 ms,
  zero acknowledged writes lost for the quiescent copy, retention kept
  `snapshot-2`/`snapshot-3` and pruned `snapshot-1`, and both retained archives
  restored their points (2001 and 2002 keys). Evidence: `Isolated backup/restore
  drill` job 140296768886 in run
  [45935223577](https://github.com/macel94/rillcache/actions/runs/45935223577);
  the protected `Tests and offline CI` check needs that job, so a regression
  fails closed. The procedure, RPO/RTO definitions, retention rule and the
  explicit online-backup/PITR exclusion contract with its freeze-and-verify
  operations are documented in
  [the restore drill](persistence.md#isolated-restore-drill-rporto-and-retention).
  Not claimed: online/incremental backup, point-in-time recovery, a backup
  scheduler, integrity beyond CRC32 plus the authenticated-journal checks, or a
  deployment-specific SLO — those numbers must be re-measured on the operator's
  hardware and archive size. This entry corrects the checklist record for #459:
  the evidence merge landed before the checkbox commit, so the record is added
  here rather than silently lost.

- [x] **D6. Stream pending and consumer state survives restart.** [P0]
  **Complete (2026-11-11).** The journal rebuild emits `XGROUP CREATECONSUMER` for
  every consumer and `XCLAIM ... TIME ... RETRYCOUNT ... JUSTID FORCE` for every
  pending entry, so owners, delivery counts and delivery timestamps survive both
  the live journal and the cooperative rewrite. Deleted/trimmed pending ids are
  recreated with ascending placeholder `XADD`s, claimed into every referencing
  group, then deleted, leaving ackable PEL tombstones without synthetic payloads.
  An empty stream with only group/consumer metadata is recreated through
  `MKSTREAM`. Consumer last-activity (`seen_at`) is persisted in an inert
  namespaced ECHO record (binary key/group/consumer plus absolute timestamp) and
  restored during replay, so `XINFO CONSUMERS` `idle`/`inactive` keep counting
  from the original activity including downtime.
  **Evidence:** `tests/persistence.rs` (consumers/PEL, empty group, trim),
  `tests/stream_pel_tombstones.rs` (deleted/trimmed/all-deleted tombstones and
  rewrite), `tests/stream_consumer_activity.rs` (live and rewrite activity aging),
  plus journal framing/round-trip unit tests; all three worker modes.
  **Retained limits:** activity is only tracked when the keyspace has an expiry
  or field TTL (the existing clock shortcut otherwise records zero); `XCLAIM` of a
  tombstone removes it per missing-entry semantics; historical state for deleted
  or renamed consumers is not retained.

- [x] **D7. Signal-driven shutdown is partial.** [P0]
  **Complete (2026-11-26) for the standalone supervisor contract across plain
  Mio, plain Tokio and native TLS.** The standalone binary handles Unix
  SIGINT/SIGTERM and Windows Ctrl-C, stops admission, joins workers and flushes
  the journal within a configurable grace period (`--shutdown-deadline-ms`,
  default 25 s, max 600 s, documented in
  [startup defaults](getting-started.md#configure-only-what-you-need)); a second
  signal or an expired deadline logs `forced termination: <reason>` and exits
  nonzero. Existing subprocess tests prove bounded command drain of a withheld
  8 MiB reply, cancellation of idle/incomplete-frame/subscribed/blocked clients
  with no replay of incomplete work, forced termination on a second signal,
  restart from the unchanged AOF with `appendfsync always`, and TLS drain with a
  valid reply prefix. The previously untested branch — **deadline exhaustion
  end-to-end** — is now covered by `tests/shutdown_deadline.rs`: with a 300 ms
  deadline and an unread admitted reply that keeps the worker blocked, each of
  the three transports exits within the test bound, logs the deadline reason,
  exits nonzero, and restarts with the acknowledged 1 MiB value intact.
  Retained limits: client receipt is not acknowledged, a reply that exceeds the
  deadline may be truncated, and a forced timeout cannot guarantee the final
  journal flush beyond the acknowledged fsync policy.
- [x] **D8. Multi-process journal ownership is unguarded.** [P1]
  **Complete (2026-11-14): local ownership plus explicit profile exclusion.**
  Startup claims `<journal>.lock` with an exclusive create recording PID and
  kernel start time; a second process on the same directory fails with a clear
  error, a crash-stale lock is reclaimed only when the recorded process is gone or
  its PID was reused, and clean shutdown removes the lock
  (`tests/journal_ownership.rs`). The [draft standalone profile](production-profile.md)
  excludes shared/network filesystems and multi-process journal ownership, so the
  supported deployment is one process on local storage. **Retained limitation:**
  there is no distributed lease or shared-volume guarantee; an operator must keep
  the journal on local storage or enforce single-process access externally.

- [x] **D9. Wall-clock rollback across restart is a documented limitation.** [P1]
  **Complete (2026-11-14):** TTL deadlines are absolute Unix-epoch milliseconds
  evaluated against the system wall clock. If the clock moves backward before an
  expired key is observed and its deletion is persisted, the deadline can become
  live again, including on restart; there is no persisted clock high-water mark.
  Time-zone and daylight-saving changes do not alter Unix-epoch time. Operators
  should keep the clock synchronized and avoid backward system-clock steps.
  `src/engine/journal_tests.rs::clock_rollback_after_restart_can_resurrect_a_persisted_expiry`
  exercises the rollback/replay case with a deterministic clock.
  **Retained limitation:** Rillcache does not prevent clock-step resurrection;
  the operational contract is explicit rather than an anti-rollback guarantee.

- [ ] **D10. Durability performance is unmeasured.** [P1]
  Persistence serializes commands through the exclusive gate; there is no
  throughput/latency evidence for `always`/`everysec`/`no`, rewrite pauses or
  large journals.
  **Closes when:** matched benchmarks report throughput, tails, fsync cost and
  rewrite pause for each supported policy and workload, with an accepted SLO.

- [ ] **D11. Recovery is not validated across platforms/filesystems/versions.** [P1]
  Tests cover ordinary restart, truncated tails and mid-file corruption. The
  Linux same-binary torn-tail drill checks checksum and HMAC journals, including
  exact retained bytes and replay after new appends. A separate Linux fixture
  restores a complete legacy RESP archive and verifies replay with a newly
  checksummed suffix; no old binary participates. Kill -9 at each boundary,
  power-loss simulation, filesystem matrices, mixed versions and long-running
  recovery remain unverified.
  **Closes when:** a repeatable recovery matrix covers those cases and retains
  failure evidence.

---

## 3. Availability and topology

- [ ] **A1. Replication is missing.** [P0]
  `PSYNC`, `SYNC`, `REPLCONF`, `REPLICAOF host port` and `SLAVEOF host port`
  return explicit replication-unsupported errors. `ROLE`/`INFO replication`
  truthfully report a standalone master; `WAIT` returns 0 and `WAITAOF` only
  acknowledges the local journal. A native TCP test pins the standalone role
  and zero replication offset after a local write and rejected reconfiguration.
  **Closes when:** full and partial synchronization, a consistent snapshot,
  bounded backlog, offsets/IDs, reconnect, replica apply and mixed-version
  interop pass; acknowledged-write loss is documented.

- [ ] **A2. Sentinel/failover is missing.** [P0]
  There is no quorum failure detection, election, promotion, fencing, client
  rediscovery or split-brain handling. A single process is a single point of
  failure.
  **Closes when:** a Sentinel-compatible control plane or equivalent is
  implemented and tested with packet loss, partitions, stale configuration,
  flapping nodes, leader loss, promotion and reconnect; RPO/RTO are stated.

- [x] **A3. Cluster mode is missing.** [P0]
  **Complete (2026-11-26) by the criterion's exclusion branch: the profile
  explicitly excludes cluster clients.** There are no 16,384 slots, hash tags,
  `MOVED`/`ASK` redirects, migration/importing state, cluster bus or cross-slot
  rejection, and sharded Pub/Sub has no cluster routing. The profile and
  compatibility boundary now state that cluster clients are excluded and why.
  Evidence: `tests/boundaries.rs`
  `cluster_surface_is_disabled_and_hash_tags_are_literal` runs in default,
  fixed-one and four-worker modes, requires `-ERR This instance has cluster
  support disabled` for `INFO`, `MYID`, `SLOTS`, `SHARDS`, `NODES`, `KEYSLOT`,
  `COUNTKEYSINSLOT`, `GETKEYSINSLOT`, `ADDSLOTS`, `DELSLOTS`, `SETSLOT`,
  `RESET`, `FAILOVER` and `REPLICATE`, keeps `CLUSTER HELP` explanatory and the
  bare command an arity error, and proves `MGET` over different hash tags
  returns both local values with no `CROSSSLOT`, `MOVED` or `ASK`. This is an
  explicit exclusion, not cluster support, and does not imply topology discovery,
  partition handling or rolling upgrades (A4).- [ ] **A4. Rolling upgrade, rollback and mixed-version compatibility are missing.** [P0]
  There is no supported version-skew matrix, on-disk format migration policy,
  downgrade procedure or canary/rollback evidence. The package is still `0.1.0`.
  **Closes when:** the profile defines supported upgrade paths, format
  compatibility and rollback; CI exercises old/new combinations and a failed
  upgrade preserves recoverable data.

- [ ] **A5. Topology runbooks and failure tests are missing.** [P1]
  There is no multi-node deployment guide, failover runbook, partition/rejoin
  test, fencing-token design or split-brain detection. The Kubernetes example is
  intentionally a single replica.
  **Closes when:** tested runbooks cover node loss, network partition, disk
  failure, promotion, client reconnect and data reconciliation.

---

## 4. Security and transport

- [ ] **S1. Independent threat model and security review are missing.** [P0]
  No external or independent review covers authentication, authorization,
  transport, denial of service, audit custody, scripting, replication, cluster or
  supply chain. Passing compatibility tests are not a security sign-off.
  **Closes when:** a written threat model and an independent review/penetration
  test are completed, findings are tracked to closure, and the review scope and
  date are published.

- [ ] **S2. ACL support is not full Redis ACL.** [P0]
  Remaining subcommand-specific categories beyond the bounded overrides,
  dynamic key sandbox rules beyond the explicitly implemented command specs,
  complete channel filtering for `PUBSUB` introspection and commands outside the
  published table are absent. `KEYS`,
  `RANDOMKEY`, `DBSIZE`, `FLUSH*` and similar commands are not a per-key sandbox.
  **Closes when:** the supported ACL subset is explicitly versioned and
  differentially tested, or full category/subcommand/key/channel semantics are
  implemented; restricted users cannot enumerate or mutate keys outside policy.

- [x] **S3. The audit path is not a complete custody chain.** [P0]
  **Complete (2026-11-27) by the criterion's second branch: the audit path is
  explicitly best-effort with documented compensating controls.** Denial records
  are emitted to an optional local file and optional UDP syslog sink with no
  sender authentication, transport encryption, federation, delivery guarantee,
  retry, backpressure or aggregation; the collector never gates command
  execution and exits nonzero with a redacted count summary when its output
  fails. Per-line HMAC and a bounded keyring (first key signs, all verify) give
  tamper evidence, not confidentiality or delivery. The profile now states the
  boundary and the required compensating controls: an independently administered
  append-only collector with its own retention, HMAC key custody and rotation,
  restricted audit-file permissions, alerting on collector failure and on
  missing expected traffic, and reliance on durable primary sources for custody.
  Evidence: `tests/audit_syslog.rs`
  `best_effort_shipping_never_gates_commands_or_crashes_without_a_reader` drives
  128 denied commands with an unread sink and again after the sink is dropped,
  with allowed commands succeeding throughout; `tests/audit_hmac.rs`,
  `tests/audit_keyring.rs` and `tests/audit_collector.rs` cover signing, keyring
  verification/rotation and bounded collector failure reporting. Not claimed:
  authenticated/encrypted transport, durable delivery, aggregation or an
  independent custody chain.- [x] **S4. Cross-process rate limiting is missing.** [P0]
  **Complete (2026-11-26) by the external-control-plane branch of the
  criterion; no distributed limiter is implemented or claimed.** Failed-auth
  budgets remain process-local: `--auth-failure-limit` is per connection, and the
  opt-in `--auth-source-limit`/`--auth-global-limit` lockouts are per process over
  `--auth-source-window-ms`. The profile now requires an authoritative
  external limiter for any deployment with more than one process, states that it
  must run before `AUTH`, fail closed when unavailable, produce a visible
  rejection and be configured identically for every replica, and notes that the
  server budgets remain defence in depth. Shared-NAT throttling between clients
  is documented as a consequence rather than hidden. Evidence:
  `tests/auth_process_isolation.rs`
  `authentication_lockout_is_not_shared_between_processes` starts two real
  binaries with identical per-source budgets, locks 127.0.0.1 out of the first
  (including after reconnect), and shows the second still authenticates, keeps
  its own failure count and is unaffected. This does not claim username
  lockout, distributed counters, or that one process's budgets protect another.

- [ ] **S5. TLS hardening is incomplete.** [P0]
  TLS 1.3 is available with exact-name SNI selection, bounded operator-triggered
  atomic certificate/OCSP/CA/CRL rotation, and verified client-certificate mapping to
  enabled ACL users. Failed identity/CRL parsing retains the last valid generation;
  live connections keep their handshake-time snapshot. A configured DER CRL (up to
  1 MiB) checks mTLS client chains and fails closed for revoked, unknown or expired
  status; distribution points are not fetched. Native server-certificate OCSP
  stapling accepts bounded operator-provisioned response bytes per SNI/default
  identity and rotates them atomically with that identity. A bounded client-certificate
  mapping file can now rotate with the TLS generation; invalid/unknown ACL mappings
  retain the last good generation, and established sessions preserve their captured
  identity. Inline maps remain startup-only. rustls 0.23.46 treats
  response content as opaque, so the server does not validate response signature,
  status, certificate identity or freshness. Require an independently
  operated TLS terminator that validates and refreshes OCSP responses, fails closed
  when missing/stale, and prevents backend bypass for server-certificate revocation.
  Exact-name SNI, CRL parsing/revocation and atomic replacement have focused tests;
  independent TLS interoperability/security review remains open. No production-
  readiness claim is made. **Closes when:** server-certificate OCSP response
  validation and tested stale/missing failure policy are implemented or supplied
  by the reviewed external terminator boundary, and the configured human security
  review is complete.

- [x] **S6. Live deauthentication/session revocation is missing.** [P1]
  **Complete (2026-11-14): documented and tested administrative revocation.**
  `CLIENT KILL USER <name>` closes every matching authenticated session, returns
  the number closed and skips the caller by default; ID/ADDR/LADDR/MAXAGE/TYPE
  filters cover narrower clients. The rotation runbook now requires
  rotate-then-revoke, and `tests/acl.rs::client_kill_user_revokes_live_sessions`
  proves the closed session, the retained unrelated session and the reconnect
  path. **Retained limitation:** revocation is operator-driven; changing
  `requirepass` or an ACL entry does not automatically deauthenticate existing
  connections, as in Redis.

- [ ] **S7. Tenant isolation is absent.** [P1]
  All ACL users share one process, keyspace, memory budget and CPU. ACL rules do
  not create separate databases, quotas or network identities.
  **Closes when:** multi-tenant deployment is explicitly excluded, or per-tenant
  isolation is implemented and tested.

- [ ] **S8. Secret handling needs hardening.** [P1]
  Passwords can be passed on the command line, ACL hashes are unsalted
  SHA-256 (Redis-compatible but fast to brute-force offline), and there is no
  secret-manager integration or complete file-permission/rotation audit.
  **Partial (2026-11-14): optional owner-only secret-file enforcement.**
  `--secure-secret-files` refuses group/other-accessible password, ACL, audit
  HMAC/keyring, AOF HMAC and TLS private-key files on Unix and enforces a
  16-byte minimum password, and the security guide documents modes, a 256-bit
  random-secret requirement, process-argument risk and rotate-then-revoke. Unit
  tests cover loose/strict modes, short/long passwords and CLI parsing.
  `--check-config --secure-secret-files` also checks Unix secret-file permissions
  before startup, with redacted diagnostics suitable for preflight logs.
  A startup file can opt in with `secure-secret-files yes`; the effective
  setting follows file/CLI precedence and does not change the default.
  Raw audit HMAC key files now have a byte-read bound shared by the server
  and collector; this does not set a file I/O deadline or change rotation.
  Secret-manager integration, enforced entropy/length for `--requirepass`, and
  automatic audit of every deployment's permissions remain open.
  **Closes when:** the supported secret sources, permissions, rotation and
  process-argument risks are documented and tested, with long random secrets
  required by deployment guidance or enforced.

- [ ] **S9. Denial-of-service and slow-client hardening is incomplete.** [P0]
  There are client/payload/frame limits and bounded output backpressure, but no
  slow-client thresholds, global connection/command budgets, per-user quotas or
  adversarial protocol fuzzing. O(N) commands can occupy the owner.
  **Partial (2026-11-14): deterministic RESP fuzz/property corpus.**
  `tests/protocol_fuzz.rs` now drives generated binary frames through every
  fragmentation boundary, random byte streams under tight frame/bulk/argument
  limits, random inline input, and multi-command pipelines. It asserts bounded
  consumption, at-most-once execution, argument/limit bounds and no panic; every
  case is reproducible from its seed. A configurable monotonic per-incomplete-
  plaintext-frame assembly deadline (60-second default, zero disables) now
  complements socket inactivity timeouts across synchronous workers, readiness,
  Tokio and TLS plaintext parsing. **Partial (2026-11-18):** complete requests no
  longer arm phantom assembly timers on their empty parser suffixes; direct
  owner/worker tests and idle-past-frame-budget wire cases cover owner/shared
  reactors, fixed workers and Tokio. Partial suffixes retain their original
  deadline. It does not bound TLS records that produce no plaintext.
  The native TLS pipeline case in V4 checks reply progress with an incomplete
  request, not steady-state slow-client limits or overload shedding. D7 also
  covers abandonment of an unread native TLS reply during shutdown; that is
  not evidence of overload shedding during normal service.
  Slow-client length/rate thresholds, global/user budgets, CPU quotas
  and a general overload shedding contract remain open.
  **Closes when:** overload/slow-client policy is enforced and tested, fuzz/
  property tests cover framing and command parsing, and a documented load
  shedding behavior exists.

- [ ] **S10. Script, replication and cluster security do not exist.** [P1]
  These features are absent, so their sandbox, credential, authorization and
  network trust boundaries are also absent. They must not be claimed as secure
  once added.
  **Closes when:** each added feature has an explicit threat model, authentication/
  authorization path and negative tests.

---

## 5. Observability and operations

- [ ] **O1. True memory accounting and enforcement are missing.** [P0]
  `maxmemory` is a key/value payload budget, not RSS or allocator memory. Keys,
  maps, indexes, thread stacks, connection buffers, kernel sockets and allocator
  fragmentation can make actual memory much larger; the kernel can OOM-kill the
  process.
  **Partial (2026-11-13): process RSS is observable.** `INFO memory` and
  `MEMORY STATS` report `used_memory_rss`, `used_memory_rss_human` and an explicit
  scope from Linux `/proc/self/status`; unsupported platforms report unavailable
  instead of a fake value. `INFO memory` and `MEMORY STATS` also report cgroup
  v2/v1 current and limit bytes plus an explicit scope when readable. Both are observational only
  and are not used for admission, eviction or OOM handling. Allocator
  accounting, enforced cgroup/headroom contracts and OS OOM behavior remain
  absent. `INFO memory` reports remaining logical key/value capacity separately;
  tests cover filling the payload budget, rejecting a write and freeing capacity.
  **Partial (2026-11-19):** persistent per-owner `WATCH` history now caps logical
  retained key bytes at 4 MiB (in addition to its entry-count cap), invalidating
  outstanding watches conservatively on overflow. This excludes hash-table
  allocation and all other process memory; transaction scratch has request-local
  bounds and is not covered by this history budget. This does not close O1.
  **Closes when:** operators can observe and enforce a documented memory bound
  (cgroup/RSS/allocator) with headroom guidance, and OOM behavior is tested.

- [ ] **O2. LFU and read-idle statistics are missing.** [P1]
  `OBJECT IDLETIME` returns an explicit unsupported error, `OBJECT FREQ` reports
  that LFU is not selected, and no LFU eviction policy exists. The current LRU is
  a mutation-order approximation.
  **Closes when:** the selected eviction/access policy is implemented and
  observable, or `OBJECT FREQ`/`IDLETIME` remain explicit exclusions in the
  profile.

- [ ] **O3. Latency distribution and per-client observability are missing.** [P1]
  `INFO` has aggregate command stats but no exact histograms, complete
  per-client resource fields or a cross-process latency view. `LATENCY` exposes
  only command and expire-cycle series; `LATENCY GRAPH` is unsupported and
  `LATENCY HISTOGRAM` reports the same bounded 32-bucket approximation as
  `INFO latencystats`, not exact HDR percentiles.
  **Partial (2026-11-20): bounded client subscription state.** `CLIENT INFO`
  and `CLIENT LIST` report distinct live `sub` (channel), `psub` (pattern) and
  `ssub` (shard-channel) counts; shard-only subscribers retain the `pubsub`
  type and `P` flag. `CLIENT INFO` also reports `age`, `b`/`P`/`N` flags and the
  WATCH count, using state already held by the session. `INFO clients` already reports
  `blocked_clients`. Per-client fd, idle time, last command, input/output buffer
  bytes, network bytes and allocator memory remain absent.
  **Partial (2026-11-14): bounded latency percentiles.** Enabling
  `commandstats-enabled` also records a 32-bucket logarithmic per-command
  histogram; `INFO latencystats` reports `p50`/`p99`/`p99.9` bucket upper bounds
  in microseconds. This is an approximation, not HDR or an exact percentile
  contract. The dispatcher takes no timer or histogram update while the
  collector is disabled.
  **Closes when:** the supported metrics contract reports per-command latency
  distributions and per-client resource/queue state with documented units and
  sampling limits.

- [ ] **O4. Operational telemetry and alerting are missing.** [P1]
  There is no metrics exporter, OpenTelemetry, structured logging, SLO dashboard,
  alert set, persistence/auth/audit telemetry, replay/rewrite duration or
  network-byte accounting. Default health is a PING probe; optional journal
  probing checks reported status only, not disk writability or recovery readiness.
  **Partial: authenticated request-service probe.** The built-in probe accepts
  a bounded password file and an optional named ACL user (permitting PING-only
  probe credentials), validates AUTH and PING authorization under one socket
  deadline and keeps credentials/server error payloads out of diagnostics.
  **Partial: opt-in journal-status and temporary-write probes.**
  `--healthcheck-require-aof` adds a bounded INFO persistence check for
  loading=0, enabled journal and healthy last write/rewrite statuses;
  missing/duplicate fields, reported errors, denied INFO and malformed replies
  fail closed. Fixed diagnostics distinguish reported loading, disabled AOF,
  last-write failure and last-rewrite failure without logging reply values.
  `--healthcheck-aof-write-probe DIR` additionally creates, writes,
  syncs and removes a unique one-byte temporary file in the operator-selected
  directory, after the status check. Unit fault-response and all-mode CLI tests
  cover both contracts, including TLS. The write probe is point-in-time and may
  block independently of the socket deadline; it does not test appending the
  live journal, future capacity, directory-entry durability or recovery.
  **Partial: verified TLS request-service probe.** An explicit bounded PEM CA
  bundle enables TLS 1.3 with chain and DNS/IP identity verification before AUTH,
  under the shared socket deadline; there is no insecure or plaintext fallback.
  CLI tests in all worker modes cover ACL/journal probing over TLS and wrong
  trust/name, malformed CA and client-certificate-required failures; a unit test
  checks the silent-handshake deadline.
  Client-certificate probes, full journal readiness and operational telemetry
  remain open. In particular, the temporary-write probe is not durability or
  recovery validation.
  **Closes when:** a redacted, documented metrics/log/health contract exists and
  is tested for overload, persistence failure, auth failure and shutdown.

- [x] **O5. Configuration reload is missing.** [P0]
  **Complete (2026-11-27): mutable settings reload atomically, immutable changes
  are rejected cleanly, and a failed reload retains the last valid
  configuration.** `CONFIG SET` stages a candidate generation and validates every
  name/value pair before publication, so a batch applies completely or not at
  all. The profile now lists the mutable subset and the startup-only surface
  (listeners, TLS material, worker topology, `dir`,
  `appendonly`/`appendfilename`, ACL/audit/secret paths, limits, shutdown
  deadline, `config-version`), and documents that startup-only or unknown names
  return explicit errors (`appendonly and appendfilename are startup-only;
  restart with the desired options`, `CONFIG SET option is unsupported or
  invalid`) and that `CONFIG REWRITE` reports the startup file as read-only.
  Evidence: `tests/runtime_config.rs`
  `mutable_settings_reload_atomically_and_startup_only_names_fail_closed` runs in
  default, fixed-one and four-worker modes: a two-setting batch applies
  together; a batch mixing a valid value with `appendfilename` is rejected with
  both prior values intact; a batch with a negative listpack value is rejected
  with both prior values intact; `CONFIG REWRITE` fails explicitly; the server
  keeps serving. TLS retains its separately documented atomic generation reload.
  Not claimed: file-triggered reload, `CONFIG REWRITE`, or mutability beyond the
  documented subset.- [ ] **O6. Slow-client and queue policy is incomplete.** [P1]
  Output is backpressured and bounded by connection limits, but there are no
  configurable slow-client thresholds, kill policy or fair-share guarantees for
  large replies, subscribers and blocked clients.
  **Closes when:** queue/admission policy is documented, observable and tested
  under slow readers, pub/sub fan-out and large multi-key replies.

- [ ] **O7. Expiry, eviction and pause interactions are incomplete.** [P1]
  Active expiration, payload-budget eviction and `CLIENT PAUSE` exist, but there
  is no background eviction, and the interaction with writes, blocked clients,
  notifications and a future replica pause is not a complete semantic contract.
  An injected-clock test exercises expiry maintenance during both pause modes:
  it observes one expiry event before unpause and verifies a deferred `SET NX`
  sees the newly absent key. A wire test also checks that unrelated eviction
  increments `evicted_keys` without completing a `BLPOP` waiter; a later push
  completes the wait and clears `blocked_clients`. Replica pause is not
  applicable until replication exists.
  **Closes when:** background eviction and the supported interaction matrix are
  documented and tested, including expiry under pause, eviction during blocked
  commands and ordering.

- [ ] **O8. Upgrade, rollback and incident runbooks are missing.** [P1]
  A [same-binary offline rollback drill](persistence.md#drill-a-rollback-to-an-offline-checkpoint)
  now documents isolation, non-overwrite and state verification. Deployment
  drills, version upgrade/downgrade, format migration, disk-full recovery, OOM,
  certificate rotation, ACL lockout and data corruption remain open here.
  **Closes when:** runbooks are published and exercised in a staging drill, with
  expected data-loss and recovery bounds.

- [ ] **O9. Long-running soak and fault evidence is missing.** [P1]
  No extended mixed-workload soak covers memory growth, expiry churn, reconnect
  storms, slow clients, persistence rewrites, credential rotation or
  termination during writes.
  **Closes when:** a repeatable soak/fault suite runs for the supported duration
  and retains throughput, latency, memory, CPU, queue and recovery evidence.

---

## 6. Resource safety and scalability

- [x] **R1. The default data path is single-owner.** [P0]
  **Complete (2026-11-27) by the criterion's first branch: the profile states a
  single-owner capacity limit.** The default `Config` keeps `storage_owners`
  unset, so data commands run on one owner thread and command-execution
  parallelism is one CPU. `--storage-owners 2..=64` remains opt-in experimental:
  its own measurements are below the one-owner path at pipeline 16 on the test
  host, no protected benchmark gates multi-owner mode, and no scaling claim is
  made. The profile now declares the enforced capacity envelope (one owner; 128
  admitted clients by default with an explicit `max number of clients reached`
  refusal; 64 MiB logical payload refused without partial application; 1 MiB
  argument and 2 MiB frame bounds) and states that actual capacity is whichever
  limit is reached first, excluding metadata, buffers, sockets and allocator
  overhead from the payload budget. Evidence: `tests/owner_capacity.rs`
  `the_declared_single_owner_envelope_is_the_default_and_enforced` asserts the
  single-owner/128-client defaults and, in default, fixed-one and four-worker
  modes, the explicit third-client refusal, continued service for admitted
  clients and capacity release after a close; `tests/maxclients_info.rs` and the
  storage-ownership tests cover reporting and owner-thread isolation. Not
  claimed: a throughput SLO, multi-owner scaling, or cross-owner capacity
  sharing.- [ ] **R2. The async Tokio transport is not promoted.** [P1]
  The opt-in Tokio mode previously passed 18/20 protected conditions and was not
  promoted; default Mio/fixed-worker paths retain polling tradeoffs. No readiness
  or all-workload win is claimed.
  **Closes when:** the async path clears the same gates as the default and is
  selected by policy, or it is explicitly a non-default experimental option.

- [ ] **R3. Fixed-worker and patient polling policy lacks a hard latency contract.** [P1]
  Fixed workers use nonblocking polling with a 1 ms idle wait, a bounded active
  window and cooperative yields. Blocking commands poll about every 10 ms.
  These are efficiency choices, not OS readiness or real-time deadlines.
  **Closes when:** the supported profile states the latency/CPU tradeoff and
  tests it under idle, mixed and oversubscribed clients, or a readiness-based
  path replaces it.

- [ ] **R4. O(N) commands can occupy the owner without a CPU budget.** [P1]
  `KEYS`, `SORT`, `SCAN` snapshots, geospatial searches, large collection
  operations and full-state rewrites can hold the execution turn for a long time.
  There is no per-command deadline, cancellation or CPU quota.
  **Closes when:** bounded-work limits and cancellation/timeout behavior are
  defined for the profile, or operators have tested capacity guidance that keeps
  these commands within the SLO.

- [ ] **R5. Connection/worker admission and per-user quotas are incomplete.** [P1]
  Client count, payload, frame and argument limits exist. Authenticated ACL users
  now have an optional `maxconnections` cap, but there is no per-user command or
  memory quota, CPU fairness guarantee, slow-client share, or cap for unauthenticated
  sessions using an implicit no-password default. **Partial:** S122 implements
  atomic authenticated-session admission and release; it does not isolate shared
  resource use. **Closes when:** the supported admission policy is explicit,
  observable and tested under overload and multi-tenant use, including the remaining
  resource and unauthenticated-session boundaries.

- [ ] **R6. Persistence serializes the whole command path.** [P1]
  With persistence enabled, commands execute through the exclusive transaction
  gate so mutation, capture and append keep one order. This is not a performance
  design for production write rates.
  **Closes when:** durability and throughput are measured for the supported
  profile and an accepted SLO, or a concurrent/ordered design is implemented.

---

## 7. Performance evidence

- [ ] **P1. The benchmark scope is not representative.** [P1]
  The protected gates use PING, SET, GET, NX-existing and XX-existing with
  64-byte values, 1024 cyclic keys, 8 clients and pipelines 1/16. There are no
  mixed read/write, TTL churn, collection, stream, pub/sub, large-value,
  persistence, TLS or multi-owner production workloads.
  **Closes when:** a realistic workload suite covers the declared profile and is
  reported separately from the narrow merge gate.

- [ ] **P2. Controlled-hardware and SLO evidence is missing.** [P1]
  Most comparisons run on shared GitHub runners. The policy itself says this is
  not stable hardware or an SLO. There is no documented capacity plan for
  memory, CPU, connections, queues or tail latency.
  **Closes when:** repeatable controlled-hardware runs and stated SLOs exist for
  the supported deployment profile.

- [ ] **P3. Tail latency is reported as batch p99 only.** [P1]
  The harness reports whole-batch p99 and warns not to divide by pipeline length.
  There is no per-request p50/p95/p99/p999 distribution, coordinated-omission
  handling or client-saturation analysis.
  **Closes when:** per-request and batch tails are measured separately and the
  SLO uses the correct unit.

- [ ] **P4. Durability and topology performance are unmeasured.** [P1]
  There is no evidence for fsync policies, rewrite pauses, replication lag,
  failover time or cluster migration. These features are largely absent, so the
  performance gate cannot cover them.
  **Closes when:** each supported durability/topology mode has matched
  throughput, tail, CPU, memory and recovery measurements.

- [ ] **P5. Non-default modes are outside the protected gate.** [P1]
  The gates measure default plain-TCP Rillcache. TLS, persistence, Tokio,
  multi-owner, fixed workers and container modes are not required to win or even
  stay within a regression budget.
  **Closes when:** the profile's production modes have their own regression
  budgets and evidence, or they are explicitly non-production experiments.

- [ ] **P6. Long-running performance and regression budgets are missing.** [P1]
  Three short repetitions per condition are not a soak or a distribution. There
  is no memory-leak trend, multi-hour stability or regression budget outside the
  merge gate.
  **Closes when:** longer runs and trend evidence are retained for the supported
  workload.

- [ ] **P7. The competitor/version/client matrix is narrow.** [P1]
  Pins exist for selected Redis, Valkey and Dragonfly versions, but there is no
  matrix across supported upstream releases, client libraries or hardware
  generations, and no client-side saturation model.
  **Closes when:** the supported version matrix and client load model are
  published and exercised.

---

## 8. Distribution, release and legal

- [ ] **L1. The project has no license.** [P0]
  There is no root `LICENSE`, `Cargo.toml` has no `license`/`license-file`, and
  `container/notices/README.txt` explicitly says the third-party notices do not
  assign a license to Rillcache. This blocks lawful redistribution outside the
  owner's control.
  **Closes when:** a project license is chosen, added to the repository and
  package metadata, and third-party notice obligations are reviewed.

- [ ] **L2. There is no stable release or support policy.** [P0]
  The package is `0.1.0`; `latest` is a convenience main-build tag, not a stable
  release. There is no semantic-version policy, support window, deprecation
  process or compatibility guarantee.
  **Closes when:** a versioned release, support policy and upgrade/deprecation
  contract are published.

- [ ] **L3. Image signing, SBOM and provenance are missing.** [P1]
  **Partial, awaiting protected review and first release evidence:** the container
  workflow now scans each exact tested platform image into an SPDX SBOM, publishes
  signed SBOM attestations for both platform digests and SLSA provenance for the
  multi-platform index, then verifies them from a fresh read-only job after an
  anonymous pull. Mutable aliases are promoted only after signing and must resolve
  to the tested digest only after a fresh read-only consumer verifies the
  attestations and anonymously pulls/runs the image. `SECURITY.md` documents
  private reporting and best-effort
  triage/disclosure; GitHub private vulnerability reporting is enabled. No main
  run has yet exercised this changed workflow, and no vulnerability scan, license
  review, supported release policy, or independent security sign-off is claimed.
  **Closes when:** a protected post-merge publication run records the exact index
  and child digests, the anonymous consumer verifies provenance and both SBOM
  attestations, the vulnerability-response route is confirmed, and any required
  reviewer decision and remaining scan/license disposition are recorded.

- [ ] **L4. Release notes and migration guidance are missing.** [P1]
  There is no changelog, compatibility delta, migration guide or rollback note
  for data/format changes.
  **Closes when:** each release carries tested upgrade/rollback and migration
  notes, including journal/format changes.

- [ ] **L5. The supported platform matrix is undefined.** [P1]
  CI targets Linux; the README mentions a non-Unix portable fallback, but there is
  no supported OS/architecture/libc matrix, arm64 performance evidence or
  filesystem contract.
  **Closes when:** supported platforms are listed and tested, and unsupported
  ones fail or are documented.

- [ ] **L6. Production deployment artifacts are incomplete.** [P1]
  There are restricted Docker/Compose/Kubernetes examples but no HA topology,
  backup job, certificate rotation, systemd unit, Helm chart/operator, or
  production configuration profiles. The optional Kubernetes password overlay
  wires the same read-only Secret into the server and both health probes; it
  does not add persistence, automatic rotation or live cluster evidence.
  **Closes when:** the profile's deployment path has reviewed manifests and
  runbooks; examples do not imply unsupported HA/persistence guarantees.

- [ ] **L7. First-package visibility is a manual release step.** [P2]
  GHCR packages are private by default; the documented fix is a manual package
  settings action. A release can therefore pass automation while remaining
  unpullable.
  **Closes when:** a release checklist and verification job make public
  visibility an explicit, checked step.

- [ ] **L8. Build reproducibility is asserted but not independently verified.** [P2]
  Dependencies are vendored and pins exist, but there is no reproducible-build
  attestation or independent rebuild comparison for release binaries/images.
  **Closes when:** a clean rebuild reproduces the published digest or the
  difference is explained and accepted.

---

## 9. Verification and documentation integrity

- [ ] **V1. Only a narrow set of upstream Tcl files is exercised.** [P0]
  The compatibility workflow runs Redis/Valkey `unit/type/string`,
  `unit/type/incr` and `unit/keyspace` plus `unit/type/set`, `unit/bitfield`,
  `unit/scan`, `unit/quit` and `unit/geo` embedded in each required keyspace
  context. Modules, cluster, integration,
  scripting, most data types and many protocol surfaces are outside it, and
  several tags/units are excluded.
  **Closes when:** the supported profile is covered by the upstream suite (or an
  equivalent differential corpus) with explicit, reviewed exclusions.

- [ ] **V2. The differential corpus is incomplete.** [P0]
  `tests/reference_baseline.rs` is a small optional string/RESP corpus. It now
  also checks deterministic hash, set, sorted-set and stream replies against the
  pinned native reference. Expiry coverage includes NX/XX/GT/LT conditions,
  equal and persistent deadlines, PERSIST/GETEX, SET KEEPTTL, APPEND, COPY/RENAME
  deadline propagation, immediate deletion and TTL sentinels, exercised against
  Redis 8.10.2 and Valkey 9.1.2 without timing sleeps. Two-client transaction
  cases also verify queue visibility, ordered EXEC results and runtime errors,
  DISCARD, queue-time EXECABORT, WATCH conflicts (including missing-key creation)
  and watch reset after abort against those pinned versions. A binary Pub/Sub
  corpus checks distinct active pattern counts in `NUMPAT` and `INFO STATS`
  across duplicate subscriptions, shared patterns and last unsubscribe. The
  corpus also checks `MSETNX` and `HSETNX` with binary data, rejected writes,
  wrong types and preserved absolute deadlines. Broader Pub/Sub behavior,
  ACL, TLS, persistence, exhaustive transaction/error cases
  and full command coverage remain open.
  **Closes when:** every supported command/type/edge has a differential test
  against the pinned versions, including binary, expiry, wrong-type, OOM and
  transaction cases.

- [ ] **V3. Independent human validation is missing.** [P0]
  The project explicitly relies on AI-generated code and human test-based
  validation, but there is no record of independent design, compatibility,
  performance or security acceptance.
  **Closes when:** named reviewers sign off the production profile, security
  review, benchmark evidence and recovery drills.

- [ ] **V4. Fuzz, property and model-based testing are missing.** [P1]
  There is no protocol/parser fuzzing, property-based state-machine testing,
  randomized model checking or sustained adversarial input corpus.
  **Partial (2026-11-14): deterministic RESP corpus.** `tests/protocol_fuzz.rs`
  adds seeded binary-frame fragmentation, random bytes under tight limits,
  random inline input and pipeline consumption properties; it runs in ordinary
  CI and every failure is reproducible from its printed seed. **Partial
  (2026-11-19):** a second seeded property now feeds multi-command binary
  pipelines through one-byte and varied-size read chunks, retaining incomplete
  suffixes and asserting exact-once dispatch, frame consumption and command
  order against an independent decoded-command model. That test models parser
  buffer behavior. A separate test in `tests/native_tls.rs` flushes valid binary
  request fragments over TLS and checks ordered replies before and after a
  partial request completes. This does not cover arbitrary TCP segmentation
  or sustained adversarial input.
  **Partial (2026-11-14): hash-field expiry state model.** A fixed-seed command
  sequence compares HSET, HPEXPIRE, HGET, HPTTL and HDEL against an independent
  map oracle under an injected clock; failures identify the seed and step.
  Broader state-machine model checking, command-level differential properties
  and sustained adversarial corpora remain absent.
  **Closes when:** those suites run in CI or a scheduled job and retain failures.

- [ ] **V5. Public documentation can still drift from executable behavior.** [P1]
  This change corrected known contradictions: sharded Pub/Sub and keyspace
  notifications were described as absent after implementation; `BZMPOP`,
  `SUBSTR` and AOF compaction were described as unsupported; rustls was described
  as the sole dependency exception; and multi-owner execution was described as
  unimplemented. `scripts/production_readiness.py --check` verifies this
  inventory and its README links. Broader claim-to-test validation and CI
  integration remain open.
  **Closes when:** a documentation/link/claim check runs in CI and this canonical
  file is linked from the README and indexes.

- [ ] **V6. There is no production acceptance runbook.** [P1]
  The [acceptance runbook](production-readiness/acceptance-runbook.md) and its
  blocker preflight now define how to collect and record profile-specific
  evidence. Independent review and execution against a release candidate are
  still required.
  **Closes when:** a reviewed acceptance runbook exists and has been executed
  against a release candidate.

- [ ] **V7. Quality gates are scoped to the merge policy, not production.** [P1]
  The protected 20-case and 10-case Valkey gates are code-quality merge criteria
  on shared CI. They are not a compatibility suite, SLO, durability proof,
  security review or production benchmark.
  **Closes when:** the release process adds the missing gates and states clearly
  which gate proves which property.

---

## Minimum path to a production claim

At minimum, close every **P0** item above, plus either close or explicitly
exclude with a reviewed profile entry every **P1** item that applies to the
declared deployment. The sequence should be:

1. Define the compatibility profile and license (C1, L1, L2).
2. Stabilize the durable format and recovery contract (D1-D7, D11) with
   crash/fault evidence.
3. Add the topology required by the profile (A1-A4) or explicitly declare a
   single-node, non-HA profile with RPO/RTO.
4. Close the security review and hardening path (S1-S5, S9).
5. Add memory/observability/configuration operations (O1-O9, R1-R6).
6. Produce realistic controlled-hardware performance and acceptance evidence
   (P1-P7, V1-V7).
7. Re-run this entire list against the release candidate; do not reuse a prior
   pass for a changed head.

## References

- [Production readiness plan](production-readiness.md)
- [Requirements index](requirements.md)
- [Compatibility boundary](requirements/compatibility.md)
- [Durability and recovery plan](plans/durability-and-recovery.md)
- [Typed storage, transactions and topology plan](plans/types-transactions-topology.md)
- [Security and operations plan](plans/security-and-operations.md)
- [Multi-owner execution limits](plans/multi-owner-execution.md)
- [PR quality gate](pr-quality-gate.md)
- [Benchmark methodology](benchmarks.md)
- [Container delivery](containers.md)
