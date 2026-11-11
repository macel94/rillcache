---
type: storage
title: Journal persistence and recovery
description: The private RESP append-only journal, CRC32 and optional HMAC records, fsync acknowledgement windows, replay and truncation rules, cooperative rewrite, offline backup and remaining durability gaps.
tags: [persistence, journal, aof, recovery, fsync, rewrite, checksum, hmac]
sources:
  - id: openwiki-source-0117ac68a72ed7ce495ae997
    resource: repo://docs/persistence.md
  - id: openwiki-source-c23acd0ff2503357d054416d
    resource: repo://docs/production-readiness-gaps.md
  - id: openwiki-source-06147ad000681154d8484755
    resource: repo://src/bin/rillcache-backup.rs
  - id: openwiki-source-48c84a451bc541a1bb256a33
    resource: repo://src/engine/discovery/mod.rs
  - id: openwiki-source-350cb0ce838ab0c30fe785a9
    resource: repo://src/engine/journal.rs
generated: { by: "opencode", at: "2026-11-11T08:27:05.780Z" }
verified:
  - by: openwiki/0.9.0
    at: 2026-11-11T08:27:05.780Z
---

# Journal persistence and recovery

Rillcache can persist supported mutations to an append-only RESP command log. The
log is a real recovery path with explicit failure semantics, but it is not a
complete Redis durability implementation and must not be treated as a system of
record.

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
| `--appendfilename` | Basename without path separators | `appendonly.aof` |
| `--appendfsync` | `always`, `everysec`, `no` | `everysec` |
| `--aof-hmac-key-file` | Path to a 16–1024-byte key | Off |
| `--aof-hmac-migrate` | One-time seal of an existing journal | Off |

The file is created with mode 0600 on Unix. The process must be able to read,
write and truncate it at startup. Startup claims `<journal>.lock` with the owner
PID and kernel start time, so a second process pointed at the same journal fails
instead of interleaving appends. A crash-stale lock is reclaimed only when its
owner is gone or its PID has been reused; clean shutdown removes it. The lock is
advisory and local-filesystem only, not a shared-volume lease.

`CONFIG GET` reports the startup mode, basename and live policy. Only
`appendfsync` is mutable at runtime:

```text
CONFIG SET appendfsync always
CONFIG SET appendfsync everysec
CONFIG SET appendfsync no
```

`appendonly` and `appendfilename` are startup-only. There is no configuration
reload and no runtime open/close toggle.

## Format

The journal is a sequence of RESP2 command arrays. It is not Redis multipart AOF.
A mutation is translated into the final state of each changed key:

```text
DEL key
SET key value
RPUSH/SADD/HSET/ZADD ...       rebuilt collection contents
XADD / XGROUP CREATE / XSETID stream entries and group cursors
XGROUP CREATECONSUMER ...      restored consumer objects
XCLAIM ... JUSTID FORCE ...    restored PEL owner/count/time
HPEXPIREAT ...                 per-field deadlines
PEXPIREAT key deadline         surviving key TTL
FLUSHALL                       full snapshot or flush boundary
```

The `DEL` makes a type change or deletion unambiguous. Collection members are
emitted in deterministic order and chunked within a bounded record budget. A
stream that has advanced beyond its last retained entry uses a sentinel
`XADD`/`XDEL` pair so the last-generated ID survives replay. Group cursors and
`XSETID` metadata are restored. A pending id whose entry was deleted or trimmed is
kept as a PEL tombstone through placeholder `XADD`/`XCLAIM`/`XDEL` records.

Each new transaction is followed by a CRC32 `ECHO` marker containing its byte
length and digest. A format header identifies the checksummed segment; legacy
RESP-only journals remain readable and switch to a checksummed segment on their
first new append. With a key configured, an anchor and per-record HMAC-SHA256
`ECHO` chain cover transaction content and order. These markers are inert when a
journal is inspected or replayed by another Redis-compatible tool.

Because records are ordinary RESP commands, replay uses the normal command
implementation. A newer/unknown command in the file is visible corruption for this
binary rather than silently skipped data.

## Mutation capture

With journaling or notifications enabled, the engine records a bounded ordered
event log of key mutations (65,536 entries). After a command, it asks `Data` for
the keys changed since the command's starting sequence and encodes their current
state while still in the owner turn.

The record and its checksum or authentication marker are appended before the
command reply is returned to the session. If the append fails, the already-written
reply is truncated and replaced with:

```text
-ERR append-only write failed: ...
```

The journal then latches a failed state and later appends fail. The in-memory
mutation may already be applied; restart replays only what reached the file and
passes verification.

A non-mutating command, failed condition, overflow or rejected argument appends
nothing. `MULTI`/`EXEC` captures the final state after the batch rather than each
intermediate command. If the bounded event log overflows, the next commit writes
`FLUSHALL` followed by a complete snapshot of every current key.

Absolute expiry is persisted as `PEXPIREAT`; replay does not depend on when the
file is loaded. A logically expired key is emitted as a deletion, so background
reaping needs no separate record.

## Fsync windows

| Policy | Acknowledgement meaning |
| --- | --- |
| `always` | Append and `sync_data` complete before the reply. A success means the records reached the filesystem. |
| `everysec` | Records are written immediately; maintenance fsyncs about once per second. A crash can lose roughly the last second and the tail of the last command. |
| `no` | Records are written to the OS without explicit fsync. Only orderly shutdown or the OS provides durability. |

