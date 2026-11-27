# Statistics and INFO observability

Rillcache exposes a deliberately small set of server counters through
`INFO stats` (also included by `INFO` and `INFO all`), a bounded per-command
table through `INFO commandstats`, and bounded reply-code counts through
`INFO errorstats`. Values are process-local and reset on restart. There is no
Prometheus endpoint, latency histogram or replication traffic statistics.
`SLOWLOG` and `LATENCY` provide bounded, process-local timing as described below.
`MONITOR` provides a bounded live command stream for debugging.

## Error statistics

`INFO errorstats` (and `INFO all`) returns a `# Errorstats` block with one
`errorstat_<CODE>:count` line per error code seen at command dispatch, such as
`ERR`, `WRONGTYPE`, `NOAUTH` or `NOPERM`. Codes are the bounded first token of the
error reply (at most 32 bytes), the table is capped at 64 distinct codes, and
`CONFIG RESETSTAT` clears it with the other counters. Protocol-level framing
errors are not command replies and are not counted.

## Fields

| Field | Meaning |
| --- | --- |
| `total_connections_received` | TCP sockets accepted by the listener, including a socket rejected because `max_clients` was already full |
| `total_commands_processed` | Client command dispatches admitted to `Session::execute`; includes session/pub-sub/admin commands and `INFO` itself |
| `rejected_connections` | Accepted sockets closed with `ERR max number of clients reached` |
| `command_budget_rejections` | Commands refused by an enabled global/per-user command budget; the command was not executed or recorded |
| `expired_keys` | Live keys removed by lazy expiration or the bounded maintenance reaper |
| `expired_subkeys` | Hash fields removed by lazy per-field TTL expiry (Redis 7.4 `expired_subkeys`) |
| `evicted_keys` | Keys removed by an enabled payload-budget eviction policy |
| `pubsub_channels` | Distinct channels with at least one live subscription |
| `pubsub_patterns` | Number of distinct active subscription patterns |
| `pubsub_queue_full_drops` | Process-lifetime failed Pub/Sub enqueue attempts due to a full subscriber queue. Excludes disconnected queues and does not reset on `CONFIG RESETSTAT`. |

`INFO keyspace` reports `keys`, `expires`, `avg_ttl` (exact mean remaining
milliseconds across live keys with a deadline, `0` when none) and `subexpiry`
(the number of live hash keys with at least one field deadline). `expired_subkeys` resets with
`CONFIG RESETSTAT`; `subexpiry` is live state and is not a counter.

The [overload policy](overload-policy.md) lists the enforced admission,
frame-assembly, write-stall and command-budget thresholds. A connection shed by
the write-stall deadline disappears from `connected_clients` and its admission
slot becomes reusable; the shedding itself is silent at the protocol level and
has no dedicated counter. `command_budget_rejections` counts only refused
commands, and resets with `CONFIG RESETSTAT` like the other counters.

`INFO clients` reports `connected_clients` (the process-local registry size),
`cluster_connections:0` and `maxclients` (the live connection limit, changeable
through `CONFIG SET maxclients`). `CLIENT LIST` exposes per-connection address/name/user/
RESP/subscription metadata.

`INFO memory` and `INFO all` report `rillcache_payload_headroom_bytes`: the
logical key/value limit minus the accounted payload, with a minimum of zero.
This is not RSS, cgroup or allocator headroom, nor a promise that any particular
write will fit.

`INFO memory` also reports the live `maxmemory_policy` and `notify_keyspace_events`,
plus `used_memory_rss`/`used_memory_rss_human` from Linux `/proc/self/status`
and an explicit `used_memory_rss_scope`. RSS is an observation: it includes
allocator, index, thread, socket and shared pages, and is not used for payload
admission or `maxmemory` enforcement. On platforms without a readable resident
set the value is `0` and the scope says `unavailable_on_this_platform`.
When a cgroup v2 (or v1 fallback) memory file is readable, `INFO memory` also
reports `used_memory_cgroup`, `used_memory_cgroup_human`, `max_memory_cgroup`
(`-1` when the limit is `max`/effectively unlimited), `max_memory_cgroup_human`
and `used_memory_cgroup_scope`. These fields are observational only and are not
used for admission, eviction or OOM handling; they are absent when no cgroup
memory files are readable. `MEMORY MALLOC-STATS` returns a short text stating
that the Rust system allocator exposes no internal statistics; `MEMORY PURGE`
replies `+OK` because there is no rillcache-owned pool to force-purge.
`MEMORY STATS` reports the RSS pair under
`used_memory_rss` / `used_memory_rss_human` with `rillcache.rss.scope` and, when
readable, the same cgroup current/limit pair with `rillcache.cgroup.scope`. `evicted_keys` resets with
`CONFIG RESETSTAT`; the policy itself does not.

