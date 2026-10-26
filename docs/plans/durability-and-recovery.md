# Durability and recovery: design plan (not implemented)

**Status: MIXED. D01/D05/D06/D07/D09/D10 have an implemented append-only subset (S29);
RDB/AOF rewrite, online backup, multi-process/file-locking, stream-pending recovery and
fault-injection drills remain unimplemented.** Rillcache can now opt into a RESP-command
append-only journal with fsync policies, startup replay, partial-tail recovery and
joined-shutdown sync. This is not full Redis durability, RDB compatibility, a backup
solution or a production-readiness claim. See [persistence](../persistence.md).

## Scope and compatibility boundary

The goal is a safe, dependency-free path to persistence for the currently supported
binary string subset, not a promise to implement all Redis persistence. Keep three
formats/claims distinct:

1. **Redis RDB interoperability** means a deliberately versioned RDB reader/writer
   whose output and inputs interoperate with specified Redis and Valkey versions. A
   private checkpoint with an RDB-like header is not RDB compatibility. Redis RDB is
   a versioned binary format with encodings/opcodes and features that evolve; Redis
   8.10's source includes data types/features outside Rillcache's string subset. A
   string-only RDB writer may be consumable by Redis, but its exact version/features
   and reciprocal acceptance must be proven against pinned native binaries. Never
   advertise general RDB compatibility from a one-way or single-version test.
2. **AOF interoperability** requires replaying and emitting Redis RESP command streams
   correctly for an explicitly declared subset and versions. It is not equivalent to
   accepting arbitrary Redis AOFs, modules, functions, multiple databases, or Redis
   7+ multipart AOF manifests/RDB preambles.
3. **Proprietary snapshot** can be a smaller first implementation, but must have a
   distinct extension, magic/version, documentation and tooling. It can establish
   crash-safe checkpoint mechanics only; it closes neither Redis RDB interoperability
   nor AOF compatibility gates.

Start with an offline immutable export/import design, then choose *one* format after
review. Do not combine private checkpointing, RDB, AOF, background rewrite and runtime
configuration in one delivery.

## Current implementation constraints

- `Store` owns `RwLock<Data>`. `Data` contains binary `HashMap<Vec<u8>, Entry>`, exact
  key/value `payload_bytes`, and one indexed expiry per key in a `BTreeSet`. Mutations,
  TTL changes and payload admission are atomic under the store write lock. A snapshot
  must capture entries, expiries and accounting at one consistent point and must
  exclude entries expired at its captured time.
- Expiry is Unix wall-clock milliseconds and existing-key visibility is `now >
  deadline`; observed expiration removes the key, preventing in-process rollback
  resurrection. Expiration work also runs in bounded 128-key batches. Socket timeouts
  use monotonic `Instant` and are unrelated. Persist absolute expiry semantics (not a
  replay-time relative TTL); validate integer/time conversion and rebuild the exact
  index/accounting before publishing loaded state.
- `max_data_bytes` is payload accounting, **not** an RSS/disk limit. Persistence must
  independently bound snapshot/replay bytes, key count, individual key/value sizes,
  record count, arithmetic and temporary disk use. Reject before allocating from
  untrusted lengths; use checked arithmetic and fallible reservations where available.
- Per-client mode and fixed 1/4-worker modes share the Store. Both have managed
  lifecycle and join workers. Any persistence writer/maintenance thread, queue,
  file descriptor and in-flight request must be bounded and joined on shutdown and
  partial-start/error paths. Do not detach a writer or acknowledge queued-but-unwritten
  work as durable.
- AOF replay of relative SET/expiry commands can extend TTL on restart. Encode a
  canonical representation with absolute deadlines (or prove an equivalent Redis
  propagation/rewrite representation) and test replay delay explicitly. Commands with
  rejected syntax, OOM or failed conditions must not enter the log.
- Wall-clock rollback across process restart is a separate unresolved semantic hazard:
  a snapshot made before an expiry can restore a key if the host clock moves backward
  past its deadline. Define and document the clock policy; do not imply monotonic TTL
  guarantees across restart. Consider recording recovery-time/clock high-water state,
  but recognize that persisted wall-clock state cannot establish real elapsed time
  through power-off without a trusted clock.

