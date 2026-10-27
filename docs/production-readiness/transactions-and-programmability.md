# Transactions and programmability

Part of the [production-readiness plan](../production-readiness.md). This file owns
the capability row so independent feature increments edit only their own area.

| Capability | Current boundary / evidence required before closing |
| --- | --- |
| Transactions and programmability | Partial implementation (S26): bounded `MULTI`/`EXEC`/`DISCARD`/`WATCH`/`UNWATCH` over supported data commands, store-gate batch isolation, expiry-aware optimistic invalidation and RESP2/RESP3 replies, with `PUBLISH`/`SPUBLISH` queued and delivered at `EXEC` (S99). Still required: scripts/functions with failure/resource semantics, connection-local commands at EXEC time, and documented absence of rollback, replication and durable commit. A sequence of individually atomic commands is not a transaction. |
