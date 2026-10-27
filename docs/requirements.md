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
| S37 | As a cache user, I can mutate, count, remove and range over hashes, lists, sets and sorted sets so that typed collections are usable beyond creation and basic reads. | S | R145–R148; binary/count/order/TTL/OOM/WRONGTYPE, multi-key set atomicity and all-mode wire tests |
| S38 | As an operator, I can list live connections with honest metadata and kill selected clients by ID/address/type/user so that stuck or abusive sessions can be managed. | XS | R149–R152; list metadata/filters, kill variants, self-protection and all-mode wire tests |
| S39 | As a cache user, I can add elements to approximate cardinality sketches, estimate the union of several sketches and merge them, without storing every member. | S | R153–R156; exact small counts, dense tolerance, TTL/merge/error atomicity and all-mode wire tests |
| S40 | As a service, I can subscribe to keyspace/keyevent notifications so that cache invalidation and audit tooling do not need to poll the keyspace. | S | R157–R160; flag/category frames, MULTI/blocking, runtime toggle, ACL and all-mode wire tests |
| S41 | As an administrator, I can generate secure ACL passwords, dry-run a user's permissions and inspect a bounded denial log so that credential operations and policy debugging are explicit. | XS | R161–R164; GENPASS source/bounds, DRYRUN, LOG aggregation/reset and all-mode wire tests |
| S42 | As a bitmap user, I can count, locate and combine bits over binary strings with byte/bit ranges and atomic destinations. | S | R165–R168; range/padding/alias/TTL/OOM/WRONGTYPE and all-mode wire tests |
| S43 | As a stream consumer, I can range-read entries, inspect and acknowledge pending deliveries, and read multiple streams without blocking so that consumer groups are usable beyond one read. | S | R169–R172; ack/pending/range/read/group lifecycle and all-mode wire tests |
| S44 | As a bitmap user, I can read and atomically update signed or unsigned bit fields of arbitrary width with wrap, saturate or fail overflow behavior. | S | R173–R176; field/offset/overflow/RO/OOM/WRONGTYPE and all-mode wire tests |
| S45 | As a location application, I can store points, inspect their coordinates, distance and geohash, and search by radius or box so that proximity queries do not require a separate index. | M | R177–R180; encoding, distance format, search/store atomicity, errors and all-mode wire tests |
| S46 | As a stream consumer-group operator, I can recover, reassign and inspect pending deliveries so that stalled consumers can be replaced without losing acknowledged work. | S | R181–R184; claim/autoclaim/introspection, errors and all-mode wire tests |
| S47 | As a sharded-channel client, I can subscribe and publish on a namespace separate from normal channels so that shard-oriented workers do not receive unrelated broadcasts. | S | R185–R188; separate fan-out/counts, shard introspection, ACL/RESET and all-mode wire tests |
| S48 | As a stream operator, I can bound retained history by length or minimum ID so that long-lived streams do not grow without limit. | XS | R189–R192; exact/approx/LIMIT/errors/TTL/WATCH and all-mode wire tests |
| S49 | As a cache operator, I can observe expired and evicted keys through keyspace notifications so invalidation tooling can react to lifecycle removal as well as explicit mutations. | S | R193–R196; flag validation, lazy/reaped expiry, eviction policy, bounded queues and all-mode wire tests |
| S50 | As a sorted-set user, I can query and store rank, score and lex ranges with reversal, limits and random member selection so that leaderboard and index workflows use one type. | M | R197–R200; range/store/lex/random, errors and all-mode wire tests |
| S51 | As a sorted-set user, I can combine multiple sets with weights and aggregation into results or destinations and count intersections so that rankings can be derived without client-side merging. | M | R201–R204; union/inter/diff/weights/aggregate/store/cardinality, errors, ACL and all-mode wire tests |
| S52 | As a sorted-set user, I can atomically pop lowest or highest members across several keys so that priority queues span multiple shards without client loops. | XS | R205–R208; key order, MIN/MAX, COUNT, TTL/WATCH and all-mode wire tests |
| S53 | As a sorted-set user, I can wait for members across several sorted sets so that consumers do not have to poll empty priority queues. | S | R209–R212; blocking timeout/wakeup, MIN/MAX/COUNT, TTL/WATCH, ACL and all-mode wire tests |
| S54 | As a stream producer, I can append with explicit retention and skip creating a missing stream so that hot streams stay bounded without a separate trim call. | S | R213–R216; XADD trim/NOMKSTREAM/errors/TTL/WATCH and all-mode wire tests |
| S55 | As a legacy geospatial client, I can use the GEORADIUS command family without rewriting clients to GEOSEARCH so existing radius and store workflows keep working. | M | R217–R220; aliases/RO/store/storedist/options, ACL and all-mode wire tests |
| S56 | As a collection user, I can sort list, set and sorted-set values with limits and optionally store the result so that ordered projections do not require client-side work. | S | R221–R224; numeric/alpha/limit/store/RO/errors and all-mode wire tests |
| S57 | As a stream operator, I can align a stream's last ID and retention counters after restore so that producers and groups resume without ID conflicts. | S | R225–R228; last-id/entries-added/max-deleted validation, XINFO, WATCH and all-mode wire tests |
| S58 | As a stream consumer, I can re-read my owned pending deliveries from a chosen ID so that recovery workflows can inspect and replay unacknowledged work. | S | R229–R232; history IDs/deleted entries/delivery updates/errors and all-mode wire tests |
| S59 | As a stream consumer, I can wait for new entries on one or more streams so that tailing clients do not have to poll. | S | R233–R236; BLOCK parsing/timeout/wakeup/$/multi-key/MULTI and all-mode wire tests |
| S60 | As a list user, I can locate repeated elements and pop across several list keys so that queue and feed operations do not require client loops. | S | R237–R240; LPOS rank/count/maxlen, LMPOP order/COUNT, errors and all-mode wire tests |
| S61 | As an operator or client, I can read the server wall clock and the actual storage encoding of a key without fabricated memory or LFU metrics. | XS | R241–R244; TIME shape, encodings, missing/unsupported cases and all-mode wire tests |
| S62 | As a stream group consumer, I can wait for new deliveries without polling empty streams so that workers can consume as producers append. | S | R245–R248; BLOCK group/timeout/wakeup/NOACK/history/MULTI and all-mode wire tests |
| S63 | As a list consumer, I can wait for elements across several lists so that queue workers do not poll empty keys. | S | R249–R252; BLMPOP timeout/wakeup/COUNT/MULTI/WATCH and all-mode wire tests |
| S64 | As a collection user, I can sort by derived weights and project joined string/hash values so that ordered views match existing application keys. | S | R253–R256; BY/GET substitution/weights/ACL/errors and all-mode wire tests |
| S65 | As a hash user, I can sample fields and atomically read-and-remove fields so that random selection and cleanup do not require multiple round trips. | XS | R257–R260; HRANDFIELD/HGETDEL shapes, TTL/WATCH/errors and all-mode wire tests |
| S66 | As a client or operator, I can ask the server which arguments are keys for a command so that routing and ACL tooling do not duplicate command metadata. | XS | R261–R264; static/counted/store extraction, errors and all-mode wire tests |
| S67 | As a stream consumer, I can read new or pending history entries across several group streams in one command so that sharded workloads avoid per-stream round trips. | S | R265–R268; multi-stream new/history/mixed/BLOCK/count/errors and all-mode wire tests |
| S68 | As a list user, I can move elements between two lists atomically, including blocking until a source element exists, so that queues and worker handoff do not need read-then-write races. | S | R269–R272; LMOVE/RPOPLPUSH sides, same-key rotation, TTL/OOM, blocking wake/timeout, MULTI/WATCH and all-mode wire tests |

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
  strict latency SLOs, fairness under many blocked clients and
  cluster/replication blocking semantics; blocking `BLMOVE`/`BRPOPLPUSH` list
  moves are specified separately by S68.
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

