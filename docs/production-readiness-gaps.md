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
| Compatibility and command surface | Partial | Scripting, RDB, replication, cluster, multiple databases, bounded command and protocol coverage |
| Durability and recovery | Partial / deferred | No RDB or interoperable AOF, no online backup/PITR/retention, no ENOSPC evidence |
| Availability and topology | Missing | No replication, failover, Sentinel or cluster |
| Security and transport | Partial | No independent review, full ACLs, cross-process rate limiting, TLS hardening or audited collector federation |
| Observability and operations | Partial | Payload-only memory reporting, no RSS/allocator accounting, no metrics/log/SLO contract |
| Resource safety and scalability | Partial | One-owner default, opt-in multi-owner is slower in its own measurements, no RSS bound |
| Performance evidence | Evidence gap | Narrow shared-CI microbenchmarks, no realistic workload, controlled hardware or SLOs |
| Distribution, release and legal | Partial | No project license, stable support policy, image signing/SBOM or migration policy |
| Verification and documentation | Partial | Only three upstream Tcl files, no fuzz/soak, stale/contradictory public claims |

---

## 1. Compatibility and command surface

- [ ] **C1. The production compatibility profile is undefined.** [P0]
  There is no published matrix of supported Redis/Valkey versions, commands,
  client libraries, topology, persistence mode and exclusions. "Drop-in" is
  therefore untestable and ambiguous.
  **Partial (2026-11-14): draft standalone profile.** The
  [production profile](production-profile.md) now declares the reference
  versions, topology, supported families, explicit exclusions, persistence and
  security boundaries, client matrix and limits. It is explicitly a draft, not a
  readiness claim. The command-by-command CI matrix that runs every profile row
  against pinned Redis and Valkey is still absent, as are the language-client,
  durability, security and SLO evidence rows.
  **Closes when:** a versioned profile lists exact upstream versions, command
  matrix, client libraries, deployment topology and excluded features; CI runs
  every row against pinned Redis and Valkey and records pass/fail.

- [ ] **C2. Scripting and functions are missing.** [P0]
  `EVAL`, `EVALSHA`, `SCRIPT LOAD|EXISTS|DEBUG|KILL`, `FUNCTION
  LOAD|DELETE|LIST|STATS|DUMP|RESTORE|KEYS` and `FCALL`/`FCALL_RO` are absent.
  Only `SCRIPT FLUSH` and `FUNCTION FLUSH` return `+OK` for an empty registry.
  Native TCP verification covers those no-ops, execution command discovery,
  and rejection of execution and representative registry subcommands. The
  dependency policy does not permit a Lua runtime.
  **Closes when:** a reviewed runtime decision exists; scripts/functions execute
  atomically with bounded CPU/memory/instruction use, deterministic journal and
  replication behavior, exact error/result mapping and no filesystem/network
  escape; or the profile explicitly excludes scripting and clients are documented
  as unsupported rather than silently failing.

- [ ] **C3. Module loading is missing.** [P1]
  `MODULE LIST` is empty and `MODULE LOAD`/`LOADEX`/`UNLOAD` return explicit
  unsupported errors. This is acceptable only if the production profile excludes
  modules and dependent clients.
  **Closes when:** module support is implemented and security-reviewed, or the
  exclusion is explicit in the profile and compatibility matrix.

- [ ] **C4. Multiple logical databases are missing.** [P0]
  Only database 0 exists. `SELECT` accepts 0, `MOVE`/`SWAPDB` return a multi-database
  error, and `COPY ... DB n` rejects nonzero databases. A two-client TCP test
  verifies that rejection leaves binary source and destination data and the
  key count unchanged, and that DB-0 COPY and FLUSHDB affect both clients'
  shared keyspace. This is native wire evidence, not a language-client matrix.
  **Closes when:** the profile either supports a validated number of databases
  with per-database keyspaces, `SELECT`/`MOVE`/`SWAPDB`/`FLUSHDB` semantics and
  per-database observability, or explicitly excludes clients that require them.

