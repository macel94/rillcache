# Policy and transport requirements

Part of the [requirements and incremental delivery index](../requirements.md). Requirements are preserved verbatim from the former monolithic EARS sections; add new requirements only in the matching area file.

## Policy and transport

- **R01 (ubiquitous):** The project shall compile with unsafe project code forbidden
  and only the reviewed rustls/Tokio/CPU-affinity dependency graphs, vendored
  and allowlisted across all targets/features. No application unsafe code or FFI is
  introduced by using their safe APIs.
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

## Default Unix Tokio plain-TCP execution (S-TOKIO-NET)

- **R-TOKIO-NET1:** When default plain TCP runs on Unix without persistence,
  ACL files or synchronous audit sinks, the server shall serve local sockets on
  runtime zero and route remote sockets through bounded channels to current-thread Tokio
  runtimes; socket reads and writes shall be async operations, not blocking
  calls or a wrapper around the former Mio connection loop. Fixed-worker mode,
  TLS, persistence/ACL-file/audit and portable non-Unix fallbacks remain available.
- **R-TOKIO-NET2:** When a route queue is full, acceptance shall apply bounded
  backpressure; the server shall not create an unbounded connection-task queue.
  Client admission, input-frame limits and response buffering shall remain bounded.
- **R-TOKIO-NET3:** While sessions use auth/ACL, transactions/WATCH, pub/sub,
  MONITOR, blocking commands, pause, or persistence, they shall continue using
  the same Store and Session behavior, ordering and accounting as other modes.
  Async socket waits shall not block a runtime thread.
- **R-TOKIO-NET4:** When idle timeout, peer cancellation, protocol error, SHUTDOWN,
  server-handle shutdown or worker failure occurs, connection registration and
  resources shall be released and every owned runtime worker shall be joined.
- **R-TOKIO-NET5:** When worker threads start, the server shall attempt safe
  best-effort CPU affinity when supported; failure shall leave scheduling to the
  OS and shall not prevent startup. Affinity shall not be described as guaranteed.
- **R-TOKIO-NET6:** Native TLS shall remain on its existing TLS accept/session
  path; only the default plaintext TCP path is migrated. Existing TLS wire and
  shutdown behavior shall continue to pass all-mode tests.
- **R-TOKIO-NET7:** When measured, metadata shall identify the Tokio default
  runtime. Performance remains subject to every original and four-CPU protected
  comparison; async architecture alone is not evidence of a throughput win.
- **R-TOKIO-NET8:** On the Unix default path, the accept loop shall share runtime
  zero with bounded local client tasks; only the remaining effective-CPU budget
  may start remote Tokio runtime threads. Append-only persistence, ACL files and
  synchronous audit sinks shall retain the non-Tokio fallback until both their
  blocking work and lock-held disk waits are isolated. ACL and paused resumption
  shall use one separately accounted, bounded, joined cold-I/O service; its
  permit shall precede retaining an extra
  request frame and shall survive waiter cancellation until the job finishes.
  EXEC shall retain the existing rejection of queued ACL session commands.
  Tests shall cover actual one-effective-CPU thread count, fallback selection,
  and healthy same-runtime GET progress during delayed cold service work.
- **R-TOKIO-NET9:** Acceptance, per-client tasks and routing shall remain bounded
  and fair under slow readers and sustained connections, while cancellation
  closes sockets before all local tasks and remote worker threads are joined.
  Async networking shall not move synchronous disk maintenance onto a Tokio
  runtime. Tests shall cover full local task/remote routing capacity, pending
  connection retention, healthy-client progress, byte backpressure, service
  saturation, failure and shutdown. The 64 KiB output batch threshold shall not
  be described as a hard limit on one atomic command reply or total RSS.

Implementation decisions, current shared-Store limits and target sequencing are
self-contained in [the transport increment](../async-transport.md).

## Legacy fallback admission waits (S-ACCEPTWAIT)

These requirements describe the managed-client fallback. Eligible default Unix
plain TCP uses S-TOKIO-NET; persistence/ACL-file/audit configurations and portable
platforms retain timeout-based fallback admission.

