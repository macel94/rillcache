# Requirements and incremental delivery

The README is the product charter. This document turns it into small, testable
increments; it is not a claim of full Redis compatibility.

## Non-negotiable constraints

- **Safe Rust only:** every project target forbids `unsafe_code`. No FFI, raw
  pointer manipulation, custom allocator, native extension or build script.
  Safe standard-library APIs are allowed; the standard library/runtime/OS have
  internals outside this project's code and are not claimed to contain no unsafe.
- **Zero external Rust dependencies**, including build, development, test and
  benchmark dependencies. The dependency-policy test inspects all targets/features.
- All project code is AI-generated. Human review, product judgment and test-based
  validation remain central. Automation does not replace that validation.
- Compatibility is incremental, byte-oriented and demonstrated by tests.
- Rust alone is not evidence of speed. Performance claims require comparable,
  repeated end-to-end measurements against current Redis and Valkey releases.
- All future main commits must come from PRs satisfying the [mandatory test and
  Valkey gate](pr-quality-gate.md), including docs/reports. No direct/admin/bot bypass.

## INVEST stories: implemented increments

Each story provides a separately testable outcome. Estimates are relative scope
(XS/S), not delivery promises. Internal design remains negotiable; the constraints
above do not. Shared RESP plumbing is an explicit technical prerequisite, not a
reason to couple all future commands into a single large story.

| ID | Story and value | Size | Acceptance evidence |
| --- | --- | --- | --- |
| S01 | As a client author, I can check connectivity and echo binary data so that I can validate my transport. | S | R10–R13; protocol unit tests and TCP tests |
| S02 | As a cache user, I can store, overwrite and retrieve strings so that independent clients can share cached bytes. | S | R20–R22; engine and TCP tests |
| S03 | As a cache user, I can check and delete keys so that I can invalidate data. | XS | R23–R24; duplicate-key and shared-connection tests |
| S04 | As a developer, I can run the same offline checks locally and on GitHub so that each change receives fast feedback. | S | R30–R32; `make ci` and CI workflow |
| S05 | As a performance investigator, I can reproduce validated workloads against all three servers so that optimization starts with evidence. | S | R40–R43; benchmark tests, smoke and comparison targets |
| S06 | As a cache user, I can create a value only when absent or replace it only when present so that concurrent clients do not race a separate existence check. | S | R25–R29; engine, pipeline, racing-client and current-reference tests |
| S07 | As a performance investigator, I can compare equal CPU budgets using unchanged binaries so that parallelism and build churn do not masquerade as implementation improvements. | S | R44–R46; recorded affinities, rotated order and cross-profile digest checks |
| S08 | As a performance investigator, I can sample server CPU, RSS, idle behavior and thread scheduling in isolated comparisons so that I can choose an I/O change from evidence rather than blaming the key-map lock. | S | R47–R51; procfs fixtures, standalone CLI tests and profiled comparisons |
| S09 | As a repository reader, I can read validated performance tables and an optional GPT-6 Luna review after each comparative run without downloading artifacts, so current tradeoffs are visible from the README. | S | R52–R56; archived-cohort, failure, publication and workflow tests |
| S10 | As a repository owner, I get an actual Copilot assessment automatically without creating credentials, with an available-model fallback and visible failure if no review completes. | S | R57–R59; workflow-token inference, usage/fallback/gate and legacy-publication tests |
| S11 | As an operator, I can select a fixed number of I/O/execution workers and verify concurrent behavior in every mode locally and on GitHub. | S | R60–R64; mode-matrix TCP, standalone-thread and benchmark tests |
| S12 | As a cache user, I can expire strings atomically and use Redis-compatible SET/expiry options without depending on polling or sleeps for correctness. | S | R65–R68; injected-clock, wire, budget and reference tests |
| S13 | As a counter user, I can atomically change signed 64-bit integer strings without lost updates, overflow or changing their TTL. | S | R69–R71; boundary, many-writer and reference tests |
| S14 | As an operator, I can run a tiny non-root image without configuration, append worker options and probe it without installing a shell/client. | S | R72–R74/R78; static/size/default-network/health/mode tests and deployment examples |
| S15 | As a user, I get tested native amd64/arm64 images from GHCR automatically, with a visible anonymous-access gate instead of an unverified public-publishing claim. | S | R75–R77; isolated publishing, tested archives, manifest and anonymous pull/run jobs |
| S16 | As a cache user, I get profiled storage fast paths without sacrificing expiry, atomic payload admission or bounded retained value capacity. | S | R79/R81; preserved profiles, capacity/accounting/expiry/OOM and concurrent tests |
| S17 | As a client user, I get bounded borrowing dispatch and specialized RESP headers with unchanged framing, errors and owned-parser behavior. | S | R80/R82; fragmentation, stack/fallback, execute-once and decimal-boundary tests |
| S18 | As a performance investigator, I can assess a bounded fixed-worker wait policy with reproducible frozen-client before/after evidence instead of assuming that busy polling is faster or free. | S | R83–R84; deterministic window/metadata tests, all-mode validation and isolated A/B workflow |
| S19 | As a CPU-limited client, I get guarded cooperative scheduling hints without changing worker counts, deadlines or correctness, and can inspect the exact policy used. | S | R85–R86/R88; policy/permit/run-budget tests and matched CPU/tail evidence |
| S20 | As a fixed-worker client, my ready complete request can execute in the receive pass without exceeding existing work or buffer bounds. | S | R87–R88; same-pass/execute-once/quantum, fragmentation, backpressure and shutdown tests |
| S21 | As a CPU-constrained client, I can share active I/O with other managed client threads without a context switch per request, while quiet sockets return to blocking owners. | S | R89–R93; lease/epoch/quantum/partial-reply tests and the full TCP suite on one CPU |
| S22 | As a performance investigator, I can verify the actual handoff policy and default-mode gains against frozen source and current Valkey without changing the benchmark's mode or CPU budget. | XS | R94; captured policy, constrained CI, rotated native A/B and reference comparisons |
| S23 | As the repository owner, I get PR-only main with mandatory tests and a reproducible Valkey-throughput merge gate, without direct/admin/bot bypasses. | S | R95–R97; active ruleset, frozen-client gate, failure/corruption/cleanup tests and live PR checks |
| S24 | As a benchmark reader, I still get generated reports without exempting automation from the PR-only quality policy. | XS | R98–R100; branch-only report PRs, exact-head check dispatch, mocked publication tests |
| S25 | As a cache user, I can atomically read or set multiple strings so that batch operations preserve ordering, expiry and payload admission. | S | R101–R104; duplicate/binary/expiry/admission tests, bounded MGET reply, all-mode wire/reference tests |
| S26 | As a client, I can queue commands and execute them as one isolated batch, with optimistic invalidation when a watched key changes, so multi-step updates are not interleaved or lost to a race. | S | R105–R108; queue/abort/atomicity, expiry, RESP2/RESP3 and all-mode wire tests |
| S27 | As an operator, I can require a secret before any data or administrative command so that a reachable cache is not open to every client on the network. | S | R109–R112; SHA-256 vectors, constant-time comparison, NOAUTH/HELLO atomicity, rotation/redaction and all-mode wire tests |
| S28 | As an operator, I can define multiple users with independent credentials, command categories and key patterns so that a shared cache is not all-or-nothing access. | S | R113–R116; ACL-file round-trip, atomic rules, live denial, protected default and all-mode wire tests |
| S29 | As an operator, I can enable an fsynced command journal so that supported data survives an ordinary restart within a documented loss window. | S | R117–R120; type/TTL replay, truncation/corruption, transaction/concurrency and all-mode restart tests |
| S30 | As a client, I can iterate large keyspaces and collections in bounded pages instead of blocking the server with KEYS, with filtering by pattern and type. | XS | R121–R124; paging, filter, type, NOVALUES, binary and invalid cursor tests in all modes |
| S31 | As a client, I can publish and receive channel or pattern messages on live connections so that independent clients can coordinate without polling the keyspace. | S | R125–R128; confirmations, delivery, subscriber mode, RESP3 push, ACL channels and all-mode wire tests |
| S32 | As an operator, I can compact an append-only journal without losing acknowledged state so that restart time and disk use do not grow forever. | XS | R129–R131; compacting replay, live-rewrite, disabled-journal and all-mode restart tests |
| S33 | As a pool/client, I can reset a reused connection's server-side state without reconnecting so that stale transactions, subscriptions or protocol state cannot leak. | XS | R132–R133; reset/auth/pubsub/RESP2 and all-mode wire tests |
| S34 | As an operator, I can inspect honest connection, command, expiration and pub/sub counters and reset them, without fabricated Redis fields. | XS | R134–R136; counter scope, reset, rejection, expiry and all-mode wire tests |
| S35 | As a queue user, I can wait for an element to arrive on one of several lists instead of polling, without blocking a worker lane or holding the store lock. | S | R137–R140; push wakeup, timeout, order, journal, idle and all-mode wire tests |
| S36 | As a cache operator, I can choose a bounded eviction policy so that writes keep succeeding within a payload budget instead of always returning OOM. | S | R141–R144; policy, volatile, TTL/LRU, journal, stats and all-mode wire tests |

