# Benchmarking from the first increment

The objective is to outperform current Redis/Valkey implementations without
unsafe project code or external Rust dependencies outside the vendored rustls
exception. This document defines how to
collect evidence, not a claim that this first server already wins.

[Raw bootstrap comparison runs](../benchmarks/bootstrap/README.md) are preserved
with source checksums and explicit shared-host limitations. They validate the
harness, not a performance victory. The
[CI benchmark review](ci-benchmark-review.md) records actual GitHub results,
methodology defects found, and the resulting corrections.

## Fast local feedback

```sh
make test                  # unit, socket, benchmark-client and dependency tests
make ci                    # same command as GitHub; correctness gates, not timing gates
make bench                 # microbenchmarks and repeated embedded TCP measurements
```

Outputs under ignored `target/benchmarks/`:

- `core.jsonl`: parser/GET/SET and conditional-SET microbenchmarks, seven samples
  of 50,000 operations, median nanoseconds/op. Store replies are checked before
  and after timing; `std::hint::black_box` prevents dead-code elimination.
- `local.json`: repeated end-to-end TCP PING/SET/GET and conditional-SET measurements.
- `smoke.json`: short, reply-validated thread-per-client smoke on every PR/main push.
- `smoke-threads-1.json`, `smoke-threads-4.json`: the same five workloads with
  eight clients in explicitly single-worker and multithreaded modes.
- `thread-modes/threads-{1,4}.json`: repeated fixed-worker runs from
  `make bench-threads` / `make bench`, also preserved by comparative CI.
- `environment.txt`: source revision, dirty-tree status, `rustc -vV`, OS and CPU.

`make test` runs the full suite three times, selecting per-client, one-worker
and four-worker socket configurations. Standalone Linux tests verify the actual
named worker count through procfs, not just the configuration flag. `make ci`
uses this same matrix and all three reply-validated smoke modes on GitHub.
On Linux with taskset, `make check`/`make ci` additionally run the complete default
TCP suite on one allowed CPU, exercising real automatic client handoffs.

`--server-threads N` configures only embedded benchmark servers and is rejected
with `--addr`. Additive JSON `server_configuration` records mode/count, fixed-worker
`active_poll_us` / `run_quantum_us`, and per-client `single_reply_yield`,
`client_handoff_idle_us` / `client_handoff_policy_version` /
`client_handoff_max_epoch_ms`. V2 requires recent reply activity to create an
epoch, returns quiet borrowers independently and bounds a busy epoch to 100 ms
between existing bounded passes; it does not add a permanent I/O worker.
It serializes the actual immutable bind-time policy, not a later CPU resample;
it is `null` for an external server whose launcher owns configuration. Thread counts
are not CPU affinity budgets. Fixed workers use 64-command and bounded I/O
quanta, a bounded output queue and a 1 ms idle polling wait. After progress they
actively poll for at most 50 µs before returning to idle parking; empty passes do
not rearm the window. This trades active CPU work for fewer premature sleeps, not
kernel readiness or a free-CPU improvement. Multiple lanes/clients also request
a cooperative yield after 100 µs of active passes; progress does not reset that
separate budget. Lone clients/single lanes avoid periodic fairness yields.
Blocking mode can yield after a single-command flush when its bind-time capacity
hint is one CPU and multiple clients are admitted. These are scheduling hints,
not correctness dependencies, hard deadlines or OS fairness guarantees. Compare
modes rather than assuming the portable scheduler is faster; see the
[kernel-guided follow-up](io-scheduling.md).
The four native comparison profiles still use managed per-client workers; the
new one-CPU [client handoff policy](client-io-handoffs.md) shares active I/O within
that same default mode. No benchmark-mode flag, worker-count or CPU-budget change
is hidden as a code speedup. Quiet sockets return to blocking owners. The
mode-scaling runs are separately validated and published as an embedded-mode
throughput/spread/batch-p99 table in the automatic report, not an extra
Redis/Valkey victory table. Their source metadata is retained beside the JSON;
failed reruns invalidate both mode files rather than mixing stale results.
New comparative workflows mark mode evidence as required, so missing/corrupt
mode groups fail report quality instead of silently disappearing; immutable old
cohorts without that marker remain readable. Run-budget evidence uses the
versioned markers: current `cooperative-v2 (required)` validates the serialized
`run_policy_version` and excludes explicit yield/park wait from the next interval.
Historical no-budget and v1 cohorts remain readable without relabeling.