## INFO commandstats

`INFO commandstats` (and the commandstats block of `INFO all`) lists only
commands with at least one recorded network dispatch. Each line uses the Redis
shape `cmdstat_<name>:calls=<n>,usec=<n>,usec_per_call=<n>,rejected_calls=<n>,failed_calls=<n>`.

Recording is **opt-in** because the merge gate requires the default dispatch path
to stay free of the per-command lookup and atomic updates. `CONFIG SET
commandstats-enabled yes` starts recording (default `no`); `CONFIG GET
commandstats-enabled` reports the live setting. Enabling begins from an empty
table on each disabled-to-enabled transition; setting `yes` while already enabled
does not reset it. Disabling keeps the existing counters readable until reset or
re-enable. Disabled commandstats performs no lookup, counter update or extra
elapsed-clock read; ordinary commands need no instrumentation timer when all
observers are disabled. A client-pause query also checks the existing atomic
zero-deadline sentinel before sampling wall time; nonzero pause states retain
their existing deadline/mode and concurrency behavior. CONFIG outside MULTI and
EXEC retain timing when needed to observe a commandstats enable during their own
dispatch, including a queued CONFIG applied by EXEC. That enabling EXEC counts
once; its individually queued commands do not.

Basic PING/GET/SET/ECHO dispatch precedes unrelated extension probes. The session
shortcut applies only after authentication/ACL and subscriber checks and outside
MULTI; RESP2 subscriber PING and RESP3 framing retain their original paths. This
is targeted dispatch work reduction, not an all-command performance claim.

- `calls` counts dispatches through a client `Session`; direct embedded
  `Store::execute` calls and commands executed inside `MULTI` are not counted
  separately. `EXEC` is counted once.
- `usec` is measured around client dispatch. Blocking wait time is excluded: a
  blocked command is accounted when it blocks, unlike `SLOWLOG`, which records
  the full wait. A blocked command that disconnects before completion keeps its
  call count but contributes only pre-block time.
- `failed_calls` counts replies whose top-level RESP frame is an error.
  `rejected_calls` counts the subset rejected with `NOAUTH` or `NOPERM`.
- Unknown commands are not tracked. Subcommands share their top-level command
  name. `INFO commandstats` does not include its own in-progress call.
- Counters are relaxed atomics and may be observed slightly asynchronously
  across worker threads. They are diagnostic, not a billing ledger.

## INFO latencystats

Enabling `commandstats-enabled` also records a bounded 32-bucket logarithmic
per-command latency histogram from the same dispatch timer. `INFO latencystats`
(and `INFO all`) emits one line per command with measured calls:

```text
latency_percentiles_usec_ping:p50=15,p99=31,p99.9=63
```

Values are microseconds and are the upper bounds of the bucket containing each
percentile, not exact HDR percentiles. Bucket zero is zero microseconds; bucket
n covers `2^(n-1)..=2^n-1` up to a saturating last bucket. Commands without
recorded calls are omitted, the disabled state returns an empty section,
`CONFIG RESETSTAT` clears the histograms, and enabling commandstats starts fresh.
No timer, histogram update or latency distribution exists while the collector is
disabled. `LATENCY HISTOGRAM` exposes the same buckets through the `LATENCY`
command.

`INFO server`, `memory` and `keyspace` remain available. Payload fields
(`rillcache_payload_*`, `total.allocated`, `dataset.bytes`) are logical key/value
accounting; process RSS is reported separately and is not allocator `used_memory`.
`used_memory` itself is not fabricated. Counters deliberately include no keyspace
hit/miss, net-byte, CPU, latency, fork or replication fields because those values
are not measured. Missing fields are honest gaps, not zero claims. This RSS field
does not enforce a memory bound; `maxmemory` remains a payload budget.

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
Client address/name lookup and cloning happen only after a measured command
passes the duration threshold, not for every fast command.

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

## MONITOR

`MONITOR` registers the calling connection for a process-local stream of commands
received from other clients. Each line is a simple string shaped as
`+<seconds>.<microseconds> [0 <peer-address>] "command" "argument" ...`; quotes,
backslashes and control bytes are escaped. Arguments are capped at 128 bytes and
lines at 4 KiB. The stream uses the same bounded queue capacity as pub/sub, so a
slow monitor drops lines instead of blocking command execution; it is
best-effort and not a durable or complete audit trail.