### Typed collection mutations (S37)

- **R145 (event-driven):** When hash mutation commands run, the server shall
  preserve binary field/value bytes and TTL; `HSETNX`/`HDEL`/`HEXISTS` shall use
  field presence, `HMGET` shall preserve argument order and nulls, `HKEYS`/`HVALS`
  shall return deterministic byte order, and `HINCRBY`/`HINCRBYFLOAT` shall reject
  malformed stored values, integer overflow or non-finite results without changing
  the hash. Removing the final field shall remove the key.
- **R146 (event-driven):** When list mutation commands run, the server shall
  preserve left/right order and negative indexes, insert relative to the first
  matching pivot, remove from the requested end, trim inclusively, and reject
  missing keys or out-of-range `LSET` indexes as documented. Empty results shall
  delete the key, and successful mutations shall preserve TTL.
- **R147 (event-driven):** When set mutation or algebra commands run, the server
  shall treat members as binary values, suppress duplicates, count removals once,
  and make `SMOVE` and the `SINTERSTORE`/`SUNIONSTORE`/`SDIFFSTORE` replacements
  atomic. Sources and existing destinations shall be unchanged on wrong type,
  invalid options or admission failure; an empty store result shall delete the
  destination and clear its TTL.
- **R148 (event-driven):** When sorted-set mutation or range commands run, the
  server shall preserve IEEE-754 score ordering (including `-inf`/`+inf`) with
  binary member tie-breaking, apply `NX`/`XX`/`GT`/`LT`/`CH`/`INCR`, exclusive
  score bounds, rank/score removal and bounded pops, and reject invalid or NaN
  scores without mutating state. Successful single-key mutations shall preserve
  TTL and delete the key when its last member is removed.

### Client registry and administrative kill (S38)

- **R149 (event-driven):** When a connection is accepted, the server shall register
  its ID, peer address, creation time and control handle; it shall update name,
  library, protocol and authenticated user after CLIENT/HELLO/AUTH/RESET and remove
  the record on close. The registry shall be bounded by `max_clients` and process-local.
- **R150 (event-driven):** When `CLIENT LIST [TYPE normal|pubsub] [ID id ...]` is
  received, the server shall return newline-delimited records for matching
  connections with ID, address, name, age, N/P flags, db, live channel/pattern
  counts, RESP mode, user and library metadata, omitting fields it does not measure.
- **R151 (event-driven):** When `CLIENT KILL` receives intersecting `ID`, `ADDR`,
  `TYPE normal|pubsub`, `USER` and `SKIPME yes` filters, the server shall shut down
  matching non-caller sockets and return the count; the legacy address form shall
  return `+OK`/`No such client`. The caller shall always be skipped, and `SKIPME no`
  shall return an explicit unsupported error.
- **R152 (ubiquitous):** `INFO clients` shall report the current registry size with
  no fabricated cluster/blocked counts. All-mode tests shall cover metadata,
  RESP3/pubsub visibility, ID/TYPE filters, self protection, ID/address/legacy kills,
  socket closure, registry removal and permission gating. Documentation shall retain
  the missing fd/idle/command/buffer/memory/network fields and cross-process view.
### HyperLogLog cardinality (S39)

- **R153 (event-driven):** When `PFADD key [element ...]` receives a missing key,
  it shall create an empty valid HLL value; subsequent distinct elements shall be
  added atomically, duplicate/no-op additions shall return `0` and a changed or
  created value shall return `1`. It shall preserve an existing TTL and reject
  non-string keys or malformed HLL payloads without mutation.
- **R154 (event-driven):** When `PFCOUNT key [key ...]` receives keys, it shall
  estimate the union cardinality, count missing keys as empty and return `0` for an
  empty union. Values up to 4,096 distinct hashes shall be counted exactly; larger
  promoted 2^14-register sketches shall use a bounded approximate estimator.
- **R155 (event-driven):** When `PFMERGE destination [source ...]` runs, it shall
  merge the existing destination value (if any) and all sources into the
  destination, preserve the destination TTL, create an empty destination for
  destination-only/missing inputs and return `+OK`. Wrong types, malformed HLL
  payloads or payload-budget failure shall leave the destination unchanged.
- **R156 (ubiquitous):** HLL values shall remain ordinary binary string keys for
  `TYPE`, expiry, keyspace, ACL key positions and append-only journal handling,
  with a private exact/dense payload rather than a Redis HLL encoding claim.
  Tests shall cover exact and dense promotion, cardinality tolerance, binary/empty
  elements, TTL, merge, malformed payloads, wrong types, ACL category metadata and
  transaction queueing in all execution modes.

### Keyspace notifications (S40)

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

### ACL credentials, dry-run and denial log (S41)

- **R161 (event-driven):** When `ACL GENPASS [bits]` receives a valid 1..=1024-bit
  request (default 256), the server shall return lowercase hex sourced from
  `/dev/urandom`; if the source is unavailable or bits are invalid, it shall return
  a bounded error without falling back to weak randomness.
- **R162 (event-driven):** When `ACL DRYRUN username command [arg ...]` is
  received, the server shall evaluate the named enabled user's ordered command,
  key and channel rules without executing the command, mutating state or recording
  a denial, returning `+OK`/`NOPERM` or a visible unknown-user error.
- **R163 (event-driven):** When permission is denied, the server shall record a
  bounded process-local log entry with reason (`noauth`/`command`/`key`/`channel`),
  context (`toplevel`/`multi`), object, username and age; identical tuples shall
  aggregate. `ACL LOG [count]` shall return newest entries as
  `[count, reason, context, object, username, age]` arrays and `ACL LOG RESET`
  shall clear them.
- **R164 (ubiquitous):** GENPASS/DRYRUN/LOG shall require the normal ACL command
  permission and all-mode tests shall cover random output length/hex, invalid bits,
  allowed/denied dry runs, unknown users, no-log DRYRUN behavior, key/channel/
  command/noauth aggregation, reset and bounded counts. Documentation shall retain
  the absence of persistent/replicated/centralized audit, rate limiting, token/
  timeout metadata and exhaustive security-event coverage.
