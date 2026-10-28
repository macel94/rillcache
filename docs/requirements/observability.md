# Observability requirements

Part of the [requirements and incremental delivery index](../requirements.md). Requirements are preserved verbatim from the former monolithic EARS sections; add new requirements only in the matching area file.

## Cursor iteration (S30)

- **R121 (event-driven):** When `SCAN cursor [MATCH pattern] [COUNT n] [TYPE type]`
  is received, the server shall reap expired keys, take a sorted live-key
  snapshot, return at most the requested examined positions of matching keys and
  a decimal continuation cursor, using `0` only when the snapshot is exhausted.
  Matching and type filtering shall not prevent cursor progress or termination.
- **R122 (event-driven):** When `HSCAN`/`SSCAN`/`ZSCAN key cursor [MATCH] [COUNT]
  [NOVALUES]` is received on a matching collection, the server shall iterate a
  deterministic sorted snapshot and return fields/values, members or member/score
  pairs; missing keys shall return cursor `0` and an empty array.
- **R123 (unwanted behavior):** If the cursor is not a nonnegative canonical
  decimal, `COUNT` is not positive, an option is incomplete/unsupported, or the
  key has the wrong type, then the server shall return one bounded error without
  mutating state. `NOVALUES` shall be accepted only for HSCAN, and `TYPE` only for
  SCAN.
- **R124 (state-driven):** While cursors are used, the server shall retain no
  per-cursor state, bound each page's examined positions and returned data, and
  document that mutation during iteration may skip or duplicate elements even
  though a stable keyspace is fully traversed.

## Publish/subscribe (S31)

- **R125 (event-driven):** When `SUBSCRIBE`/`PSUBSCRIBE` is received, the server
  shall register the connection, remove duplicate registrations, and return one
  confirmation per channel/pattern containing the current total subscription count;
  `UNSUBSCRIBE`/`PUNSUBSCRIBE` shall remove named or all local subscriptions and
  confirm each, including the no-subscription null-channel reply.
- **R126 (event-driven):** When `PUBLISH channel message` is received, the server
  shall deliver one preformatted message frame to each current channel subscriber
  and each matching pattern subscriber, return the number delivered, and never let
  a slow/dead subscriber block the publisher or another subscriber. Delivery is
  bounded, at-most-once, non-persistent and not an exactly-once claim.
- **R127 (state-driven):** While a RESP2 connection has subscriptions, only
  subscribe/unsubscribe variants, `PING` (subscriber array form) and `QUIT` shall
  execute; RESP3 shall retain normal commands and use push frames. `PUBSUB
  CHANNELS/NUMSUB/NUMPAT` shall expose current state; sharded variants shall fail
  visibly. ACL channel patterns shall gate subscribe/publish arguments with
  `NOPERM`, while introspection remains category-governed.
- **R128 (ubiquitous):** Pub/sub tests shall run offline in all execution modes and
  cover binary channels/messages, patterns, counts, unsubscribe-all, RESP2/RESP3
  framing, subscriber-mode errors/PING/QUIT, ACL channel denial, disconnect cleanup
  and joined shutdown. Documentation shall retain missing sharding, keyspace
  notifications, persistence, replication, ordering and durability guarantees.

## Process statistics (S34)

- **R134 (event-driven):** When `INFO stats` is requested, the server shall return
  a RESP bulk string with `total_connections_received`,
  `total_commands_processed`, `rejected_connections`, `expired_keys`,
  `evicted_keys`, `pubsub_channels` and `pubsub_patterns` under a `# Stats`
  section, and include that section for `INFO`/`INFO all` without inventing fields
  it does not measure.
- **R135 (state-driven):** While the process serves clients, connection counters
  shall count accepted and max-client-rejected sockets, command counters shall
  count connection dispatch (including rejected commands and `INFO`), expiration
  counters shall count lazy/maintenance removals, and pub/sub counters shall reflect
  live hub state. Counters are relaxed diagnostics, not a transaction ledger.