## EARS acceptance requirements

### Policy and transport

- **R01 (ubiquitous):** The project shall compile with unsafe project code forbidden
  and without any external dependency in any target or feature.
- **R10 (event-driven):** When a complete supported RESP2 command is received, the
  server shall execute it once and return one correctly framed response.
- **R11 (state-driven):** While a command is incomplete, the server shall retain
  its bytes without executing it; when completed, it shall process the command.
- **R12 (event-driven):** When multiple commands arrive in a pipeline, the server
  shall return their responses in command order, without waiting for another read.
- **R13 (event-driven):** When `PING [message]` or `ECHO message` is received with
  valid arity, the server shall return Redis-compatible PONG or binary bulk data.
- **R14 (unwanted behavior):** If framing or configured request limits are invalid,
  then the server shall emit a protocol error and close only that connection.
- **R15 (unwanted behavior):** If a command is unknown or has invalid arguments,
  then the server shall return an error without mutating data or closing the
  connection. Unknown-command errors shall not reflect arbitrary control bytes.
- **R16 (event-driven):** When `QUIT` is received, the server shall flush `+OK`,
  close the connection and not execute subsequent pipelined commands.
- **R17 (unwanted behavior):** If the client capacity is exhausted, then the server
  shall reject additional connections without creating additional workers.
- **R18 (event-driven):** When an embedded server is shut down or its handle is
  dropped, the server shall interrupt client I/O and join every worker.
- **R19 (state-driven):** While a client performs no socket I/O for the configured
  inactivity timeout, the server shall release that connection.

### Data

- **R20 (event-driven):** When plain `SET key value` is received within the payload
  budget, the server shall atomically replace the value and reply `+OK`.
- **R21 (event-driven):** When `GET key` is received, the server shall return the
  stored bytes or a null bulk for a missing key, distinguishing an empty value.
- **R22 (unwanted behavior):** If an admitted SET would exceed the key/value
  payload budget, then the server shall return OOM and preserve the previous
  data. Overwrite and deletion shall reclaim payload budget. This is not an absolute RSS limit.
- **R23 (event-driven):** When `DEL key [key ...]` is received, the server shall
  atomically remove existing keys and count each removed key only once.
- **R24 (event-driven):** When `EXISTS key [key ...]` is received, the server shall
  count each existing argument, including duplicates, as Redis does.
- **R25 (event-driven):** When `SET key value NX` is received, the server shall
  atomically create the key and return `+OK` if absent; otherwise it shall return
  a null bulk and preserve the existing value, including an empty value.
- **R26 (event-driven):** When `SET key value XX` is received, the server shall
  atomically replace the value and return `+OK` if present; otherwise it shall
  return a null bulk without creating the key.
- **R27 (unwanted behavior):** If SET options contain both NX and XX or an
  unsupported token, then the server shall return a syntax error before checking
  conditions or changing data. Complete ASCII NX/XX tokens shall be
  case-insensitive; repeated identical conditions shall be accepted.
- **R28 (event-driven):** When concurrent clients perform NX writes to one absent
  key with sufficient payload budget and no intervening deletion, the server
  shall accept exactly one write and preserve that winner's bytes.
- **R29 (state-driven):** While a SET condition is not satisfied, the server shall
  neither allocate stored payload nor consume its budget; admitted writes shall
  retain R22's atomic OOM behavior and budget reclamation.

### Multi-key strings

- **R101 (event-driven):** When `MGET key [key ...]` is received, the server shall
  return one RESP2 array in argument order, preserving duplicate positions,
  empty/binary values and nulls for missing/expired keys from one consistent store
  observation.
- **R102 (event-driven):** When valid `MSET key value [key value ...]` pairs are
  received, the server shall atomically apply the final value for each distinct
  key, clear TTLs on successful replacement, and return `+OK`; if aggregate final
  payload admission fails, no live value or TTL shall be partially changed.