## Safe standard-library feasibility and limits

Possible building blocks (not an implementation recommendation): `std::fs::File`,
`OpenOptions::create_new`, `Read`/`Write`, `Write::write_all`, `File::sync_data`/
`sync_all`, `std::fs::rename`, `remove_file`, `metadata`, `Path`, and checked/fallible
allocation. Use bounded streaming encoders/decoders; never read a whole untrusted file
based solely on its advertised length. Use a sibling temporary file on the same
filesystem, flush/sync it, then rename, and retain the previous valid generation
until the replacement is validated and published.

The standard library does not provide one portable contract for power-loss-safe atomic
replacement, syncing a containing directory, exclusive cross-process file locks,
durable create/delete of directory entries, or filesystem/hardware cache behavior.
`rename` replacement behavior varies across platforms (notably when the destination
exists on Windows); `File::sync_all` on a file is not a portable guarantee that the
parent directory entry is durable. `sync_data` may omit metadata. A successful write
or fsync does not protect against dishonest storage hardware, controller caches,
filesystem bugs, or all network filesystems. If the project remains std-only, document
supported OS/filesystem guarantees conservatively and test actual supported platforms;
if the required guarantee cannot be made portably, fail closed or require an explicitly
reviewed platform-specific boundary (no unsafe/FFI/dependency exception is authorized
by this plan).

## Proposed INVEST stories and EARS acceptance

All IDs below are **NOT FULLY IMPLEMENTED**. S29 closes the append-only subset of
D01/D05/D06/D07/D09/D10 only; D02/D03/D04/D08 remain unimplemented. Sizes are relative
estimates, not delivery commitments. Confirm remaining format/platform policy with a
human before starting those stories.