- **R136 (event-driven):** When `CONFIG RESETSTAT` succeeds, it shall zero the
  counters without deleting data or subscriptions; tests shall cover the reset
  snapshot including the counting of the reset/INFO commands, maximum-client
  rejection, lazy expiration and all execution modes. Documentation shall retain
  the missing hit/miss, byte, latency, slowlog, client-list, RSS and metrics gaps.

## Keyspace notifications (S40)

- **R157 (event-driven):** When `--notify-keyspace-events` or `CONFIG SET
  notify-keyspace-events` receives supported flags (`K`, `E`, `g`, `$`, `l`, `s`,
  `h`, `z`, `t`, `x`, `e`, `A`), the server shall atomically update the live classes;
  unsupported/unknown classes and malformed flags shall fail without changing
  configuration.
- **R158 (event-driven):** When a supported mutating command changes keys, the
  server shall publish one `__keyspace@0__:<key>` message with the event name if
  `K` and the category are enabled and one `__keyevent@0__:<event>` message with
  the key if `E` and the category are enabled. Non-mutating/rejected commands shall
  publish nothing; MULTI shall publish per queued command and blocking pops shall
  publish on success. Successful MSETEX writes shall publish `set` followed by
  `expire` for explicit expiry in supplied pair order, including duplicate keys;
  KEEPTTL shall publish no new `expire`, and rejected/OOM writes shall publish no
  write events even if unrelated expired keys are reaped.
- **R159 (unwanted behavior):** If notifications are disabled, a category is not
  selected, a subscriber lacks ACL channel permission, or a command is unknown/
  non-mutating, then no notification event shall be emitted or delivered to that
  subscriber. Keymiss/new-key/module and `FLUSH*` keyless events shall not be
  claimed; their flags shall be rejected.
- **R160 (state-driven):** While notifications are enabled, mutation capture and
  publication shall follow command execution order and remain at-most-once and
  non-persistent. All-mode tests shall cover keyspace/keyevent frames, category and
  runtime toggles, binary channels, rename/expire, MULTI, blocking pops, HLL,
  expired/evicted classes and `CONFIG`/`INFO` exposure. Documentation shall retain
  the missing event classes, durable/replicated delivery, global happens-before
  and exhaustive-audit limits.

## Sharded publish/subscribe (S47)

- **R185 (event-driven):** When `SSUBSCRIBE channel [channel ...]` and
  `SUNSUBSCRIBE [channel ...]` run, the server shall maintain an independent
  sharded-channel set per connection, return one confirmation per name using the
  shard subscription count, remove all named or all local shard subscriptions, and
  use a null channel confirmation when no shard subscriptions remain.
- **R186 (event-driven):** When `SPUBLISH channel message` runs, it shall deliver
  one preformatted `smessage` frame to each current sharded subscriber only and
  return the delivered count. Normal `PUBLISH` shall not reach sharded subscribers
  and `SPUBLISH` shall not reach normal/pattern subscribers. `PUBSUB
  SHARDCHANNELS [pattern]` and `PUBSUB SHARDNUMSUB [channel ...]` shall expose only
  the sharded namespace.
- **R187 (state-driven):** While a RESP2 connection has any normal or sharded
  subscription, only the documented subscribe/unsubscribe, PING, QUIT and RESET
  commands shall execute; `RESET`/close/QUIT cleanup shall remove shard registry
  entries exactly once. ACL channel patterns shall gate `SSUBSCRIBE`,
  `SUNSUBSCRIBE` and `SPUBLISH`.
- **R188 (ubiquitous):** Sharded delivery shall use the existing bounded
  per-subscriber queue and at-most-once non-blocking fan-out, add no permanent
  worker or dependency, and be covered by unit/all-mode wire tests for binary
  channels, counts, separation, introspection, subscriber-mode errors, RESP3 push
  frames and cleanup. There is no cluster hash-slot routing, persistence,
  replication or durable delivery claim.

## Expired and evicted notification classes (S49)

- **R193 (event-driven):** When `notify-keyspace-events` receives `x` or `e`, the
  server shall enable `expired` and `evicted` event classes respectively; `A`
  shall include both, and the canonical string shall round-trip them. Unknown
  flags and still-unsupported keymiss/new-key/module flags shall fail without
  changing configuration.
