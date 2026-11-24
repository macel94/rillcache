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
  restart. A deterministic clock test shall cover TTL replay after a backward
  system-clock step, and documentation shall state that timezone/daylight-saving
  changes do not affect Unix-epoch deadlines while a backward system-clock step
  before persisted deletion can make a key live again. Documentation shall retain
  missing RDB/AOF-rewrite, compaction, checksums, encryption, multi-process
  ownership, backup drills, signal drain, stream-pending durability and
  disk-fault/soak guarantees.

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
  While the journal is enabled it shall also report whether a cooperative
  rewrite is in progress, the last rewrite and write statuses and the current
  file size when known. It shall omit RDB, backlog, bio and other unmeasured
  persistence fields rather than fabricating them.
- **R314 (state-driven):** The reported write status shall reflect the journal's
  latched failure state, the rewrite status shall reflect the last completed
  rewrite attempt, and the current size shall come from the open live journal.
- **R315 (ubiquitous):** Tests shall cover disabled and enabled servers, the
  measured field set, nonempty size after a write, status after rewrite, `INFO
  all` inclusion and every worker mode. Reported status does not guarantee
  recovery or characterize physical storage faults.
- **R466 (state-driven):** While a cooperative rewrite is active, the persistence
  report shall expose the number of bytes already written to its temporary
  file as `aof_rewrite_bytes_written`, counting successful framing and
  reconciliation writes. The field shall be zero when the enabled journal is
  idle and omitted if disabled or the journal cannot be inspected. It shall
  not imply fsync completion, published file size or percentage complete.
- **R471 (event-driven):** When rewrite setup succeeds, the persistence report
  shall expose its elapsed monotonic microseconds as `aof_last_rewrite_start_usec`.
  Include key enumeration and staging initialization, but not later rewrite
  steps, request queueing or network time. Omit the field before a successful
  setup; a failed or redundant start shall not replace the previous measurement.
- **R481 (unwanted behavior):** If rewrite staging initialization fails,
  `aof_last_bgrewrite_status` shall report `err` without latching a live append
  error or keeping a rewrite active. The original journal shall be preserved
  and a later successful rewrite shall clear the rewrite error.
- **R484 (event-driven):** When rewrite finalization succeeds, the persistence
  report shall expose `aof_last_rewrite_finish_usec`, the elapsed monotonic
  microseconds from acquiring exclusive data access through final tail
  reconciliation, journal sync and publication, and rewrite bookkeeping. Omit
  the field before first success; a failed attempt shall not overwrite it.
- **R493 (unwanted behavior):** If the final staging rename fails before the
  live journal is replaced, report a failed rewrite and drop the rewrite job
  without disabling live appends. The previous journal shall remain intact,
  acknowledged writes shall replay after restart, and a subsequent rewrite
  shall be possible. Test with a disposable local Unix fixture.
- **R495 (event-driven):** When a key expires and is removed after its snapshot
  slice was encoded, rewrite reconciliation shall remove the stale record's
  effect from replay and preserve subsequent acknowledged writes. In a fixture
  with a controlled clock, replay before the original expiry to distinguish
  reconciliation from a stale record hidden by its deadline.
- **R497 (unwanted behavior):** If a complete AOF group fails its CRC check,
  startup shall refuse it without modifying the journal, even if it is the
  final group. A verified replacement shall replay without manual removal of
  the ownership lock after the refused startup.

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
- **R478 (state-driven):** After `WAITAOF 1 0 0` reports `[1, 0]`, a
  same-host process kill and restart shall recover the prior values under
  `appendfsync no` and `everysec`, without a graceful shutdown flush. That
  regression does not assert a power-loss or real storage-fault guarantee.

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

## Offline AOF backup and restore (S118)

- **R426 (event-driven):** When `rillcache-backup create` is requested for an
  existing regular AOF, it shall acquire the journal's normal exclusive owner
  lock, replay/validate the journal under configured data/protocol limits, force
  a sync, and stream a versioned length+CRC32 archive using bounded memory. If a
  server owns the journal, validation fails, the file exceeds the 16 GiB archive
  bound, or the output exists, the utility shall fail without publishing over an
  existing path.
- **R427 (event-driven):** When `verify` or `restore` reads an archive, it shall
  validate the version, declared length, CRC32, absence of trailing bytes and the
  inner AOF replay before reporting success. Restore shall write and validate a
  temporary AOF, then publish only when the destination AOF and its owner lock do
  not exist; it shall never overwrite an existing journal.
- **R428 (unwanted behavior):** If archive framing/checksum is invalid, inner AOF
  replay fails, or the destination is occupied, then restore shall return failure
  without making an invalid/partial journal visible at the destination. Archive
  mode shall not be described as cryptographic authentication, encryption, online
  backup, PITR, retention or RPO/RTO policy.
- **R429 (ubiquitous):** End-to-end tests shall cover live-writer refusal, empty
  and nonempty journals, archive corruption, valid outer CRC with inner AOF
  corruption, no-overwrite behavior, and restored strings, TTL and stream PEL
  across restart. Documentation shall state the required server quiescence,
  configurable replay limits, AOF-only scope, local-filesystem assumptions and
  online backup/PITR/retention/RPO/RTO gaps.