Only Rust/rustup and GNU Make are needed for these targets. After the pinned
Rust toolchain is installed, the Cargo commands use `--locked --offline`.

## Compare native standalone servers

On Linux, install normal C/C++ build tools (`cc`, `c++`, GNU Make), Git, `curl`,
`tar` and `sha256sum`, then run:

```sh
make compare
make compare BENCH_ARGS='--requests 100000 --clients 8 --pipeline 1 --keyspace 1024 --value-size 64 --warmup 1000 --runs 3'
```

The first invocation downloads checksum-verified, version-pinned Redis and Valkey
source archives into `target/competitors/` and builds only their server targets.
The build uses upstream default optimization and allocator settings (jemalloc on
Linux), no extra modules or TLS. Subsequent invocations reuse those binaries.
These C projects are **optional external benchmark tools**, not Rillcache code,
Cargo dependencies, vendored sources in Git, or part of its release archive.
No global installations, containers or root privileges are needed. Each source
cache is extracted once and given isolated, empty Git metadata, without staging
or committing files. This prevents parent-repository discovery and Git's
outside-repository no-index diff fallback from changing generated version metadata
on every build. Archive identity comes from the pinned SHA-256, not this synthetic
metadata. CI verifies unchanged reference binary digests across all profiles.

Pins are in [`benchmarks/targets.mk`](../benchmarks/targets.mk): Redis 8.10.2 and
Valkey 9.1.2, checked against upstream latest stable releases during bootstrap.
They are reproducible pins, not a promise to remain “latest” forever. Before a
published performance claim, check upstream releases again, update version and
archive SHA-256 together, and preserve results from both old and new versions.
Checksum pinning detects changed downloads; it is not a claim of signature
verification or upstream authenticity beyond the HTTPS download source.

Optional overrides:

```sh
make competitors COMPETITOR_JOBS=4
make compare REDIS_SERVER=/absolute/path/to/redis-server VALKEY_SERVER=/absolute/path/to/valkey-server
make compare RILLCACHE_PORT=17379 REDIS_PORT=17380 VALKEY_PORT=17381
make compare SERVER_CPUS=0 CLIENT_CPUS=1-3 BENCH_ORDER='redis valkey rillcache'
make compare PROFILE=1 PROFILE_IDLE_MS=1000
```

CPU pinning is optional and requires `taskset` (util-linux). Select CPUs allowed
by your host/cgroup, not blindly the example's 0–3. Child server/client threads
inherit the configured affinity. Requested and actual affinities and server order
are recorded; the JSON CPU count reflects the benchmark client's available CPUs.
`BENCH_ORDER` must name all three servers exactly once. The default leaves both
CPU masks inherited and uses `rillcache redis valkey` order.

Each server runs as a standalone native process on the same host and loopback TCP
(ports 16379/16380/16381 by default). Redis/Valkey are started without a config
file, with RDB/AOF disabled, no modules loaded and default I/O-thread settings.
All servers get the same Rust client, keys, bytes, request count, concurrency,
pipeline, warmup and run count. Wire conformance is checked against each
competitor before it is timed, in all three Rillcache execution modes. The
launcher supplies both `RILLCACHE_REFERENCE_ADDR` and `RILLCACHE_REFERENCE_KIND`
(`redis` or `valkey`). A separate GETEX error-edge test exposes their known
vendor difference instead of claiming identical behavior where they disagree. The recipe verifies process liveness/readiness and
cleans up all child processes on success, error or interruption. Only **one
comparison server is alive at a time**: its peers' idle polling/cron threads must
not contend for the measured CPU budget.

