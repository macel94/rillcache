---
type: operations
title: Observability and health
description: Rillcache's INFO sections, process counters, command/error statistics, slowlog, LATENCY, MONITOR, client metadata, MEMORY reporting and PING healthcheck.
tags: [observability, info, metrics, slowlog, latency, healthcheck]
sources:
  - id: openwiki-source-37d619b6bb7c649a39dcca8f
    resource: repo://docs/containers.md
  - id: openwiki-source-82117ce254a2c2e4df240c93
    resource: repo://docs/introspection.md
  - id: openwiki-source-d7644d9029aaf56c189e640b
    resource: repo://docs/observability.md
  - id: openwiki-source-0117ac68a72ed7ce495ae997
    resource: repo://docs/persistence.md
  - id: openwiki-source-ccfe8cc4504f25c304787a46
    resource: repo://src/engine.rs
  - id: openwiki-source-f324750a2c32a18d17cf0054
    resource: repo://src/engine/commands.rs
  - id: openwiki-source-48c84a451bc541a1bb256a33
    resource: repo://src/engine/discovery/mod.rs
  - id: openwiki-source-ec63893b3c530e2d112f1b96
    resource: repo://src/engine/introspection.rs
  - id: openwiki-source-84304c3a8d6d447af08a384f
    resource: repo://src/engine/latency.rs
  - id: openwiki-source-9607bfe862332e4141e98c0b
    resource: repo://src/engine/slowlog.rs
  - id: openwiki-source-a49be758573b96f47b54fa8f
    resource: repo://src/healthcheck.rs
generated: { by: "opencode", at: "2026-11-11T08:27:05.780Z" }
verified:
  - by: openwiki/0.9.0
    at: 2026-11-11T08:27:05.780Z
---

# Observability and health

Rillcache reports what it measures and omits what it does not. Counters are
process-local, reset on restart, and deliberately do not pretend to be Redis's
full telemetry surface. There is no Prometheus exporter, OpenTelemetry pipeline,
structured log schema or SLO dashboard.

## INFO sections

`INFO` with no argument returns `server`, `memory`, `keyspace`, `stats`, `clients`
and `errorstats`. `INFO all` adds `commandstats`, `replication` and `persistence`.
A single section name is accepted; an unsupported name returns
`ERR unsupported INFO section`.

### Server, memory and keyspace

```text
# Server
rillcache_version:...
rillcache_mode:standalone
# Memory
rillcache_payload_bytes:...
rillcache_payload_limit_bytes:...
rillcache_payload_scope:key_and_value_bytes_not_RSS
maxmemory_policy:...
# Keyspace
db0:keys=...,expires=...,avg_ttl=...,subexpiry=...
```

Payload bytes count key and value bytes only. They are not RSS, allocator usage,
map capacity, thread stacks or socket buffers. `avg_ttl` is the exact mean
remaining milliseconds across live keys with a deadline, or zero when none.
`subexpiry` counts live hash keys with at least one field deadline.

### Stats and clients

```text
# Stats
total_connections_received:...
total_commands_processed:...
rejected_connections:...
expired_keys:...
expired_subkeys:...
evicted_keys:...
pubsub_channels:...
pubsub_patterns:...
audit_log_write_failures:...
audit_syslog_write_failures:...

# Clients
connected_clients:...
cluster_connections:0
maxclients:...
auth_failure_limit:...
auth_source_limit:...
blocked_clients:...
```

`total_commands_processed` counts session dispatches, including session, pub/sub
and admin commands and `INFO` itself. It does not count direct embedded
`Store::execute` calls. A rejected command still counts as processed.
`total_connections_received` includes a socket accepted and then rejected at the
client limit; `rejected_connections` counts those rejections.

Command counters are split into per-client shards and summed when read, reducing
cross-worker cache-line contention. They are relaxed atomics, so an `INFO`
snapshot can lag a concurrent dispatch slightly. They are diagnostics, not a
billing ledger.

`connected_clients` is the live client registry size. `blocked_clients` is the
current blocked-session count. `cluster_connections` is always zero because
cluster support is absent.

## Error statistics

Each command reply whose first byte is `-` contributes to `INFO errorstats`. Codes
are the bounded first token (at most 32 bytes) and the table is capped at 64
distinct codes. Examples are `ERR`, `WRONGTYPE`, `NOAUTH` and `NOPERM`.

Protocol framing errors occur before a command reply exists and are not counted.
Error statistics are counters, not an error log; the message text is not retained.
`CONFIG RESETSTAT` clears them with the other counters.

## Command statistics

`INFO commandstats` is opt-in because the default hot path should not pay for a
per-command lookup and atomic updates.

```text
CONFIG SET commandstats-enabled yes
INFO commandstats
```

Each line has the Redis shape:

```text
cmdstat_set:calls=...,usec=...,usec_per_call=...,rejected_calls=...,failed_calls=...
```

- `calls` counts network dispatches through a client `Session`; commands inside
  `MULTI` are not counted separately and `EXEC` counts once.
- `usec` measures dispatch around the command. Blocking wait time is excluded.
- `failed_calls` counts replies whose top-level frame is an error.
- `rejected_calls` is the subset rejected with `NOAUTH` or `NOPERM`.
- Unknown commands are not tracked; subcommands share their top-level name.

