# Append-only persistence and recovery

> **Current scope:** durability completeness is explicitly deferred. The journal
> remains implemented and tested as best-effort, but there is no RDB, authenticated
> journal records, background fork/COW rewrite, ENOSPC/soak validation, backup drill,
> point-in-time recovery or signal-driven drain. Do not treat it as a durable
> system of record.

Rillcache can persist supported keyspace mutations to an append-only command log.
This is a real recovery path with explicit failure semantics, but it is **not** a
complete Redis durability implementation: there is no RDB format, no
background/forked AOF rewrite, no shared-filesystem ownership, no redis.conf
integration, and stream consumer activity is restored only for keyspaces where the server tracks wall-clock activity.

## Configuration

```sh
rillcache \
  --appendonly yes \
  --dir /data \
  --appendfilename appendonly.aof \
  --appendfsync everysec
```

| Option | Values | Default |
| --- | --- | --- |
| `--appendonly` | `yes`, `no` | `no` |
| `--dir` | Existing directory | `.` |
| `--appendfilename` | Basename without `/` or `\` | `appendonly.aof` |
| `--appendfsync` | `always`, `everysec`, `no` | `everysec` |

The log is created with mode `0600` on Unix. The process must be able to read,
write and truncate it at startup. Pointing two server processes at the same file
is protected by a local ownership lock; shared/network filesystem coordination
is unsupported.
The scratch container's root filesystem is read-only, so mount a writable volume
at `/data` and add `--appendonly yes --dir /data` when persistence is required.

## Runtime configuration visibility

`CONFIG GET appendonly appendfilename appendfsync` (and `CONFIG GET *`) reports
the configured append-only mode, journal basename and live fsync policy:

```text
CONFIG GET append*
1) "appendfilename"
2) "appendonly.aof"
3) "appendfsync"
4) "everysec"
5) "appendonly"
6) "no"
```

`appendfsync` is the only journal setting that can change at runtime:

```text
CONFIG SET appendfsync always
CONFIG SET appendfsync everysec
CONFIG SET appendfsync no
```

The update applies to the open journal immediately and affects subsequent
appends. Switching away from `no` first syncs records that were previously only
written to the operating system, so the stronger policy covers acknowledged
writes already in the file; a sync failure leaves the journal failed and returns
an error. `appendonly` and `appendfilename` are startup-only and reject
`CONFIG SET` with a visible error. There is no `redis.conf` reload and no
runtime `appendonly yes`/`no` toggle: restart with the desired flags to open,
close or rename the journal. When append-only persistence is disabled the
reported policy is still updated, but it has no effect because no journal exists.

## Format

The file is a plain sequence of RESP2 command arrays. Each mutating command is
translated into the final state of the keys it changed:

- `DEL key` first, so a type change or deletion is unambiguous;
- `SET key value` for strings;
- chunked `RPUSH`, `SADD`, `HSET`, `ZADD` and per-entry `XADD` for collections
  and streams, in deterministic order;
- `XGROUP CREATE key group id` for stream group IDs;
- an absolute `PEXPIREAT key deadline` for surviving TTLs;
- `FLUSHALL` when the database was cleared or the bounded mutation log overflowed.

This is a Redis-compatible command subset, not a byte-format claim. A real Redis
AOF containing commands this server does not implement fails startup visibly
rather than being silently skipped. Our log has no embedded checksums; corruption
is detected by RESP parsing and command failure. Without an explicit rewrite the
file grows with every write and old records remain as history.

No file is created when `--appendonly no`. ACL users are separate and live in the
ACL file; data persistence never writes passwords.

## Mutation capture

The store records a bounded ordered event log of key mutations (65,536 entries).
After each command, the key's current value is captured while holding the data
lock, and the assembled records are appended in order. With persistence enabled,
commands execute through the exclusive transaction gate, so mutation, capture and
append keep a single global order. This trades throughput for deterministic
replay; persistence mode is not a performance claim.

A command that does not mutate data (for example a failed `SET NX`, an overflowed
`INCR`, or a rejected argument) appends nothing. `MULTI`/`EXEC` captures the final
state after the whole queued batch, not each intermediate command. If the bounded
event log overflows, the server writes `FLUSHALL` followed by a complete snapshot
of every current key, which is expensive but cannot miss a key.

Expiration is persisted as an absolute Unix-epoch-millisecond deadline and
compared with the operating system wall clock. Replay applies the same deadline,
independent of load time. Once an expired key is observed and removed, its `DEL`
is persisted; however, if the clock moves backward before that removal is observed
and recorded, the key can become live again, including after restart. There is no
persisted clock high-water mark or anti-rollback state. Operators should keep the
system clock synchronized and avoid backward steps while using TTLs. Time-zone and
daylight-saving changes do not alter Unix-epoch time and therefore do not affect
TTL deadlines; manual or time-sync-induced system-clock steps can.

## Acknowledgment and fsync policy

- `always`: append and `sync_data` complete before the command's reply is written.
  A successful reply means the records reached the filesystem.
- `everysec`: records are written immediately and fsynced by the maintenance loop
  about once per second. A crash can lose up to roughly one second of acknowledged
  writes and can lose the tail of the last command.
- `no`: records are written to the OS but never explicitly fsynced. Only an orderly
  shutdown (or the OS) provides durability.

The policy can be changed with `CONFIG SET appendfsync` as described above. A
policy change is not a durability guarantee for writes made before the change:
the strongest statement is that records acknowledged under `always` were synced
before their replies, while `everysec` and `no` retain their documented windows.

On orderly shutdown, joined workers drop the store and the journal's `Drop`
performs a final sync/flush. The `SHUTDOWN [NOSAVE|SAVE] [NOW] [FORCE]` command
forces that flush before stopping the accept loop and joining workers, so clients
can request a durable stop over the protocol. `NOSAVE` and `SAVE` are accepted
for client compatibility but neither writes an RDB file; both flush the journal
when append-only persistence is enabled. `NOW`/`FORCE` are no-ops because shutdown
is already immediate, and `ABORT` returns `No shutdown in progress.` because no
delayed shutdown state exists. The standalone binary handles SIGINT/SIGTERM with
worker joins and an explicit journal flush under a 25-second deadline. Signal
shutdown stops listener admission and drains replies for commands already
dispatched, but it is not a full client drain: idle/incomplete clients,
subscriptions, blocked commands and replies that exceed the deadline may be
abandoned. Forced termination can still lose the unsynced tail. `always` is the
only policy that does not depend on a clean process exit for recent acknowledged
writes unless a client sends `SHUTDOWN`.

If an append or fsync fails, the command reply is replaced with
`ERR append-only write failed: ...`, the journal enters a failed state and later
writes fail. The in-memory mutation may already be applied; a restart replays only
complete, checksum-verified transactions. Disk-full, quota and permission errors
are visible rather than silently converted into success.

## Transaction checksums

Each newly written transaction is followed by a standard RESP `ECHO` checksum
record (CRC32 plus byte length). A format marker identifies the checksummed
segment. Legacy RESP-only journals remain readable and switch to a checksummed
segment on their first new append; rewriting compacts the legacy prefix into one
checksummed snapshot.

At startup, Rillcache verifies each complete checksum group before replaying any
command. Valid-RESP byte corruption fails closed with a checksum mismatch. A torn
final transaction/checksum is truncated to the previous verified boundary, so
commands from an incomplete group are never replayed. The `ECHO` markers have no
database effect if the RESP journal is inspected or replayed by another
Redis-compatible server.

CRC32 detects accidental corruption, not a malicious editor that recomputes the
checksum. Encryption at rest and key ownership/rotation remain open in production
gap D3; optional record authentication is described next.

## Journal authentication

Set `--aof-hmac-key-file PATH` (16..=1024 bytes after stripping one final LF or
CRLF) to authenticate journal transactions with HMAC-SHA256. The server and
backup utility read at most 1027 bytes and reject oversize files rather than
accepting a prefix. The byte limit is not an I/O deadline. The in-repo SHA-256 and HMAC
construction is the same dependency-free code used for audit-line authentication;
the key is loaded once at startup and is not exposed in `CONFIG GET`, `INFO` or
process arguments.

With a key configured on an empty journal, the first append writes an anchor
record and then a keyed hash chain over every following checksum group. On
restart, the complete chain is verified before any command is replayed: a
modified value, a reordered transaction, or a transaction inserted before the
final marker, a malformed marker, or a wrong key fails startup. A crash-torn final unauthenticated group is discarded to the last
verified marker, matching the checksum recovery rule. The marker records are
ordinary RESP `ECHO` commands, so a journal can still be inspected or replayed by
a Redis-compatible tool; without the key they are inert and only CRC32 validation
applies.

Enabling a key on an existing journal requires an explicit one-time startup flag:

```text
rillcache --appendonly yes --dir /data \
  --aof-hmac-key-file /run/secrets/rillcache-aof.key --aof-hmac-migrate