Results and logs: `target/benchmarks/comparison/`. Its `environment.txt` records
exact binary versions, checksums and configuration/client arguments in addition
to the compiler/hardware/source metadata. Files are overwritten on the next run;
copy the directory to preserve a baseline. Old JSON/log files are invalidated
before a rerun, so failures cannot masquerade as another server's old results.

The scheduled/manual workflow preserves four profiles: `pipeline-1`,
`pipeline-16`, `single-core-pipeline-1`, and `single-core-pipeline-16`. Both
single-core profiles place every server on the same one allowed logical CPU and
clients on the remaining allowed CPUs. The default-threading profiles leave
scheduling inherited. Order rotates across profiles. Each uses three repetitions,
100,000 commands at pipeline 1 or 1,000,000 at pipeline 16 to reduce short-sample
noise. Pinned binaries are built once and their checksums checked after every
profile; these files and environment logs are downloadable artifacts. Profiling
is enabled for these four CI profiles, including a one-second idle sample per
server. These are logical-CPU budgets, not dedicated physical cores: SMT siblings and shared-VM
resources can still contend, so they do not replace controlled-hardware tests.

For an already-running disposable server:

```sh
cargo run --release --locked --offline --bin rillcache-bench -- --addr 127.0.0.1:6379 --label my-build --requests 100000 --clients 8 --pipeline 16 --runs 3 > result.json
```

**Never point the tool at production.** It writes `rillcache:bench:*` keys and does
not flush the database. The optional wire-conformance test also writes a small,
reserved `rillcache:conformance:*` namespace. No AUTH/TLS is supported yet.

## What exactly is measured?

- Workloads: PING, SET, GET, `set-nx-existing` and `set-xx-existing`; `all` includes
  all five. Conditional workloads deliberately use seeded existing keys: NX
  rejects every write with a null bulk; XX admits every overwrite with `+OK`.
  They measure different paths and must not be blended into one throughput score.
  Successful NX creation and missing-key XX rejection also have correctness tests;
  these are not the steady-state conditional TCP benchmark workloads.
- Deterministic, cyclic key rotation, not random/Zipfian traffic. Every value is
  the requested number of `x` bytes, including a possible zero-length value.
- GET and both conditional workloads seed all keys with acknowledged SETs and
  verify them via GET before timing. Conditional workloads also verify the
  resulting data after timing, including when only some keys are accessed.
  Values are deliberately identical on repeated writes; tests with different
  bytes separately verify whether conditional writes actually preserve/replace.
- Connections, seed/verification traffic, thread creation and configurable
  per-client warmup are outside the timer. Every connection first completes a
  validated PING, even with `--warmup 0`: a completed TCP handshake alone does not
  prove the server has accepted it or created its worker. Connections are retained
  through final accounting and closed outside the timer, before post-run checks.
- Each client has its own thread and connection. Exactly `--requests` operations
  are divided across clients, including non-divisible counts and partial batches.
- Every reply is checked byte-for-byte, including GET payloads. Wrong replies,
  timeouts, disconnects, worker failures and incomplete counts fail the benchmark.
  A JSON result is emitted only if the entire invocation succeeds.
- Throughput is completed commands / wall time across all clients. Client buffer
  copying, socket calls, response checking, scheduling and controller/join overhead
  contribute to measured time. This is an end-to-end measurement, not server-only
  CPU time. The client itself can be the bottleneck.
- p50/p95/p99/max are nearest-rank **whole-batch completion latencies**, including
  response validation. A pipeline of 16 does not produce 16 independent latency
  samples. Do not divide batch p99 by pipeline length and call it request p99.
  Partial final batches also contribute one batch sample.
- `--addr` selects an external server; otherwise an embedded local server uses
  the same TCP path. Embedded results are useful for regressions, but **only the
  standalone `make compare` results** should be used for cross-server comparisons.