- **R103 (event-driven):** When valid MSETNX pairs are received, the server shall
  return `:0` without writes if any distinct target is live, otherwise atomically
  apply all final values and return `:1`; duplicate absent keys count once and the
  last value wins.
- **R104 (unwanted behavior):** If a complete MGET RESP reply would exceed 2 MiB,
  the server shall return one bounded error response without emitting a partial
  array; it shall not modify any live stored value. Its reply-size check includes
  RESP framing and duplicate results.

### Transactions and optimistic locking (S26)

- **R105 (event-driven):** When `MULTI` is received outside an active transaction,
  the server shall start queueing connection state and reply `+OK`; a queued
  supported command with valid public arity shall reply `+QUEUED`, and nested
  `MULTI`, `EXEC`/`DISCARD` without `MULTI`, and `WATCH` inside `MULTI` shall
  return documented errors without executing or partially clearing a queue.
- **R106 (event-driven):** When `EXEC` runs after a dirty queue, the server shall
  return `EXECABORT` and execute nothing; otherwise it shall append one RESP array
  containing each queued command's reply in order, with runtime errors as array
  elements and no rollback of already completed commands.
- **R107 (state-driven):** While `EXEC` executes, ordinary commands and background
  reclamation shall not observe or mutate the keyspace between queued commands;
  the queue shall be bounded, owned by the connection, preserve binary arguments,
  survive worker handoffs and be discarded by `DISCARD`/`QUIT`/close.
- **R108 (unwanted behavior):** If a watched key is written, deleted, expires,
  changes TTL, or is cleared after `WATCH`, then `EXEC` shall return a null array
  (RESP2 `*-1`, RESP3 `_`) without changing data; bounded version eviction and
  flushes shall fail closed rather than miss a conflict. `UNWATCH` shall clear all
  watches.

### Password authentication (S27)

- **R109 (state-driven):** While a default-user password is configured, a
  connection shall remain unauthenticated until a valid `AUTH`/`HELLO AUTH`, and
  shall receive `NOAUTH Authentication required.` before data, transaction,
  discovery or administrative work for every other command; only `AUTH`, `HELLO`
  and `QUIT` may execute. With no password configured, data commands shall remain
  available and `AUTH` shall return Redis's explicit no-password error.
- **R110 (event-driven):** When `AUTH [username] password` or
  `HELLO <2|3> AUTH username password [SETNAME name]` is received, the server shall
  accept only the `default` user and a constant-time SHA-256 match, returning
  `+OK`/HELLO information on success or a generic `WRONGPASS` error on failure.
  A failed HELLO shall leave protocol, metadata and authentication state unchanged.
- **R111 (event-driven):** When authenticated `CONFIG SET requirepass <value>` is
  received, the server shall atomically set a new digest or disable authentication
  for an empty value, leaving thresholds and credential unchanged if any pair is
  invalid; `CONFIG GET requirepass` shall report unset/redacted state without
  exposing the secret. Startup flag/file parsing shall fail closed on empty or
  oversized files and never log the password.
- **R112 (ubiquitous):** Authentication tests shall run offline in all execution
  modes, cover pipelined rejection/order, runtime rotation, `QUIT`, RESP2/RESP3
  HELLO, secret redaction and disabled/rotated credentials. Documentation shall
  retain the missing brute-force, audit, native-TLS, durability and
  live-deauthentication boundaries; a single password is not full Redis security.

### Multi-user ACLs (S28)

- **R113 (event-driven):** When `--aclfile` is configured, the server shall load
  users/rules before accepting clients, require an enabled `default` user, reject
  malformed or missing files at startup, reject combining `--aclfile` with
  `--requirepass`, and preserve the single-password `--requirepass` mode otherwise.
- **R114 (state-driven):** While a connection is authenticated as a user, the
  server shall evaluate ordered command/category rules and declared key positions
  before dispatch/store access and deny with `NOPERM`; disabled/deleted users shall
  fail with `NOAUTH` on the next command. `AUTH`, `HELLO`, `QUIT` and `ACL WHOAMI`
  shall retain their documented exceptions. Denied commands shall not mutate data
  and shall mark an active transaction dirty; `EXEC` shall re-check every queued
  command so later revocation cannot commit it.
- **R115 (event-driven):** When `ACL SETUSER`, `DELUSER`, `WHOAMI`, `USERS`, `LIST`,
  `GETUSER`, `CAT`, `SAVE` or `LOAD` is received with valid permission, the server
  shall apply Redis-shaped user rules atomically, protect `default` from deletion,
  expose hashes instead of plaintext, and save/load a complete ACL file through a
  mode-0600, fsynced temporary-file rename within documented file/user/rule/pattern
  bounds. Invalid modifiers shall not partially change a user.
- **R116 (ubiquitous):** ACL tests shall run offline in all execution modes and cover
  password/category/command/key rules, live revocation, transaction dirtiness,
  startup failures, file hash/rule round-trips and secret-free diagnostics.
  Documentation shall retain the missing selectors, audit/log/dry-run/gen-pass,
  channel enforcement, rate limiting, native TLS, key-spec-blind commands and
  process-level tenant-isolation boundaries.

### Append-only persistence (S29)

- **R117 (event-driven):** When `--appendonly yes` is supplied, the server shall
  open/create the configured journal with mode 0600, replay it into a fresh store
  before accepting clients, truncate only an incomplete final command, and fail
  startup visibly for malformed mid-file data, unsupported/failed commands or
  configured limit violations. Defaults shall remain persistence-off.
- **R118 (event-driven):** When a supported command mutates keys, the journal shall
  append a replay-safe `DEL` plus final-state rebuild and absolute `PEXPIREAT`
  records for each changed key in a deterministic order; non-mutating/rejected
  commands shall append nothing, and a `MULTI` batch shall be captured once after
  the whole batch. Content/timestamp overflow shall force `FLUSHALL` plus a full
  snapshot rather than a missed key.
- **R119 (state-driven):** While configured, `always` shall fsync before replying,
  `everysec` shall bound expected loss to the maintenance interval, and `no` shall
  only write without an explicit sync. Append/sync failure shall replace the reply
  with a clear error and latch the journal; orderly shutdown shall sync and flush.
  The format shall be RESP commands, not an RDB or byte-compatible multipart-AOF claim.
- **R120 (ubiquitous):** Persistence tests shall run offline in all execution modes
  and cover strings/binary values, all supported collections, streams/groups, TTLs,
  transactions, concurrent writers, flushes, partial tails, mid-file corruption and
  restart. Documentation shall retain missing RDB/AOF-rewrite, compaction,
  checksums, encryption, multi-process ownership, backup drills, signal drain,
  stream-pending durability and disk-fault/soak guarantees.