### Bitmap algorithms (S42)

- **R165 (event-driven):** When `BITCOUNT key [start end [BYTE|BIT]]` receives a
  live string, it shall count set bits in the inclusive normalized range, using
  byte or bit length for negative indexes and clamping, and return `0` for missing
  keys or empty/reversed ranges. Argument counts, integers and unit tokens shall be
  validated before key access.
- **R166 (event-driven):** When `BITPOS key bit [start [end [BYTE|BIT]]]` receives
  bit `0`/`1`, it shall return the first matching MSB-first position or `-1`.
  Without an explicit end, a clear-bit search over a nonempty existing string shall
  treat the right side as zero-padded; a missing key shall return `0` for bit `0`
  and `-1` for bit `1`. Explicit ranges, negative indexes and unit boundaries shall
  be normalized without overflow.
- **R167 (event-driven):** When `BITOP AND|OR|XOR destination source [source ...]`
  or `BITOP NOT destination source` runs, it shall combine the longest source with
  missing/short bytes as zero, require one source for `NOT`, read aliased sources
  from their pre-command values, delete the destination for an empty result, clear
  destination TTL on replacement and return the result byte length.
- **R168 (unwanted behavior):** If a bitmap command has malformed arguments, an
  invalid bit/unit, a wrong-type source or destination admission failure, then it
  shall return one bounded error without partially changing any key. Successful
  mutations shall use exact payload accounting, journal/WATCH versions and atomic
  publication; tests shall cover BYTE/BIT and negative ranges, padding, empty keys,
  aliases, destination TTL, wrong type and all execution modes.

### Stream pending, range and multi-stream reads (S43)

- **R169 (event-driven):** When `XACK key group id [id ...]` succeeds, it shall
  remove each owned pending entry from the group and its consumer and return the
  acknowledged count; missing keys/groups and unknown IDs shall return zero without
  mutation, and wrong types shall return WRONGTYPE.
- **R170 (event-driven):** When `XPENDING key group` summary or
  `XPENDING key group [IDLE ms] start end count [consumer]` extended forms run,
  they shall return count/min/max/consumer counts or bounded
  `[id, consumer, idle, deliveries]` rows with inclusive bounds, consumer and idle
  filtering; missing key/group shall return NOGROUP and invalid arguments shall
  fail before mutation.
- **R171 (event-driven):** When `XRANGE`/`XREVRANGE` or non-blocking
  `XREAD [COUNT] STREAMS ...` run, they shall return bounded entries/streams in
  requested order with inclusive IDs, `-`/`+` and incomplete-ID expansion, `$`
  resolved at command time and `*-1` when no stream has new entries. Blocking
  XREAD shall return an explicit unsupported error.
- **R172 (event-driven):** `XGROUP DESTROY`, `CREATECONSUMER` and `DELCONSUMER`
  shall update group/consumer metadata atomically, report created/removed counts,
  delete a consumer's pending references, preserve TTL/accounting and touch the
  stream for WATCH/journal capture. All-mode tests shall cover ack/pending/range/
  read/group lifecycle, errors and multi-stream reads; docs shall retain missing
  blocking/claim/trim/XINFO subcommands, volatile pending across restart and
  dynamic-key ACL limitations.

### Arbitrary-width bitfields (S44)

- **R173 (event-driven):** When `BITFIELD key` receives `GET type offset`,
  `SET type offset value` and `INCRBY type offset increment` subcommands, it shall
  execute them left-to-right and return one RESP array element per operation:
  `GET` returns the field, `SET` returns the previous field, and `INCRBY` returns
  the new field. Types shall be signed `i1`..`i64` or unsigned `u1`..`u63`; offsets
  shall be numeric bit positions or `#n` field-width multiples within the bounded
  32-bit offset range. Missing bytes shall read as zero.
- **R174 (event-driven):** While `OVERFLOW WRAP|SAT|FAIL` applies to following
  `SET`/`INCRBY` operations, the server shall wrap (default), saturate or return a
  null array element without writing when the value cannot fit. Successful writes
  shall zero-fill gaps, preserve TTL and publish the complete new string only after
  all parse, offset and payload-admission checks succeed.
- **R175 (event-driven):** `BITFIELD_RO` shall permit only `GET` (and `OVERFLOW`)
  and return `ERR BITFIELD_RO only supports the GET subcommand` for writes. A
  wrong-type key, invalid type/offset/value/overflow token or payload-budget
  failure shall return one bounded error without exposing partial writes.
- **R176 (ubiquitous):** The commands shall publish truthful command metadata and
  the `@bitmap` ACL category, never require external dependencies or unsafe code,
  and be covered by unit and all-mode wire tests for signed/unsigned fields,
  `#` offsets, overflow policies, zero-fill, TTL, RO/error paths, transactions and
  OOM atomicity. `u64` remains unsupported and no Redis-internal encoding guarantee
  is claimed.

### Geospatial sorted sets (S45)

- **R177 (event-driven):** When `GEOADD key [NX|XX] [CH] longitude latitude member
  ...` receives triples, it shall validate every longitude/latitude and option
  before any key access, reject unparseable/NaN coordinates with the float error
  and out-of-range/non-finite pairs with the bounded pair error, encode each point into a 52-bit interleaved geohash score,
  and apply `ZADD` NX/XX/CH semantics while preserving TTL and exact payload
  accounting.
- **R178 (event-driven):** When `GEOPOS`, `GEOHASH` or `GEODIST` reads members, it
  shall decode geohash scores and return 17-fractional coordinate text, an
  11-character standard geohash, or a four-decimal distance in M/KM/FT/MI
  respectively. Missing members shall return the command's null shape; wrong-type
  keys shall return WRONGTYPE; invalid units or coordinates shall fail before
  producing a result.
- **R179 (event-driven):** When `GEOSEARCH` runs, it shall require exactly one of
  FROMMEMBER/FROMLONLAT and exactly one of BYRADIUS/BYBOX, support ASC/DESC,
  COUNT with optional ANY validation and WITHCOORD/WITHDIST/WITHHASH, resolve
  FROMMEMBER against the live source, scan the bounded sorted set with the
  documented haversine/rectangle checks, and return at most COUNT deterministic
  results. A missing source shall return an empty array.
- **R180 (unwanted behavior):** When `GEOSEARCHSTORE` stores matches, it shall
  perform one atomic destination replacement, clear the destination TTL, support
  STOREDIST, delete the destination for an empty result and never partially write
  on malformed options, wrong source type or payload OOM. `GEOADD` and
  `GEOSEARCHSTORE` shall publish `zadd`/`geosearchstore`/`del` events, expose
  truthful `@geo` metadata and two ordered keys for the store command, and be
  covered by unit and all-mode wire tests. Legacy `GEORADIUS*` aliases are
  delivered in S55; native neighbour indexes and RDB interchange shall remain
  documented as unsupported.