- The JSON includes schema/tool version, label, mode, address, OS/architecture,
  CPU count and all workload parameters. Keep the environment metadata beside it.

Example parameter sweeps:

```sh
make bench BENCH_ARGS='--requests 100000 --clients 1 --pipeline 1 --keyspace 1 --value-size 64 --warmup 1000 --runs 5'
make bench BENCH_ARGS='--requests 100000 --clients 8 --pipeline 16 --keyspace 10000 --value-size 1024 --warmup 1000 --runs 5'
```

Resource guards: requests 1..10,000,000, clients 1..128 and no more than requests,
pipeline 1..65,536, keyspace 1..1,000,000, value size 0..1 MiB, warmup
0..1,000,000 per client and runs 1..100. Estimated encoded templates must fit in
64 MiB and a batch in 16 MiB; reduce sizes if validation rejects a combination.

## Optional Linux server accounting

```sh
make compare PROFILE=1 PROFILE_IDLE_MS=1000
# Or supply the PID of an already-running, disposable loopback server:
./target/release/rillcache-bench --addr 127.0.0.1:6379 --server-pid "$server_pid" --profile-idle-ms 1000 > profiled.json
```

Profiling is off by default; ordinary embedded/remote benchmarks remain portable.
`--server-pid` requires Linux procfs, an explicit loopback address and a separate
process. The caller must supply the PID actually serving that endpoint; the tool
checks PID/start-time identity and liveness, **not socket ownership**.
`make compare` supplies its owned child PID. `--profile-idle-ms` is 0..10000 (default 0)
and a nonzero value requires profiling; the Make override defaults to 1000 ms.
The idle sample precedes benchmark connections; an otherwise isolated, disposable
server is essential. Existing keys/background work are not automatically removed.

Schema 1 gains additive fields: top-level `idle_profile` and each result's
`server_profile`; either is `null` when disabled. Each profile contains:

- Server PID and kernel process-start ticks, identifying the sampled lifetime.
- Process-wide user/system CPU tick deltas from `/proc/PID/stat`, **excluding
  child processes** but including exited server threads. Clock frequency comes
  from the kernel's native-width/endian `AT_CLKTCK` in `/proc/self/auxv`; it is
  never guessed and no C API, FFI or external collector is used.
- `observation_seconds` and `cpu_percent_one_logical_cpu`: total observed server
  CPU seconds / observation wall time × 100, **not** divided by available CPUs.
  Multithreaded use can exceed 100%. Tick quantization can produce zero on short
  samples or percentages slightly above an affinity budget; zero is not proof of
  free CPU. Use longer runs, not smoke timings, for CPU conclusions.
- `rss_bytes_before/after` and `lifetime_peak_rss_bytes` from `VmRSS`/`VmHWM`.
  Linux `kB` is converted to bytes with checked multiplication. These are
  approximate kernel RSS estimates, not PSS, allocator totals, a per-workload
  peak, continuously sampled memory, or the configured key/value payload budget.
  Lifetime high water includes startup, seed/warmup and previous workloads.
- Voluntary/involuntary context-switch deltas summed from
  `/proc/PID/task/TID/status`, matched by **TID and thread start time**. The leader's
  status alone omits other threads. Scope is always `matched_thread_lifetimes`;
  thread/sample/match counts and `thread_set_unchanged` expose endpoint churn or
  sampling races. Missing/exited threads are not invented as zero. Even unchanged
  endpoints cannot detect a thread created and destroyed entirely between them.

The collector takes endpoint snapshots outside the request timer, after client
warmup and before the start gate, then after every client thread is joined but
**before its socket is dropped**. Thus server client-thread counters remain
observable. CPU/RSS/context files are sequential, non-atomic reads; the separate
observation window brackets the timed workload and includes some sampling/gate/
join overhead and idle server activity. Do not divide CPU usage by a different
window or treat these coarse counters as function-level profiling. Profiling can
perturb caches/scheduling even outside the timer; compare matching modes and
cross-check against an independent collector before attributing small differences.