- [ ] **C5. The command and subcommand surface is incomplete.** [P0]
  The registry contains 243 commands, but common surfaces are absent or bounded:
  `DUMP`/`RESTORE`/`MIGRATE`; `OBJECT FREQ|IDLETIME` (explicit unsupported);
  `PFDEBUG`/`PFSELFTEST`; `CLIENT TRACKING|NO-TOUCH`;
  `COMMAND DOCS`; `CONFIG REWRITE`; `LATENCY GRAPH|HISTOGRAM`; and Redis 8+ command extensions. Some return explicit
  errors, others are unknown commands.
  **Partial (2026-11-14): named exclusions for common surfaces.**
  `DUMP`/`RESTORE`/`MIGRATE`, `PFDEBUG`/`PFSELFTEST`, `COMMAND DOCS`,
  `CONFIG REWRITE` now returns a specific
  capability error with normal arity validation, documented in
  [unsupported surfaces](unsupported.md). `OBJECT IDLETIME|FREQ` and
  `LATENCY GRAPH|HISTOGRAM` were already explicit. `CLIENT REPLY ON|OFF|SKIP`
  now suppresses immediate, paused and blocked-completion replies (RESET
  restores `ON`); `CLIENT NO-EVICT ON|OFF` is accepted because established
  clients are never evicted; and `CLIENT GETREDIR`/`TRACKINGINFO`/`CACHING`
  report the disabled tracking state. `CLIENT TRACKING|NO-TOUCH`; `MEMORY PURGE` is an accepted
  no-op and `MEMORY MALLOC-STATS` reports the system-allocator boundary; Redis 8+
  extensions and the command-by-command differential matrix remain missing.
  **Closes when:** a command-by-command differential matrix exists for supported
  data types and admin surfaces; every absent command is either implemented or
  explicitly excluded with a client-facing error and profile entry.

- [ ] **C6. RESP3 and session behavior are a bounded subset.** [P1]
  RESP3 is limited to the current commands' simple strings, errors, integers,
  bulk strings/arrays and null frames. There is no full RESP3 type coverage,
  client tracking, full push/attribute semantics, or multiple-database select.
  Empty/null request arrays are rejected.
  **Closes when:** every RESP3 type and session feature used by the supported
  client matrix has a documented, tested behavior, including malformed and
  fragmented frames.

- [ ] **C7. Real client-library compatibility is incomplete.** [P0]
  Pinned `redis-cli` RESP2/RESP3/AUTH and metadata interop is covered.
  **Partial (2026-11-14):** the optional standalone
  [redis-py 5.2.1 harness](redis-py-conformance.md) exercises RESP2/RESP3,
  named ACL authentication, binary strings, pipelines/transactions, Pub/Sub and
  reconnect against managed candidate servers in three synchronous modes.
  This bounded harness is not a CI gate. TLS identity, negative/error paths,
  Redis-version coverage, node-redis, go-redis and broader client behavior remain
  unverified; arbitrary clients may issue unsupported discovery commands.
  **Closes when:** a versioned language-client matrix runs handshake, AUTH/ACL,
  RESP2/RESP3, pipelines, pub/sub, TLS identity, reconnect and error-path tests
  against the supported profile, including redis-py, node-redis and go-redis.

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

- [ ] **C10. HyperLogLog is not Redis-interoperable.** [P1]
  The private payload uses an exact hash set then a 2^14-register sketch, not
  Redis's sparse/dense encoding. `PFDEBUG`, `PFSELFTEST` and sparse-threshold
  configuration are absent.
  **Closes when:** the profile either implements Redis/Valkey-compatible HLL
  payloads and diagnostics, or explicitly excludes HLL persistence/interchange
  and documents the accuracy/representation differences.

- [ ] **C11. Geospatial search is linear.** [P2]
  `GEOSEARCH` and the legacy radius aliases scan the source sorted set linearly;
  there is no native geohash neighbour index. This is correct for small sets but
  is not evidence for large production keyspaces.
  **Closes when:** complexity and memory are documented and benchmarked at the
  target scale, with a native index or an accepted latency limit.

- [ ] **C12. Pub/Sub is deliberately non-durable.** [P1]
  Delivery is in-process, at-most-once, unordered across publishers, bounded by
  per-client queues and not persisted or replicated. Sharded channels are a
  separate standalone namespace with no cluster hash-slot routing.
  **Closes when:** the profile documents this boundary and uses Streams or an
  external transport for reliable delivery; or durable/replicated delivery is
  implemented and tested.

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