| ID / status | Small outcome; size | EARS acceptance and evidence |
|---|---|---|
| D01 — PARTIAL (S29 flags/defaults/path/replay-before-serve) | Define durability/configuration contract; XS | **When** persistence is enabled, the operator shall select a documented format/path and acknowledgment policy; **if** configuration is invalid, path aliases the wrong object, or an unsupported platform/format is selected, startup shall fail before serving writes. Unit/CLI tests cover defaults (off), parse errors, paths and no silent fallback. |
| D02 — UNIMPLEMENTED | Consistent bounded checkpoint export/import in a private versioned format; S | **When** export runs, it shall encode one lock-consistent view of live binary keys/values and absolute expiries with validated counts/lengths/checksum; **if** import sees truncation, trailing bytes, duplicate/invalid records, unsupported version, overflow, expired records, or a limit violation, it shall reject without changing live Store state. Tests cover empty/binary/empty-value/persistent/expiring data, exact accounting/index reconstruction and all malformed boundaries. Explicitly does not close RDB gate. |
| D03 — UNIMPLEMENTED | Crash-safe checkpoint publication and restore; S | **When** a checkpoint is published, it shall use same-directory temporary output, complete writes, file sync, verified atomic replacement where supported, and preserve/recover the last valid generation; **if** any create/write/flush/sync/rename/cleanup step fails, then no success is acknowledged and the previous valid file remains recoverable. Fault-injection tests at each step, subprocess crash points, checksum/old-generation recovery and filesystem matrix. |
| D04 — UNIMPLEMENTED | Pinned Redis RDB string-subset import/export; M | **When** an explicitly supported RDB version is imported/exported, it shall interoperate byte-for-byte semantically with declared Redis/Valkey reference versions for binary strings, empty values, absolute expiry, and dataset metadata; **if** RDB version, opcode/type, checksum, database, module/function metadata, or record is unsupported/corrupt, then loading fails closed before live state replacement. Include bidirectional native tests and version matrix; publish no broader compatibility claim than tested. Human review of format-version policy required. |
| D05 — PARTIAL (S29 final-state records; no all-or-nothing failure latch) | Serialized AOF mutation journal and recovery; M | **When** a supported successful mutation is committed, the journal shall contain exactly one complete, ordered, replay-safe command/record reflecting that mutation and absolute expiry; **if** validation, condition, payload admission, append or synchronization fails, then it shall not report success or permit a later write to leapfrog the failure. Test SET/NX/XX/GET/TTL, GETEX, DEL, PERSIST, expiration and counters (as supported), concurrent same-key and different-key writers, pipelines and failure atomicity. |
| D06 — PARTIAL (S29 streaming replay/tail truncation; no temp candidate state or explicit warning) | AOF replay validation and tail policy; S | **When** replay starts, it shall be bounded by configured file, frame, argument, key/value and record limits and build a temporary candidate state; **if** a final incomplete record matches the documented tail policy, only that tail may be discarded with an explicit warning/repair result; if corruption occurs in the middle, an unsupported command/version appears, or any bound is exceeded, startup shall fail without exposing partial data. Test byte-by-byte truncation, malformed RESP, valid prefix + corrupt interior, duplicate/unknown options, late relative TTL replay and malicious length arithmetic. |
| D07 — PARTIAL (S29 policies + visible errors; no ENOSPC injection/queue latch) | Acknowledgment/fsync contract and disk-full behavior; S | **When** policy is `always`, `everysec`, or `no` (if supported), replies and documented loss window shall match the actual append/sync state; **if** storage fills, write/sync fails, or the writer queue is full, then the server shall stop accepting affected mutations, return a clear error where possible, latch/fail storage health, and never claim durability. Inject short writes/ENOSPC/sync errors; assert response ordering, no applied-but-unlogged writes, queue bounds, restart replay and failure visibility. Choose policy names only after human agreement; do not describe every-second fsync as zero loss. |
| D08 — UNIMPLEMENTED | Checkpoint/AOF rotation and online backup; M | **When** rotation/checkpoint runs concurrently with writes, it shall produce a complete base plus ordered delta boundary with an atomic manifest/generation switch and retain a recoverable prior generation until publication; **if** child/writer/manifest work fails, old base plus journal remain authoritative. Validate backup from a quiescent copy and the supported online procedure; restore into an isolated directory and compare full key/value/expiry state. Do not copy a changing AOF set or manifest blindly. |
| D09 — PARTIAL (S29 startup-before-serving, joined drop sync, all-mode tests; no drain deadline) | Managed persistence lifecycle and shutdown; S | **When** either server mode starts/stops, persistence resources shall be initialized before serving, bounded, drained or explicitly failed under the chosen shutdown deadline, and every started thread joined; **if** startup partially fails, shutdown begins during append/fsync/rewrite, or a writer panics/exits, then handles/queues/files are closed consistently and no success reply exceeds the configured acknowledgment guarantee. Run all durability wire tests in per-client, 1-worker and 4-worker modes, plus repeated create/drop and partial-start tests. |
| D10 — PARTIAL (S29 restart/tail/corruption tests; no backup or platform matrix) | Backup/restore and recovery drill corpus; S | **When** a supported backup is restored on each supported platform/version, the recovered state shall match expected binary bytes and TTL/accounting at a deterministic test clock; **if** backup corruption, stale generation, rollback, disk-full or process-kill occurs, then the documented recovery procedure either recovers the last valid point or refuses startup visibly. Preserve reproducible fixture hashes and explicit expected data-loss bounds. |

### Next-wave implementation order and ownership (proposed)

1. **Decision gate (human-owned):** select private-first vs RDB-first, in-memory and
   on-disk limits, format/version matrix, fsync/acknowledgment semantics, default-off
   config, allowed paths/platforms, crash consistency claims, and whether `SAVE` or
   background operation is actually in scope. No source changes before this review.
2. **D01 + D02, separate PR:** likely `src/persistence/mod.rs`, private codec module(s),
   and new `src/persistence_tests.rs`/module-specific tests. The implementation owner
   owns final `src/engine.rs` and `src/server.rs` integration; first add a narrowly
   specified Store snapshot/replace API that holds the lock only to capture/publish,
   not while slow file I/O runs. Recompute expiry index and payload bytes from
   validated records; never persist map internals or rely on iteration order. Test
   golden-format fixtures, bounded parser properties via table-driven adversarial
   bytes, clock expiry and all-or-nothing import.
3. **D03, separate PR:** new persistence file helper and fault-injectable I/O boundary;
   same-volume temp file, checksummed read-back, sync/rename/preserve-old sequence.
   Test each injected operation failure and subprocess termination before/after
   sync/rename. Document the directory-sync/platform boundary rather than treating
   rename as universally durable.