Missing/denied/malformed accounting, target exit/PID reuse or decreased counters
fail the invocation without emitting JSON. A task disappearing during sampling
instead marks partial endpoint coverage. Nothing reads command lines/environment
or emits raw auxiliary-vector values/addresses; no root privileges are required
when procfs policy permits access to the server's accounting.

Sources: [stat](https://man7.org/linux/man-pages/man5/proc_pid_stat.5.html),
[status](https://man7.org/linux/man-pages/man5/proc_pid_status.5.html),
[task directories](https://man7.org/linux/man-pages/man5/proc_pid_task.5.html),
[auxv format](https://man7.org/linux/man-pages/man5/proc_pid_auxv.5.html),
[AT_CLKTCK](https://man7.org/linux/man-pages/man3/getauxval.3.html),
[Linux procfs accounting caveats](https://docs.kernel.org/filesystems/proc.html).

## How to make a defensible performance claim

1. Use an idle, controlled machine; record CPU model, governor, affinity, kernel,
   compiler and exact source revision. Never compare a dirty tree as a release.
   Tiny smoke numbers cannot establish a regression or a performance victory.
2. Match functionality, durability and memory conditions. A small cache-only
   subset is not equivalent to Redis/Valkey's complete feature set.
3. Test both latency-oriented (pipeline 1) and throughput-oriented configurations,
   multiple clients/keyspaces/value sizes, misses and write-heavy/mixed traffic.
   The current tool does **not** implement mixed traffic or a configurable miss
   ratio yet; those require new stories and correctness tests.
4. Normalize/record the CPU budget and allocator, tune current upstream versions
   fairly (including their I/O threads), profile both client and server, and run
   separate-host clients when necessary to find client-side saturation.
5. Repeat, alternate server order, preserve raw runs, and report spread, not just
   the best number. The Make recipe accepts explicit order and CI rotates it;
   full counterbalancing and frequency/thermal control are still needed before
   publishing a general comparison.
6. Measure memory/RSS and server CPU separately with `PROFILE=1`, cross-check the
   collector independently, and inspect coverage/quantization. Endpoint RSS and
   payload accounting are not continuously sampled memory or allocator profiles.
7. Introduce performance budgets only after stable dedicated-runner baselines.
   Shared GitHub runners are correctness/smoke and trend evidence, not a reliable
   source of hard throughput pass/fail thresholds.

## Current architecture and next optimization decisions

The initial server uses a capped, managed blocking worker per client, `TCP_NODELAY`,
a borrowing parser, buffered pipeline replies and a single `RwLock<HashMap>`.
The accept loop sleeps for up to 1 ms when idle; connection setup is untimed.
S21 adds one-CPU busy-client I/O leases between those existing threads; there is
no new permanent poller and quiet sockets return to blocking readers.
GET uses shared locking; SET/DEL use exclusive locking. Locks are released before
network I/O; multi-key DEL/EXISTS are atomic with respect to other commands.

This is a correctness/performance baseline, not the final scalability design.
The [profile-guided hot-path investigation](hot-paths.md) records concrete
allocation/lookup/encoding reductions, the bounded fixed-worker policy and an
optional matched before/after workflow with raw evidence and readable tables.
Contention, mixed TTL/traffic, client saturation and thread-per-client scaling
still need profiling. Changing synchronization or I/O must retain safe Rust,
the rustls-only dependency exception, command atomicity and all conformance tests.

References:
[Valkey benchmark pitfalls](https://valkey.io/topics/benchmark/),
[Redis benchmarking guidance](https://redis.io/docs/latest/operate/oss_and_stack/management/optimization/benchmarks/),
[Redis source build](https://github.com/redis/redis#build-redis-from-source),
[Valkey source build](https://github.com/valkey-io/valkey#building-valkey-using-makefile),
[taskset](https://man7.org/linux/man-pages/man1/taskset.1.html),
[Git diff no-index behavior](https://git-scm.com/docs/git-diff).
