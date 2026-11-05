# Append-only persistence and recovery

> **Current scope:** durability completeness is explicitly deferred. The journal
> remains implemented and tested as best-effort, but there is no RDB, checksum,
> background/incremental rewrite, crash/ENOSPC/soak validation, backup drill,
> point-in-time recovery or signal-driven drain. Do not treat it as a durable
> system of record.

Rillcache can persist supported keyspace mutations to an append-only command log.
This is a real recovery path with explicit failure semantics, but it is **not** a
complete Redis durability implementation: there is no RDB format, no
background/forked AOF rewrite, no multi-process log ownership, no redis.conf
integration, and stream consumer-group pending state is not restored.

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
is unsupported and can corrupt or interleave histories; use one owner per file.
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

Expiration is persisted as an absolute deadline, so replay does not depend on when
the log is loaded. Logically expired keys are emitted as `DEL`; background reaping
does not need its own record because the deadline reconstructs the same behavior.
A clock rollback cannot resurrect a key beyond its stored deadline in the current
process; replay uses the same local clock rules.

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
delayed shutdown state exists. The safe-std server still has no portable SIGTERM
handler; `always` is the only policy that does not depend on a clean process exit
for recent acknowledged writes unless a client sends `SHUTDOWN`.

If an append or fsync fails, the command reply is replaced with
`ERR append-only write failed: ...`, the journal enters a failed state and later
writes fail. The in-memory mutation may already be applied; a restart replays only
what reached the file. Disk-full, quota and permission errors are visible rather
than silently converted into success.

## Single-process ownership

A journal is owned by exactly one process. Startup claims `<journal>.lock`
(`appendonly.aof.lock`) with an exclusive create containing the owner's PID and
kernel start time; a second server pointed at the same `--dir` fails with
`append-only journal ... is already owned by process N` instead of interleaving
appends. A lock left by a crash is reclaimed only when the recorded process is
gone or its PID has been reused (the start time no longer matches), so a live owner
is never displaced. Clean shutdown removes the lock; the lock is absent when
`--appendonly no`.

This is an **advisory local-filesystem lock**. It does not coordinate writers on a
shared/network filesystem, and such a volume is not a supported deployment; keep a
journal on local storage or guarantee single-process access externally.

## Compaction with BGREWRITEAOF

`BGREWRITEAOF` replaces the log with a compact snapshot of the current keyspace:
`FLUSHALL` followed by every live key's rebuild records. Since the cooperative
rewrite change it is **not** a single blocking pass:

1. The command materialises the key list and creates a mode-0600 temporary file in
   the journal directory, then replies
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

`INFO persistence` reports `aof_rewrite_in_progress` while a rewrite is running and
`aof_last_bgrewrite_status:ok|err` for the last completed attempt. A crash before
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
- malformed RESP or a command that returns an error fails startup with the byte
  offset; startup never silently skips a corrupt middle section;
- allocation/payload limits apply during replay, so an OOM while loading is a
  startup failure rather than partial data;
- a replayed command may be rejected if it was produced by a newer/unknown command
  set; that is visible corruption for this binary.

Replay is sequential and single-threaded. After replay, the file is positioned for
new appends. Shutdown/reopen is the only supported way to pick up external edits;
do not edit a live file.

## Streams limitation

Stream entries, last IDs and consumer-group IDs are rebuilt. Consumer objects and
pending-entry-list metadata (delivery counts, owners, timestamps) are **not**
reconstructed because the journal records entries and group cursors only.
`XACK`/`XCLAIM`/`XAUTOCLAIM` operate on that process-memory pending state; treat
it as volatile across restart and do not use this subset as a durable work queue.

## Deliberate gaps

- No RDB import/export, manifest/multi-part AOF, checksums, encryption at rest or
  replication of the log. `BGREWRITEAOF` is cooperative and incremental, but it is
  not Redis's fork-based rewrite: the key list is materialised in memory and the
  rewrite runs on the owner thread between maintenance ticks.
- No graceful signal-driven drain; see [container delivery](containers.md).
- No shared-filesystem coordination: ownership is a local advisory lock
  (`<journal>.lock`), not a distributed lease. Crash-consistency of append and
  rewrite is covered by `tests/durability_crash.rs`.
- No backup/restore tooling, point-in-time recovery, retention policy or log
  rotation. Operators must size/rotate files and test restores.
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
rewrite, recover a truncated tail, persist `FLUSHALL`, and reject corrupt logs at
bind time.
