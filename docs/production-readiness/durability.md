# Durability

Part of the [production-readiness plan](../production-readiness.md). This file owns
the capability row so independent feature increments edit only their own area.

| Capability | Current boundary / evidence required before closing |
| --- | --- |
| Durability | Partial implementation (S29): opt-in RESP-command append-only journal, mutation-only final-state records with absolute TTLs, `always`/`everysec`/`no` fsync policies, startup replay, partial-tail truncation, visible mid-file corruption failure, joined-shutdown sync, protocol `SHUTDOWN` with forced journal flush, configurable maintenance-triggered automatic AOF compaction, all-mode restart tests across strings/collections/streams/groups, truthful `INFO persistence` AOF enabled/size/write/rewrite status, and explicit `SAVE`/`BGSAVE` RDB-unsupported errors with `LASTSAVE` reporting no completed RDB save (S92), read-only `CONFIG GET` visibility of the startup append-only mode and journal basename, and runtime `CONFIG SET appendfsync`, and bounded `WAITAOF` local-journal acknowledgement with replica counts truthfully reported as zero (S94), without RDB or asynchronous-rewrite claims. Still required: RDB import/export, background/incremental AOF rewrite, checksums/encryption, multi-process ownership, stream pending/consumer durability, disk-full/ENOSPC injection, backup/restore drills, crash/soak testing, point-in-time recovery and signal-driven drain. The journal is not byte-compatible multipart AOF or an RDB claim. |