- **R194 (event-driven):** When a key is logically removed by lazy lookup
  expiration or by bounded background reclamation, the server shall publish
  `expired` with the key through the enabled `K`/`E` channels. Setting a TTL
  without removal shall publish no `expired` event.
- **R195 (event-driven):** When a configured eviction policy actually removes a
  victim, the server shall publish `evicted` with the victim key. `noeviction`
  rejection, failed admission and explicit deletion shall not publish `evicted`,
  and the victim's command event shall be suppressed when eviction occurs within
  the same command.
- **R196 (ubiquitous):** Lifecycle records shall use bounded per-kind in-memory
  queues drained after the triggering command or maintenance pass, remain
  at-most-once and non-persistent, add no worker or dependency, and be covered by
  unit and all-mode wire tests for injected-clock lazy expiry, reaped expiry,
  eviction, `CONFIG` round-trip and flag/category filtering. Durable delivery,
  replay and complete-audit guarantees shall remain documented as absent.

## Runtime clock and object introspection (S61)

- **R241 (event-driven):** `TIME` shall return a two-element array of decimal bulk
  strings containing the current Unix seconds and microsecond component from the
  system wall clock, with exactly one argument and no keyspace mutation.
- **R242 (event-driven):** `OBJECT ENCODING key` and `OBJECT REFCOUNT key` shall
  return the key's actual storage representation and an owned-value refcount of
  one, respectively, or a null bulk for a missing/expired key. Encoding names
  shall reflect the selected list/set/hash/zset/stream representation rather than
  fabricated Redis encodings.
- **R243 (event-driven):** `OBJECT IDLETIME` and `OBJECT FREQ` shall return
  explicit unsupported/not-selected errors because read-idle time and LFU
  frequency are not tracked. `OBJECT HELP` shall return bounded help rows, and
  unknown subcommands or wrong arities shall return the Redis-shaped OBJECT error.
- **R244 (ubiquitous):** Discovery/ACL metadata shall expose TIME and OBJECT with
  truthful arity, category, readonly/fast flags and key position; tests shall
  cover time shape, encoding transitions, missing/expired keys, refcount,
  unsupported cases, help, malformed subcommands, MULTI and all worker modes.
  Allocator/RSS, LFU, radix-tree and true read-idle statistics remain documented
  as unsupported.

## Command key introspection (S66)

- **R261 (event-driven):** `COMMAND GETKEYS <command> [args ...]` shall resolve the
  command case-insensitively, validate its advertised arity, and return the
  declared key arguments or the documented invalid-command/arity/no-key errors.
- **R262 (event-driven):** Key extraction shall reuse the ACL key-spec logic,
  including multi-key ranges, counted key lists, STORE destinations, legacy
  GEORADIUS destinations and sorted-set source/destination rules.
- **R263 (event-driven):** Commands with no declared keys and dynamic lists that
  cannot be extracted from the supplied arguments shall return bounded errors
  rather than fabricating keys. BY/GET-derived SORT keys remain outside static
  extraction and are documented as such; XREAD/XREADGROUP stream keys are
  extracted from the supplied `STREAMS` block.
- **R264 (ubiquitous):** Tests shall cover string, multi-key, counted, STORE,
  GEORADIUS and no-key commands, invalid command/arity and all worker modes.
  COMMAND GETKEYS does not change keyspace state or bypass ACL enforcement.

## Key access flags (S116)

- **R420 (event-driven):** `COMMAND GETKEYSANDFLAGS <command> [args ...]` shall
  return an array of two-element arrays containing the key and a one-element
  flags array. Each flags array shall report the primary ACL access class of that
  key: `RO` for read-only access and `RW` for writable or read-modify-write
  access. Command resolution, arity validation and key extraction shall match
  `COMMAND GETKEYS`, including counted key lists, STORE destinations and
  read-source/write-destination ordering.