- **R-ACCEPT1 (state-driven):** While the default plain Linux acceptor is idle,
  it shall use a positive 10 ms kernel receive-timeout target before enabling
  blocking accept. Only safe standard-library socket-option operations on an
  owned duplicate may be used; the option handle shall perform no stream I/O or
  shutdown. If setup fails or the platform is unsupported, the existing
  nonblocking accept/backoff path shall remain available.
- **R-ACCEPT2 (event-driven):** When a socket is admitted, its configured client
  read/write timeouts shall still be installed before client I/O. Shutdown shall
  retain joined ownership, and maintenance shall retain its bounded work and
  100 ms scheduling target, checked between accept waits. Kernel timer rounding,
  accept-wait slack and descheduling preclude a hard real-time deadline. Fixed
  workers, TLS, foreground expiry/accounting and handoff epochs shall not change.
- **R-ACCEPT3 (ubiquitous):** Tests shall cover failed timeout setup without a
  blocking-mode change, timeout-before-mode ordering, positive timed empty
  accept, listener lifetime and independent accepted-client timeout restoration.
  All-mode TCP/shutdown/persistence tests remain mandatory. Idle syscall/CPU
  captures shall be separate from normal throughput evidence; neither replaces
  either unchanged protected performance gate.

## Explicit multithreaded execution

- **R60 (optional feature):** Where `--threads N` is supplied for N in 1..64,
  the server shall create exactly N managed I/O/execution workers, independent of
  client count. Without the option it shall use S-TOKIO-NET on Unix and the managed
  thread-per-client fallback elsewhere. In the legacy fallback, S21 may lend
  busy-client I/O between those existing per-client workers; it
  does not replace their admission/thread-count contract or add a permanent lane.
  The accept/control thread is additional; one worker is not one OS thread total.
- **R61 (state-driven):** While clients are idle, slow readers or submitting large
  pipelines, fixed workers shall retain bounded buffers, fair command/I/O quanta,
  connection limits and independent service to other clients, without unsafe code.
- **R62 (event-driven):** When a server stops or a worker fails, it shall close
  client sockets and join every started worker, including partial-start cleanup.
- **R63 (event-driven):** When `make test` or `make ci` runs locally or on GitHub,
  socket correctness shall run in default/readiness, one-worker and four-worker
  configurations, with concurrent clients and reply-validated mode benchmark smoke.
- **R64 (event-driven):** When embedded thread modes are benchmarked, results shall
  record the selected mode/worker count. Fixed worker options shall be rejected
  for external targets; mode and affinity shall not be conflated or used to claim
  linear scaling. Optional full comparisons shall preserve mode-scaling evidence;
  reports shall validate and display it separately from native competitor rows,
  reject corrupt/mismatched mode groups and retain historical cohort compatibility.

## Connection reset (S33)

- **R132 (event-driven):** When a valid `RESET` is received, the server shall
  discard the active transaction, clear watches, remove all channel/pattern
  subscriptions, return to RESP2, select database zero, clear the authenticated
  user and reply `+RESET`; it shall be available without prior authentication and
  from RESP2 subscriber mode. Client name/library metadata shall be retained.
- **R133 (unwanted behavior):** If `RESET` has extra arguments, it shall return the
  normal arity error without changing state. Tests shall cover queued transactions,
  watches, pub/sub cleanup, RESP3-to-RESP2 conversion, deauthentication and
  reauthentication, subscriber-mode reset and all execution modes.

## Payload budget eviction (S36)

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

## Runtime payload budget (S96)

- **R359 (event-driven):** `CONFIG GET maxmemory` (including pattern and
  wildcard matches) shall report the current positive payload byte budget, and
  `CONFIG SET maxmemory <positive integer>` shall update it atomically with other
  valid `CONFIG SET` pairs. Invalid, negative, non-numeric or zero values shall
  return an explicit error and leave both the budget and other pairs unchanged.