### Cursor iteration (S30)

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

### Publish/subscribe (S31)

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

### Journal compaction (S32)

- **R129 (event-driven):** When `BGREWRITEAOF` is received on an enabled journal,
  the server shall encode a complete current-state snapshot (`FLUSHALL` plus every
  live key with absolute TTLs), write a mode-0600 temporary file in the journal
  directory, fsync it, atomically rename it over the journal and reopen it.
- **R130 (state-driven):** While the rewrite runs, the server shall hold the
  exclusive transaction gate so no mutation can be lost or appended to the replaced
  file; command service may pause for the synchronous rewrite. A failed write/sync/
  rename shall retain the previous journal and return a visible error.
- **R131 (unwanted behavior):** If the journal is disabled or the rewrite cannot be
  completed, then the command shall return a bounded error without claiming
  compaction. Tests shall cover a compacted live log, replay after rewrite, disabled
  journals, failure atomicity and all execution modes. Documentation shall not call
  this background, incremental or automatically scheduled.

### Connection reset (S33)

- **R132 (event-driven):** When a valid `RESET` is received, the server shall
  discard the active transaction, clear watches, remove all channel/pattern
  subscriptions, return to RESP2, select database zero, clear the authenticated
  user and reply `+RESET`; it shall be available without prior authentication and
  from RESP2 subscriber mode. Client name/library metadata shall be retained.
- **R133 (unwanted behavior):** If `RESET` has extra arguments, it shall return the
  normal arity error without changing state. Tests shall cover queued transactions,
  watches, pub/sub cleanup, RESP3-to-RESP2 conversion, deauthentication and
  reauthentication, subscriber-mode reset and all execution modes.

### Process statistics (S34)

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

### Blocking list pops (S35)

- **R137 (event-driven):** When `BLPOP`/`BRPOP key [key ...] timeout` finds a live
  nonempty list, it shall pop one left/right element in key order and reply a
  two-element `[key, value]` array; a wrong-type key shall return WRONGTYPE and a
  missing/empty set of keys shall return the null array at expiry.
- **R138 (state-driven):** While waiting, the connection shall hold no store lock,
  apply no ordinary idle disconnect, parse no further commands from its bounded
  input, and remain joined to its worker; polling shall be bounded (about 10 ms in
  default mode), not a push/wakeup or latency guarantee. Disconnect/shutdown shall
  drop the blocked state.
- **R139 (unwanted behavior):** If the timeout is negative, NaN, infinite or out of
  range, or arity is invalid, then the command shall return a bounded error before
  mutating or blocking. ACL key patterns shall be checked before blocking, blocked
  pops shall use the normal journal gate and be recoverable after restart, and
  blocking commands shall be rejected inside `MULTI` with dirty-transaction
  semantics (documented divergence).
- **R140 (ubiquitous):** All-mode wire tests shall cover waiting for a push,
  multi-key/left/right order, timeout nulls, RESP3 conversion, pipelined commands
  after unblock, idle-timeout immunity, wrong-type/float errors and shutdown.
  Documentation shall retain the absence of a condition-variable/push design,
  strict latency SLOs, `BRPOPLPUSH`/`BLMOVE`, fairness under many blocked clients
  and cluster/replication blocking semantics.

### Payload budget eviction (S36)

- **R141 (event-driven):** When a write's final payload exceeds the configured
  budget, the server shall first reap logically expired keys, then apply the
  selected policy. `noeviction` shall preserve the existing atomic OOM behavior;
  other policies shall evict eligible non-target keys until admission succeeds.
- **R142 (unwanted behavior):** If the selected policy cannot evict enough keys
  (for example `volatile-*` with no TTL keys), then the server shall return OOM
  without deleting any key. It shall never evict the key being written, and an
  admitted write shall apply its complete final payload atomically.
- **R143 (state-driven):** While `allkeys-random`/`volatile-random` run, candidate
  choice shall be bounded and rotating; `*-lru` shall use documented mutation
  recency rather than claiming read LRU; `volatile-ttl` shall choose the nearest
  deadline. Evictions shall update expiry/accounting/WATCH versions and be captured
  by an enabled append-only journal as normal key deletions.
- **R144 (event-driven):** `CONFIG GET/SET maxmemory-policy` and
  `--maxmemory-policy` shall expose the policies; `INFO memory` shall report the
  live policy and `INFO stats` the evicted count, reset by `CONFIG RESETSTAT`.
  All-mode tests shall cover noeviction, allkeys/volatile behavior, persistent-key
  protection, TTL/LRU selection, journal replay and stats. Documentation shall
  retain the payload-not-RSS, mutation-LRU, O(N) scan, no-LFU/background
  eviction/notification and replication gaps.

### Development and delivery

- **R30 (event-driven):** When `make check` is run, the project shall check formatting,
  lint every target, enforce the dependency policy and execute its tests offline.
- **R31 (event-driven):** When a pull request or main push is checked on GitHub, CI
  shall run the same `make ci` command as local development, including a release
  benchmark smoke test that validates actual replies.
- **R32 (event-driven):** When main passes CI, the workflow shall make a Linux
  release archive, checksum and benchmark evidence available as downloadable
  artifacts, without automatically deploying an unauthenticated experimental
  server. The separate S15 container workflow may publish tested images to GHCR;
  publishing an image is not deployment or a production-readiness claim.

### Performance evidence

- **R40 (event-driven):** When a benchmark is run, the tool shall validate every
  measured response and fail on incorrect responses, I/O failure or worker failure.
- **R41 (event-driven):** When a result is reported, it shall identify workload,
  request count, clients, pipeline, keyspace, value size, warmup, run number,
  elapsed time, throughput and batch-latency percentiles in machine-readable JSON.
- **R42 (event-driven):** When GET is benchmarked, the tool shall seed and verify
  data before timing; setup, connection creation and warmup shall be excluded.
- **R43 (ubiquitous):** Comparative results shall use the same client and workload
  parameters for all servers and record source/compiler/hardware and competitor
  versions. Shared-host CI timings shall not be presented as stable-hardware
  guarantees. S23 explicitly applies a narrow hard PR merge threshold on shared
  CI; noise may block a PR and does not justify relaxing that threshold.
- **R44 (event-driven):** When unchanged reference sources are reused across
  profiles, the comparison workflow shall verify unchanged executable digests.
  Source-cache bookkeeping shall not discover the enclosing project's Git history
  or turn generated build output into source-version changes.
- **R45 (event-driven):** When CPU budgets are configured, the comparison shall
  apply them to every server and the benchmark client and record requested and
  actual affinities. CI shall include equal single-core server budgets with
  clients on separate CPUs, alongside default-threading measurements.
