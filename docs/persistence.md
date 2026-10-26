# Append-only persistence and recovery

Rillcache can persist supported keyspace mutations to an append-only command log.
This is a real recovery path with explicit failure semantics, but it is **not** a
complete Redis durability implementation: there is no RDB format, no AOF rewrite
or compaction, no multi-process log ownership, no redis.conf integration, and
stream consumer-group pending state is not restored.

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

On orderly shutdown, joined workers drop the store and the journal's `Drop`
performs a final sync/flush. The safe-std server still has no portable SIGTERM
handler; `always` is the only policy that does not depend on a clean process exit
for recent acknowledged writes.

If an append or fsync fails, the command reply is replaced with
`ERR append-only write failed: ...`, the journal enters a failed state and later
writes fail. The in-memory mutation may already be applied; a restart replays only
what reached the file. Disk-full, quota and permission errors are visible rather
than silently converted into success.

## Compaction with BGREWRITEAOF

`BGREWRITEAOF` replaces the log with a compact snapshot of the current keyspace:
`FLUSHALL` followed by every live key's rebuild records. The implementation is
**synchronous** despite the Redis-compatible name: it holds the exclusive
transaction gate while encoding the snapshot, writing a mode-0600 temporary file
in the journal directory, fsyncing it, atomically renaming it over the journal and
reopening it. Command service pauses for the duration. There is no fork, copy-on-
write page sharing, incremental rewrite, automatic size trigger or background
progress reporting.

A successful `+OK` means the old log has been atomically replaced with a complete
snapshot that includes every mutation committed before the rewrite. A crash during
the temporary write leaves the previous log intact; a failure returns an error and
the journal retains its prior contents/position unless the rename already
completed. Concurrent appends cannot race the replacement because the same gate
serializes commands. Rewriting a disabled journal returns a visible error. Operators
must schedule rewrites during acceptable pauses and size the file between runs;
there is no automatic rewrite policy.

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
reconstructed because the currently supported command subset has no
`XGROUP CREATECONSUMER`/`XCLAIM` equivalent. Treat pending delivery state as
volatile across restart; do not use this subset as a durable work queue.

## Deliberate gaps

- No RDB import/export, background/incremental AOF rewrite, automatic rewrite
  thresholds, manifest/multi-part AOF, checksums, encryption at rest or replication
  of the log. `BGREWRITEAOF` is a blocking full-state compaction, not Redis's
  fork-based background rewrite.
- No graceful signal-driven drain; see [container delivery](containers.md).
- No multi-process file locking, shared-filesystem guarantees or crash-consistency
  testing across kernel/filesystem failure modes.
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
rewrite compaction. Wire tests restart real servers in all worker modes, verify data
and TTLs survive, compact a live log with `BGREWRITEAOF`, recover a truncated tail,
persist `FLUSHALL`, and reject corrupt logs at bind time.