Leaving `no` for a stronger policy first syncs records previously handed to the
OS. A sync failure latches the journal as failed. `WAITAOF numlocal numreplicas`
supports at most one local acknowledgement, synchronously flushes when enabled,
and always reports zero replicas. With AOF disabled, `numlocal=1` is an explicit
error.

Orderly shutdown or `SHUTDOWN` forces a journal flush before workers stop.
`SAVE`/`NOSAVE` do not write RDB files. The standalone binary has no portable
SIGTERM handler, so a container SIGTERM does not exercise this flush path.

## Replay and recovery

Replay happens before the listener accepts clients, into a fresh authenticated
store:

1. read the file in bounded chunks;
2. verify every complete checksum group and, when configured, the HMAC chain;
3. execute each complete command through the ordinary command implementation;
4. truncate a torn final transaction/checksum group to the previous verified
   boundary, or discard a crash-torn final unauthenticated group;
5. fail startup with a byte offset on malformed RESP, a complete checksum mismatch,
   a bad HMAC marker or any command that returns an error;
6. apply payload limits during replay, so an over-budget load fails rather than
   producing partial data.

The server never silently skips a corrupt middle section. Replay is sequential;
after it completes, the file is positioned at the end for new appends. Do not edit
a live journal.

An observed expired key cannot be resurrected by a later clock rollback within
the same process because the entry is already removed. Across restart, absolute
deadlines still depend on the host clock; there is no clock high-water mark or
monotonic recovery policy.

## Cooperative rewrite

`BGREWRITEAOF` replaces the log with a compact full-state snapshot:

```text
FLUSHALL
every live key's DEL + rebuild records
```

Despite the Redis-compatible name, there is no fork or copy-on-write. The command
materialises the key list, creates a mode-0600 temporary file with a checksum
header, and replies `+Background append only file rewriting started`. The
maintenance loop then encodes at most 512 keys or 1 MiB per tick between client
commands. Keys changed since the previous round are re-encoded as whole values, so
a key visited twice cannot duplicate collection members. The snapshot is fsynced
outside the exclusive gate; a short commit appends the residual, fsyncs, renames
the temporary file and reopens the replacement at the snapshot end. A crash before
the rename leaves the previous journal intact.

The start pause is bounded by key-name materialisation, not by key values, but it
still scales with the number of keys. Completion time also scales with the
keyspace; the slice size bounds each round, not the total. Automatic rewrite checks
the file about every 100 ms and starts the same cooperative path once both
`auto-aof-rewrite-min-size` and the growth percentage are exceeded. The defaults
are 64 MiB and 100%; percentage zero disables it.

## Offline backup and restore

The separate `rillcache-backup` binary creates, verifies and restores a versioned,
bounded-memory archive of a quiescent journal:

```text
rillcache-backup create --dir /data --output /backups/rillcache.rcb
rillcache-backup verify --input /backups/rillcache.rcb
rillcache-backup restore --input /backups/rillcache.rcb --dir /restore
```

`create` opens the journal through the normal ownership lock, replays it, forces a
sync and copies the AOF plus its length and CRC32 in 1 MiB chunks, with a 16 GiB
archive limit. The temporary archive is mode 0600, synced and hard-linked into
place without replacing an existing output. A running writer therefore makes
backup fail without publishing an archive. `restore` verifies the outer bytes,
replays the inner journal in a temporary directory, refuses to overwrite an
existing journal or lock, and publishes only after validation. Pass
`--aof-hmac-key-file` for an authenticated journal. The archive contains only the
AOF, not ACL files or server configuration.

This is offline, quiescent copy, not online backup, point-in-time recovery,
retention or rotation. CRC32 detects accidental corruption but does not
authenticate an archive against malicious modification.

## Observability

`INFO persistence` reports:

```text
loading:0
aof_enabled:1
aof_rewrite_in_progress:0|1
aof_last_bgrewrite_status:ok|err
aof_last_write_status:ok|err
aof_current_size:...
```

`aof_rewrite_in_progress` reflects the cooperative maintenance rewrite. When
persistence is disabled only `loading:0` and `aof_enabled:0` are returned. RDB,
backlog, fork and bio fields are omitted because they are not measured.

## What is not durable

- **No RDB.** `SAVE`/`BGSAVE` return unsupported; `LASTSAVE` reports no completed
  RDB save.
- **No encryption at rest.** CRC32 detects accidental corruption; HMAC-SHA256
  authenticates content and order when a key is configured, but is not encryption.
- **No anti-rollback or key management.** A whole-journal replacement with an
  older valid file, durable-suffix truncation, key rotation and keyrings are not
  detected or provided without external state.
- **No fork-based rewrite.** The cooperative rewrite still has a start pause that
  scales with key count and no background fork.
- **No shared-filesystem coordination.** Ownership is a local advisory lock.
- **No crash/ENOSPC/power-loss corpus.** Short writes, quota, permissions, fsync
  failure and kill-at-each-boundary behaviour are not fully proven.
- **No online backup, point-in-time recovery, retention policy or rotation.**
  Offline quiescent backup, verify and restore are implemented.
- **No signal-driven drain.**
- **Stream consumer activity time is not restored.** Consumer objects and PEL
  owners, delivery counts and timestamps are restored, including deleted-id
  tombstones; `seen_at`/`inactive` reflect replay time.

The journal therefore narrows the loss window but does not close the production
durability gate. The full checklist is in
[compatibility and production gaps](../boundaries/compatibility-and-production-gaps.md).
The mutation and expiry model is in
[data model and expiry](data-model-and-expiry.md).