### Stream claiming and consumer introspection (S46)

- **R181 (event-driven):** When `XCLAIM key group consumer min-idle-time id [id ...]`
  runs, it shall claim existing pending IDs only when their idle time meets the
  threshold, transfer ownership and update delivery time/count, prune pending IDs
  whose stream entry no longer exists, and return claimed entries in argument
  order. `FORCE` shall create pending state for existing entries; `JUSTID`,
  `IDLE`, `TIME`, `RETRYCOUNT` and `LASTID` shall apply as documented without
  changing entry payloads.
- **R182 (event-driven):** `XAUTOCLAIM key group consumer min-idle-time start
  [COUNT count] [JUSTID]` shall scan at most a bounded multiple of `COUNT`
  pending entries from `start`, claim eligible entries, prune message-less
  pending IDs, and return exactly `[next-start-id, claimed, deleted]` with
  `JUSTID` selecting ID-only claimed rows. `COUNT` shall be positive.
- **R183 (event-driven):** `XINFO GROUPS key` / `XINFO CONSUMERS key group` shall
  return bounded RESP2 maps for group name/consumer count/pending count/
  last-delivered-id/`entries-read` (null)/`lag` and consumer name/pending count/
  `idle`/`inactive`, with missing-key, missing-group and wrong-type errors.
- **R184 (unwanted behavior):** If a claim command has malformed minimum-idle/
  start/ID/option/COUNT input or a missing/wrong-type key/group, it shall return
  one bounded error without partial transfer. Successful claims shall touch the
  stream for WATCH/journal capture, publish `xclaim`/`xautoclaim` notifications,
  preserve payload accounting and TTL, keep pending metadata process-local, and
  be covered by unit and all-mode wire tests. Blocking and history reads shall
  remain documented as unsupported.


### Sharded publish/subscribe (S47)

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
### Stream trimming (S48)

- **R189 (event-driven):** When `XTRIM key MAXLEN|MINID [~|=] threshold
  [LIMIT count]` runs, it shall parse the strategy, optional approximate marker
  and LIMIT before key access, reject negative MAXLEN/LIMIT, invalid IDs,
  conflicting strategies and LIMIT without `~` with the documented bounded
  errors.
- **R190 (event-driven):** Exact `MAXLEN` shall remove oldest entries until the
  retained length is at most the threshold; exact `MINID` shall remove entries
  with IDs below the threshold. Approximate mode shall remove at most the given
  LIMIT (default 10,000, LIMIT 0 unbounded) and return the removed count.
- **R191 (event-driven):** `XTRIM` shall return zero for a missing key and
  WRONGTYPE for a non-stream key, preserve TTL and payload accounting, retain
  pending/consumer references to removed IDs (as XDEL does), and touch the stream
  for WATCH/journal capture and an `xtrim` keyspace event when entries are
  removed.
- **R192 (unwanted behavior):** Invalid or partial trimming arguments shall not
  mutate the stream. Tests shall cover exact/approximate MAXLEN/MINID, `=` and
  `~`, LIMIT bounds, empty and missing streams, TTL, pending retention, WATCH and
  wrong types in unit and all-mode wire tests. Blocking reads and history reads
  shall remain documented as unsupported.

### Expired and evicted notification classes (S49)

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
### Sorted-set range and selection completion (S50)

- **R197 (event-driven):** `ZRANGE key start stop [BYSCORE|BYLEX] [REV]
  [LIMIT offset count] [WITHSCORES]` shall preserve rank-mode behavior and add
  score/lex selection with optional reversal. `LIMIT` shall be valid only with
  `BYSCORE`/`BYLEX`; `WITHSCORES` shall be rejected with `BYLEX`; score and lex
  bounds shall use inclusive/exclusive markers and `-`/`+` terminal bounds.
- **R198 (event-driven):** `ZRANGESTORE destination source start stop
  [BYSCORE|BYLEX] [REV] [LIMIT offset count]` shall apply the same selection,
  atomically replace the destination sorted set, clear its TTL, delete it for an
  empty selection and preserve the destination on wrong-type source or payload
  OOM.
- **R199 (event-driven):** `ZLEXCOUNT key min max` and
  `ZREMRANGEBYLEX key min max` shall count/remove members in the lex range,
  deleting the key when the set becomes empty, preserving TTL and touching the
  stream for WATCH/journal capture. `ZRANDMEMBER key [count [WITHSCORES]]` shall
  return one random member, up to `count` distinct members for positive counts,
  `abs(count)` samples with replacement for negative counts (bounded by the
  collection limit), and flat member/score pairs when `WITHSCORES` is requested.
- **R200 (unwanted behavior):** Invalid range/mode/LIMIT/COUNT arguments,
  malformed score or lex bounds, wrong types and missing keys shall return the
  documented empty, null or bounded errors without partial writes. Tests shall
  cover rank/score/lex ranges, `REV`, `LIMIT`, `WITHSCORES` combinations,
  `ZRANGESTORE` self/empty/TTL/OOM cases, lex count/removal, random counts,
  WATCH and all worker modes. Blocking `BZMPOP` is delivered separately in S53.

### Sorted-set algebra (S51)

- **R201 (event-driven):** `ZUNION`, `ZINTER` and `ZDIFF` shall parse a strictly
  positive `numkeys`, exactly that many source keys, and the documented trailing
  options. Union and intersection shall accept `WEIGHTS weight [weight ...]`
  (one per key) and `AGGREGATE SUM|MIN|MAX`; all three shall accept `WITHSCORES`
  for non-store variants. `ZDIFF` shall reject weights and aggregation. Missing
  source keys shall count as empty sets, and any wrong-type source shall fail
  without a result write. Replies shall be ordered by score then binary member.
- **R202 (event-driven):** Weighted scores shall multiply each input score and
  aggregate with the selected operation; a NaN intermediate or aggregate score
  shall become zero as in Redis, while infinities remain. `ZDIFF` shall return
  first-set members absent from every later set with their original scores.
  `ZINTERCARD numkeys key [key ...] [LIMIT limit]` shall count members present in
  every set, cap the reply at a positive limit, use the full count for zero, and
  reject negative or non-integer limits.
- **R203 (event-driven):** `ZUNIONSTORE`, `ZINTERSTORE` and `ZDIFFSTORE` shall
  compute the same result and atomically replace the destination, clear its TTL,
  delete it for an empty result, support a destination that is also a source,
  enforce payload and element limits before replacing the destination, and
  publish a `zunionstore`/`zinterstore`/`zdiffstore` event for the destination
  only.
