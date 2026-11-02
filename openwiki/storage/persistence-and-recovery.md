---
type: storage
title: Journal persistence and recovery
description: The private RESP append-only journal, its deterministic final-state records, fsync acknowledgement windows, replay and truncation rules, synchronous rewrite and remaining durability gaps.
tags: [persistence, journal, aof, recovery, fsync, rewrite]
sources:
  - id: openwiki-source-0117ac68a72ed7ce495ae997
    resource: repo://docs/persistence.md
  - id: openwiki-source-85b3cd0f317695a3155768a3
    resource: repo://docs/production-readiness.md
  - id: openwiki-source-c4ffe2571f4b3f3ab07b4a87
    resource: repo://docs/production-readiness/durability.md
  - id: openwiki-source-350cb0ce838ab0c30fe785a9
    resource: repo://src/engine/journal.rs
generated: { by: "opencode", at: "2026-11-02T15:32:39.597Z" }
verified:
  - by: openwiki/0.7.3
    at: 2026-11-02T16:04:12.575Z
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

The file is created with mode 0600 on Unix. The process must be able to read,
write and truncate it at startup. Two processes pointed at one file are
unsupported and can corrupt or interleave histories. The scratch container needs
a writable volume mounted at `/data`.

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

The journal is a sequence of RESP2 command arrays. It is not Redis multipart AOF
and has no binary header. A mutation is translated into the final state of each
changed key:

```text
DEL key
SET key value
RPUSH/SADD/HSET/ZADD ...       rebuilt collection contents
XADD / XGROUP CREATE / XSETID stream entries and group cursors
HPEXPIREAT ...                 per-field deadlines
PEXPIREAT key deadline         surviving key TTL
FLUSHALL                       full snapshot or flush boundary
```

The `DEL` makes a type change or deletion unambiguous. Collection members are
emitted in deterministic order and chunked within a bounded record budget. A
stream that has advanced beyond its last retained entry uses a sentinel
`XADD`/`XDEL` pair so the last-generated ID survives replay. Group cursors and
`XSETID` metadata are restored.

Because records are ordinary RESP commands, replay uses the normal command
implementation. It also means a newer/unknown command in the file is visible
corruption for this binary rather than silently skipped data.

## Mutation capture

With journaling or notifications enabled, the engine records a bounded ordered
event log of key mutations (65,536 entries). After a command, it asks `Data` for
the keys changed since the command's starting sequence and encodes their current
state while still in the owner turn.

The record is appended before the command reply is returned to the session. If
the append fails, the already-written reply is truncated and replaced with:

```text
-ERR append-only write failed: ...
```

The journal then latches a failed state and later appends fail. The in-memory
mutation may already be applied; restart replays only what reached the file.

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
2. execute each complete command through the ordinary command implementation;
3. stop at an incomplete final command and truncate the file back to the last
   complete record;
4. fail startup with a byte offset on malformed RESP or any command that returns
   an error;
5. apply payload limits during replay, so an over-budget load fails rather than
   producing partial data.

The server never silently skips a corrupt middle section. Replay is sequential;
after it completes, the file is positioned at the end for new appends. Do not edit
a live journal.

An observed expired key cannot be resurrected by a later clock rollback within
the same process because the entry is already removed. Across restart, absolute
deadlines still depend on the host clock; there is no clock high-water mark or
monotonic recovery policy.

## Synchronous rewrite

`BGREWRITEAOF` replaces the log with a compact full-state snapshot:

```text
FLUSHALL
every live key's DEL + rebuild records
```

Despite the Redis-compatible name, the implementation is synchronous. It holds
the owner turn/exclusive path, writes a mode-0600 temporary file in the same
directory, fsyncs it, renames it over the journal, fsyncs the directory where
supported, and reopens the replacement positioned at the snapshot end. Command
service pauses for the duration. There is no fork, copy-on-write, incremental
rewrite or background progress.

A crash during the temporary write leaves the previous log intact. A failure
returns an error and retains the prior file unless the rename already completed.
Automatic rewrite checks the file about every 100 ms and triggers the same
synchronous path once both `auto-aof-rewrite-min-size` and the growth percentage
are exceeded. The defaults are 64 MiB and 100%; percentage zero disables it.

## Observability

`INFO persistence` reports:

```text
loading:0
aof_enabled:1
aof_rewrite_in_progress:0
aof_last_bgrewrite_status:ok|err
aof_last_write_status:ok|err
aof_current_size:...
```

`aof_rewrite_in_progress` is always zero because there is no background rewrite.
When persistence is disabled only `loading:0` and `aof_enabled:0` are returned.
RDB, backlog, fork and bio fields are omitted because they are not measured.

## What is not durable

- **No RDB.** `SAVE`/`BGSAVE` return unsupported; `LASTSAVE` reports no completed
  RDB save.
- **No checksums or authenticated records.** Corruption is detected by RESP
  parsing and command failure, not a per-record digest.
- **No encryption at rest.** Disk encryption is an operator concern.
- **No background rewrite.** Rewrite blocks command service.
- **No multi-process ownership.** Two writers can corrupt the file.
- **No crash/ENOSPC/power-loss corpus.** Short writes, quota, permissions, fsync
  failure and kill-at-each-boundary behaviour are not proven.
- **No backup, restore drill, retention policy or point-in-time recovery.**
- **No signal-driven drain.**
- **Stream pending/consumer state is volatile.** Entries, last IDs and group
  cursors are rebuilt; PEL owners, delivery counts, timestamps and consumer
  activity are not.

The journal therefore narrows the loss window but does not close the production
durability gate. The full checklist is in
[compatibility and production gaps](../boundaries/compatibility-and-production-gaps.md).
The mutation and expiry model is in
[data model and expiry](data-model-and-expiry.md).