```

`--aof-hmac-migrate` validates the existing checksum groups, seals the entire
current prefix with an anchor, and syncs it before serving clients. Start it only
with the server stopped and the existing file trusted: anything already in the
prefix is authenticated as-is. On every later start, omit the flag; without an
anchor the server fails closed rather than silently sealing an attacker-supplied
file. `--aof-hmac-migrate` without a key, or authentication without
`--appendonly yes`, is rejected.

Authentication covers transaction content, order and the migrated prefix. It does
not provide encryption, key rotation/keyrings, protection from someone who can
replace both the key and the whole journal, or detection of an externally replayed
older complete journal. Bytes after the last durable marker can be removed
exactly like a crash-torn tail, so an attacker can cause suffix data loss;
truncating an older durable transaction is indistinguishable from rollback
without external state.
Keep the key file readable only by the service account with filesystem and
secret-store controls, back it up separately from the journal, and do not remove
the key from a running deployment and then append: those later unauthenticated
writes are discarded by the next authenticated start. `BGREWRITEAOF` writes a
fresh authenticated snapshot when a key is configured.

The offline `rillcache-backup` tool accepts the same `--aof-hmac-key-file` for
`create`, `verify` and `restore`; supply it to authenticate the inner AOF during
archive validation. Without it, backup verification still checks the outer CRC and
checksums but cannot detect keyed modification. `create` refuses a nonempty
journal that has no anchor when a key is supplied; run the server once with
`--aof-hmac-migrate` after stopping it, then stop the server and take the backup.

## Single-process ownership

A journal is owned by exactly one process. Startup claims `<journal>.lock`
(`appendonly.aof.lock`) with an exclusive create containing the owner's PID and
kernel start time; a second server pointed at the same `--dir` fails with
`append-only journal ... is already owned by process N` instead of interleaving
appends. A lock left by a crash is reclaimed only when the recorded process is
gone or its PID has been reused (the start time no longer matches), so a live owner
is never displaced. Clean shutdown removes the lock; the lock is absent when
`--appendonly no`.

This is an **advisory local-filesystem lock**. On Linux the PID plus `/proc`
start-time lets startup reclaim a stale lock after a crash. On platforms without
that process identity source, startup fails closed on any existing lock; after
confirming the old process is gone, an operator removes the stale `.lock` file.
The lock does not coordinate writers on a shared/network filesystem; such a volume
is unsupported and is explicitly excluded by the
[standalone production profile](production-profile.md). Keep journals on local
storage or guarantee single-process access externally.

## Offline backup and restore

The `rillcache-backup` binary supports a versioned, bounded-memory archive of a
quiescent AOF. Stop the source server first; `create` opens the journal through
the normal exclusive ownership lock, replays it, and forces a journal sync before
copying. A running writer therefore causes backup to fail without publishing an
archive. The archive stores the AOF bytes with their length and CRC32, streams in
1 MiB chunks, and is limited to 16 GiB. The temporary archive is mode 0600 on Unix,
synced, then hard-linked into place without replacing an existing output. Pass
`--aof-hmac-key-file` for an authenticated journal; see the authentication section
above. The archive and destination must be on a local filesystem that supports same-directory
hard links and directory fsync; shared/network filesystems are unsupported.

```text
rillcache-backup create --dir /data --output /backups/rillcache-2026-11-07.rcb
rillcache-backup verify --input /backups/rillcache-2026-11-07.rcb --temp-dir /verify-scratch
rillcache-backup restore --input /backups/rillcache-2026-11-07.rcb --dir /restore
```

`--filename` selects the source AOF basename for `create` and the destination
basename for `restore`. They need not match. Set the recovering server's
`appendfilename` to the restored name before starting it; otherwise it may open
or create a different journal. If the source uses non-default `max-data-bytes`,
protocol frame/bulk or argument limits, pass matching
`--max-data-bytes`, `--max-frame-bytes`, `--max-bulk-bytes`, and
`--max-arguments` values to create/verify/restore. If replay exceeds the logical
payload budget, verification and restore fail without publishing a partial
journal. Retry with the source's configured budget if appropriate; this limit
is not an RSS bound. Destination directories must
already exist and the restored AOF/lock must be absent. Restore verifies archive
length/CRC and replays a temporary journal before atomically publishing it; it
never overwrites an existing journal. A complete legacy RESP archive restores
byte-for-byte; subsequent appends add CRC-framed records without changing its
legacy prefix. That does not retroactively checksum the prefix or establish
compatibility with older binaries. Use a dedicated offline directory per
restore. If two restore commands target the same empty directory, only one can
publish its journal; inspect the command results before starting a server and
use a new empty directory to inspect the other archive. If source validation
would require truncating an incomplete tail, `create` fails without modifying the source or
publishing an archive. Preserve the original and investigate before choosing
whether to recover a copy with server startup. Both `verify` and `restore` reject an
archive if replay truncates the staged journal, even when the archive's outer
length and CRC match. This is stricter than server startup recovery, which can
retain a valid prefix after a torn write. Start a separate Rillcache instance on the
restored directory to perform the operational restore drill. For authenticated
archives, a wrong-key restore is an error, not a partial restore. The hosted
integration test checks that normal failure in a fresh local destination leaves
no staged journal or lock, then retries with the matching key in the same
directory and reads back the restored value. The archive remains unchanged.
This does not imply cleanup after interruption or filesystem failure.

Verification stages a temporary copy of the journal payload. Use `verify --temp-dir DIR`
with an existing, trusted writable local directory to select a volume with space
for the payload (up to 16 GiB). Without the option, verify uses the platform
temporary directory. The staged file is created exclusively with mode 0600 on
Unix and is removed on normal success or error when the filesystem permits;
interrupted verification can leave a partial file. The option is not accepted
by `create` or `restore` and does not change restore's publication path.

This is offline copy, not online backup, point-in-time recovery, rotation or
retention. The archive contains only the AOF—not ACL files or server configuration.
RPO/RTO are deployment-specific and are not claimed here. CRC32 detects accidental
corruption but does not authenticate an archive or protect against malicious
modification. Keep archives on trusted storage with separate access controls.

### Drill a rollback to an offline checkpoint

1. On an isolated standalone deployment, quiesce application writes. Before
   stopping the server, record expected binary values, key count, absolute
   expiry deadlines (`PEXPIRETIME`), and logical payload bytes (`INFO memory`,
   `rillcache_payload_bytes`) for the drill's test data. Record the candidate
   digest, shutdown time, and checkpoint boundary. After the server stops, use
   `rillcache-backup create` and `verify` as above; retain the verified archive
   separately from the working journal.
2. Restart the working instance and make a distinct post-checkpoint write.
   Simulate a rollback decision, stop that instance, and retain its journal,
   configuration and credentials. Do not overwrite the newer journal: any
   acknowledged writes made after the checkpoint are absent from the rollback.
3. Restore the checkpoint into a **new, empty** directory. Start the same
   binary with that directory and the matching limits, AOF basename and HMAC
   settings, on an isolated loopback endpoint with no application traffic.
   Check the recorded values, key count, absolute expiry deadlines and logical
   payload bytes, and confirm the post-checkpoint write is absent. If keys
   expired during the drill, check their expected absence instead. Stop and
   investigate on a mismatch; do not switch traffic to an unverified restore.
4. Record the interval of acknowledged writes that the checkpoint cannot
   recover and the time from the rollback decision to verified readiness.
   Preserve both journal generations, the archive and test output per the
   deployment's retention policy, even on failure.

`tests/aof_backup.rs` exercises the same-binary scenario, including refusal to
restore over the newer journal and byte-for-byte preservation of that journal
and the checkpoint archive. It does not measure a deployment's RPO/RTO or
validate a downgrade across binary versions.

## Compaction with BGREWRITEAOF

`BGREWRITEAOF` replaces the log with a compact snapshot of the current keyspace:
`FLUSHALL` followed by every live key's rebuild records. Since the cooperative
rewrite change it is **not** a single blocking pass:

1. The command materialises the key list, creates a mode-0600 temporary file and
   checksum-format header in the journal directory, then replies
   `+Background append only file rewriting started`. No keyspace values are
   encoded under this first pause; it is bounded by the number of key names.
2. The maintenance loop advances one bounded slice per tick (at most 512 keys or
   1 MiB of encoded state). Between slices the server keeps serving commands.
   Every key changed since the previous round is re-encoded as a **whole value**
   (`DEL` plus rebuild), never as the original incremental command, so a key
   visited twice cannot duplicate collection members.
3. The snapshot is fsynced outside the exclusive gate. A final round re-encodes
   the keys written during that fsync, then a short gate appends the residual,
   fsyncs, renames the temporary file over the journal and reopens it at the
   snapshot end.

`INFO persistence` reports `aof_rewrite_in_progress` while a rewrite is running,
the current `aof_rewrite_phase` (`snapshot`, `reconcile`, `sync`, `commit`, or
`none` when idle), and stable snapshot progress as
`aof_rewrite_keys_done`/`aof_rewrite_keys_total`. Reconciliation keys may be
visited more than once, so the done count deliberately stops at the snapshot
total and does not invent duplicate progress. `aof_rewrite_bytes_written` counts
bytes written to the temporary file, including framing and reconciliation. It
resets to zero when the rewrite finishes or aborts, and is omitted if AOF is
disabled or journal status is unavailable. It does not represent durable bytes
or a completion percentage. `aof_last_rewrite_start_usec` measures the latest
successful rewrite setup in whole microseconds, from starting snapshot key
collection through writing the staging header and initial record. It is omitted
before the first successful setup and excludes later rewrite work and time spent
waiting for the request to reach the owner. `aof_last_rewrite_finish_usec`
measures the latest successful finalization under exclusive data access, from
acquiring that access through encoding remaining changes, syncing and replacing
the journal, and resetting rewrite bookkeeping. It excludes earlier work and
client-observed delays, and is not a pause limit or a standalone fsync measure.
It is omitted before the first success and retained across failed attempts.
`aof_last_bgrewrite_status:ok|err` describes the last completed attempt,
including failure to initialize the staging file. A staging setup failure does
not latch the live append status. The final snapshot checksum is written and
synced before the atomic rename. A crash before
the rename leaves the previous journal intact; the temporary file is removed on the
next start and never replayed. Killing the server mid-rewrite keeps every
acknowledged write (covered by `tests/durability_crash.rs`).

Measured on the development host with 20,000 keys of 64-byte values (2.5 MB
journal): the old synchronous pass blocked command service for **130 ms**; the
cooperative rewrite replies in **9–20 ms** (key-list materialisation), encodes
**1.6–2.4 ms** per 512-key slice, commits in under 1 ms beyond its residual fsync,
and completes the background pass for that dataset in roughly 4–5 s while clients
are served between slices. Completion time and the start pause still scale with
the keyspace: there is no fork, copy-on-write page sharing, or incremental
generation beyond this. A write storm during the rewrite makes the reconciliation
rounds longer; the slice size bounds each round, not the total.

Automatic rewrite thresholds are configurable at runtime:

```text
CONFIG SET auto-aof-rewrite-percentage 100 auto-aof-rewrite-min-size 67108864
```

`auto-aof-rewrite-percentage` defaults to `100` and `0` disables automatic
rewrites. `auto-aof-rewrite-min-size` defaults to 64 MiB. The maintenance loop
checks the open journal about every 100 ms and starts the same cooperative rewrite
once the file is at least the minimum size and has grown by the configured
percentage over the size recorded at the last rewrite. A successful rewrite resets
that baseline. Rewrites no longer hold the exclusive gate for the whole snapshot;
transaction gate; there is no fork, background progress or incremental rewrite
claim.

## Recovery

At startup the journal is replayed into a fresh, authenticated store before the
listener starts accepting clients:

- a complete prefix of valid commands is executed;
- an incomplete final command (a crash during append) is ignored and the file is
  truncated back to the last complete command;
- malformed RESP, a command that returns an error, or a complete transaction with
  a checksum mismatch fails startup with the byte offset; startup never silently
  skips a corrupt middle section;
- allocation/payload limits apply during replay, so an OOM while loading is a
  startup failure rather than partial data;
- a replayed command may be rejected if it was produced by a newer/unknown command
  set; that is visible corruption for this binary.

Replay is sequential and single-threaded. After replay, the file is positioned for
new appends. Shutdown/reopen is the only supported way to pick up external edits;
do not edit a live file.

## Streams limitation

Stream entries, last IDs, consumer-group cursors, **consumer objects and their
pending-entry lists** are rebuilt. The encoder emits `XGROUP CREATECONSUMER` for
every consumer and `XCLAIM ... TIME <delivered-at> RETRYCOUNT <deliveries> JUSTID
FORCE` for every pending id, so PEL owners, delivery counts and delivery
timestamps survive restart.

A pending id can outlive its stream entry after `XDEL` or trimming. During a
snapshot/rewrite the encoder adds a minimal placeholder `XADD` for each such id in
ascending order, claims it into every group that references it, and then `XDEL`s
the placeholder. The stream key and its PEL tombstone remain, so `XPENDING` still
reports the original owner/count/time and `XACK` can remove it; `XLEN`/`XRANGE`
never expose the synthetic payload. This uses ordinary RESP commands, so a
Redis-compatible replay sees the same create/claim/delete sequence.

Consumer last-activity (`seen_at`) is also rebuilt. After the group/consumer/PEL
records the encoder writes one namespaced RESP `ECHO` record per consumer with a
binary length-prefixed key/group/consumer and the absolute millisecond timestamp.
The record is inert for other Redis-compatible readers; Rillcache's replay
intercepts it and sets the existing consumer's `seen_at`. `XINFO CONSUMERS`
`idle`/`inactive` therefore keep counting from the original activity time,
including downtime, instead of resetting at replay. The record is covered by the
same CRC32 and optional HMAC chain as every other transaction byte.

Limits of this subset: the server's existing clock shortcut records activity as
zero when the keyspace has no expirations or field TTLs, so `idle`/`inactive` stay
zero before and after restart there; adding a TTL later makes the stored value
meaningful. `XCLAIM` of a tombstone removes it from the PEL (matching the command's
missing-entry behavior) rather than returning a payload, and full historical
activity (for consumers that were deleted or renamed) is not retained.

## Deliberate gaps

- No RDB import/export, manifest/multi-part AOF, encryption at rest, key rotation
  or replication of the log. New RESP transactions have CRC32 checksums, and an
  optional HMAC-SHA256 chain authenticates transaction content/order when a key is
  configured. HMAC is not encryption, does not protect a whole-file replacement by
  an attacker with the key, and is not an anti-rollback mechanism.
  `BGREWRITEAOF` is cooperative and incremental, but it is not Redis's fork-based
  rewrite: the key list is materialised in memory and the
  rewrite runs on the owner thread between maintenance ticks.
- No full client-draining signal shutdown; the standalone signal path joins
  workers and flushes the journal but may lose in-flight replies. See
  [container delivery](containers.md).
- No shared-filesystem coordination: ownership is a local advisory lock
  (`<journal>.lock`), not a distributed lease. Crash-consistency of append and
  rewrite is covered by `tests/durability_crash.rs`.
- `rillcache-backup` implements offline, quiescent AOF backup/verify/restore.
  There is no online backup, point-in-time recovery, retention policy or log
  rotation. Operators must size/rotate archives and test restores.
- Per-command acknowledgment is not a transaction commit in the distributed sense;
  `MULTI` atomicity is process-local, and replica/failover semantics do not exist.

These gaps keep the production durability gate open even though ordinary restart
recovery is implemented and tested.

## Evidence

Unit tests cover strings, binary data, lists, sets, hashes, sorted sets, streams,
groups, absolute TTL restoration, rejected non-mutating commands, transactions,
concurrent writers, flush records, partial-tail truncation, mid-file corruption and
rewrite compaction. Automatic rewrite tests cross configured growth thresholds,
verify compaction and that the baseline resets, and replay the compacted file.
Wire tests restart real servers in all worker modes, verify data and TTLs survive,
compact a live log with `BGREWRITEAOF`, exercise maintenance-triggered automatic
rewrite, recover legacy and checksummed truncated tails, upgrade legacy logs,
detect valid-RESP byte corruption, persist stream consumers/PEL, and reject corrupt
logs at bind time.