- **R421 (unwanted behavior):** Invalid commands, wrong arity, commands without
  key arguments and arguments from which declared keys cannot be extracted shall
  return the same bounded errors as `COMMAND GETKEYS`, using
  `command|getkeysandflags` in the wrong-arity message. The command shall not
  change keyspace state, bypass ACL enforcement or reflect arbitrary bytes.
- **R422 (ubiquitous):** Tests shall cover string, multi-key, read-source and
  write-destination, and no-key cases plus invalid command and arity in every
  worker mode. Documentation shall retain that flags report only the coarse
  read/write access class enforced by the bounded ACL model; granular Redis
  key-spec flags (`OW`, `RM`, `access`, `update`, `insert`, `delete`,
  `variable_flags`), subcommand key specs and full command docs remain
  unsupported.

## Slow command log (S70)

- **R276 (event-driven):** `SLOWLOG GET [count]`, `LEN`, `RESET` and `HELP` shall
  return newest-first entries `[id, timestamp, duration_us, argv, client_address,
  client_name]`, the current bounded length, an acknowledgement plus cleared
  entries, and help text. `count` shall default to 10, accept `-1` as all,
  `0` as empty and reject smaller or non-integer values with bounded errors.
- **R277 (state-driven):** While `slowlog-log-slower-than` is non-negative and
  `slowlog-max-len` is non-zero, commands whose measured duration reaches the
  threshold shall be retained up to the configured maximum. `CONFIG GET/SET`
  shall expose and update both values atomically; negative thresholds disable
  recording, zero maximum length clears/disables recording, lowering the maximum
  trims immediately, and the maximum shall be bounded at 1,000,000 entries.
- **R278 (event-driven):** Each entry shall retain at most 32 arguments and 128
  bytes per argument with explicit remainder markers, redact credentials supplied
  through `AUTH`, `HELLO AUTH`, sensitive `CONFIG SET` values and
  `ACL SETUSER`/`DELUSER`/`GETUSER`, and measure the full wait for blocking
  commands. Queued `MULTI` commands shall not be logged separately; `EXEC` shall
  be logged once for the batch, documented as a divergence.
- **R279 (ubiquitous):** Tests shall cover recording, ordering, count semantics,
  truncation, redaction, configuration validation and bounds, blocking duration,
  `MULTI` behavior, disabling and every worker mode. The log shall remain
  process-local, in-memory and absent from persistent storage or `INFO`.

## Latency event monitoring (S73)

- **R288 (event-driven):** `LATENCY LATEST`, `HISTORY event`, `RESET [event ...]`,
  `DOCTOR` and `HELP` shall return Redis-shaped arrays, plain-text reports, reset
  counts and bounded errors. `HISTORY` for an unknown event shall return an empty
  array; resetting unknown events shall count zero; `GRAPH` and `HISTOGRAM` shall
  return explicit unsupported errors rather than partial output.
- **R289 (state-driven):** While `latency-monitor-threshold` is non-zero, only
  measured events whose duration reaches the threshold shall be retained, up to
  160 samples per event with latest and all-time maximum values. The default
  threshold shall be `0` (disabled), `CONFIG GET/SET` shall expose and atomically
  update it, and negative or non-integer values shall be rejected without
  changing the active value.
- **R290 (event-driven):** The server shall record only events it actually
  measures: command-attempt duration and bounded expire-cycle reclamation.
  Blocking waits, fork, AOF write/fsync, eviction-cycle, active-defrag and
  replication latency shall not be represented. `DOCTOR` output shall derive only
  from measured series and documented limitations.
- **R291 (ubiquitous):** Tests shall cover configuration and validation, latest
  and history ordering/shapes, reset counts, the sample bound, threshold changes,
  unsupported subcommands and every worker mode. Data shall remain process-local,
  in-memory and absent from persistence or fabricated `INFO` fields.

## Explicit unsupported cluster, module and replica surface (S74)

- **R292 (event-driven):** `CLUSTER <subcommand>` shall return the documented
  cluster-disabled error for known subcommands, one bounded explanatory row for
  `HELP`, and the normal arity error without a subcommand. `READONLY`,
  `READWRITE` and `ASKING` shall return the same disabled error. No slot table,
  node identity, redirection or migration behavior shall be fabricated.