- [ ] **D1. RDB, DUMP/RESTORE and migration interoperability are missing.** [P0]
  `SAVE`/`BGSAVE` return explicit RDB-unsupported errors, `LASTSAVE` reports 0,
  and `DUMP`/`RESTORE`/`MIGRATE` are absent. The append-only journal is a private
  RESP command stream, not multipart AOF or RDB.
  **Closes when:** pinned Redis/Valkey RDB import/export and `DUMP`/`RESTORE` work
  for the declared types/versions with checksums, fail-closed behavior and
  bidirectional native tests; or the profile explicitly excludes RDB/AOF
  interchange and provides a supported migration tool.

- [ ] **D2. AOF rewrite is synchronous and blocks command service.** [P0]
  **Partial (2026-11-05): cooperative rewrite implemented.** `BGREWRITEAOF` now
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
  **Remaining:** no fork or copy-on-write, so the start pause is O(key names) and
  enormous keyspaces still need a measured pause SLO; a sustained write storm
  lengthens reconciliation rounds, so byte progress has no fixed total and
  cannot report a percentage. RDB and online backup remain separate items.
  **Closes when:** the start pause is bounded independently of key count (or an
  accepted SLO is recorded) and progress reporting covers bytes/keys completed.

- [ ] **D3. Journal integrity and encryption at rest are Partial.** [P0]
  **Partial (2026-11-07): transaction CRC32 checksums added.** Each new journal
  transaction is followed by a standard RESP ECHO marker containing the byte
  length and CRC32. Legacy RESP-only journals remain readable and switch to a
  checksummed segment on their next append; rewrite snapshots are checksummed as
  one segment. Startup preflights all checksum groups before replay, fails closed
  on valid-RESP corruption and truncates a torn final transaction to the previous
  verified boundary. Tests cover a standard CRC32 vector, valid-RESP corruption,
  legacy upgrade and a fully written transaction with a torn marker.
  **Partial (2026-11-10): optional HMAC-SHA256 journal authentication.** With
  `--aof-hmac-key-file`, a streaming keyed hash chain covers every appended
  transaction, checksum record and rewrite slice; startup verifies the chain
  before replay and fails closed on modified or reordered content, an insertion
  before the final marker, a malformed marker or the wrong key. An empty journal starts with an anchor on its
  first append. Existing journals require the explicit, one-time
  `--aof-hmac-migrate` startup seal, which authenticates the already checksummed
  prefix as-is; later starts without an anchor fail closed. A torn final
  unauthenticated group is discarded to the last verified marker. The offline
  backup tool accepts the key for create/verify/restore. Both key-file readers
  enforce a byte limit during the read and reject oversized files. Tests cover restart,
  wrong key, modified tags, migration refusal and completion, authenticated
  rewrite plus post-rewrite appends, torn markers, partial next appends, and real
  server/backup paths.
  **Limit:** HMAC is not encryption. The key is a single operator-managed file with
  no rotation/keyring, secure-store integration or file-permission enforcement;
  a whole-journal replacement with an older valid file, and truncation of a
  durable suffix, are not detectable without external anti-rollback state. If the
  key is removed and new writes are appended, the next authenticated start
  discards those unauthenticated writes. Legacy prefixes are only authenticated
  after an explicit migration or rewrite.
  **Closes when:** the profile has a reviewed encryption-at-rest and key-management
  contract (or an explicit exclusion), and recovery authentication covers the
  deployment's rollback/truncation threat model with documented external state.

- [ ] **D4. Crash, ENOSPC and disk-fault behavior is unverified.** [P0]
  There is no fault-injection corpus for short writes, ENOSPC, quota, permission
  loss, fsync failure, process kill or power loss at each journal/rewrite
  boundary. The journal latches a failed state, but the full acknowledgment and
  recovery contract is not proven.
  **Partial evidence (2026-11-05):** `tests/durability_crash.rs` spawns the real
  binary and asserts, after a `SIGKILL` and restart, that every client-acknowledged
  write survives with `appendfsync always` (so the reply cannot precede the
  durable append) and that `everysec` recovery yields a contiguous prefix with no
  holes. A separate Linux drill kills the process after a local `WAITAOF`
  acknowledgement with either `no` or `everysec` and checks binary values after
  restart; it does not simulate device loss. The Linux torn-tail drill truncates
  a disposable journal after process exit, then verifies that restart discards the final integrity
  group for a multi-key `MSET` and that new writes survive another restart.
  This models a torn integrity marker, not a real disk write failure. `EXEC`
  currently commits queued writes as separate journal groups, so recovery from
  a torn tail need not preserve an entire `EXEC` batch as specified by R118.
  ENOSPC, quota, permission loss, fsync failure and power loss remain unverified.
  **Closes when:** a crash/fault harness injects each failure at every boundary,
  proves no applied-but-unlogged success reply, bounds the loss window, and
  verifies restart recovery and visible failure states.