- **R46 (event-driven):** When a comparison is rerun, the harness shall discard
  previous result files before starting; if the run fails, it shall not leave
  another server's stale JSON presented as that run's evidence.

- **R47 (optional feature):** Where Linux server profiling is explicitly enabled
  for a separate loopback server PID, the tool shall report process user/system
  CPU tick deltas, the kernel-provided clock frequency, observation duration,
  CPU percentage relative to one logical CPU, RSS endpoint samples and lifetime
  peak RSS, without FFI, external commands or additional Rust dependencies.
- **R48 (event-driven):** When context switches are sampled, the tool shall match
  per-thread counters by TID and thread start time, report their deltas and coverage,
  and label endpoint thread-set changes or sampling races. It shall not call the
  leader's counters process totals or imply recovery of exited-thread counters.
- **R49 (event-driven):** When a timed workload starts, every client shall have
  completed a validated untimed PING and its configured warmup. Profiling reads
  shall occur outside the request timer, with sockets retained through the final
  snapshot and disconnected before post-run data verification.
- **R50 (unwanted behavior):** If the profile target exits, its PID/start-time
  identity changes, counters decrease, or required accounting is unavailable or
  malformed, then the invocation shall fail without emitting result JSON.
  Disappearing nonleader tasks shall instead be explicitly marked as partial
  endpoint coverage; stable endpoints cannot prove absence of intermediate churn.
- **R51 (state-driven):** While one comparison server is measured, the harness
  shall not leave other comparison servers running. Optional idle sampling shall
  precede benchmark connections and report its actual observation duration.

- **R52 (event-driven):** When a comparative benchmark finishes, reporting shall
  validate workload counts, matching parameters, timers, percentiles, CPU/thread
  accounting, source identity, affinities and recorded executable digests before
  presenting comparative tables. Markdown shall identify source/run, data quality,
  medians, spread, batch p99, CPU, RSS and context-switch scope.
- **R53 (optional feature):** Where Copilot credentials and a complete successful
  cohort are available, the reviewer shall prefer `gpt-6-luna` through pinned
  GitHub Copilot tooling with bounded time/credits and no model tools, repository
  instructions or MCP access. AI interpretation shall be visibly separate from
  deterministic measurements; any available-model fallback shall be disclosed
  and attributed using actual Copilot usage, not labelled as Luna.
- **R54 (unwanted behavior):** If measurements are incomplete/invalid or Copilot
  authentication, model access, inference, budget or output fails, then reporting
  shall clearly identify the missing assessment and preserve available validated
  numeric profiles. It shall not reuse a previous narrative as a new review or
  turn benchmark failure into a performance victory.
- **R55 (event-driven):** When a default-branch report is published, automation
  shall retain an immutable per-run/review-attempt report and update the README-
  linked dashboard only for a newer workflow run/attempt. Older reruns shall not
  clobber newer results. Publication shall stage only report paths, never force-
  push source changes, and retain a link to the last successful numeric report.
- **R56 (ubiquitous):** Reporting tests shall run offline without credentials or
  external Rust dependencies. Benchmarking/inference jobs shall not have repository
  write permission; a separate default-branch publication job alone shall receive
  the narrowly scoped permission needed to push generated Markdown.

- **R57 (event-driven):** When a complete successful default-branch comparison
  reaches review, the job shall authenticate with its built-in short-lived
  `GITHUB_TOKEN` and `copilot-requests: write`, without requiring a stored personal
  token or granting inference repository-write access. Only trusted default-branch
  reviews shall receive that token in the model process environment.
- **R58 (unwanted behavior):** If the pinned CLI rejects the preferred model
  before inference with an absent or verified zero-usage file, then the reviewer shall permit
  one automatic available-model selection within the original deadline/credit
  bound. It shall accept only completed Copilot-provider accounting with concrete
  models and generated tokens, record those models and the selection/auth method,
  and never retry authentication, network or charged-inference failures by routing
  them to another model.
- **R59 (event-driven):** When an eligible default-branch comparison has no accepted
  AI assessment, the workflow shall fail its AI-completion gate while preserving
  and publishing validated numeric reporting. Publication retries shall preserve
  immutable legacy manifest bytes rather than rewriting old archives with new fields.

### Explicit multithreaded execution

- **R60 (optional feature):** Where `--threads N` is supplied for N in 1..64,
  the server shall create exactly N managed I/O/execution workers, independent of
  client count. Without the option it shall retain managed thread-per-client I/O.
  S21 may lend busy-client I/O between those existing per-client workers; it
  does not replace their admission/thread-count contract or add a permanent lane.
  The accept/control thread is additional; one worker is not one OS thread total.
- **R61 (state-driven):** While clients are idle, slow readers or submitting large
  pipelines, fixed workers shall retain bounded buffers, fair command/I/O quanta,
  connection limits and independent service to other clients, without unsafe code.
- **R62 (event-driven):** When a server stops or a worker fails, it shall close
  client sockets and join every started worker, including partial-start cleanup.
- **R63 (event-driven):** When `make test` or `make ci` runs locally or on GitHub,
  socket correctness shall run in thread-per-client, one-worker and four-worker
  configurations, with concurrent clients and reply-validated mode benchmark smoke.
- **R64 (event-driven):** When embedded thread modes are benchmarked, results shall
  record the selected mode/worker count. Fixed worker options shall be rejected
  for external targets; mode and affinity shall not be conflated or used to claim
  linear scaling. Optional full comparisons shall preserve mode-scaling evidence;
  reports shall validate and display it separately from native competitor rows,
  reject corrupt/mismatched mode groups and retain historical cohort compatibility.

### Expiration and integer compatibility

- **R65 (event-driven):** When SET receives EX/PX/EXAT/PXAT, GET or KEEPTTL with
  valid nonconflicting options, it shall apply the selected condition/value/TTL
  atomically and return Redis-compatible old-value replies. Identical expiry kinds
  may repeat with the final value winning; all syntax and final numeric/overflow
  validation shall precede condition evaluation and mutation.
- **R66 (state-driven):** While an entry has a deadline, reads, conditions and
  multi-key counts shall use Redis's millisecond visibility boundary; observed
  expiration shall not resurrect after a clock rollback. Expiration shall reclaim
  payload, maintain at most one indexed expiry per key and never cause false OOM.
  Injected-clock tests shall not depend on wall-clock sleeps.