- **R204 (unwanted behavior):** Invalid `numkeys`, insufficient keys, malformed
  weights, unknown aggregation or options, non-store `WITHSCORES` misuse and
  wrong types shall return the documented bounded errors without partial writes.
  ACL key patterns shall cover the counted source list and the store destination.
  Tests shall cover union/intersection/difference with weights and every
  aggregation, missing keys, destination replacement/TTL/self/empty/OOM cases,
  NaN handling, cardinality limits, errors and all worker modes.

### Sorted-set multi-pop (S52)

- **R205 (event-driven):** `ZMPOP numkeys key [key ...] MIN|MAX [COUNT count]`
  shall require a positive counted key list, choose the first live non-empty
  sorted-set key in argument order, reject a wrong-type key encountered before a
  candidate, and parse `MIN`/`MAX` plus at most one positive `COUNT` before key
  access.
- **R206 (event-driven):** On success it shall return
  `[key, [[member, score], ...]]` with up to COUNT score-then-member ordered
  members, delete the key when emptied, preserve remaining TTL and payload
  accounting, and touch the key for WATCH/journal capture with a `zpopmin` or
  `zpopmax` notification matching the direction.
- **R207 (event-driven):** If no listed key has a non-empty sorted set, it shall
  return a null array without creating keys. Missing keys in the list shall be
  skipped, and a key appearing again after being emptied shall not be recreated.
- **R208 (unwanted behavior):** Invalid numkeys/COUNT/direction/extra options,
  wrong types and negative or zero counts shall return bounded errors without
  partial pops. Tests shall cover first-key selection, MIN/MAX, COUNT, missing
  keys, emptied keys, TTL, MULTI, WATCH, notifications, errors and all worker
  modes. Blocking `BZMPOP` is delivered separately in S53.

### Blocking sorted-set multi-pop (S53)

- **R209 (event-driven):** `BZMPOP timeout numkeys key [key ...] MIN|MAX
  [COUNT count]` shall parse the float-seconds timeout before key access, reject
  NaN/negative/out-of-range timeouts with the documented errors, require a
  positive counted key list and exactly one direction, and accept at most one
  positive `COUNT`. Timeout zero shall wait indefinitely.
- **R210 (event-driven):** On success it shall pop from the first live non-empty
  sorted-set key in argument order, return `[key, [[member, score], ...]]` with up
  to `COUNT` score-then-member ordered members, delete the key when emptied,
  preserve remaining TTL and payload accounting, commit journal state, and publish
  a `zpopmin`/`zpopmax` event matching the direction. A wrong-type key before a
  candidate shall fail without popping; missing keys shall be skipped and later
  keys re-checked when blocked.
- **R211 (state-driven):** While no listed key is ready, the connection shall wait
  using the existing bounded polling model without holding the store lock. A
  positive timeout shall produce a null array when it expires; disconnect,
  shutdown or RESET shall clear the blocked state. Inside `MULTI`, `BZMPOP` shall
  execute as its non-blocking multi-pop equivalent at `EXEC` while still
  validating the timeout, and `WATCH` conflicts shall abort `EXEC` without pops.
- **R212 (unwanted behavior):** Invalid timeout/numkeys/direction/COUNT/options,
  wrong types and empty results shall return documented bounded errors or a null
  array without partial pops. ACL key patterns shall cover the counted key list
  before blocking. Tests shall cover immediate pops, MIN/MAX, COUNT, key order,
  timeout nulls, wake-on-write, TTL, delete-on-empty, MULTI/WATCH, notifications,
  ACL, errors and all worker modes.

### Stream append retention and NOMKSTREAM (S54)

- **R213 (event-driven):** `XADD key [NOMKSTREAM]
  [MAXLEN|MINID [~|=] threshold [LIMIT count]] id|* field value ...` shall parse
  the options before key access, reject conflicting/negative/invalid trim or ID
  arguments, and require at least one field/value pair after the ID.
- **R214 (event-driven):** `NOMKSTREAM` shall return a null bulk reply when the
  key does not exist and shall not create a stream. Existing streams shall append
  normally.
- **R215 (event-driven):** After appending, `MAXLEN`/`MINID` shall remove entries
  with the same exact or bounded approximate rules as XTRIM, including `LIMIT`
  only with `~`, return the newly generated/explicit ID, preserve TTL and exact
  payload accounting, and leave the previous stream unchanged on parse,
  wrong-type or payload-admission failure. An emptied retained stream may remain
  as an existing zero-length stream key.
- **R216 (unwanted behavior):** Tests shall cover exact/approximate MAXLEN and
  MINID, `=`/`~`, LIMIT bounds, NOMKSTREAM on missing/existing keys, TTL,
  wrong types, malformed options/IDs, MULTI, WATCH, OOM and all worker modes.
  XADD trim shall publish the `xadd` event only.

### Legacy geospatial radius aliases (S55)

- **R217 (event-driven):** `GEORADIUS key longitude latitude radius unit` and
  `GEORADIUSBYMEMBER key member radius unit` shall parse WGS-84 coordinates or a
  live member origin, a non-negative radius and a supported M/KM/FT/MI unit, then
  apply the existing bounded radius search. They shall accept
  `WITHCOORD`/`WITHDIST`/`WITHHASH`, `COUNT count [ANY]`, `ASC`/`DESC`, `STORE key`
  and `STOREDIST key` in any order, with the same distance/coordinate/hash reply
  text as `GEOSEARCH`. Missing sources shall return an empty array, a missing
  member in an existing source shall return the member-decode error, and
  wrong-type sources shall return WRONGTYPE.
- **R218 (event-driven):** `GEORADIUS_RO` and `GEORADIUSBYMEMBER_RO` shall return
  the same reads but reject `STORE`/`STOREDIST` with a syntax error. For the
  writable variants, the last `STORE`/`STOREDIST` wins, the destination stores
  geohash scores or distances in the requested unit respectively, and combining a
  store option with `WITHCOORD`/`WITHDIST`/`WITHHASH` shall fail with the legacy
  GEORADIUS incompatibility error.
- **R219 (event-driven):** Legacy store variants shall atomically replace the
  destination, clear its TTL, delete it for an empty result or missing source and
  return the stored count, without partial writes on wrong type, malformed options
  or payload OOM. They shall commit journal state while publishing no separate
  destination notification, matching standalone Redis. Source and last-wins store
  destination shall both be covered by ACL key patterns.
- **R220 (unwanted behavior):** Invalid floats/coordinates, negative radius,
  unsupported units, non-positive or non-integer COUNT, ANY without COUNT, unknown
  options, invalid arity and read-only store attempts shall return bounded errors
  before any destination write. Tests shall cover both origins, every option
  combination, RO variants, store/storedist and last-wins behavior, TTL, missing
  keys/members, wrong types, ACL, discovery metadata and all worker modes.

### Bounded collection sorting (S56)

- **R221 (event-driven):** `SORT key [ASC|DESC] [ALPHA] [LIMIT offset count]
  [STORE destination]` and `SORT_RO ...` shall accept list, set and sorted-set
  sources, reject other types, parse every option before key access, and return
  an empty array for a missing source. Numeric mode shall reject non-double/NaN
  elements with the documented error; `ALPHA` shall compare raw bytes.