- [ ] **D5. Backup, restore and point-in-time recovery are Partial.** [P0]
  `rillcache-backup` now creates, verifies and restores versioned, bounded-memory
  CRC32 archives from a quiescent AOF. Creation holds the normal exclusive journal
  lock and syncs before copying; restore verifies the outer archive and replays an
  isolated temporary AOF before no-overwrite publication. End-to-end tests cover
  active-writer refusal, archive corruption, inner AOF corruption, strings, TTLs,
  stream PEL state and a restart from the restored journal. The format contains
  only the AOF; CRC32 is not authentication. Online copy, PITR, retention,
  configuration/ACL capture and deployment-specific RPO/RTO remain open. Pass
  `--aof-hmac-key-file` to create/verify/restore for authenticated journals.
  Offline verification can stage the temporary journal on a chosen local volume
  with `verify --temp-dir DIR`; it still requires scratch space for the payload.
  Verification and restore reject a torn journal that startup replay would
  otherwise shorten to a valid prefix, even if the outer CRC and length match.
  Creation also rejects a torn source before publishing and preserves its bytes;
  legacy, checksum and HMAC fixtures distinguish this from startup tail recovery.
  The isolated offline rollback test checks exact binary data, absolute expiry,
  key count and logical payload bytes while preserving the newer journal.
  The same-binary authenticated restore test rejects a wrong key without
  publishing or leaving staging files, then restores into the same directory
  with the matching key and verifies the archive is unchanged. An insufficient
  logical payload budget also rejects verify and restore without publication or
  residue; retry at a sufficient budget restores both original binary values.
  A two-process restore case checks that only one archive is published into a
  shared offline destination, with both originals and the winning bytes intact.
  **Closes when:** a documented backup/restore procedure has tested quiescent and
  supported online copies, restores exact data/TTL/accounting into an isolated
  instance, and states RPO/RTO and retention; or online backup/PITR are explicitly
  excluded from the production profile with a reviewed operational contract.

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

- [ ] **D7. Signal-driven shutdown is partial.** [P0]
  **Partial (2026-11-19):** the standalone binary handles Unix SIGINT/SIGTERM
  and Windows Ctrl-C, stops admission, joins workers and explicitly flushes the
  journal within a 25-second deadline; a second signal or expired deadline logs
  forced termination and exits nonzero. Kubernetes now allows a 30-second grace
  period. **Partial (2026-11-19):** shutdown stops listener admission and
  half-closes client read sides; replies for already-dispatched commands are
  written before those connections close, within the existing deadline.
  Subprocess tests withhold an 8 MiB reply until SIGTERM and verify both the full
  response and journal recovery in default and opt-in Tokio plain-TCP modes and
  over TLS. All three transports also have a second-SIGTERM test that requires
  a forced-termination log and nonzero exit while an admitted reply is unread.
  On Linux, the Mio, Tokio and TLS forced-exit tests also restart from the
  unchanged AOF directory and verify an acknowledged write under `appendfsync
  always`. This does not cover unsynced writes or power loss. The 25-second
  deadline outcome is separately unit-tested, not timed end-to-end for each
  transport. A separate plain-TCP test in Mio and Tokio leaves the reply unread
  past a single SIGTERM, verifies successful exit within five seconds and a
  properly truncated reply, then checks on Linux that the acknowledged
  `appendfsync always` value is intact after restarting. This exercises the
  bounded client drain on working storage. The matching native TLS subprocess
  test leaves its reply unread through one SIGTERM and verifies successful
  exit within five seconds with only a valid reply prefix left to read. Neither
  test exhausts the separate 25-second supervisor deadline.
  Subprocess cancellation tests leave idle, incomplete-frame, subscribed and
  blocked clients connected through successful exit, then verify after restart
  that neither an incomplete SET nor a SET queued behind BLPOP was replayed.
  **Retained gap:** this is not a full client drain; replies that exceed a
  bounded transport write timeout or the shutdown deadline may be abandoned.
  Client receipt is not acknowledged, and forced timeout cannot guarantee the
  final journal flush. **Closes when:** all supported transports have verified
  command-drain, cancellation, join and forced-timeout behavior within the
  supervisor grace contract.

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