- **R67 (event-driven):** When TTL/PTTL/EXPIRETIME/PEXPIRETIME, EXPIRE/PEXPIRE/
  EXPIREAT/PEXPIREAT with NX/XX/GT/LT, or PERSIST is used, the store shall preserve
  missing/persistent sentinels, rounding, option compatibility and failure atomicity.
- **R68 (event-driven):** When SETEX/PSETEX/SETNX/GETSET/GETDEL/GETEX is used, the
  store shall implement value/expiry and reply semantics, including past absolute
  deadlines, GETEX PERSIST and rejected/OOM writes retaining old state.
- **R69 (event-driven):** When INCR/DECR/INCRBY/DECRBY receives a canonical signed
  64-bit integer string (missing means zero), it shall atomically calculate and
  return the new integer and retain an existing TTL. Concurrent updates shall not
  lose or duplicate values.
- **R70 (unwanted behavior):** If integer parsing, arithmetic, DECRBY negation or
  payload admission fails, then the command shall return the corresponding error
  without changing the live value or TTL, in debug and release builds.
- **R71 (event-driven):** When optional native conformance runs, the new command
  common corpus shall match pinned Redis and Valkey wire replies in per-client,
  one-worker and four-worker Rillcache modes before comparative timing begins.
  Deliberate vendor divergences shall have explicit per-vendor expectations;
  Rillcache shall follow the documented Redis behavior.

### Thin container delivery (S14–S15)

- **R72 (ubiquitous):** The Linux runtime image shall use scratch, contain a
  statically linked safe-std-only Rillcache binary and runtime license notices,
  run as numeric non-root UID/GID 65532, and stay below 8 MiB uncompressed.
  Compilers, shells, package managers and benchmark/review executables shall not ship.
- **R73 (event-driven):** When the image is run without arguments, it shall listen
  on container TCP port 6379 on all IPv4 interfaces. Appended options (including
  worker counts and an overriding bind address) shall work without a wrapper script.
  The standalone binary/library shall retain loopback defaults.
- **R74 (event-driven):** When `--healthcheck IP:port` runs, the same binary shall
  send a read-only RESP2 PING, require an exact PONG, remain quiet on success and
  fail on rejection, malformed/truncated replies, connection errors or a bounded
  one-second I/O deadline. No external runtime probe tool shall be required.
- **R75 (event-driven):** When trusted main/tag/manual publication runs, automation
  shall first pass offline checks and native amd64/arm64 container tests in all
  three worker modes, then publish only those tested images as one GHCR manifest
  with source tags/labels. PRs shall test but receive no publishing credential.
  Only a separate trusted publishing job shall receive `packages: write`.
- **R76 (unwanted behavior):** If an architecture/test fails, an image is too large
  or dynamically linked, or a version tag disagrees with Cargo, then publication
  shall fail. A manifest shall not silently omit a required architecture.
- **R77 (event-driven):** After publication, a separate credential-free check
  shall verify anonymous registry access and run the published image. If GitHub's
  first-package private visibility blocks this, then the job shall fail with the
  explicit one-time public-visibility action, not claim public availability.
- **R78 (ubiquitous):** Docker/Kubernetes examples shall use restricted networking,
  non-root/read-only execution and bounded resources; keep data-loss, payload-not-
  RSS, missing authentication/TLS and non-graceful process-termination limitations
  explicit. A single replica/Service shall not imply persistence, replication or HA.

### Profile-guided hot-path delivery acceptance (S16–S18)

- **R79 (event-driven):** Before changing a measured hot path, profiling shall
  identify workload-specific costs using preserved baseline binaries/source/flags.
  Sampled user-space cycles, syscall counts and uninstrumented elapsed/CPU results
  shall remain separate; profiler-distorted throughput shall not become a speed claim.
- **R80 (event-driven):** When the server dispatches a complete RESP request with
  at most eight arguments, it shall borrow stack-held argument references without
  allocating an argument vector. Larger commands retain bounded fallback storage.
  The public owned Request API, framing/length errors, fragmentation, one-command
  consumption, ordered pipelines and execute-once behavior shall remain compatible.
- **R81 (state-driven):** While reads/writes reuse already-validated lookup/admission
  metadata or value capacity, the store shall retain randomized hashing, exclusive
  atomic writes, exact payload/expiry accounting, TTL and failed-write semantics.
  Disproportionate shrink/empty overwrites shall release retained value capacity.
- **R82 (event-driven):** When RESP array/bulk headers are encoded, specialized
  decimal encoding shall produce byte-identical results for zero, digit boundaries
  and usize::MAX without heap formatting or weakening overflow checks.
- **R83 (optional feature):** Where bounded active polling is adopted for fixed
  workers, its active interval shall expire after real progress stops, retain idle
  parking, unchanged fairness/backpressure/admission and joined shutdown, and expose
  its actual policy in benchmark evidence. Busy polling shall not be called epoll
  readiness or a free/idle-CPU improvement.
- **R84 (event-driven):** When an optimization is assessed, repeated before/after
  measurements shall use the same frozen client, workloads, CPU budgets and runtime
  settings, rotate order, preserve digests/raw evidence and disclose spread/CPU/tails.
  Relevant debug/release/reference and all-mode tests shall pass before publication.
  Slower/noisy results and remaining blockers shall remain visible; no all-workload
  performance victory or production-compatibility claim shall follow from a subset.

### Profile-guided scheduling and ready I/O (S19–S20)

- **R85 (optional feature):** Where a bind-time single-CPU capacity hint and
  multiple admitted clients make short-reply scheduling eligible, blocking
  workers may yield once after a single-command flush, without yielding after
  batched replies, holding a store lock, removing socket deadlines or changing
  default thread-per-client admission/shutdown behavior. Scheduling hints shall
  not be relied on for correctness or advertised as OS fairness guarantees.
- **R86 (state-driven):** While multiple fixed lanes and multiple clients remain
  active, real progress shall not indefinitely rearm a separate cooperative run
  budget. Its expiry shall request a yield between bounded passes, retain the
  post-progress poll window and idle parking, and reset after explicit yielding
  or parking returns, without charging that wait to its next interval. A single
  lane or a lone client shall not incur periodic fairness yields.
- **R87 (event-driven):** When a fixed worker receives a complete command into an
  empty input buffer, it shall be able to dispatch it in that same bounded pass.
  Each pass shall retain the read, command, reply/backpressure and frame limits;
  partial/malformed frames, EOF and QUIT shall preserve execute-once/order rules.
- **R88 (event-driven):** When scheduling policies are benchmarked, embedded
  results shall record the actual bind-time policy used by the server, not a
  later recomputed CPU estimate. New run-budget evidence shall be required and
  validated independently of historical poll-only cohorts. Before adoption,
  frozen-client native comparisons shall include CPU, tails, singleton/batched
  controls and oversubscribed workers; rejected/noisy experiments stay explicit.