4. **D04, only after compatibility decision:** `src/persistence/rdb.rs`, fixtures and
   standalone optional integration harness (no Rust dependency). Pin exact Redis and
   Valkey server versions/digests in test tooling. Test Rillcache export loaded by each
   reference and reference-generated fixture loaded by Rillcache; compare binary
   values and expiry with tolerances controlled by injected/reference clocks. Reject
   non-string records and unsupported features before mutation. Keep policy and
   benchmark gate files untouched.
5. **D05–D07, after D01 contract and D02 safe state replacement:** `src/persistence/aof.rs`
   and a single sequenced writer/journal owner; add transactional Store integration
   through a minimal private API. Define record commit point and failure state before
   implementation. Verify ordered concurrent mutations, partially written tails,
   mid-file corruption, OOM/condition rejection, relative TTL canonicalization,
   response-vs-sync ordering, ENOSPC, and debug/release arithmetic. No concurrent
   timing benchmarks; correctness/fault injection comes first.
6. **D08–D10 last:** rotation/manifest and backup tooling, then managed server wiring
   and operations/recovery drills. Integrate one mode at a time and test thread-per-
   client, one-worker and four-worker startup, concurrency, shutdown/drop, accepted
   writes and re-opened state. Final test run: fmt assigned Rust files; offline unit
   and wire suite; strict Clippy; debug and release tests; Redis/Valkey interoperability
   corpus on pinned native binaries. Preserve every failure and do not rerun for a
   preferred result. Performance benchmarking is a later isolated task, never
   concurrent with other workers.

Proposed ownership boundaries for an independent durability implementation: the
implementation owner should own persistence codec/filesystem modules, Store API changes,
final dispatch/config integration, and new persistence-owned tests in one coherent
feature scope. Policy/CI/Cargo/benchmark gates remain protected and are not durability
deliverables. Existing shared tests must not be weakened; update unsupported-command
expectations only if a future implementation genuinely adds those commands.

## Failure model, invariants, and unresolved decisions

- **Atomic state:** load into a fresh candidate, validate every byte/record/checksum/
  limit, then swap the whole state under one write lock. Never clear the live store and
  fill it incrementally. During append, serialize mutation order with the journal and
  explicitly choose whether the durable record precedes applying in-memory state.
  Partial write must poison the writer until repair/restart; do not continue at an
  unknown file offset. On recovery, replay is a state transition once per complete
  record.
- **Acknowledgments:** distinguish accepted in-memory write, appended-to-kernel-cache
  write, and fsynced durable write. `write_all` is not durable. `everysec` necessarily
  permits a documented loss window and may lose more under stalls/failure; `always`
  still depends on filesystem/device guarantees. A queued background sync cannot be
  called durable before completion. What `+OK` promises is a human product decision.
- **Disk full/permission/I/O:** cover ENOSPC during record header/body, flush, file
  sync, checkpoint write, manifest write, rename and cleanup; preserve the old
  recoverable generation; surface health failure; bound retries and queue. Avoid
  unbounded retries, silent fallback to volatile mode, or successful no-ops.
- **Crash/rename:** test abrupt process death at every publication boundary. Distinguish
  process-crash safety from power-loss durability. A temp file can be complete but
  unpublished, or a rename visible but directory metadata not durable. Windows and
  network filesystems need explicit behavior. Multi-file AOF needs an atomic manifest
  protocol and crash states; one file rename is not sufficient for a multipart set.
- **Corruption/repair:** preserve an untouched copy before repair. Only a terminal
  incomplete append record may be repairable, and only if the exact record framing
  lets the loader prove it ends at EOF. Never truncate through valid later records or
  auto-skip middle corruption. Report repaired offset/bytes and retain source evidence.
  Checksums detect many errors, not malicious tampering/authenticity.
- **Expiry/accounting:** include binary keys/values (including empty values), exact
  payload limit, persistent/expiring records, past deadlines, boundary `now ==
  expires`, duplicate keys and one expiry-index entry. Decide whether checkpoint
  serializes logically expired-but-not-yet-reaped entries (it should omit them at
  capture time). Validate restored total before publication and do not false-OOM.