- [ ] **A3. Cluster mode is missing.** [P0]
  `CLUSTER` is disabled; there are no 16,384 slots, hash tags, `MOVED`/`ASK`
  redirects, migration/importing state, cluster bus or cross-slot rejection.
  Sharded Pub/Sub has no cluster routing. A native TCP test verifies explicit
  rejection of the `SHARDS`, `SLOTS`, and `NODES` topology probes without
  changing local data or advertising a replica.
  **Closes when:** slot mapping, redirects, migration, cross-slot atomicity,
  topology discovery, partition/rejoin and mixed-version rolling upgrade pass a
  cluster corpus, or the profile explicitly excludes cluster clients.

- [ ] **A4. Rolling upgrade, rollback and mixed-version compatibility are missing.** [P0]
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

- [ ] **S3. The audit path is not a complete custody chain.** [P0]
  The UDP collector has no sender authentication, transport encryption,
  federation, retry/backpressure, aggregation or delivery guarantee. Per-line
  HMAC and manual keyring rotation exist, but full-chain signing/key-ID and hot
  rotation do not. Collector output failures emit a redacted count summary and
  exit nonzero. This does not retry or guarantee delivery.
  **Closes when:** authenticated and encrypted transport, sender identity,
  key-ID/hot rotation, durable delivery/backpressure and aggregation are
  implemented and tested; or the audit path is explicitly best-effort with a
  documented compensating control.

- [ ] **S4. Cross-process rate limiting is missing.** [P0]
  Failed-auth budgets remain process-local. Shared NAT addresses can throttle
  each other; there is no cross-process/distributed policy or username lockout.
  **Partial (2026-11-18):** when the bounded per-IP table fills, new sources
  share an overflow budget instead of evicting tracked source state. **Partial
  (2026-11-19):** opt-in `--auth-global-limit` counts failures across peers in
  one process and locks credential attempts for the configured source window;
  it defaults off and can itself deny legitimate users. Neither control is
  shared between server processes. **Closes when:** a bounded shared/distributed
  rate-limit design is implemented or the deployment requires an external
  control plane with documented behavior.

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
  only command and expire-cycle series; `LATENCY GRAPH|HISTOGRAM` are
  unsupported.
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

- [ ] **O5. Configuration reload is missing.** [P0]
  Listener/worker changes are startup-only; there is no general reload, atomic
  configuration generation, rollback or validation report. TLS is a scoped
  exception: a native test checks that one invalid SNI key rejects the
  whole generation and that a repaired generation updates both default and
  SNI stapled bytes. See also C13 and S5.
  **Closes when:** mutable settings can be atomically reloaded, immutable changes
  are rejected cleanly, and failed reloads retain the last valid configuration.

- [ ] **O6. Slow-client and queue policy is incomplete.** [P1]
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

- [ ] **R1. The default data path is single-owner.** [P0]
  `--storage-owners 1` is the default. Opt-in `--storage-owners N` exists, but its
  own measurements show it below the one-owner path at pipeline 16 on the test
  host; no protected benchmark gates multi-owner mode. The thread-per-core target
  is not complete.
  **Closes when:** the declared profile either states a single-owner capacity
  limit or demonstrates multi-owner scaling with matched workloads, CPU budgets,
  tails and cross-owner atomicity tests.

- [ ] **R2. The async Tokio transport is not promoted.** [P1]
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
  The container workflow explicitly sets `provenance: false` and `sbom: false`.
  There is no signature, attestation, vulnerability scan or dependency/license
  inventory for the image.
  **Closes when:** signed digests, SBOM/provenance attestations and a documented
  vulnerability-response process are part of release.

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
  production configuration profiles.
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

- [ ] **V1. Only three upstream Tcl files are exercised.** [P0]
  The compatibility workflow runs Redis/Valkey `unit/type/string`,
  `unit/type/incr` and `unit/keyspace` only. Modules, cluster, integration,
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
  across duplicate subscriptions, shared patterns and last unsubscribe. Broader
  Pub/Sub behavior, ACL, TLS, persistence, exhaustive transaction/error cases
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