### Single-CPU client I/O handoffs (S21–S22)

- **R89 (optional feature):** Where the bind-time capacity hint is one CPU,
  managed per-client workers may lend I/O only after actual reply progress and
  while multiple clients are admitted. A new epoch shall require recent reply
  activity within its window; isolated sparse replies shall not allocate a
  lease or activate post-reply polling. An active epoch may accept new loans.
  Fixed modes, multicore/unknown hints and an initially lone client shall retain
  their existing policy. Quiet borrowers shall return independently while busy
  peers continue; the coordinator shall keep its own socket serviced. Quiet
  epochs shall return sockets to blocking owners rather than permanently polling.
- **R90 (state-driven):** While a client's socket is lent, its original worker
  shall wait and perform no I/O through its handle. Exactly one lease shall own
  its buffers/active I/O; returning it shall restore the shared socket's blocking
  option before the original loop resumes. No store lock shall span socket I/O.
- **R91 (event-driven):** When a lease returns buffered input or a partly sent
  response, the original loop shall parse ready bytes before reading again and
  send only the unsent reply suffix. Fragments, errors, EOF and QUIT shall retain
  ordered, execute-once behavior and existing deadlines/frame limits.
- **R92 (unwanted behavior):** If an epoch closes while a sender is in flight,
  a send fails, leadership unwinds or shutdown starts, then each active/queued
  lease shall return ownership exactly once or close before waking its owner.
  There shall be no stranded waiting worker, detached thread or stale epoch.
  Continuous progress shall not extend the separate 100 ms maximum epoch,
  checked between bounded passes, or indefinitely retain a closed coordinator's
  admission slot. Returning an epoch shall preserve ready input and reply offsets.
- **R93 (ubiquitous):** Handoffs shall retain aggregate admission, one outstanding
  lease per admitted client, bounded input/output, 64-command and bounded I/O
  quanta, atomic shared storage, backpressure and joined library shutdown. They
  shall add no permanent worker, unsafe code or Rust dependency. At most one
  additional socket handle per loan and bounded channel metadata are permitted;
  this resource/active-poll tradeoff is not readiness notification or free CPU.
- **R94 (event-driven):** When handoffs are measured, embedded JSON shall record
  the actual captured idle window, maximum epoch and policy version. Linux
  local/ordinary CI shall
  exercise the complete default TCP suite on one allowed CPU when taskset is
  available, alongside all existing modes. Frozen-client native comparisons
  shall retain baseline/default modes, CPU budgets, singleton/batched controls,
  process CPU, whole-batch tails and regressions; no all-traffic victory follows.

### PR-only main and scoped Valkey merge acceptance (S23–S24)

- **R95 (ubiquitous):** Every future change to main shall come through an up-to-date
  PR with passing distinct Tests and offline CI and Valkey performance gate checks
  from GitHub Actions. Active rules shall have no bypass actors, block deletion/
  force pushes, require squash merges and resolve review threads. CI/policy files
  shall require code-owner review; ordinary implementation PRs need no blanket
  approving-review quota.
- **R96 (event-driven):** When a PR benchmark runs, it shall use the protected base
  harness, frozen Rust client/compiler and checksum-pinned Valkey, default server
  modes, three alternating native repetitions, both CPU budgets and both pipeline
  lengths across five workloads. The exact [20-condition policy](pr-quality-gate.md)
  shall retain source/binary identity, affinities, reply validation and owned-process
  readiness/cleanup. Only the installing PR may bootstrap against the exact pinned
  pre-policy source; a missing future base harness shall fail closed.
- **R97 (unwanted behavior):** If any median is tied/slower than Valkey or evidence
  is missing, invalid, duplicated, nonfinite, mismatched, changed or incomplete,
  then the required performance job shall fail. It shall revalidate all 120 raw
  records and hashes, retain every row/spread/CPU/whole-batch tail and upload failures.
  No path exemption, historical pass, fastest-sample substitution, automatic retry
  until green or threshold relaxation shall permit merge. Passing is scoped
  throughput acceptance on shared CI, not statistical certainty or production readiness.
- **R98 (event-driven):** When a generated comparison report is proposed for main,
  publication shall validate immutable identity/order, commit only generated report
  paths on a non-main branch and open/reuse a PR, never push main or force-update it.
- **R99 (state-driven):** While an automated report PR awaits merging, the same
  mandatory test/Valkey checks shall apply to its exact current up-to-date head.
  Only trusted publication may receive scoped contents/PR/action write permissions
  to open the PR and dispatch read-only checks, without a new stored credential,
  privileged PR execution or inference access in the benchmark process.
- **R100 (unwanted behavior):** If publication is unchanged, retried, has invalid
  context or check dispatch fails, then it shall respectively no-op, reuse its
  existing PR, reject before Git/API work or fail visibly, without hidden main
  commits, overwritten immutable history or a false publication success. Reports
  remain readable in Actions while their PR waits for the protected merge path.

## Compatibility boundary (v0.1)