- **Concurrency/admission:** snapshots are point-in-time; writes after snapshot need
  ordered AOF delta if claiming a newer recovery point. Define memory-copy pressure and
  avoid holding the Store lock over disk I/O. Persistence queues/workers and accepted
  clients must be bounded independently; backpressure/failures must not make worker
  count or threads unbounded. Test mutation overlap with snapshot and every worker mode.
- **Clock and shutdown:** absolute expiration avoids restart-time extension from
  relative replay but cannot solve host clock rollback while offline. Persist and
  document chosen assumption; add deterministic injected-clock tests plus an explicit
  rollback scenario. Shutdown must stop admission, settle in-flight journal requests
  according to acknowledgment policy, perform any promised final sync/checkpoint, close
  resources and join writer/maintenance/client threads. `Drop` cannot return errors;
  explicit shutdown must report them and unexpected drop must not be claimed as a
  successful flush.
- **Backup/restore:** backup is a separately verified artifact, not merely a copy of
  a live file. Define quiescent or online procedure, integrity verification, version
  metadata, retention, encryption/storage handled by operator, restore target isolation,
  and a drill proving recovery. A locally valid file is not a tested backup. No
  replication, HA, authentication or TLS follows from local persistence.

## Commands deliberately not implemented / not promised

No command or persistence mode is implemented here. In particular, `SAVE`, `BGSAVE`,
`LASTSAVE`, `BGREWRITEAOF`, `INFO persistence`, `CONFIG GET/SET`, `SHUTDOWN SAVE/ NOSAVE`,
Redis backup command-family operations, RDB import/export commands, Redis 7+ multipart
AOF manifests, arbitrary AOF replay, replication/PSYNC, `WAIT`, transactions and all
unsupported data types remain unimplemented. Even a future internal persistence
configuration does not imply command compatibility. Each public command requires its
own reviewed grammar, state/error/authorization semantics, reference tests and
compatibility declaration.

## Upstream references reviewed

Official docs are living pages; record the fetch date/revision in implementation PR
fixtures and pin executable versions rather than treating `/latest/` as a fixed spec.

- [Redis persistence: RDB, AOF, fsync, rewrite, truncation/corruption and backup guidance](https://redis.io/docs/latest/operate/oss_and_stack/management/persistence/) — reviewed for point-in-time RDB, temporary output/replacement, AOF policies, tail recovery, middle corruption, and Redis 7 multipart AOF/manifest behavior.
- [SAVE](https://redis.io/docs/latest/commands/save/) and [BGSAVE](https://redis.io/docs/latest/commands/bgsave/) — synchronous vs background snapshot semantics and operational cautions.
- [BGREWRITEAOF](https://redis.io/docs/latest/commands/bgrewriteaof/) — rewrite scheduling and concurrent persistence constraints.
- [Redis configuration](https://redis.io/docs/latest/operate/oss_and_stack/management/config/) — configuration is versioned/operational, not inferred from a file name.
- [Redis replication and full synchronization](https://redis.io/docs/latest/operate/oss_and_stack/management/replication/) — RDB transfer and command-stream recovery are not equivalent to local backup or HA.
- [Redis source: `rdb.h`](https://github.com/redis/redis/blob/unstable/src/rdb.h) and [`rdb.c`](https://github.com/redis/redis/blob/unstable/src/rdb.c) — upstream format constants/reader/writer; `unstable` moves and is not a compatibility pin.
- [Rust `File`](https://doc.rust-lang.org/std/fs/struct.File.html), [`OpenOptions`](https://doc.rust-lang.org/std/fs/struct.OpenOptions.html), [`fs::rename`](https://doc.rust-lang.org/std/fs/fn.rename.html), and [`Write`](https://doc.rust-lang.org/std/io/trait.Write.html) — available safe std APIs, not a cross-platform crash-consistency certification.

## Human validation checklist

Before coding, a human owner must decide and approve: supported format/version/platform
matrix; private-vs-RDB-first sequence; relative limits and disk quotas; persistence
on/off defaults; accepted durability acknowledgment/loss window; data-path ownership,
permissions and symlink/path policy; exact startup corruption/tail-repair behavior;
clock rollback semantics; shutdown timeout/error behavior; backup retention and restore
procedure; and the scope of any compatibility claim. Until then, all stories remain
unimplemented and Rillcache remains volatile.