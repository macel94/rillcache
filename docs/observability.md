# Statistics and INFO observability

Rillcache exposes a deliberately small set of server counters through
`INFO stats` (also included by `INFO` and `INFO all`). Values are process-local
and reset on restart. There is no Prometheus endpoint, latency histogram,
per-command table or replication statistics. `SLOWLOG` and `LATENCY` provide
bounded, process-local timing as described below.

## Fields

| Field | Meaning |
| --- | --- |
| `total_connections_received` | TCP sockets accepted by the listener, including a socket rejected because `max_clients` was already full |
| `total_commands_processed` | Client command dispatches admitted to `Session::execute`; includes session/pub-sub/admin commands and `INFO` itself |
| `rejected_connections` | Accepted sockets closed with `ERR max number of clients reached` |
| `expired_keys` | Live keys removed by lazy expiration or the bounded maintenance reaper |
| `expired_subkeys` | Hash fields removed by lazy per-field TTL expiry (Redis 7.4 `expired_subkeys`) |
| `evicted_keys` | Keys removed by an enabled payload-budget eviction policy |
| `pubsub_channels` | Distinct channels with at least one live subscription |
| `pubsub_patterns` | Total pattern subscriptions (not distinct patterns) |

`INFO keyspace` reports `keys`, `expires` and `subexpiry` (the number of live
hash keys with at least one field deadline). `expired_subkeys` resets with
`CONFIG RESETSTAT`; `subexpiry` is live state and is not a counter.

`INFO clients` reports `connected_clients` (the process-local registry size),
`cluster_connections:0` and `maxclients` (the configured connection limit). `CLIENT LIST` exposes per-connection address/name/user/
RESP/subscription metadata.

`INFO memory` also reports the live `maxmemory_policy` and `notify_keyspace_events`. `evicted_keys` resets with
`CONFIG RESETSTAT`; the policy itself does not.

`INFO server`, `memory` and `keyspace` remain available. Memory fields are the
logical key/value payload and expiry counts, not allocator or RSS measurements;
`used_memory` is not fabricated. Counters deliberately include no keyspace hit/
miss, net-byte, CPU, latency, fork or replication fields because those values are
not measured. Missing fields are honest gaps, not zero claims.

## SLOWLOG

`SLOWLOG GET [count]`, `SLOWLOG LEN`, `SLOWLOG RESET` and `SLOWLOG HELP` expose
the most recent commands whose measured duration reached
`slowlog-log-slower-than` microseconds. `GET` returns the newest entries first,
defaults to 10, treats `-1` as all entries and `0` as an empty array. Each entry
is `[id, unix_seconds, duration_us, argv, client_address, client_name]`.

`CONFIG GET slowlog-log-slower-than slowlog-max-len` reports the active values and
`CONFIG SET` updates them at runtime. The default threshold is 10000 microseconds
and the default maximum length is 128 entries. A negative threshold disables
recording; a zero maximum length clears and disables recording. The maximum length
is bounded at 1,000,000 entries, and lowering it trims the log immediately.

Retention is deliberately bounded independently of request size: at most 32
arguments are kept, with the last slot replaced by
`... (N more arguments)`, and each argument is cut at 128 bytes with a
`... (N more bytes)` suffix. `AUTH`, `HELLO AUTH`, sensitive `CONFIG SET` values
and `ACL SETUSER`/`DELUSER`/`GETUSER` arguments are stored as `(redacted)`.

Durations are measured around a client command, so a blocking command records
the total wait until it succeeds or times out. Commands queued inside `MULTI` are
not logged separately; `EXEC` is logged as one entry whose duration covers the
batch. This differs from Redis's per-queued-command entries and is intentional
for this session-level implementation. The log is in-memory, process-local and
not included in `INFO` or persisted across restart.

## LATENCY

`LATENCY LATEST` returns `[event, latest_timestamp_seconds, latest_ms, max_ms]`
for each event series, sorted by event name. `LATENCY HISTORY event` returns the
oldest-to-newest `[timestamp_seconds, latency_ms]` samples, up to 160 per event.
`LATENCY RESET [event ...]` clears one or more series (or all without names) and
returns the number removed. `LATENCY DOCTOR` returns a plain-text report derived
only from measured series, and `LATENCY HELP` lists the supported subcommands.
Unknown history events return an empty array and resetting unknown events returns
zero.

Samples are recorded only when `latency-monitor-threshold` milliseconds is
non-zero and a measured event reaches that threshold. The default is `0`
(disabled); `CONFIG GET/SET latency-monitor-threshold` exposes and updates it at
runtime and rejects negative or non-integer values. The server measures exactly
two event classes:

- `command`: the duration of a client command attempt, excluding a blocking
  wait; blocked commands record their immediate attempt only.
- `expire-cycle`: bounded expiry reclamation in the maintenance loop.

Fork, AOF write/fsync, eviction-cycle, active-defrag and replication latency are
not measured and are never fabricated. `LATENCY GRAPH` and `LATENCY HISTOGRAM`
return explicit unsupported errors rather than partial data. Data is
process-local, in-memory and reset on restart or benchmark/manual reset.

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

## INFO persistence

`INFO persistence` (and `INFO all`) reports only measured append-only journal
state:

```text
# Persistence
loading:0
aof_enabled:1
aof_rewrite_in_progress:0
aof_last_bgrewrite_status:ok
aof_last_write_status:ok
aof_current_size:1234
```

With persistence disabled only `loading:0` and `aof_enabled:0` are returned.
`aof_last_write_status` reflects the journal's latched failure state and
`aof_last_bgrewrite_status` the last synchronous `BGREWRITEAOF`, so
`aof_rewrite_in_progress` is always `0`. RDB, background-rewrite progress,
backlog, bio and fork fields are omitted because they are not measured.

## Deliberate gaps

- No per-policy eviction latency or memory-pressure counters beyond `evicted_keys`.
- No per-client fd/idle/command/buffer/memory/network fields, blocked-client count
  or cross-process client view. `INFO clients` intentionally omits those fields.
- No `INFO commandstats` or `latencystats`, latency histograms, CPU/RSS
  accounting, network-byte counters or per-database stats. `LATENCY` exposes
  only the two measured event series listed above.
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