- **R360 (state-driven):** Applying a lower budget shall evict keys down to it
  using the active policy, with `allkeys` and eligible `volatile` candidates only.
  When the policy cannot evict enough (`noeviction`, or volatile without TTL
  keys), the server shall remain above the new budget and refuse later
  over-budget admissions with OOM rather than deleting ineligible keys. Raising
  the budget shall take effect immediately for later admissions.
- **R361 (ubiquitous):** All-mode tests shall cover visibility, lowering with
  eviction, invalid/zero values, atomic rejection with another pair and every
  worker mode. Documentation shall retain that this is a key/value payload budget,
  not RSS/allocator memory, and that zero (unlimited) is deliberately rejected
  to preserve bounded operation.

## Protocol shutdown (S75)

- **R296 (event-driven):** `SHUTDOWN [NOSAVE|SAVE] [NOW] [FORCE]` shall accept the
  documented modifiers, reject unknown modifiers and incompatible combinations
  with a bounded syntax error, and return `No shutdown in progress.` for `ABORT`
  because no delayed shutdown state exists. Successful shutdown shall write no
  reply and close the calling connection.
- **R297 (state-driven):** Before stopping, SHUTDOWN shall force a journal flush
  when append-only persistence is enabled and only then request process stop. A
  flush failure shall return a bounded error and leave the server running.
  `NOSAVE`/`SAVE` shall not write an RDB file; `NOW`/`FORCE` shall be accepted
  no-ops because shutdown is immediate.
- **R298 (event-driven):** A shutdown request shall stop accepting clients, wake
  and join existing workers, close paused/blocked connections, and return success
  from the server loop in thread-per-client and fixed-worker modes. `SHUTDOWN`
  shall be rejected inside `MULTI` with dirty-transaction semantics.
- **R299 (ubiquitous):** Tests shall cover flag validation, `ABORT`, MULTI
  rejection, caller close, all worker modes, journal durability across restart,
  and documentation of the remaining absence of a portable SIGTERM handler.

## Client registry and administrative kill (S38)

- **R149 (event-driven):** When a connection is accepted, the server shall register
  its ID, peer and local addresses, creation time and control handle; it shall update name,
  library, protocol and authenticated user after CLIENT/HELLO/AUTH/RESET and remove
  the record on close. The registry shall be bounded by `max_clients` and process-local.
- **R150 (event-driven):** When `CLIENT LIST [TYPE normal|pubsub|master|replica|slave] [ID id ...]` is
  received, the server shall return newline-delimited records for matching
  connections with ID, peer and local addresses, name, age, N/P flags, db, live channel/pattern
  counts, RESP mode, user and library metadata, omitting fields it does not measure.
- **R151 (event-driven):** When `CLIENT KILL` receives intersecting `ID`, `ADDR`,
  `LADDR`, `TYPE normal|pubsub|master|replica|slave`, `USER` and `SKIPME yes|no`
  filters, the server shall shut down matching sockets and return the count,
  skipping the caller by default. Absent master/replica types shall match nothing.
  `SKIPME no` may kill the caller; its socket closes and the reply may race with
  that close. The legacy address form shall return `+OK`/`No such client`.
- **R152 (ubiquitous):** `INFO clients` shall report the current registry size with
  no fabricated cluster/blocked counts. All-mode tests shall cover metadata,
  RESP3/pubsub visibility, ID/TYPE filters, self protection, ID/address/legacy kills,
  socket closure, registry removal and permission gating. Documentation shall retain
  the missing fd/idle/command/buffer/memory/network fields and cross-process view.

## Client command pause (S72)

- **R284 (event-driven):** `CLIENT PAUSE timeout [WRITE|ALL]` shall parse a
  non-negative integer millisecond timeout, default to `ALL`, reject negative or
  non-integer timeouts and unknown modes, and reply `+OK`. `timeout` zero shall
  be a no-op. A later deadline shall win, and an active `ALL` pause shall not be
  downgraded by a later `WRITE` pause. `CLIENT UNPAUSE` shall clear the pause.
