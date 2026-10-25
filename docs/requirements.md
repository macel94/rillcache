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
  versions. Shared-host CI timings shall not be treated as stable performance gates.
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
  post-progress poll window and idle parking, and reset after parking. A single
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

## Compatibility boundary (v0.1)

Supported: RESP2 nonempty arrays of non-null bulk strings, case-insensitive ASCII
command names, binary keys/values, multiple connections and ordered pipelines;
`PING`, `ECHO`, `SET` with NX/XX, GET, KEEPTTL, EX/PX/EXAT/PXAT; `GET`, `DEL`,
`EXISTS`, `QUIT`; SETEX/PSETEX/SETNX/GETSET/GETDEL/GETEX; TTL/PTTL/EXPIRETIME/
PEXPIRETIME, EXPIRE/PEXPIRE/EXPIREAT/PEXPIREAT with NX/XX/GT/LT, PERSIST;
INCR/DECR/INCRBY/DECRBY. See [string semantics](strings.md) for grammar, clocks,
rounding, integer limits and atomic error behavior. Conditions are atomic and
case-insensitive. Option tokens must match completely; NULs/prefixes are rejected.

Deliberately not supported: inline/telnet commands, empty/null command arrays,
RESP3/HELLO, digest/value-match SET conditions, other data types, SELECT,
AUTH/ACL/TLS, persistence, replication, clustering, transactions, pub/sub and
streams. Unsupported SET options return `ERR syntax error` and do not write.
Redis normally ignores empty request arrays; this strict initial subset rejects
them. Unknown-command and unsupported-expire-option errors omit arbitrary input bytes
rather than reflecting them into RESP error frames. Limits are deliberately smaller than Redis defaults.

Default limits: 128 clients, 30-second socket inactivity timeout, 1 MiB bulk,
2 MiB complete request frame, 1024 arguments, 64 MiB key/value payload. Payload
accounting excludes map capacity, allocator and connection/thread buffers.
The CLI uses loopback by default. Do not expose it to untrusted clients.

## Next independently deliverable stories

Not implemented or implied by the completed increments:

1. RESP3/HELLO and common client handshake metadata, with real client compatibility tests.
2. Authentication/ACL and a safe transport-security deployment boundary; durability
   with crash/recovery and Redis-format interoperability tests.
3. Richer data types, multi-key strings, transactions, scripts, pub/sub, streams,
   replication and clustering: follow the [production-readiness plan](production-readiness.md).
   NX plus TTL alone is not a complete distributed-lock protocol; token-safe
   release, failover and fencing semantics are not implemented.
4. Continue investigating the single-core, low-pipeline and tail-latency weakness:
   extend S16–S18's [profile-guided work](hot-paths.md) with mixed TTL/traffic and
   client/server saturation evidence, using PING as a no-key-map control. Evaluate
   a safe readiness boundary before speculative sharding; retain idle CPU,
   backpressure, shutdown and multi-key atomicity gates. Unsafe project code and
   external Rust dependencies remain forbidden.
5. Controlled-hardware performance budgets and repeated current-release comparisons;
   later, richer types, pub/sub and streams driven by product judgment.

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
