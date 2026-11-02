# Transactions and programmability

Open items are tracked in the canonical [production-readiness gaps](../production-readiness-gaps.md); this file records current behavior and boundaries, not a second checklist.

Part of the [production-readiness plan](../production-readiness.md). This file owns
the capability row so independent feature increments edit only their own area.

| Capability | Current boundary / evidence required before closing |
| --- | --- |
| Transactions and programmability | Partial implementation (S26): bounded `MULTI`/`EXEC`/`DISCARD`/`WATCH`/`UNWATCH` over supported data commands, store-gate batch isolation, expiry-aware optimistic invalidation and RESP2/RESP3 replies, with `PUBLISH`/`SPUBLISH` queued and delivered at `EXEC` (S99). Open work (scripts/functions, queue-time divergences and explicit rollback/replication/durable-commit limits) is tracked in the canonical gaps file (C2, C14). A sequence of individually atomic commands is not a transaction. |
