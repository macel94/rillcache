# Resource and operational safety

Open items are tracked in the canonical [production-readiness gaps](../production-readiness-gaps.md); this file records current behavior and boundaries, not a second checklist.

Part of the [production-readiness plan](../production-readiness.md). This file owns
the capability row so independent feature increments edit only their own area.

| Capability | Current boundary / evidence required before closing |
| --- | --- |
| Resource/operational safety | Partial: bounded payload/client/request limits, a bounded client registry with CLIENT LIST/KILL (peer/local addresses, ID, type, user and age filters), INFO clients and a runtime `CONFIG SET maxclients` limit enforced by both accept paths, joined shutdown, persistence-aware final sync and honest `INFO stats` counters for connections/commands/rejections/expiration/pub-sub, a bounded best-effort `MONITOR` stream with credential redaction (S93), bounded `INFO errorstats` reply-code counts, opt-in `INFO commandstats` calls/time/failures (`CONFIG SET commandstats-enabled yes`) and `CONFIG RESETSTAT` clearing. Payload-budget eviction policies cover noeviction/allkeys/volatile random/LRU/volatile-TTL with honest mutation-LRU and stats, `CONFIG GET/SET maxmemory` tunes the bounded payload byte budget at runtime (S96), and `MEMORY STATS`/`DOCTOR` expose payload high-water and bounded pressure reporting (S98). Open work is tracked in the canonical gaps file (O1–O9, R1–R6). |
