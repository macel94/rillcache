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
| Durability and recovery | Partial / deferred | No RDB or interoperable AOF, no background rewrite, no crash/ENOSPC/backup evidence |
| Availability and topology | Missing | No replication, failover, Sentinel or cluster |
| Security and transport | Partial | No independent review, full ACLs, global rate limiting, TLS hardening or audited collector federation |
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
  **Closes when:** a versioned profile lists exact upstream versions, command
  matrix, client libraries, deployment topology and excluded features; CI runs
  every row against pinned Redis and Valkey and records pass/fail.

- [ ] **C2. Scripting and functions are missing.** [P0]
  `EVAL`, `EVALSHA`, `SCRIPT LOAD|EXISTS|DEBUG|KILL`, `FUNCTION
  LOAD|DELETE|LIST|STATS|DUMP|RESTORE|KEYS` and `FCALL`/`FCALL_RO` are absent.
  Only `SCRIPT FLUSH` and `FUNCTION FLUSH` return `+OK` for an empty registry.
  The dependency policy does not permit a Lua runtime.
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
  error, and `COPY ... DB n` rejects nonzero databases.
  **Closes when:** the profile either supports a validated number of databases
  with per-database keyspaces, `SELECT`/`MOVE`/`SWAPDB`/`FLUSHDB` semantics and
  per-database observability, or explicitly excludes clients that require them.

- [ ] **C5. The command and subcommand surface is incomplete.** [P0]
  The registry contains 243 commands, but common surfaces are absent or bounded:
  `DUMP`/`RESTORE`/`MIGRATE`; `OBJECT FREQ|IDLETIME` (explicit unsupported);
  `PFDEBUG`/`PFSELFTEST`; `CLIENT TRACKING|NO-EVICT|NO-TOUCH|REPLY|CACHING|
  GETREDIR`; `COMMAND DOCS`; `CONFIG REWRITE`; `MEMORY MALLOC-STATS|PURGE`;
  `LATENCY GRAPH|HISTOGRAM`; and Redis 8+ command extensions. Some return explicit
  errors, others are unknown commands.
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

- [ ] **C7. Real client-library compatibility is unverified.** [P0]
  Pinned `redis-cli` RESP2/RESP3/AUTH and metadata interop is covered. Python,
  Node.js and Go clients are not in CI, and arbitrary clients may issue
  unsupported discovery commands.
  **Closes when:** a versioned language-client matrix (for example redis-py,
  node-redis and go-redis) runs handshake, AUTH/ACL, RESP2/RESP3, pipelines,
  pub/sub, TLS, reconnect and error-path tests against the supported profile.

- [ ] **C8. Keyspace notification coverage is incomplete.** [P1]
  `keymiss`, new-key and `FLUSH*` event classes are unsupported; field-expiry
  event parity (`hexpire`/`hdel`/`hexpired`) is not implemented; delivery is
  at-most-once and process-local.
  **Closes when:** the event matrix matches the declared Redis/Valkey version for
  supported mutations, with ordering, duplicate/loss and reconnect tests; durable
  delivery remains explicitly out of scope unless required by the profile.

- [ ] **C9. Streams remain a bounded subset with volatile delivery state.** [P1]
  Entries, last IDs and group cursors persist, but consumer objects and pending
  entry lists (owners, delivery counts, timestamps) are not restored. `XINFO`
  omits some Redis fields (`entries-read` is null, `nacked-count` is zero, inactive
  time is approximated), and approximate `XTRIM` follows a different granularity.
  **Closes when:** pending/consumer state is durable and restored, metadata fields
  match the declared version or are explicitly excluded, and trim/read semantics
  pass a differential corpus.

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
  There is no `redis.conf` parser, general configuration file, `CONFIG REWRITE`
  or complete runtime configuration surface. Startup uses a small CLI subset;
  only selected `CONFIG GET/SET` options exist.
  **Closes when:** the supported configuration schema is versioned, validated
  before bind, documented with defaults/precedence, and runtime-mutable settings
  are atomic and complete for the profile.