- **R222 (event-driven):** Sorting shall apply ASC/DESC, deterministic
  member-byte tie-breaking and post-sort LIMIT clamping (negative offset to zero,
  negative count as unbounded). `SORT_RO` shall reject `STORE`; `BY` and `GET`
  patterns shall be rejected explicitly rather than ignored.
- **R223 (event-driven):** `STORE destination` shall write the selected values as
  a list, replace any destination type, clear its TTL, return the count, delete
  the destination for an empty result and support source==destination from the
  pre-command source snapshot. A wrong-type source or payload OOM shall leave the
  destination unchanged and publish `sortstore`/`del` notifications only on
  successful mutation.
- **R224 (ubiquitous):** ACL key extraction shall cover the source and STORE
  destination, discovery shall expose truthful `SORT`/`SORT_RO` metadata and the
  documented categories, and tests shall cover numeric/alpha/ASC/DESC/LIMIT,
  list/set/zset sources, missing/empty/self/overwrite/TTL/OOM/wrong-type cases,
  SORT_RO, BY/GET rejection, MULTI, WATCH, notifications and all worker modes.
  Dynamic BY/GET patterns remain documented as unsupported.

### Stream ID and metadata control (S57)

- **R225 (event-driven):** `XSETID key last-id [ENTRIESADDED count]
  [MAXDELETEDID id]` shall parse the stream ID and optional metadata pairs,
  reject malformed IDs/options, non-integer or negative `ENTRIESADDED`, and
  require an existing stream key. It shall reject a last ID below the greatest
  retained entry, below the effective maximum-deleted ID, or an entries-added
  value below the retained length with the documented Redis errors and no partial
  update.
- **R226 (state-driven):** Streams shall track entries-added across accepted
  XADD operations (including immediately trimmed entries) and the maximum deleted
  entry ID across XDEL, XTRIM and XADD trimming. `XSETID` shall update
  last-generated-id, entries-added and max-deleted-entry-id, and `XINFO STREAM`
  (with and without FULL) shall expose `entries-added` and
  `max-deleted-entry-id` truthfully.
- **R227 (event-driven):** `XSETID` shall touch the key for WATCH/journal capture
  and publish no separate keyspace event, matching standalone Redis. Journal
  snapshots shall emit an `XSETID` record preserving last ID, entries-added and
  max-deleted-entry-id so restart reproduces the metadata and enforces subsequent
  XADD ID ordering.
- **R228 (unwanted behavior):** Missing/wrong-type keys, invalid IDs, bad option
  ordering/arity, negative counters and inconsistent metadata shall return bounded
  errors without modifying the stream. Tests shall cover metadata changes, ID
  ordering, XDEL/trim counter updates, MULTI, WATCH conflicts, restart replay and
  all worker modes.

### Stream consumer history reads (S58)

- **R229 (event-driven):** `XREADGROUP GROUP group consumer [COUNT n] [NOACK]
  STREAMS key id` shall treat `>` as a new-entry read but any other ID as a
  history read of that consumer's pending entries. `$` shall return the documented
  "meaningless" error, malformed IDs shall fail before key access, and `COUNT`
  shall cap the returned pending range while `NOACK` shall not suppress history
  metadata updates.
- **R230 (event-driven):** A history read shall return `[key, entries]` for the
  single supported stream, selecting pending entries owned by that consumer with
  IDs greater than the requested ID in ascending order. Present entries shall
  include their fields, deleted pending IDs shall return a null fields array, and
  an empty result shall still return the key with an empty entries array. The
  consumer shall be created when absent, and each returned pending record shall
  increment its delivery count and advance its delivery time while leaving the
  group's last-delivered ID unchanged.
- **R231 (unwanted behavior):** Missing keys/groups shall return bounded NOGROUP
  errors naming the requested key/group, and wrong-type keys shall return
  WRONGTYPE. History reads shall touch the stream for WATCH/journal capture and
  follow the existing `xreadgroup` notification behavior without claiming durable
  pending state. Multiple history streams are specified in S66.
- **R232 (ubiquitous):** Tests shall cover full/partial/deleted pending history,
  ordering, COUNT, no-pending and absent-consumer behavior, delivery-count/time
  updates, unchanged group cursors, NOACK, `$`/malformed-ID/NOGROUP/WRONGTYPE
  errors, WATCH conflicts and all worker modes. Durable pending restoration shall
  remain documented as unsupported.

### Blocking stream reads (S59)

- **R233 (event-driven):** `XREAD [COUNT n] [BLOCK ms] STREAMS key [key ...]
  id [id ...]` shall accept an integer millisecond BLOCK value, use zero for an
  indefinite wait, reject negative/non-integer/out-of-range values and syntax
  errors before stream access, and let a later duplicate option win. Existing
  COUNT, `$`, explicit-ID, multi-key and unbalanced-list validation shall remain.
- **R234 (event-driven):** A blocking read shall return immediately when any
  listed stream already has entries above its start. Otherwise the connection
  shall wait using the existing bounded polling model; `$` shall be resolved once
  at block time to each live last-generated ID, and appends to any listed stream
  shall wake the read with the same RESP array shape as non-blocking XREAD.
  Positive timeouts shall return a null array; expiry, missing keys, wrong types
  and disconnect/shutdown shall not leak blocked state.
- **R235 (state-driven):** Inside `MULTI`, `BLOCK` shall execute the non-blocking
  read once at `EXEC` without waiting. Blocking reads shall not mutate data or
  journal state, shall not hold the store lock while waiting, and shall remain
  at-most-once observations of live entries rather than durable subscriptions.
- **R236 (ubiquitous):** Tests shall cover immediate reads, indefinite and finite
  waits, wake-on-append, `$` capture, multi-key wake selection, COUNT, timeout
  nulls, MULTI non-blocking behavior, wrong types, malformed BLOCK/IDs/STREAMS and
  all worker modes. Multi-stream history shall remain documented as unsupported.

### List position and multi-key pop (S60)

- **R237 (event-driven):** `LPOS key element [RANK rank] [COUNT count]
  [MAXLEN maxlen]` shall search lists forward for positive ranks and backward for
  negative ranks, reject rank zero and negative COUNT/MAXLEN, and limit examined
  elements when MAXLEN is positive. Without COUNT it shall return the zero-based
  index or a null bulk; with COUNT it shall return an index array, with COUNT 0
  meaning all matches.
- **R238 (event-driven):** `LMPOP numkeys key [key ...] LEFT|RIGHT [COUNT count]`
  shall require a positive counted key list and COUNT, choose the first live
  non-empty list in argument order, reject a wrong-type key encountered before a
  candidate, and return `[key, [element, ...]]` in pop order (head-to-tail for
  LEFT, tail-to-head for RIGHT).