- **R285 (state-driven):** While a pause is active, new client commands shall be
  deferred per connection without holding the store lock and without parsing
  later pipelined commands. Under `WRITE`, read-only commands shall continue;
  commands with the write flag and `EXEC` shall wait. `CLIENT PAUSE` and
  `CLIENT UNPAUSE` shall remain exempt so management remains possible. Already
  executing or already-blocked commands, expiry/eviction and journal syncing
  shall not be treated as paused.
- **R286 (event-driven):** When the pause deadline expires or `CLIENT UNPAUSE`
  runs, each deferred command shall execute once and its reply shall keep wire
  order with following pipelined commands. Disconnect or shutdown shall drop a
  deferred command without executing it and shall not delay joined shutdown.
- **R287 (ubiquitous):** Tests shall cover `WRITE` read passthrough, deferred
  writes and `ALL` reads, deadline expiry, explicit unpause, zero timeout,
  self-pause/unpause, pipelining order, invalid arguments, disconnect/shutdown
  behavior and every worker mode. A zero-deadline client-pause query shall return
  without sampling wall time, verified by an injected counting clock through
  ordinary session dispatch. Nonzero-state deadline/mode semantics shall remain
  unchanged. Documentation shall retain the absence of replication,
  expiry/eviction pausing and replica-traffic pause semantics.

## Configured connection limit visibility (S82)

- **R319 (event-driven):** When `INFO clients` or `INFO all` is requested, the
  server shall report `maxclients` equal to the configured maximum connection
  limit, alongside `connected_clients` and `cluster_connections:0`.
- **R320 (ubiquitous):** Tests shall cover a custom and the default limit in
  every worker mode and `INFO all` inclusion. Documentation shall retain that the
  field reports configuration, not current capacity, blocked clients, per-client
  file descriptors or a cross-process view.

## Client kill by age (S83)

- **R321 (event-driven):** When `CLIENT KILL MAXAGE <milliseconds>` receives a
  non-negative integer, the server shall kill matching connections whose age is
  at least that value, intersecting the filter with `ID`/`ADDR`/`TYPE`/`USER`
  filters, returning the killed count and always skipping the caller.
- **R322 (ubiquitous):** Tests shall cover old connections being killed, a young
  connection surviving, caller protection, invalid `MAXAGE` values, the
  unchanged legacy form and every worker mode. Documentation shall retain the
  absence of fd/idle/buffer/memory metadata and a cross-process view.

## Client kill by local address (S89)

- **R339 (event-driven):** When a connection is accepted, the server shall record
  both its peer and local socket addresses. `CLIENT LIST` and `CLIENT INFO` shall
  expose the local address as `laddr=<ip:port>` without fabricating unmeasured
  fields.
- **R340 (event-driven):** When `CLIENT KILL LADDR <ip:port>` receives a valid
  address string, the server shall intersect it with the other supported filters,
  shut down matching non-caller sockets and return the killed count. All-mode
  tests shall cover matching and non-matching local addresses, CLIENT LIST/INFO
  exposure, combined filters, caller protection and the unchanged legacy form.

## Client types and SKIPME (S95)

- **R356 (event-driven):** `CLIENT LIST TYPE` shall accept `normal`, `pubsub`,
  `master`, `replica` and `slave`. Because this standalone server has no
  replication clients, the latter three shall return an empty result; unknown
  types shall retain the explicit error.
- **R357 (event-driven):** `CLIENT KILL TYPE` shall accept the same five values,
  with absent types intersecting to zero kills. `SKIPME yes` shall remain the
  default and always protect the caller; `SKIPME no` shall allow the caller to be
  killed, closing its socket even though the reply may race with the close.
  Unknown `SKIPME` values shall return a syntax error.
- **R358 (ubiquitous):** All-mode tests shall cover absent-type list/kill
  emptiness, `SKIPME no` targeting another client, caller self-kill and its
  cleanup, invalid filters, the unchanged legacy form and every worker mode.
  Documentation shall retain the absence of fd/idle/buffer/memory metadata and
  a cross-process view.

## Runtime connection limit (S97)