- **R293 (event-driven):** `MODULE LIST` shall return an empty array because no
  modules are loadable. `MODULE HELP` shall return one bounded explanatory row;
  `LOAD`/`LOADEX`/`UNLOAD` shall return an explicit unsupported error without
  executing code; unknown subcommands shall return a bounded unknown-subcommand
  error.
- **R294 (event-driven):** `WAIT numreplicas timeout` shall validate integer
  arguments, reject a negative timeout with the Redis timeout error, and return
  zero without sleeping because no replicas are configured. It shall not claim
  replication or acknowledgment semantics.
- **R295 (ubiquitous):** Discovery metadata/categories shall expose the supported
  command shapes truthfully. Tests shall cover cluster subcommands, help, arity,
  module list/load/unknown, WAIT validation and zero, READONLY/READWRITE/ASKING,
  ACL access and all worker modes. Documentation shall retain the absence of
  cluster routing, module execution, replication and Redis-8 extension support.

## Per-command statistics (S-CS)

- **R-CS1 (event-driven):** While `commandstats-enabled` is `yes` (default `no`,
  set through `CONFIG SET commandstats-enabled yes|no` and read through `CONFIG
  GET`), `INFO commandstats` or `INFO all` shall return a RESP bulk string with a
  `# Commandstats` section and one
  `cmdstat_<name>:calls=..,usec=..,usec_per_call=..,rejected_calls=..,failed_calls=..`
  line for each command dispatched through a client session since enabling or the
  last reset. Recording shall start from an empty table, registered commands with
  no calls and unknown commands shall be omitted, and `usec_per_call` shall be
  integer `usec / calls`. When disabled, the dispatch path shall incur no
  per-command lookup, elapsed-clock read or counter update solely for commandstats.
  Existing counters shall remain readable while disabled; a disabled-to-enabled
  transition shall clear them before recording the enabling command. With all
  observers disabled, ordinary dispatch shall create no instrumentation timer;
  CONFIG outside MULTI and EXEC may retain timing because they can enable
  commandstats directly or through a queued CONFIG during dispatch. The enabling
  EXEC shall be counted once, not its individually queued commands.
- **R-CS2 (state-driven):** While recording, `calls` shall count client
  dispatches (including failed and rejected calls, `INFO` and `CONFIG RESETSTAT`,
  but not direct embedded `Store::execute` or individually queued `MULTI`
  commands), `usec` shall accumulate measured dispatch time, `failed_calls` shall
  count top-level error replies, and `rejected_calls` shall count the subset
  rejected with `NOAUTH`/`NOPERM`. Blocking wait time shall be excluded and
  documented as a deliberate difference from `SLOWLOG`. Subcommands shall share
  their top-level command name.
- **R-CS3 (event-driven):** When `CONFIG RESETSTAT` succeeds, it shall zero every
  per-command counter together with the existing process counters without deleting
  data or subscriptions; the reset command itself may be counted afterwards.
- **R-CS4 (ubiquitous):** Tests shall cover default-disabled behavior, the
  `CONFIG GET/SET commandstats-enabled` contract, calls, aggregate time shape,
  failed-call classification, authentication/ACL rejection classification,
  unknown-command exclusion, `INFO all` inclusion, reset clearing and every worker
  mode. Documentation shall retain the absence of latency histograms/percentiles,
  cross-process aggregation, persistence and billing-grade exactness. SLOWLOG
  shall capture client address/name only after a sample passes its threshold;
  tests shall prove rejected/disabled samples do not invoke metadata capture.

## Lifecycle notification delivery cost (S76)

- **R300 (state-driven):** While neither the expired nor the evicted notification
  class is enabled, command dispatch shall not acquire the store write lock solely
  to drain lifecycle queues, and pending lifecycle keys shall remain bounded.
- **R301 (event-driven):** When notification flags change, the server shall discard
  pending undispatched lifecycle keys before storing the new flags, so enabling an
  expired/evicted class only delivers events observed after that point.