Supported: RESP2 nonempty arrays of non-null bulk strings and Redis-style inline
commands with whitespace, quotes and escapes, case-insensitive ASCII command names,
binary keys/values (including escaped inline `\xHH` bytes), multiple connections
and ordered mixed-protocol pipelines; `PING`, `ECHO`, `SET` with NX/XX/GET/KEEPTTL/
EX/PX/EXAT/PXAT and IFEQ (plus Redis IFNE/IFDEQ/IFDNE); `GET`, `DEL`, `EXISTS`,
`QUIT`; SETEX/PSETEX/SETNX/GETSET/GETDEL/GETEX; TTL/PTTL/EXPIRETIME/
PEXPIRETIME, EXPIRE/PEXPIRE/EXPIREAT/PEXPIREAT with NX/XX/GT/LT, PERSIST;
INCR/DECR/INCRBY/DECRBY/INCRBYFLOAT; MGET/MSET/MSETNX/MSETEX;
MULTI/EXEC/DISCARD/WATCH/UNWATCH with a bounded queue, atomic batch execution
and optimistic key invalidation; multi-user ACL users/rules, key patterns and
atomic ACL-file save/load; opt-in RESP-command append-only persistence with
fsync policies, restart recovery and synchronous `BGREWRITEAOF` compaction;
payload-budget eviction policies (`noeviction`, allkeys/volatile random/LRU and
volatile TTL); Redis DIGEST/DELEX
and Valkey DELIFEQ; APPEND/STRLEN/GETRANGE/SETRANGE/SUBSTR; SETBIT/GETBIT; LCS
(LEN/IDX/WITHMATCHLEN/MINMATCHLEN); LPUSH/RPUSH/LPOP/RPOP/LLEN/LRANGE/BLPOP/BRPOP;
SADD/SMEMBERS/SCARD/SISMEMBER; HSET/HGET/HGETALL/HLEN; ZADD/ZRANGE/ZSCORE/ZCARD;
SCAN/HSCAN/SSCAN/ZSCAN bounded cursor iteration with MATCH/COUNT/TYPE/NOVALUES;
PUBLISH/SUBSCRIBE/PSUBSCRIBE/UNSUBSCRIBE/PUNSUBSCRIBE and PUBSUB introspection.
See [string semantics](strings.md), [string ranges](string-ranges.md),
[bitmap/LCS/float limits](string-algorithms.md), [multi-key string limits](multikey-strings.md),
[typed collections](collections.md), [transactions](transactions.md),
[security](security.md), [persistence](persistence.md) and [pub/sub](pubsub.md)
for grammar,
clocks, rounding, integer limits, resource bounds, queue isolation and atomic
error behavior. Conditions are atomic and
case-insensitive. Option tokens must match completely; NULs/prefixes are rejected.
A bounded stream subset supports XADD, XDEL, XLEN, XGROUP CREATE/SETID,
XREADGROUP GROUP with COUNT/NOACK for new entries on one stream, XINFO STREAM
(with FULL). See [stream limits](streams.md). All types share expiry, payload
accounting, TYPE, RENAME and deep same-database COPY; byte-oriented KEYS and
RANDOMKEY inspect that same keyspace. See [keyspace limits](keyspace.md).
CONFIG GET/SET collection thresholds select actual collection representations;
other runtime settings remain unsupported. FLUSHDB/FLUSHALL clear all types,
CONFIG RESETSTAT retains its empty-statistics behavior, and MEMORY USAGE reports
a deterministic logical estimate, not allocator/RSS measurements.

The default wire mode is RESP2. `HELLO` can inspect the current mode or
negotiate RESP2/RESP3, and the connection retains its negotiated mode and common
`CLIENT` metadata across worker handoffs. Supported session commands are `CLIENT
ID`, `SETNAME`, `GETNAME`, `SETINFO LIB-NAME|LIB-VER`, `INFO`, `SELECT 0` and
`RESET`.
The RESP3 reply subset covers the current commands' simple strings, errors,
integers, bulk strings/arrays, and nulls; null bulk and null array frames are
converted by frame boundaries, preserving binary payloads. See
[client sessions](client-sessions.md) for limits and response details. Real
redis-cli/library handshake interoperability is not yet verified.

Deliberately not supported: empty/null request arrays, other RESP3 types and
pushes, commands beyond the documented collection/stream subsets, nonzero SELECT
indices, native TLS, RDB/AOF-rewrite compatibility, replication, clustering,
sharded pub/sub, keyspace notifications and scripts. Cluster mode,
multi-database, and arbitrary Redis AOF/RDB files are not accepted.
Single-password and multi-user ACL-file authentication are supported with
`AUTH`, `HELLO ... AUTH`, `ACL` and `CONFIG SET requirepass`; ACL selectors, audit
logging, dry-run, generated passwords and channel enforcement remain absent. See
[security](security.md).
Connection-local commands are rejected rather than queued inside MULTI. Unsupported SET options return
`ERR syntax error` and do not write. Redis normally ignores empty request arrays;
this strict initial subset rejects them. Unknown-command and unsupported-expire-
option errors omit arbitrary input bytes rather than reflecting them into RESP
error frames. Conservative defaults remain smaller than Redis limits.

Default limits: 128 clients, 30-second socket inactivity timeout, 1 MiB bulk,
1..=1024-byte password (via flag or file; ACL files add hashed users),
2 MiB complete request frame, 1024 arguments, 64 MiB key/value payload and a 2 MiB
maximum MGET RESP reply. CLI `--max-bulk-bytes` and `--max-frame-bytes` may raise
request bounds up to 512 MiB (bulk must fit within frame); larger requests remain
finite and grow buffers only as bytes arrive. Compatibility-only upstream runs
match native and candidate at 8 MiB bulk and 16 MiB frame; MGET remains capped at
2 MiB. Over-limit replies return `ERR MGET reply limit exceeded`
without a partial array. Payload accounting excludes map capacity, allocator and
connection/thread buffers.
The CLI uses loopback by default. Do not expose it to untrusted clients.

## Next independently deliverable stories

Not implemented or implied by the completed increments:

1. Validate HELLO/CLIENT session negotiation against real pinned redis-cli and
   language clients; add command discovery and multiple databases only as separate
   compatible increments.
2. ACL selectors and channel enforcement (with pub/sub), brute-force controls, an
   audit path and a safe transport-security deployment boundary; then AOF
   rewrite/compaction, RDB interoperability, backup/restore drills and disk-fault
   recovery beyond the current append-only restart path.
3. Richer data types, scripts, pub/sub, expanded streams, replication and
   clustering: follow the [production-readiness plan](production-readiness.md).
   NX plus TTL alone is not a complete distributed-lock protocol; token-safe
   release, failover and fencing semantics are not implemented.
4. Extend the [measured client-I/O gains](../benchmarks/studies/client-handoffs/README.md)
   beyond the current cohort: investigate idle accept polling, explicit fixed-worker
   and multicore tails, mixed TTL/traffic and client/server saturation, using PING
   as a no-key-map control. Evaluate
   a safe readiness boundary before speculative sharding; retain idle CPU,
   backpressure, shutdown and multi-key atomicity gates. Unsafe project code and
   external Rust dependencies remain forbidden.
5. Controlled-hardware performance budgets and repeated current-release comparisons;
   later, richer types, pub/sub and additional stream capabilities driven by product judgment.

Definition of done for each future story: EARS acceptance cases and limits agreed,
unit and wire tests added, dependency/safety policy intact, `make ci` passing,
relevant benchmark captured, compatibility documentation updated, human validation.

## Sources

- [INVEST](https://agilealliance.org/glossary/invest/)
- [EARS patterns](https://alistairmavin.com/ears/)
- [RESP specification](https://redis.io/docs/latest/develop/reference/protocol-spec/)
- [Redis command reference](https://redis.io/docs/latest/commands/)
- [Safe/unsafe boundary](https://doc.rust-lang.org/nomicon/safe-unsafe-meaning.html)
- [Cargo lint policy](https://doc.rust-lang.org/cargo/reference/manifest.html#the-lints-section)