Credentials are redacted before formatting: `AUTH` shows `AUTH (redacted)`,
`HELLO ... AUTH ...` hides the password, `ACL SETUSER` hides `>`, `<`, `#` and
`!` credential rules, and `CONFIG SET requirepass` hides the value. `RESET` or a
disconnect leaves monitor mode. MONITOR is an admin/dangerous command and cannot
be queued inside `MULTI`.

## LATENCY

`LATENCY LATEST` returns `[event, latest_timestamp_seconds, latest_ms, max_ms]`
for each event series, sorted by event name. `LATENCY HISTORY event` returns the
oldest-to-newest `[timestamp_seconds, latency_ms]` samples, up to 160 per event.
`LATENCY RESET [event ...]` clears one or more series (or all without names) and
returns the number removed. `LATENCY DOCTOR` returns a plain-text report derived
only from measured series, and `LATENCY HELP` lists the supported subcommands.
Unknown history events return an empty array and resetting unknown events returns
zero.

`LATENCY HISTOGRAM [command ...]` returns the bounded per-command latency
histogram collected by `commandstats-enabled`, as an array of
`[command, [calls, <count>, histogram_usec, [bucket_usec, <count>, ...]]]`
entries in registry order. Bucket keys are inclusive upper bounds in
microseconds for the same 32 logarithmic buckets as [INFO
latencystats](#info-latencystats); bucket 0 (zero microseconds) has upper bound
0, and bucket `n` covers `2^(n-1)..=2^n-1`. Only non-empty buckets are listed,
in increasing order. If no command names are given, every command with recorded
dispatches is returned; otherwise only the named ones (case-insensitive) that
have recorded data, so unknown names simply match nothing. The reply is empty
while `commandstats-enabled` is `no` or after `CONFIG RESETSTAT`. Bin counts are
relaxed-atomic observations, not exact per-request timings, and the reply is a
RESP2 array shape for both protocols (this server does not emit RESP3 map
frames).

Samples are recorded only when `latency-monitor-threshold` milliseconds is
non-zero and a measured event reaches that threshold. The default is `0`
(disabled); `CONFIG GET/SET latency-monitor-threshold` exposes and updates it at
runtime and rejects negative or non-integer values. The server measures exactly
two event classes:

- `command`: the duration of a client command attempt, excluding a blocking
  wait; blocked commands record their immediate attempt only.
- `expire-cycle`: bounded expiry reclamation in the maintenance loop.

Fork, AOF write/fsync, eviction-cycle, active-defrag and replication latency are
not measured and are never fabricated. `LATENCY GRAPH` returns an explicit
unsupported error rather than partial data. Event samples and command
histograms are process-local, in-memory and reset on restart or
benchmark/manual reset.

## Metrics, logs and health contract

This is the supported operational surface. Anything not listed is not a contract.

### Metrics

`INFO` sections and their units: `server` (versions, uptime seconds), `clients`
(connection counts, `maxclients`, auth budgets), `memory` (logical key/value
bytes and limits, process RSS and cgroup current/limit with an explicit scope,
`maxmemory_policy`), `persistence` (journal enabled state, rewrite phase,
`aof_last_write_status`/`aof_last_bgrewrite_status`), `stats` (command/expiry/
eviction/key-miss counters), `replication` (`role:master`, zero offsets),
`keyspace` (`db0` keys/expires/avg_ttl), `commandstats` (per-command calls,
usec, usec_per_call), `latencystats` (`p50`/`p99`/`p99.9` **32-bucket
logarithmic approximations**, not exact percentiles) and `errorstats`
(per-error counters). Sampling limits: histograms update only while
`commandstats-enabled` is on; counters are process-lifetime unless
`CONFIG RESETSTAT`; rates are not computed.

### Redaction

Metrics and logs never carry credentials or values: `MONITOR` redacts `AUTH`
arguments, `SLOWLOG` truncates arguments, `CLIENT LIST/INFO` omit passwords,
audit lines hex-encode user and command without keys/values, and healthcheck
diagnostics omit paths and values so they can be retained. `INFO` reports
logical payload accounting, not allocator internals.

### Logs

Startup reports the bound listener and TLS listener; shutdown reports received
signal, completion, or `forced termination: <reason>` (nonzero exit); the audit
path reports denials and a redacted collector-failure count; all failure
messages are bounded and avoid echoing secrets.

### Health probe

`--healthcheck ADDR [--healthcheck-user U --healthcheck-password-file P]
[--healthcheck-tls-ca-file CA --healthcheck-server-name NAME]
[--healthcheck-require-aof] [--healthcheck-min-payload-headroom BYTES]
[--healthcheck-aof-write-probe DIR]` returns exit 0 on success and nonzero on
any failure within a one-second deadline. It fails closed when the server cannot
admit the probe (overload), when authentication or ACL denies it, when
`--healthcheck-require-aof` sees `aof_enabled:0` or a non-`ok` write/rewrite
status, when the payload headroom is below the requested minimum, and when the
optional temporary write probe cannot publish and fsync. It does **not** prove
durability, recovery from a torn journal, or full storage capacity.

Evidence: `tests/observability_contract.rs` proves the probe fails under
admission saturation and recovers after capacity is released, fails when a
journal is required but disabled, and rejects an impossible headroom
requirement; `tests/cli.rs` covers authenticated/ACL-denied probes and redacted
password-file handling; the signal suites cover shutdown logging.

## Scope and reset

`total_commands_processed` covers connection-level dispatch, so embedding
`Store::execute` directly without a `Session` does not increment it. A `MULTI`
counts `MULTI`, each queued command and `EXEC` as separate dispatches. A rejected
command still counts as processed; it does not increment data-specific counters.
Health-check PING probes are ordinary connections and commands.

`CONFIG RESETSTAT` zeroes the connection, command, rejected, expired, evicted,
pub/sub and per-command counters. It does not delete data, clear subscriptions, reset the keyspace or reset
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
aof_rewrite_phase:none
aof_rewrite_keys_done:0
aof_rewrite_keys_total:0
aof_last_bgrewrite_status:ok
aof_last_write_status:ok
aof_rewrite_bytes_written:0
aof_current_size:1234
```

With persistence disabled only `loading:0` and `aof_enabled:0` are returned.
`aof_last_write_status` reflects the journal's latched failure state and
`aof_last_bgrewrite_status` the last completed `BGREWRITEAOF`. Since the
cooperative rewrite change, `aof_rewrite_in_progress` reports whether a rewrite is
currently running, `aof_rewrite_phase` reports the bounded phase, and
`aof_rewrite_keys_done`/`aof_rewrite_keys_total` report stable snapshot progress.
Reconciliation may revisit keys, so key progress never exceeds the snapshot
total. `aof_rewrite_bytes_written` counts bytes written to the temporary file,
including headers, integrity records and reconciliation. It resets to zero
after completion or abort, is omitted if journal status cannot be read, and
is not a durability or percent-complete metric. `aof_current_size` still
describes the live journal, not the temporary file.
`aof_last_rewrite_start_usec` is absent before the first successful setup. It
measures setup (snapshot key collection and staging writes), not the full
rewrite or client-observed latency. `aof_last_rewrite_finish_usec` measures
successful finalization under exclusive data access, including tail
reconciliation and publication. It is absent before the first successful
completion, and a failed rewrite does not replace it. Both timings are
process-local and not persisted.
RDB, backlog, bio and fork fields are omitted because they are not implemented or
measured.

## Deliberate gaps

- No per-policy eviction latency or memory-pressure counters beyond `evicted_keys`.
- No per-client fd/idle/command/buffer/memory/network fields, blocked-client count
  or cross-process client view. `INFO clients` intentionally omits those fields.
- `INFO latencystats` uses a bounded 32-bucket logarithmic approximation, not an
  HDR histogram or exact percentile contract. There is no CPU accounting,
  network-byte counter or per-database stats. `LATENCY` exposes only the two
  measured event series listed above; the commandstats histogram is opt-in and
  its bucket upper bounds are the documented unit. RSS is reported but not enforced.
- No metrics exporter, OpenTelemetry, structured logs, health beyond PING, alerts,
  SLOs, dashboards or long-running soak evidence.
- Error statistics are a bounded per-code count, not a full error log or message
  history.
- No persistence/ACL/auth telemetry. Journal replay and rewrite durations are not
  reported. A future observability increment must preserve the same no-fabrication
  rule instead of filling Redis-shaped fields with guesses.

## Evidence

`tests/observability.rs` runs in all worker modes and verifies connection/command
counts across two clients, reset behavior, maximum-client rejection visibility and
lazy expiration counting. `tests/commandstats.rs` verifies per-command calls,
failed/rejected classification, unknown-command exclusion, `INFO all` inclusion
and `CONFIG RESETSTAT` clearing in every worker mode. `tests/errorstats.rs`
verifies per-code counts, `INFO all` inclusion and reset clearing. These wire
tests parse INFO replies as real RESP bulk values rather than asserting on
implementation strings alone. `tests/commandstats_exec.rs` covers enabling from
a queued CONFIG with every observer initially disabled. `tests/core_dispatch.rs`
guards authentication, ACLs, MULTI queueing, RESP3 binary/null framing and
subscriber PING distinctions.