- [ ] **C14. Transaction semantics diverge from Redis.** [P1]
  Connection-local commands are rejected at queue time rather than executed at
  `EXEC` time. There is no rollback, replication of transactions or durable
  distributed commit; the single-process owner is the only isolation boundary.
  **Closes when:** the supported profile documents the divergence and tests it,
  or queue-time/EXEC-time behavior matches the declared upstream version and the
  absence of rollback/durable commit is explicit.

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
  **Measured (host-local):** old synchronous pass 130 ms full stall and 129 ms
  worst PING for 20,000 keys (2.5 MB); cooperative start pause 9–20 ms
  (key-list materialisation), 1.6–2.4 ms per 512-key slice, commit under 1 ms plus
  the residual fsync. Completion time still scales with the dataset.
  **Remaining:** no fork or copy-on-write, so the start pause is O(key names) and
  enormous keyspaces still need a measured pause SLO; a sustained write storm
  lengthens reconciliation rounds; there is no progress percentage, only the
  in-progress flag. RDB and online backup remain separate items.
  **Closes when:** the start pause is bounded independently of key count (or an
  accepted SLO is recorded) and progress reporting covers bytes/keys completed.

- [ ] **D3. Journal integrity and encryption at rest are incomplete.** [P0]
  **Partial (2026-11-07): transaction CRC32 checksums added.** Each new journal
  transaction is followed by a standard RESP ECHO marker containing the byte
  length and CRC32. Legacy RESP-only journals remain readable and switch to a
  checksummed segment on their next append; rewrite snapshots are checksummed as
  one segment. Startup preflights all checksum groups before replay, fails closed
  on valid-RESP corruption and truncates a torn final transaction to the previous
  verified boundary. Tests cover a standard CRC32 vector, valid-RESP corruption,
  legacy upgrade and a fully written transaction with a torn marker.
  **Limit:** CRC32 detects accidental corruption, not malicious modification;
  legacy prefixes have no retroactive checksums until rewritten. There is still no
  authenticated record format, encryption-at-rest design or key-management
  contract.
  **Closes when:** every journal byte is authenticated (including legacy import),
  recovery fails closed on malicious modification, and the profile documents disk
  encryption/key ownership or implements application-level encryption.

- [ ] **D4. Crash, ENOSPC and disk-fault behavior is unverified.** [P0]
  There is no fault-injection corpus for short writes, ENOSPC, quota, permission
  loss, fsync failure, process kill or power loss at each journal/rewrite
  boundary. The journal latches a failed state, but the full acknowledgment and
  recovery contract is not proven.
  **Partial evidence (2026-11-05):** `tests/durability_crash.rs` spawns the real
  binary and asserts, after a `SIGKILL` and restart, that every client-acknowledged
  write survives with `appendfsync always` (so the reply cannot precede the
  durable append) and that `everysec` recovery yields a contiguous prefix with no
  holes. ENOSPC, quota, permission loss, fsync failure, power loss and kill during
  rewrite remain unverified.
  **Closes when:** a crash/fault harness injects each failure at every boundary,
  proves no applied-but-unlogged success reply, bounds the loss window, and
  verifies restart recovery and visible failure states.

- [ ] **D5. Backup, restore and point-in-time recovery are absent.** [P0]
  There is no backup tooling, restore drill, generation/retention policy, PITR or
  documented RPO/RTO. Copying a live journal is not supported.
  **Closes when:** a documented backup/restore procedure is tested from a
  quiescent and supported online copy, restores into an isolated instance with
  exact data/TTL/accounting, and states RPO/RTO and retention.

- [ ] **D6. Stream pending and consumer state is volatile across restart.** [P0]
  **Partial (2026-11-05): consumers and PEL now persist.** The journal rebuild
  emits `XGROUP CREATECONSUMER` for every consumer and `XCLAIM ... TIME ...
  RETRYCOUNT ... JUSTID FORCE` for every pending entry, so owners, delivery counts
  and delivery timestamps survive restart; both the live journal and the
  cooperative rewrite use the same encoder. `tests/persistence.rs` covers
  consumers with and without pending entries, cross-consumer claims with an
  explicit retry count, restart replay, and acknowledging the replayed PEL.
  **Remaining:** pending ids whose entries were trimmed or deleted cannot be
  re-claimed on replay and are dropped; consumer `seen_at`/`inactive` time is not
  restored (it reflects replay time); trim/claim-after-trim durability has no
  dedicated test yet.
  **Closes when:** deleted-entry PEL tombstones are represented, consumer activity
  time is preserved, and trim/claim interactions are covered by restart tests; or
  Streams are explicitly excluded as a durable queue.