- **R456 (unwanted behavior):** If replay would truncate a torn source journal,
  then `rillcache-backup create` shall fail before publishing an archive, keep
  the source bytes unchanged, and release the ownership lock. Cover legacy,
  CRC and HMAC tails and confirm that ordinary startup still recovers the
  verified prefix.
- **R470 (unwanted behavior):** If replaying a valid archive exceeds the logical
  payload limit, `verify` and `restore` shall fail without retaining staging files
  or publishing a destination journal. A retry with sufficient budget shall
  leave the source archive unchanged and restore all its keys, not just the
  prefix that fit. This limit is not a bound on allocator or RSS usage.
- **R473 (unwanted behavior):** If two restores of valid archives target the
  same empty offline directory on a supported local filesystem, at most one
  shall publish a journal. On normal completion, the journal shall match the
  successful restore's payload, neither archive shall change, and staging
  files shall not remain. This does not cover concurrent server startup or
  interrupted cleanup.
- **R474 (event-driven):** When a complete legacy RESP journal is archived and
  restored by the current binary, preserve the original journal and archive
  bytes. After restoring, new CRC-framed appends shall retain the original
  prefix and both old and new data shall replay after restart. This is a
  format-level test, not a version-skew or downgrade guarantee.
- **R482 (event-driven):** When `create --filename` selects a non-default AOF
  and `restore --filename` names a different destination AOF, restore shall
  preserve exact journal bytes and shall not create the default journal. An
  isolated server using the restored basename shall replay the original data.
- **R485 (unwanted behavior):** If the destination has an ownership lock but no
  AOF, `restore` shall refuse without changing the lock or archive or leaving
  staging files. A retry in the empty offline fixture shall restore the exact
  journal and replay the original value. This does not cover concurrent startup.
- **R488 (unwanted behavior):** If a regular backup archive's length disagrees
  with its header, `verify` and `restore` shall reject it before creating a
  staging file. Matching length does not waive streamed length and CRC checks,
  trailing-data detection, or journal replay validation.
- **R503 (unwanted behavior):** If the sync required to change `appendfsync`
  fails, mark journal write health as failed, retain the current policy, and do
  not publish any other settings or evict keys for a budget in that `CONFIG SET`.
- **R499 (unwanted behavior):** If `create` targets an existing archive, refuse
  without changing the archive or source journal or leaving staging or lock
  files. A retry to a new filename shall capture the newer journal while the
  original archive remains usable for an isolated rollback.

## Authenticated journal records (S119)

- **R430 (event-driven):** When `--appendonly yes` and `--aof-hmac-key-file PATH`
  are configured, startup shall read a 16..=1024-byte key, strip one trailing
  newline, and authenticate each newly written transaction with an HMAC-SHA256
  chain over its checksum group. An empty journal shall write an anchor on the
  first append; a rewrite shall write an authenticated snapshot and continue the
  chain for later appends.
- **R431 (state-driven):** While a key is configured, startup shall verify the
  anchor and every completed chain link before replaying any command, and shall
  fail closed on a modified, reordered or inserted completed group before the
  final marker, a malformed marker or a different key. A trailing unauthenticated
  suffix (a crash-torn final group) shall be truncated to the last verified marker
  without dropping the preceding authenticated transaction; truncation of an
  earlier durable suffix is outside the authentication guarantee.
- **R432 (event-driven):** If a nonempty journal has no authentication anchor,
  then startup without `--aof-hmac-migrate` shall fail with an actionable error.
  With the migration flag and the server stopped, startup shall seal the existing
  checksum-verified prefix with a synced anchor before serving; later startups
  without an anchor or migration shall still fail. The flag without a key, or
  authentication without `--appendonly yes`, shall be rejected.
- **R433 (ubiquitous):** Tests shall cover standard SHA-256/HMAC vectors,
  streaming equivalence, restart, wrong key, modified chain tags, migration
  refusal and completion, authenticated rewrite plus post-rewrite appends, torn
  final markers, a partial next append, key-file bounds, server configuration and
  authenticated backup/restore. Documentation shall state that HMAC is not
  encryption, does not cover suffix rollback or whole-file replacement, has no key
  rotation/keyring contract, and that unauthenticated writes after key removal
  are discarded by the next authenticated start.
- **R455 (unwanted behavior):** If offline restore of an authenticated archive
  fails because the supplied key does not match, it shall leave the archive
  unchanged and publish no journal. In a working, empty destination directory,
  it shall remove its staging files and lock so a retry with the matching key
  can restore the archive without manual cleanup. Test by restarting the same
  binary against the restored journal and reading the expected value.
- **R464 (unwanted behavior):** When loading the journal authentication key,
  the server and backup utility shall read at most 1027 bytes and shall not
  accept a prefix of an oversized file. The 16..=1024-byte key range after
  stripping one final LF or CRLF remains unchanged.