- **R239 (event-driven):** LMPOP shall cap pops at the list length and COUNT,
  delete emptied lists, preserve remaining TTL and payload accounting, touch keys
  for WATCH/journal capture and publish an `lmpop` event on mutation. A missing
  or exhausted key set shall return a null array.
- **R240 (unwanted behavior):** Invalid rank/COUNT/MAXLEN/numkeys/direction/extra
  options, wrong types and missing keys shall return bounded errors, nulls or
  empty arrays without partial pops. Tests shall cover rank/count/maxlen
  combinations, binary elements, LEFT/RIGHT order, COUNT bounds, TTL, MULTI,
  WATCH, notifications and all worker modes. Blocking `BLMPOP` remains
  documented as unsupported.

### Runtime clock and object introspection (S61)

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

### Blocking list multi-pop (S63)

- **R249 (event-driven):** `BLMPOP timeout numkeys key [key ...] LEFT|RIGHT
  [COUNT count]` shall parse a non-negative finite float-seconds timeout, a
  positive counted key list, exactly one direction and at most one positive
  COUNT before key access. Zero timeout shall wait indefinitely; NaN/negative/
  out-of-range timeouts, malformed keys/counts/directions/options shall return
  bounded errors.
- **R250 (event-driven):** If a listed list already has elements, BLMPOP shall
  immediately perform LMPOP selection: pop from the first live non-empty list in
  argument order, return `[key, [element, ...]]` in head-to-tail LEFT or
  tail-to-head RIGHT order, delete emptied lists and preserve remaining TTL and
  payload accounting. A wrong-type key before a candidate shall fail without a
  pop. Missing keys shall be skipped.
- **R251 (state-driven):** Otherwise the connection shall wait using the bounded
  polling model without holding the store lock. A positive timeout shall produce
  a null array when it expires; append wakeups, disconnect, shutdown and RESET
  shall clear blocked state. Inside `MULTI`, BLMPOP shall validate the timeout but
  execute the non-blocking LMPOP form once at `EXEC`; WATCH conflicts shall abort
  without pops.
- **R252 (unwanted behavior):** Invalid timeout/numkeys/count/direction/options,
  wrong types and empty results shall return bounded errors or a null array
  without partial pops. ACL key patterns shall cover the counted key list before
  blocking. Tests shall cover immediate/wake/timeout paths, MIN/MAX-equivalent
  LEFT/RIGHT order, COUNT, TTL, delete-on-empty, MULTI/WATCH, notification,
  disconnect behavior and all worker modes.

### SORT dynamic weights and projections (S64)

- **R253 (event-driven):** `SORT`/`SORT_RO` shall accept `BY pattern` and one or
  more `GET pattern` options before or after other supported options, validate
  their arguments before key access, and reject STORE on SORT_RO.
- **R254 (event-driven):** `BY` with `*` shall substitute the first occurrence per
  source element, support a non-empty `->field` hash dereference, and use the
  resulting value as numeric or ALPHA weight. Missing/wrong-type weight keys
  shall contribute zero/empty, while present non-numeric numeric weights shall
  return the conversion error. A `BY` pattern without `*` shall disable sorting
  and use deterministic native list/zset order, reversed by DESC.
- **R255 (event-driven):** `GET` shall substitute each selected element, support
  `GET #`, and return null for missing/wrong-type references in reply mode or an
  empty string when storing. Multiple GET patterns shall multiply output length.
  Because BY/GET patterns derive key names, restricted ACL users without all-key
  access shall be denied before any derived lookup; constant BY patterns remain
  allowed with source-key access.
- **R256 (unwanted behavior):** Invalid/missing pattern arguments, malformed
  numeric weights, wrong types, missing keys and OOM shall return bounded errors
  or documented null/empty values without partial destination writes. Tests shall
  cover string/hash weights, missing and invalid weights, constant BY, ALPHA,
  multiple GETs, store projections, ACL denial/allkeys, TTL/OOM, MULTI/WATCH and
  all worker modes. Cluster routing and durable indexes remain unsupported.

### Blocking consumer-group reads (S62)

- **R245 (event-driven):** `XREADGROUP GROUP group consumer [COUNT n] [BLOCK ms]
  [NOACK] STREAMS key id` shall parse the same integer-millisecond BLOCK values as
  XREAD, reject negative/non-integer/out-of-range options and `$` with the
  documented errors, and apply waiting only to a `>` new-entry read. A history ID
  with BLOCK shall return history immediately without waiting.
- **R246 (event-driven):** When a `>` read has no new entries, a positive BLOCK
  shall wait using the bounded polling model and return null on expiry; zero shall
  wait indefinitely. Appends shall wake the read, deliver the normal response
  shape, advance the group cursor, add pending entries unless NOACK is set, update
  consumer activity, and publish the existing `xreadgroup` notification after
  journal capture. Missing keys/groups and wrong types shall fail immediately
  without blocking.
- **R247 (state-driven):** Inside `MULTI`, BLOCK shall execute one non-blocking
  `>` read at `EXEC` and never wait. Blocked connections shall not hold the store
  lock, shall clear state on disconnect/shutdown, and shall remain bounded polling
  observations rather than durable subscriptions.
- **R248 (ubiquitous):** Tests shall cover immediate group reads, finite and
  indefinite waits, wake-on-append, NOACK, cursor/pending updates, history with
  BLOCK, MULTI non-blocking behavior, missing/wrong-type errors, malformed
  BLOCK/IDs/STREAMS, notification direction and all worker modes. Multi-stream
  history and replication/durable pending remain unsupported.

### Hash random fields and atomic get-delete (S65)

- **R257 (event-driven):** `HRANDFIELD key [count [WITHVALUES]]` shall return one
  random field without count, distinct fields for positive counts, up to
  `abs(count)` duplicate-capable fields for negative counts, and flat field/value
  pairs with WITHVALUES. Missing keys and zero counts shall return empty/null
  shapes as documented; wrong types and malformed counts/options shall fail.
- **R258 (event-driven):** `HGETDEL key FIELDS numfields field [field ...]` shall
  require the FIELDS token, a positive numfields exactly matching the supplied
  fields, and return one value-or-null per field while atomically deleting
  present fields.
- **R259 (event-driven):** HGETDEL shall preserve remaining TTL and payload
  accounting, delete an emptied hash key, leave missing keys null-filled, and
  touch the key for WATCH/journal capture. Successful deletions shall publish
  `hdel`, or `del` when the hash becomes empty.
- **R260 (unwanted behavior):** Invalid/missing FIELDS, non-positive/mismatched
  numfields, wrong types, malformed random counts and options shall return
  bounded errors without partial mutation. Tests shall cover random single/count/
  negative/WITHVALUES shapes, missing keys, TTL/empty deletion, WATCH/MULTI,
  notifications and all worker modes. Hash-field expiration remains unsupported.

### Command key introspection (S66)

