# Statistics and INFO observability

Rillcache exposes a deliberately small set of server counters through
`INFO stats` (also included by `INFO` and `INFO all`). Values are process-local
and reset on restart. There is no Prometheus endpoint, latency histogram, slow log,
per-command table or replication statistics.

## Fields

| Field | Meaning |
| --- | --- |
| `total_connections_received` | TCP sockets accepted by the listener, including a socket rejected because `max_clients` was already full |
| `total_commands_processed` | Client command dispatches admitted to `Session::execute`; includes session/pub-sub/admin commands and `INFO` itself |
| `rejected_connections` | Accepted sockets closed with `ERR max number of clients reached` |
| `expired_keys` | Live keys removed by lazy expiration or the bounded maintenance reaper |
| `evicted_keys` | Keys removed by an enabled payload-budget eviction policy |
| `pubsub_channels` | Distinct channels with at least one live subscription |
| `pubsub_patterns` | Total pattern subscriptions (not distinct patterns) |

`INFO clients` reports `connected_clients` (the process-local registry size) and
`cluster_connections:0`. `CLIENT LIST` exposes per-connection address/name/user/
RESP/subscription metadata.

`INFO memory` also reports the live `maxmemory_policy` and `notify_keyspace_events`. `evicted_keys` resets with
`CONFIG RESETSTAT`; the policy itself does not.

`INFO server`, `memory` and `keyspace` remain available. Memory fields are the
logical key/value payload and expiry counts, not allocator or RSS measurements;
`used_memory` is not fabricated. Counters deliberately include no keyspace hit/
miss, net-byte, CPU, latency, fork or replication fields because those values are
not measured. Missing fields are honest gaps, not zero claims.

## Scope and reset

`total_commands_processed` covers connection-level dispatch, so embedding
`Store::execute` directly without a `Session` does not increment it. A `MULTI`
counts `MULTI`, each queued command and `EXEC` as separate dispatches. A rejected
command still counts as processed; it does not increment data-specific counters.
Health-check PING probes are ordinary connections and commands.

`CONFIG RESETSTAT` zeroes the connection, command, rejected, expired, evicted and
pub/sub counters. It does not delete data, clear subscriptions, reset the keyspace or reset
payload accounting. Because the reset command itself is counted before dispatch,
the next `INFO` typically reports `total_commands_processed:1` (the `INFO` command).

Counters are relaxed atomics and may be observed slightly asynchronously across
worker threads; they are diagnostic, not a transaction ledger or billing source.
Expiration counts are exact for removals observed by this process but never include
keys that merely crossed their deadline and were not yet reaped/lazily removed.

## Deliberate gaps

- No per-policy eviction latency or memory-pressure counters beyond `evicted_keys`.
- No per-client fd/idle/command/buffer/memory/network fields, blocked-client count
  or cross-process client view. `INFO clients` intentionally omits those fields.
- No `INFO commandstats`, `latencystats`, `slowlog`, full `clients` section,
  `CLIENT LIST`, CPU/RSS accounting, network-byte counters or per-database stats.
- No metrics exporter, OpenTelemetry, structured logs, health beyond PING, alerts,
  SLOs, dashboards or long-running soak evidence.
- No persistence/ACL/auth telemetry. Journal replay and rewrite durations are not
  reported. A future observability increment must preserve the same no-fabrication
  rule instead of filling Redis-shaped fields with guesses.

## Evidence

`tests/observability.rs` runs in all worker modes and verifies connection/command
counts across two clients, reset behavior, maximum-client rejection visibility and
lazy expiration counting. It parses `INFO stats` as a real RESP bulk reply rather
than asserting on an implementation string alone.