Disabling commandstats keeps existing counters readable until reset. Re-enabling
from disabled starts from an empty table. `CONFIG RESETSTAT` clears it.

## SLOWLOG

`SLOWLOG GET [count]`, `LEN`, `RESET` and `HELP` expose recent slow commands.
Defaults are a 10,000 µs threshold and 128 entries. A negative threshold disables
recording; a zero maximum clears and disables. The maximum is bounded at 1,000,000.

Each entry is `[id, unix_seconds, duration_us, argv, client_address,
client_name]`. Arguments are bounded to 32 entries and 128 bytes each, with a
suffix describing omitted bytes or arguments. `AUTH`, `HELLO AUTH`, sensitive
`CONFIG SET` values and `ACL SETUSER`/`DELUSER`/`GETUSER` credentials are stored
as `(redacted)`.

A blocking command's slowlog duration includes the full wait. A queued `MULTI`
command is not logged separately; `EXEC` is one entry covering the batch. The log
is in-memory and process-local, and is not persisted or included in `INFO`.

## MONITOR

`MONITOR` registers the connection for a best-effort stream of commands from
other clients. Lines are simple strings with escaped binary arguments, bounded to
4 KiB. The stream uses the same bounded queue model as pub/sub, so a slow monitor
drops lines instead of blocking command execution.

Credentials are redacted before formatting: `AUTH` shows `AUTH (redacted)`,
`HELLO ... AUTH` hides the password, ACL credential rules and
`CONFIG SET requirepass` values are hidden. `RESET` or disconnect leaves monitor
mode. MONITOR is admin/dangerous and cannot be queued inside `MULTI`.

MONITOR is a debugging aid, not a durable audit trail or a complete command log.

## LATENCY

`LATENCY LATEST`, `HISTORY`, `RESET`, `DOCTOR` and `HELP` expose a bounded
process-local event history. Each series keeps at most 160 samples. Samples are
recorded only when `latency-monitor-threshold` is nonzero and an event reaches the
threshold in milliseconds.

The server measures exactly two classes:

- `command`: a command attempt, excluding blocking wait time;
- `expire-cycle`: bounded expiry reclamation.

Fork, AOF write/fsync, eviction-cycle, active-defrag and replication latency are
not measured. `LATENCY GRAPH` and `HISTOGRAM` return explicit unsupported errors
rather than partial data. There are no percentiles or histograms.

## OBJECT, MEMORY and TIME

`TIME` returns wall-clock Unix seconds and microseconds. It is not monotonic.

`OBJECT ENCODING` reports the actual representation: `raw`, `listpack`,
`quicklist`, `intset`, `hashtable`, `skiplist` or `stream`. All strings report
`raw`; there is no Redis `embstr`/integer-object cache claim. `OBJECT REFCOUNT`
returns 1 because values are uniquely owned. `IDLETIME` and `FREQ` are explicit
unsupported errors rather than fabricated counters.

`MEMORY USAGE` returns a deterministic logical entry-size estimate: fixed header,
key length and payload length. `MEMORY STATS` reports current and peak payload,
live key count, percentage of the configured payload limit and the scope label
`key_and_value_bytes_not_RSS`. `MEMORY DOCTOR` is a bounded text report from the
same snapshot. Allocator, RSS, fragmentation, Lua and replication-buffer fields are
omitted rather than set to zero.

## Persistence status

`INFO persistence` reports `aof_enabled` and, when enabled, the current size,
latched write status, last rewrite status and `aof_rewrite_in_progress`. The
rewrite is cooperative and advances in bounded slices between commands, not a
background fork. RDB, fork, backlog and bio fields are absent because they are not
implemented or measured.

## Healthcheck and readiness

```sh
rillcache --healthcheck 127.0.0.1:6379
```

The healthcheck opens one TCP connection, writes a canonical RESP2 `PING`, reads
exactly `+PONG\r\n` and exits 0. Connection refusal, timeout, truncation, a wrong
reply or trailing missing bytes exit 1 with a stderr message. It is quiet on
success.

The probe shares one one-second deadline across connect, write and read. It is a
request-service check, not a journal, persistence, replication or capacity check.
A server at its client limit can reject the probe; leave headroom for supervisors.

The readiness tests cover pipelines crossing read and command quanta, fragmented
binary input, backpressured multi-megabyte replies, blocked clients, pub/sub
wakeups, file-descriptor reuse and shutdown with idle/incomplete/blocked clients.
These are integration evidence, not a production SLO.

## Deliberate gaps

- No RSS/allocator accounting, network-byte counters or per-client memory fields.
- No per-policy eviction latency, memory-pressure counters or eviction-cycle
  latency series beyond `evicted_keys`.
- No keyspace hit/miss counters, CPU/fork metrics or replication traffic.
- No metrics endpoint, structured logs, tracing, alerting or dashboards.
- No long-running soak evidence or controlled-hardware SLO.

These omissions are intentional. A missing field is not a zero claim. The full
production blocker list is in
[compatibility and production gaps](../boundaries/compatibility-and-production-gaps.md).
