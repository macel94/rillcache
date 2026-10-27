# Persistence requirements

Part of the [requirements and incremental delivery index](../requirements.md). Requirements are preserved verbatim from the former monolithic EARS sections; add new requirements only in the matching area file.

## Append-only persistence (S29)

- **R117 (event-driven):** When `--appendonly yes` is supplied, the server shall
  open/create the configured journal with mode 0600, replay it into a fresh store
  before accepting clients, truncate only an incomplete final command, and fail
  startup visibly for malformed mid-file data, unsupported/failed commands or
  configured limit violations. Defaults shall remain persistence-off.
- **R118 (event-driven):** When a supported command mutates keys, the journal shall
  append a replay-safe `DEL` plus final-state rebuild and absolute `PEXPIREAT`
  records for each changed key in a deterministic order; non-mutating/rejected
  commands shall append nothing, and a `MULTI` batch shall be captured once after
  the whole batch. Content/timestamp overflow shall force `FLUSHALL` plus a full
  snapshot rather than a missed key.
- **R119 (state-driven):** While configured, `always` shall fsync before replying,
  `everysec` shall bound expected loss to the maintenance interval, and `no` shall
  only write without an explicit sync. Append/sync failure shall replace the reply
  with a clear error and latch the journal; orderly shutdown shall sync and flush.
  The format shall be RESP commands, not an RDB or byte-compatible multipart-AOF claim.
- **R120 (ubiquitous):** Persistence tests shall run offline in all execution modes
  and cover strings/binary values, all supported collections, streams/groups, TTLs,
  transactions, concurrent writers, flushes, partial tails, mid-file corruption and
  restart. Documentation shall retain missing RDB/AOF-rewrite, compaction,
  checksums, encryption, multi-process ownership, backup drills, signal drain,
  stream-pending durability and disk-fault/soak guarantees.

## Journal compaction (S32)

- **R129 (event-driven):** When `BGREWRITEAOF` is received on an enabled journal,
  the server shall encode a complete current-state snapshot (`FLUSHALL` plus every
  live key with absolute TTLs), write a mode-0600 temporary file in the journal
  directory, fsync it, atomically rename it over the journal and reopen it
  positioned at the snapshot end for subsequent appends.
- **R130 (state-driven):** While the rewrite runs, the server shall hold the
  exclusive transaction gate so no mutation can be lost or appended to the replaced
  file; command service may pause for the synchronous rewrite. A failed write/sync/
  rename shall retain the previous journal and return a visible error. The
  reopened handle shall append after the snapshot rather than at offset zero.
- **R131 (unwanted behavior):** If the journal is disabled or the rewrite cannot be
  completed, then the command shall return a bounded error without claiming
  compaction. Tests shall cover a compacted live log, replay after rewrite, disabled
  journals, failure atomicity, append-after-rewrite replay and all execution modes. Documentation shall not call
  this background, incremental or automatically scheduled.

## Automatic journal compaction (S78)

- **R306 (event-driven):** `CONFIG GET/SET auto-aof-rewrite-percentage` and
  `auto-aof-rewrite-min-size` shall expose and atomically update the thresholds.
  Percentage shall default to 100 and disable automatic rewrites at 0; minimum
  size shall default to 64 MiB. Negative/non-integer values and unsupported
  options shall fail without changing configuration.
- **R307 (state-driven):** While a journal is enabled and the percentage is
  non-zero, the maintenance loop shall compare the open file size with the size
  recorded at open/last rewrite and trigger compaction once both the minimum size
  and configured growth percentage are met. Disabled journals or unmet thresholds
  shall perform no rewrite and add no command-path overhead beyond the periodic
  maintenance check.
- **R308 (event-driven):** A triggered rewrite shall use the existing synchronous
  full-state compaction under the transaction gate, reset the baseline to the new
  file size, keep the previous journal intact on failure, and preserve replayable
  data/TTLs across restart. It shall not claim fork-based background or incremental
  rewrite behavior.
- **R309 (ubiquitous):** Tests shall cover defaults, `CONFIG GET/SET` round trips
  and validation, growth/baseline triggering, no-op thresholds/disabled journals,
  maintenance-triggered compaction, restart replay and all worker modes.
  Documentation shall retain the absence of background/incremental rewrite,
  checksums, encryption and crash-consistency guarantees.

## Truthful persistence status (S80)