- [ ] **D7. Signal-driven graceful drain is missing.** [P0]
  Managed shutdown joins workers and protocol `SHUTDOWN` flushes the journal, but
  the standalone process has no portable SIGTERM/SIGINT handler. Container
  termination can lose in-flight replies and unsynced journal tails.
  **Closes when:** a reviewed, safe drain path stops admission, flushes within a
  deadline, joins workers and reports forced termination; or the profile states
  the supervisor contract and accepted loss window.

- [ ] **D8. Multi-process journal ownership is unguarded.** [P1]
  **Partial (2026-11-05): local exclusive ownership enforced.** Startup claims
  `<journal>.lock` with an exclusive create recording PID and kernel start time; a
  second process on the same directory fails with a clear error, a crash-stale lock
  is reclaimed only when the recorded process is gone or its PID was reused, and
  clean shutdown removes the lock (`tests/journal_ownership.rs`). The lock is
  advisory and local: shared/network filesystems are explicitly unsupported.
  **Remaining:** no distributed lease or shared-volume guarantee; an operator must
  keep the journal on local storage or enforce single-process access externally.
  **Closes when:** shared-volume behavior is either implemented with a real lease
  or the deployment profile excludes it explicitly (the current documentation says
  unsupported).

- [ ] **D9. Wall-clock rollback across restart is unresolved.** [P1]
  Absolute TTLs are persisted, but a host clock moved backwards can resurrect a
  key that expired before shutdown. No high-water or recovery-time policy is
  defined.
  **Closes when:** the clock policy is documented and tested for backwards jumps,
  daylight/timezone changes and restart, with an explicit guarantee or limitation.

- [ ] **D10. Durability performance is unmeasured.** [P1]
  Persistence serializes commands through the exclusive gate; there is no
  throughput/latency evidence for `always`/`everysec`/`no`, rewrite pauses or
  large journals.
  **Closes when:** matched benchmarks report throughput, tails, fsync cost and
  rewrite pause for each supported policy and workload, with an accepted SLO.

- [ ] **D11. Recovery is not validated across platforms/filesystems/versions.** [P1]
  Tests cover ordinary restart, truncated tails and mid-file corruption, but not
  kill -9 at each boundary, power-loss simulation, filesystem matrices, mixed
  versions or long-running recovery.
  **Closes when:** a repeatable recovery matrix covers those cases and retains
  failure evidence.

---

## 3. Availability and topology

- [ ] **A1. Replication is missing.** [P0]
  `PSYNC`, `SYNC`, `REPLCONF`, `REPLICAOF host port` and `SLAVEOF host port`
  return explicit replication-unsupported errors. `ROLE`/`INFO replication`
  truthfully report a standalone master; `WAIT` returns 0 and `WAITAOF` only
  acknowledges the local journal.
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
  Sharded Pub/Sub has no cluster routing.
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
  dynamic key sandbox rules, complete channel filtering for `PUBSUB`
  introspection and commands outside the published table are absent. `KEYS`,
  `RANDOMKEY`, `DBSIZE`, `FLUSH*` and similar commands are not a per-key sandbox.
  **Closes when:** the supported ACL subset is explicitly versioned and
  differentially tested, or full category/subcommand/key/channel semantics are
  implemented; restricted users cannot enumerate or mutate keys outside policy.

- [ ] **S3. The audit path is not a complete custody chain.** [P0]
  The UDP collector has no sender authentication, transport encryption,
  federation, retry/backpressure, aggregation or delivery guarantee. Per-line
  HMAC and manual keyring rotation exist, but full-chain signing/key-ID and hot
  rotation do not.
  **Closes when:** authenticated and encrypted transport, sender identity,
  key-ID/hot rotation, durable delivery/backpressure and aggregation are
  implemented and tested; or the audit path is explicitly best-effort with a
  documented compensating control.

- [ ] **S4. Global/cross-process rate limiting is missing.** [P0]
  Failed-auth budgets are process-local and per-connection/per-IP. Shared NAT
  addresses can throttle each other; an attacker can exhaust the source table;
  there is no username lockout, global budget or distributed policy.
  **Closes when:** a bounded, shared rate-limit design is implemented or the
  deployment requires an external control plane with documented behavior.