- **R302 (ubiquitous):** Core dispatch benchmarks shall record the improvement and
  all-mode notification tests shall still cover expired/evicted delivery, runtime
  toggles, bounded queues and joined shutdown. Documentation shall retain the
  process-local, at-most-once and non-persistent delivery limits.

## Command registry filters (S77)

- **R303 (event-driven):** `COMMAND LIST FILTERBY PATTERN glob` shall return only
  registered command names matching the byte-oriented glob, preserving registry
  (lower-case sorted) order without reflecting arbitrary input in errors.
- **R304 (event-driven):** `COMMAND LIST FILTERBY ACLCAT category` shall accept a
  bare or `@`-prefixed category, reuse the ACL category matcher for registered
  names, and return an empty list for unknown categories. `FILTERBY MODULE name`
  shall return an empty list because no modules are loadable.
- **R305 (ubiquitous):** Missing values, unknown filters, duplicate/multiple
  FILTERBY clauses and extra arguments shall return bounded syntax errors without
  partial output. Tests shall cover pattern/category/module filters, `@` prefixes,
  unknown categories, syntax failures and all worker modes. No module metadata or
  Redis extended command docs are implied.

## Truthful standalone replication status (S79)

- **R310 (event-driven):** When `ROLE` is received, the server shall return the
  three-element master array `["master", 0, []]` because no replication stream or
  replicas exist, reject extra arguments with the normal arity error, and expose
  the documented `@admin`/`@fast`/`@dangerous` ACL categories. It shall not claim
  PSYNC, replica acknowledgement or failover support.
- **R311 (event-driven):** When `INFO replication` or `INFO all` is requested,
  the server shall return a `# Replication` block with `role:master`,
  `connected_slaves:0`, `master_failover_state:no-failover` and
  `master_repl_offset:0`. It shall omit `master_replid`, replica rows, link
  status and backlog fields rather than fabricating them, and `INFO` without
  arguments shall keep its existing sections.
- **R312 (ubiquitous):** Tests shall cover the exact `ROLE` shape, arity failure,
  replication fields, `INFO all` inclusion, ACL category metadata and every
  worker mode. Documentation shall retain the absence of replication, PSYNC,
  replica management, Sentinel, cluster routing, failover and mixed-version
  interoperability.

## Sub-key expiry observability (S81)

- **R316 (event-driven):** When `INFO keyspace` or `INFO all` is requested, the
  server shall include `subexpiry` in the `db0` line, counting live hash keys
  with at least one unexpired field deadline.
- **R317 (event-driven):** When `INFO stats` or `INFO all` is requested, the
  server shall include `expired_subkeys`, counting hash fields removed by lazy
  per-field TTL expiry since startup or the last `CONFIG RESETSTAT`.
- **R318 (ubiquitous):** Tests shall cover lazy field expiry updating both
  values, a live field deadline keeping `subexpiry` set, `CONFIG RESETSTAT`
  clearing `expired_subkeys` without changing live `subexpiry`, and every worker
  mode. Documentation shall retain that field expiry is lazy, not actively
  scanned, and that neither value is an RSS or payload-accounting measurement.

## Error reply statistics (S84)

- **R323 (event-driven):** `INFO errorstats` and `INFO all` shall return a
  `# Errorstats` block with one `errorstat_<CODE>:<count>` line per observed
  command error code. The code shall be the bounded first token of the error reply
  (maximum 32 bytes), and unknown/empty codes shall not be recorded.
- **R324 (state-driven):** Counts shall increment once per error reply emitted for
  a dispatched command, be process-local, and be cleared by `CONFIG RESETSTAT`
  together with the other statistics. The table shall be bounded to 64 distinct
  codes; protocol framing errors are not command replies and are excluded.
- **R325 (ubiquitous):** Tests shall cover empty stats, multiple codes and counts,
  `INFO all` inclusion, reset clearing and all worker modes. This is a bounded
  count table, not an error log, message history, durable audit or replication
  statistic.

## Keyspace average TTL (S85)