- **R362 (event-driven):** `CONFIG GET maxclients` (including wildcard and `*`
  results) shall report the live maximum connection count. `CONFIG SET maxclients
  <positive-integer>` shall update that limit atomically with the other valid
  `CONFIG SET` pairs. Zero, negative, empty and non-integer values shall return a
  bounded error and leave the limit unchanged.
- **R363 (state-driven):** Both the thread-per-client and fixed-worker accept
  paths shall enforce the live limit rather than the startup snapshot. Lowering
  the limit below the current admitted count shall not close existing clients;
  new sockets shall receive `ERR max number of clients reached` and increment
  `rejected_connections` until capacity is available. Raising the limit shall
  admit new connections without a restart.
- **R364 (ubiquitous):** Tests shall cover the default and wildcard reporting,
  live admission in every worker mode, lowering and raising, invalid values,
  existing-client survival and `INFO clients` reflecting the live value.
  Documentation shall retain that this is a connection count, not a
  file-descriptor, memory or cross-process capacity guarantee.

## Blocked-client visibility (S100)

- **R371 (event-driven):** When a connection enters any supported blocking
  command, the server shall mark it blocked in the client registry; when the
  command completes, times out or the connection resets/disconnects, the mark
  shall be cleared.
- **R372 (event-driven):** `INFO clients` and `INFO all` shall report a
  `blocked_clients` count derived from that registry state, and `CLIENT LIST`
  shall include `b` in the flags of blocked connections alongside `N`/`P`.
- **R373 (ubiquitous):** Tests shall cover a waiter becoming visible, wake-up
  clearing the count, timeout/disconnect/reset clearing, `CLIENT LIST` flags and
  every worker mode. Documentation shall retain that this is a process-local
  registry view with no fd/idle/buffer/memory metadata or cross-process claim.

## CLIENT UNBLOCK (S114)

- **R414 (event-driven):** `CLIENT UNBLOCK <client-id> [TIMEOUT|ERROR]` shall
  request a wake-up for a currently blocked connection and return `:1`; an
  unknown or non-blocked ID shall return `:0`. `TIMEOUT` (the default) makes the
  blocked command complete as a null array/value, while `ERROR` terminates it with
  `UNBLOCKED client unblocked via CLIENT UNBLOCK`. Invalid IDs, extra arguments
  and unknown modes shall return bounded errors without changing another client.
- **R415 (state-driven):** A request shall wake only the target connection, clear
  its blocked state and `CLIENT LIST`/`INFO clients` blocked marker, and be
  consumed once. A request for a completed, disconnected or reset connection shall
  be discarded rather than affecting a later command. RESP3 timeout wake-ups shall
  use the protocol null frame, and the wake-up shall occur within the existing
  blocked-client polling bound.
- **R416 (ubiquitous):** Tests shall cover `:1`/`:0`, TIMEOUT and ERROR modes,
  RESP3 null conversion, invalid input, connection isolation and every worker
  mode. Documentation shall retain that this is a process-local same-server
  control, requires the normal `CLIENT` permission and is not a cross-process or
  distributed unblock mechanism.

## Real redis-cli interoperability (S103)

- **R381 (event-driven):** When a pinned `redis-cli` connects over RESP2 or
  negotiates RESP3, the server shall complete the handshake and serve `PING`,
  `SET`, `GET`, `APPEND` and `CLIENT SETNAME` with the documented replies. Client
  metadata shall remain connection-local; a separate CLI connection shall not
  inherit another connection's name.
- **R382 (event-driven):** When the default user requires a password,
  `redis-cli --user default -a <password>` shall authenticate and receive normal
  replies; a missing password shall receive `NOAUTH` and an incorrect password
  shall receive `WRONGPASS` without reflecting the supplied secret.
- **R383 (ubiquitous):** The interop test shall use `RILLCACHE_REDIS_CLI` or a
  `redis-cli` on `PATH`, record its version, run in every worker mode, and skip
  visibly with a diagnostic when no client is available unless
  `RILLCACHE_REQUIRE_REDIS_CLI=1` is set. Language-client suites, richer protocol
  negotiation and nonzero database selection remain separate increments.