- [ ] **S5. TLS hardening is incomplete.** [P0]
  TLS 1.3 is available, but SNI certificate selection, hot reload, OCSP/CRL,
  client-certificate-to-ACL mapping, rotation drills and independent TLS
  interoperability/security review are absent.
  **Closes when:** those controls are implemented or explicitly replaced by a
  reviewed external terminator, with rotation and failure tests.

- [ ] **S6. Live deauthentication/session revocation is missing.** [P1]
  Rotating or disabling a password affects new authentication decisions; an
  already authenticated connection remains authenticated. There is no bounded
  reauthentication or administrative revoke-and-close policy.
  **Closes when:** a documented revocation policy is implemented and tested for
  live sessions, or operators have a tested process to close affected sessions.

- [ ] **S7. Tenant isolation is absent.** [P1]
  All ACL users share one process, keyspace, memory budget and CPU. ACL rules do
  not create separate databases, quotas or network identities.
  **Closes when:** multi-tenant deployment is explicitly excluded, or per-tenant
  isolation is implemented and tested.

- [ ] **S8. Secret handling needs hardening.** [P1]
  Passwords can be passed on the command line, ACL hashes are unsalted
  SHA-256 (Redis-compatible but fast to brute-force offline), and there is no
  secret-manager integration or complete file-permission/rotation audit.
  **Closes when:** the supported secret sources, permissions, rotation and
  process-argument risks are documented and tested, with long random secrets
  required by deployment guidance or enforced.

- [ ] **S9. Denial-of-service and slow-client hardening is incomplete.** [P0]
  There are client/payload/frame limits and bounded output backpressure, but no
  slow-client thresholds, global connection/command budgets, per-user quotas or
  adversarial protocol fuzzing. O(N) commands can occupy the owner.
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
  `INFO` has aggregate command stats but no `INFO latencystats`, histograms,
  percentiles, per-client fd/idle/command/buffer/network/memory fields or
  blocked-client count. `LATENCY` exposes only command and expire-cycle series;
  `LATENCY GRAPH|HISTOGRAM` are unsupported.
  **Closes when:** the supported metrics contract reports per-command latency
  distributions and per-client resource/queue state with documented units and
  sampling limits.

- [ ] **O4. Operational telemetry and alerting are missing.** [P1]
  There is no metrics exporter, OpenTelemetry, structured logging, SLO dashboard,
  alert set, persistence/auth/audit telemetry, replay/rewrite duration or
  network-byte accounting. Health is a PING probe, not persistence/readiness.
  **Closes when:** a redacted, documented metrics/log/health contract exists and
  is tested for overload, persistence failure, auth failure and shutdown.

- [ ] **O5. Configuration reload is missing.** [P0]
  Listener/worker changes are startup-only; there is no general reload, atomic
  configuration generation, rollback or validation report. See also C13.
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
  Replica pause is not applicable until replication exists.
  **Closes when:** background eviction and the supported interaction matrix are
  documented and tested, including expiry under pause, eviction during blocked
  commands and ordering.

- [ ] **O8. Upgrade, rollback and incident runbooks are missing.** [P1]
  There is no tested procedure for version upgrade, format migration, rollback,
  disk-full recovery, OOM, certificate rotation, ACL lockout or data corruption.
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
  Client count, payload, frame and argument limits exist; there is no per-user
  command/connection/memory quota, CPU fairness guarantee or slow-client share.
  **Closes when:** the supported admission policy is explicit, observable and
  tested under overload and multi-tenant use.

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
  `tests/reference_baseline.rs` is a small optional string/RESP corpus. It does
  not cover collections, streams, pub/sub, transactions, ACL, TLS, persistence,
  errors or all supported commands.
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
  **Closes when:** those suites run in CI or a scheduled job and retain failures.

- [ ] **V5. Public documentation can still drift from executable behavior.** [P1]
  This change corrected known contradictions: sharded Pub/Sub and keyspace
  notifications were described as absent after implementation; `BZMPOP`,
  `SUBSTR` and AOF compaction were described as unsupported; rustls was described
  as the sole dependency exception; and multi-owner execution was described as
  unimplemented. There is still no automated check that README/area claims match
  tests or that open gaps are listed only in this canonical file.
  **Closes when:** a documentation/link/claim check runs in CI and this canonical
  file is linked from the README and indexes.

- [ ] **V6. There is no production acceptance runbook.** [P1]
  No single checklist maps the declared profile to repeatable pre-production,
  canary, load, failure, security and rollback gates with pass/fail evidence.
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