- **R326 (event-driven):** When `INFO keyspace` or `INFO all` is requested, the
  server shall include `avg_ttl` in the `db0` line: the exact integer mean of
  remaining milliseconds for live keys with a deadline, or `0` when no live key
  has a deadline.
- **R327 (ubiquitous):** Tests shall cover the empty state, mixed expiring and
  persistent keys, post-deletion reset to zero and every worker mode.
  Documentation shall retain that this is an exact mean over live key deadlines,
  not Redis's sampled estimate, and not an RSS or payload measurement.

## Bounded MONITOR stream (S93)

- **R350 (event-driven):** `MONITOR` with no arguments shall return `+OK`,
  register the caller for the process-local command stream and stream commands
  from other connections. Wrong arity shall return the standard arity error, and
  the monitor's own commands shall not be echoed back to it. `RESET`, `QUIT` and
  disconnect shall remove the registration.
- **R351 (event-driven):** Each monitor line shall be a simple string shaped as
  `+<seconds>.<microseconds> [0 <peer-address>] "<command>" "<argument>" ...`,
  escaping backslash, quotes and control bytes. Arguments shall be capped at 128
  bytes and lines at 4 KiB. Delivery shall use a bounded queue with best-effort
  drops for slow readers so a monitor can never block command execution.
  `AUTH`, `HELLO ... AUTH`, `ACL SETUSER` credential rules and `CONFIG SET
  requirepass` values shall be redacted before formatting.
- **R352 (ubiquitous):** MONITOR shall be an admin/dangerous command, shall
  return an explicit error when queued inside `MULTI` because it cannot change
  session mode during `EXEC`, and all-mode tests shall cover streaming, redaction,
  RESET, arity, slow-reader service and every worker mode. Documentation shall
  retain that the stream is process-local, not persisted, not replayed,
  best-effort under backpressure and not a complete audit trail.

## Payload memory reporting (S98)

- **R365 (event-driven):** `MEMORY STATS` shall return a flat RESP2 map containing
  `peak.allocated`, `total.allocated`, `keys.count`, `dataset.bytes`,
  `dataset.percentage`, `rillcache.payload.limit_bytes` and
  `rillcache.payload.scope`. The peak shall be the process-lifetime payload
  high-water mark and shall survive deletes, flush and `CONFIG RESETSTAT`;
  `keys.count` shall count live keys, while byte fields report currently held
  key-plus-value payload. Extra arguments shall return a bounded arity error.
- **R366 (event-driven):** `MEMORY DOCTOR` shall return a bounded plain-text
  report derived from the same snapshot, distinguishing an empty payload, high
  pressure at or above 90% with `noeviction`, pressure at or above 75%, and a
  healthy state. It shall always identify the report as payload accounting, not
  RSS or allocator health. `MEMORY HELP` shall list the supported subcommands and
  `MEMORY USAGE` shall retain its existing logical estimate.
- **R367 (ubiquitous):** Tests shall cover the empty and populated fields, peak
  retention after deletion, limit changes, the pressure and empty doctor states,
  HELP, arity and every worker mode. Documentation shall retain the absence of
  RSS, allocator active/resident, fragmentation, replication-buffer and
  background-eviction measurements.

## Published messages in transactions (S99)

- **R368 (event-driven):** Inside `MULTI`, valid `PUBLISH channel message` and
  `SPUBLISH channel message` commands shall return `+QUEUED` and deliver nothing
  at queue time. `EXEC` shall publish each command once in queue order through
  the existing normal or sharded fan-out and append its delivered-subscriber
  count as the corresponding array element.
- **R369 (unwanted behavior):** Wrong arity shall dirty the transaction and
  produce `EXECABORT` without publishing. ACL command/channel checks shall run
  again before `EXEC`; a denial shall abort the transaction before any queued
  message is published.
- **R370 (ubiquitous):** Tests shall cover queued normal and sharded messages,
  no early delivery, EXEC reply order/counts, subscriber delivery and every
  worker mode. Documentation shall retain that subscription commands, `PUBSUB`
  introspection and blocking pops remain rejected inside `MULTI`, and that
  delivery is process-local and at-most-once rather than durable.
