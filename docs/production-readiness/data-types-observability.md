# Data types and command surface: Observability, operations and remaining surfaces

Part of the [production-readiness plan](../production-readiness.md). This file owns
the `Observability, operations and remaining surfaces` clauses of the `Data types and command surface` capability row. The
original row began with the lead-in `Implemented S37/S39/S40/S42/S44/S45/S46/S47/S48/S49/S50/S51/S52/S53/S54/S55/S56/S57/S58/S59/S60/S61/S62/S63/S64/S65/S66/S67/S68/S69/S70/S71/S72/S73/S74/S75/S-HC: `; it applies to every clause below.

| Capability | Current boundary / evidence required before closing |
| --- | --- |
| Data types and command surface: observability, operations and remaining surfaces | bounded cursor iteration; keyspace/keyevent notifications including expired/evicted classes (S40/S49); normal/pattern/sharded pub/sub (S31/S47); `TIME`/`OBJECT` runtime clock/encoding/refcount introspection (S61); COMMAND GETKEYS declared-key introspection (S66); bounded SLOWLOG command timing (S70); `CLIENT PAUSE`/`UNPAUSE` client-command gating (S72); bounded `LATENCY` command/expire-cycle event series (S73); explicit cluster-disabled/module/replica boundaries (S74); protocol `SHUTDOWN` with forced journal flush and joined stop (S75); Connection RESET (S33) and process statistics (S34) are implemented separately. Still required: LFU/read-idle statistics, durable notification delivery, cluster hash-slot routing, module execution and broader Redis-8 extension behavior. |