## Deleted-entry stream PEL durability (S120)

- **R434 (event-driven):** When a journal snapshot or rewrite encodes a stream
  whose consumer-group PEL references an id missing from the stream (deleted or
  trimmed), it shall emit an ascending placeholder `XADD` for that id, restore
  every referencing group's pending owner, delivery count and delivery time with
  `XCLAIM`, and then emit `XDEL` for the placeholder before the final `XSETID`.
  The placeholder shall not remain visible in `XLEN` or `XRANGE`.
- **R435 (state-driven):** While replay is in progress, placeholder ids shall keep
  the XADD sequence strictly ascending across real entries, tombstones, and any
  sentinel; the final `XSETID` shall restore the recorded last-generated,
  entries-added and max-deleted metadata even though placeholder deletion
  temporarily raises max-deleted.
- **R436 (unwanted behavior):** If a placeholder id equals a live entry, the
  encoder shall use the live entry once rather than duplicating it. Multiple
  groups referencing the same tombstone shall each receive its PEL metadata. An
  empty stream that has only group/consumer metadata shall be recreated with
  `XGROUP CREATE ... MKSTREAM`, not an invalid `XADD 0-0` sentinel.
- **R437 (ubiquitous):** Tests shall cover deleted and trimmed pending ids in the
  live journal and after rewrite, intermediate and highest ids, preserved
  owner/delivery count, `XACK` of a tombstone, empty `XRANGE`/unchanged `XLEN`,
  an all-deleted stream, an empty stream with a group/consumer, and all execution
  modes. Consumer `seen_at` and `active_at` are persisted by the
  consumer-activity records below; full history for deleted/renamed consumers
  is not restored.

## Consumer activity-time durability (S121)

- **R438 (event-driven):** When a journal snapshot or rewrite encodes a consumer,
  it shall append namespaced RESP `ECHO` records containing binary
  length-prefixed key, group and consumer names plus the absolute millisecond
  `seen_at` and `active_at`, after the group's `CREATECONSUMER`/`XCLAIM` records
  so claims cannot overwrite the restored values.
- **R439 (state-driven):** During replay, well-formed consumer-activity records
  shall set the existing consumer's `seen_at` and `active_at` before serving; a
  malformed record, a missing key/group/consumer or trailing bytes shall fail
  replay closed rather than being ignored. The record shall be covered by CRC32 and optional HMAC
  authentication like every other transaction byte, and shall remain inert for
  readers that do not implement the private replay interception.
- **R440 (unwanted behavior):** The consumer-activity record shall not be
  accepted or acted on through the normal client command path, shall not alter
  stream entries or PEL contents, and shall clamp negative timestamps to zero.
- **R441 (ubiquitous):** Tests shall cover binary names, both record types' framing and
  truncation, live-journal and rewrite restarts, consumers with and without
  pending entries, exact `idle`/`inactive` aging across restart, malformed/
  missing-target failure, and all execution modes. Documentation shall retain the
  no-expiry clock shortcut, tombstone-claim semantics and the absence of history
  for deleted/renamed consumers.

## Bounded offline string import

- **R509 (event-driven):** Given an offline RESP stream consisting of an
  optional initial `SELECT 0` and string `SET` records with optional positive
  absolute `PXAT` expirations, emit a legacy Rillcache journal containing the
  exact binary keys, values and expiry commands. Reject malformed, unsupported
  or oversized input before publication. Never modify the source or replace
  an existing destination. This does not establish native AOF interchange.

## Offline backup verification staging (S126)

- **R448 (event-driven):** When `rillcache-backup verify --temp-dir DIR` runs,
  it shall stage the journal payload exclusively in that existing directory,
  with mode 0600 on Unix and no fallback to system scratch. With no explicit
  directory it shall keep using the platform temporary directory. Verification
  shall retain archive length, checksum and replay checks (including HMAC when
  configured), and attempt to remove its staging file on success or error. It
  shall reject empty, missing or repeated option values, missing or non-directory
  paths, and use of the option with `create` or `restore`.

- **R449 (unwanted behavior):** If replay changes the length of the staged
  journal, then offline `verify` and `restore` shall fail even if the archive
  length and CRC are valid. Neither command shall change the input archive;
  `restore` shall not publish a recovered prefix as a successful restore.
  Server startup's torn-tail recovery policy shall remain unchanged.

- **R452 (ubiquitous):** When an offline checkpoint is restored to an empty
  directory with the same binary and settings, the rollback drill shall
  verify the checkpoint's binary value, absolute expiration, key count and
  logical payload bytes, and confirm that a post-checkpoint write is absent.
  Restoring over the newer journal shall fail; the isolated restore shall not
  alter that journal or the checkpoint archive.

## Torn journal restart drill

- **R453 (ubiquitous):** If a Linux restart encounters a torn final checksum or
  HMAC marker in a disposable journal, the drill shall verify that the binary
  truncates to the exact previously verified prefix, applies none of the torn
  transaction, and can append a write that survives another restart. Explicit
  fixture truncation is not a real storage failure or a power-loss test.
