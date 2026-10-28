# Delivery and performance requirements

Part of the [requirements and incremental delivery index](../requirements.md). Requirements are preserved verbatim from the former monolithic EARS sections; add new requirements only in the matching area file.

## Development and delivery

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

## Performance evidence

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
  external Rust dependencies outside the rustls exception. Benchmarking/inference jobs shall not have repository
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

## Thin container delivery (S14–S15)

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
  RSS, missing authentication/transport hardening and non-graceful process-termination limitations
  explicit. A single replica/Service shall not imply persistence, replication or HA.

## Profile-guided hot-path delivery acceptance (S16–S18)

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

## Profile-guided scheduling and ready I/O (S19–S20)

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

## Single-CPU client I/O handoffs (S21–S22)

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

## PR-only main and scoped Valkey merge acceptance (S23–S24)

- **R95 (ubiquitous):** Every future change to main shall come through an up-to-date
  PR with passing distinct Tests and offline CI and Valkey performance gate checks
  from GitHub Actions. Active rules shall have no bypass actors, block deletion/
  force pushes, require squash merges and resolve review threads. CI/policy files
  shall require code-owner review; ordinary implementation PRs need no blanket
  approving-review quota.
- **R96 (event-driven):** When a PR benchmark runs, it shall use the protected base
  harness, frozen Rust client/compiler and checksum-pinned Valkey, default server
  modes, three alternating native repetitions, both CPU budgets and both pipeline
  lengths across five workloads. The exact [20-condition policy](../pr-quality-gate.md)
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