- **R313 (event-driven):** When `INFO persistence` or `INFO all` is requested, the
  server shall return a `# Persistence` block with `loading:0` and `aof_enabled`.
  While the journal is enabled it shall also report `aof_rewrite_in_progress:0`,
  the last rewrite and write statuses and the current file size; while disabled
  it shall report only `aof_enabled:0`. It shall omit RDB, backlog, bio and other
  unmeasured persistence fields rather than fabricating them.
- **R314 (state-driven):** The reported write status shall reflect the journal's
  latched failure state, the rewrite status shall reflect the last completed
  synchronous `BGREWRITEAOF`, and the size shall come from the open journal file.
  Because rewrites are synchronous, `aof_rewrite_in_progress` shall remain `0`
  and no asynchronous progress claim shall be made.
- **R315 (ubiquitous):** Tests shall cover disabled and enabled servers, the
  measured field set, nonempty size after a write, status after rewrite, `INFO
  all` inclusion and every worker mode. Documentation shall retain the absence
  of RDB, asynchronous/background rewrite, checksum, encryption, ENOSPC and
  crash-recovery guarantees.

## Journal configuration visibility and runtime fsync (S87)

- **R332 (event-driven):** `CONFIG GET appendonly`, `appendfilename` and
  `appendfsync` (including wildcard patterns and `*`) shall report the configured
  append-only mode, the journal basename and the live fsync policy as `yes`/`no`,
  the exact basename and `always`/`everysec`/`no`. `appendonly` and
  `appendfilename` shall remain startup-only: `CONFIG SET` for either shall return
  a visible error without changing configuration. `CONFIG SET appendfsync` shall
  accept the three valid policies and reject other values.
- **R333 (state-driven):** Changing `appendfsync` shall update the open journal so
  subsequent appends obey the new policy. Leaving `no` shall first sync records
  that were previously only handed to the operating system; a sync failure shall
  leave the journal failed and return an error. With append-only disabled the
  configured policy shall still be reported and updated, without enabling
  persistence or affecting a future process that chooses its own startup options.
- **R334 (ubiquitous):** Tests shall cover default and configured startup values,
  live updates, invalid and startup-only rejection, switching away from `no`,
  restart replay and every worker mode. Documentation shall retain that
  `appendonly` cannot be toggled at runtime, there is no configuration-file
  reload, and changing the fsync policy changes the durability/latency trade-off
  rather than providing a crash-proof guarantee.

## WAITAOF durability acknowledgements (S94)

- **R353 (event-driven):** `WAITAOF numlocal numreplicas timeout` shall validate
  `numlocal` (0 or 1), non-negative `numreplicas` and non-negative millisecond
  timeout. With AOF enabled it shall fsync the local journal and report
  `[1, 0]`; without AOF it shall report local `0` when `numlocal` is 0 and return
  the documented AOF-disabled error when `numlocal` is 1. Invalid integers shall
  return the standard integer error.
- **R354 (state-driven):** While `numreplicas` is greater than zero and no
  replicas exist, the command shall remain blocked until the timeout and then
  return the actual `[local, 0]` counts, or block indefinitely when the timeout
  is 0. Inside `MULTI`/`EXEC` it shall not block and shall return the current
  counts immediately.
- **R355 (ubiquitous):** All-mode tests shall cover both AOF states, argument and
  arity errors, absent-replica timeout duration, transaction non-blocking
  behavior and every worker mode. Documentation shall retain that this is a
  local-journal acknowledgement only, not replication, failover, RDB or a
  strong-consistency guarantee.

## Explicit RDB, replication and database boundaries (S92)

- **R347 (event-driven):** `LASTSAVE` shall return `0` because no successful RDB
  save exists. `SAVE` shall reject with an explicit RDB-unsupported error;
  `BGSAVE` shall accept the documented `SCHEDULE` token syntactically and reject
  with the same error, while malformed extra tokens and wrong arity shall keep
  their syntax/arity errors. None shall mutate data or claim a successful save.
- **R348 (event-driven):** `REPLICAOF NO ONE` and `SLAVEOF NO ONE` shall return
  `+OK` because the server is already a standalone master. A host/port form,
  `PSYNC`, `SYNC`, `REPLCONF` and `FAILOVER` shall return an explicit
  replication-unsupported error rather than an unknown-command error, with arity
  checks preserved.
- **R349 (ubiquitous):** `MOVE` and `SWAPDB` shall return an explicit
  database-0-only error and leave existing keys readable. All-mode tests shall
  cover every boundary response, syntax/arity variants, data preservation and
  every worker mode. Documentation shall retain the absence of RDB persistence,
  replication/failover, multiple databases, scripts and cluster routing.