- **R261 (event-driven):** `COMMAND GETKEYS <command> [args ...]` shall resolve the
  command case-insensitively, validate its advertised arity, and return the
  declared key arguments or the documented invalid-command/arity/no-key errors.
- **R262 (event-driven):** Key extraction shall reuse the ACL key-spec logic,
  including multi-key ranges, counted key lists, STORE destinations, legacy
  GEORADIUS destinations and sorted-set source/destination rules.
- **R263 (event-driven):** Commands with no declared keys and dynamic lists that
  cannot be extracted from the supplied arguments shall return bounded errors
  rather than fabricating keys. BY/GET-derived SORT keys and dynamic XREAD keys
  remain outside static extraction and are documented as such.
- **R264 (ubiquitous):** Tests shall cover string, multi-key, counted, STORE,
  GEORADIUS and no-key commands, invalid command/arity and all worker modes.
  COMMAND GETKEYS does not change keyspace state or bypass ACL enforcement.

### Multi-stream consumer-group reads (S67)

- **R265 (event-driven):** `XREADGROUP GROUP group consumer [COUNT n] [BLOCK ms]
  [NOACK] STREAMS key [key ...] id [id ...]` shall require an even, non-empty
  key/ID list and parse each ID independently as `>` or a history ID. `$`,
  malformed IDs and unbalanced lists shall fail before key access; COUNT shall
  apply per stream.
- **R266 (event-driven):** New `>` streams shall deliver entries above their own
  group cursor, advance cursors and add pending records unless NOACK is set.
  History streams shall return that consumer's owned pending IDs above the given
  ID, including the four stream shape and null fields for deleted entries, even
  when empty. Delivery counts/times are updated for returned history records.
  Results shall follow key argument order.
- **R267 (state-driven):** The whole command shall validate every key type and
  group before mutating any stream; a missing key/group or wrong type on a later
  stream shall fail the command without earlier cursor/pending changes. Mixed
  new/history and all-history commands shall return immediately. BLOCK shall wait
  only when every ID is `>` and shall wake on appends to any listed stream;
  finite timeouts return null, and `MULTI` runs one non-blocking attempt.
- **R268 (ubiquitous):** Tests shall cover multi-stream new/new, history/history,
  mixed reads, per-stream COUNT, NOACK, empty/deleted history, argument-order
  replies, validation atomicity, BLOCK all-new wake/timeout, mixed BLOCK
  immediacy, MULTI, `$`/unbalanced/missing-group/wrong-type errors, notifications
  and all worker modes. Durable pending restoration and replication remain
  unsupported. (feat: add multi-stream consumer-group reads)
### List element moves (S68)

- **R269 (event-driven):** `LMOVE source destination LEFT|RIGHT LEFT|RIGHT` and
  `RPOPLPUSH source destination` shall atomically pop the requested source side
  and push onto the requested destination side, replying the moved element, null
  for an absent or empty source, and WRONGTYPE without mutation for a wrong-type
  source or destination.
- **R270 (state-driven):** While `source` and `destination` are the same key, the
  command shall rotate that list in place exactly once, without duplicating or
  losing elements. Moving the last source element shall delete the source key; an
  existing destination shall keep its TTL; a newly created destination shall have
  no TTL; and an over-budget move shall reply OOM without changing either key.
- **R271 (event-driven):** `BLMOVE ... timeout` and `BRPOPLPUSH ... timeout` shall
  perform the move immediately when a live source element exists, otherwise wait
  without holding the store lock and reply null on expiry. `0` shall wait
  indefinitely and negative, non-float or out-of-range timeouts shall fail before
  mutation. Inside `MULTI` both commands shall execute once, non-blockingly, at
  `EXEC`; a conflicting `WATCH` shall abort without moving.
- **R272 (ubiquitous):** Successful moves shall be journaled and shall emit
  `lpush`/`rpush` on the destination, `lpop`/`rpop` on the source, and generic
  `del` when the move empties the source. Tests shall cover all sides, same-key
  rotation, wrong types, TTL, OOM, blocking wake/timeout, MULTI/WATCH,
  notifications, journal replay and every worker mode.

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
volatile TTL); keyspace/keyevent notifications for supported mutation classes;
ACL GENPASS/DRYRUN and a bounded denial log; Redis DIGEST/DELEX
and Valkey DELIFEQ; APPEND/STRLEN/GETRANGE/SETRANGE/SUBSTR; SETBIT/GETBIT/
BITCOUNT/BITPOS/BITOP/BITFIELD/BITFIELD_RO; LCS (LEN/IDX/WITHMATCHLEN/
MINMATCHLEN); lists, sets, hashes and sorted sets with
creation, mutation, removal, bounded random selection, set algebra/store
commands, LPOS/LMPOP list position/multi-pop, score/rank/lex ranges, range storage,
random members, sorted-set
algebra/cardinality/multi-pop, blocking multi-pop, pops, bounded
`BLPOP`/`BRPOP`, stream `XSETID` metadata control and consumer history reads
documented
in [typed collections](collections.md); geospatial sorted sets (`GEOADD`,
`GEOPOS`, `GEODIST`, `GEOHASH`, `GEOSEARCH`, `GEOSEARCHSTORE`) documented in
[geospatial limits](geospatial.md); SCAN/HSCAN/SSCAN/ZSCAN bounded cursor
iteration with MATCH/COUNT/TYPE/NOVALUES; PFADD/PFCOUNT/PFMERGE approximate
cardinality; PUBLISH/SPUBLISH, SUBSCRIBE/PSUBSCRIBE/SSUBSCRIBE,
UNSUBSCRIBE/PUNSUBSCRIBE/SUNSUBSCRIBE and PUBSUB introspection.
See [string semantics](strings.md), [string ranges](string-ranges.md),
[bitmap/LCS/float limits](string-algorithms.md), [multi-key string limits](multikey-strings.md),
[typed collections](collections.md), [HyperLogLog](hyperloglog.md),
[transactions](transactions.md), [security](security.md),
[persistence](persistence.md) and [pub/sub](pubsub.md)
for grammar,
clocks, rounding, integer limits, resource bounds, queue isolation and atomic
error behavior. Conditions are atomic and
case-insensitive. Option tokens must match completely; NULs/prefixes are rejected.
A bounded stream subset supports XADD with NOMKSTREAM/MAXLEN/MINID retention,
XDEL, XLEN, XGROUP CREATE/SETID/DESTROY/
CREATECONSUMER/DELCONSUMER, XREADGROUP GROUP with COUNT/NOACK for new entries on
one or more streams with new-entry and history reads, XACK, XPENDING summary/extended,
XRANGE/XREVRANGE, non-blocking and blocking XREAD over multiple streams,
XCLAIM/XAUTOCLAIM, XTRIM, XSETID, blocking group reads, and XINFO STREAM (with
FULL)/GROUPS/CONSUMERS. See [stream limits](streams.md). All types share expiry, payload
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
