# Benchmarking from the first increment

The objective is to outperform current Redis/Valkey implementations without
unsafe project code or external Rust dependencies. This document defines how to
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
- `smoke.json`: short, reply-validated end-to-end smoke used on every PR/main push.
- `environment.txt`: source revision, dirty-tree status, `rustc -vV`, OS and CPU.

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
competitor before it is timed. The recipe verifies process liveness/readiness and
cleans up all child processes on success, error or interruption.

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
profile; these files and environment logs are downloadable artifacts. These are
logical-CPU budgets, not dedicated physical cores: SMT siblings and shared-VM
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
  per-client warmup are outside the timer.
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
6. Measure memory/RSS and server CPU separately. These are not yet measured by the
   Rust tool, and payload accounting is not a substitute for RSS.
7. Introduce performance budgets only after stable dedicated-runner baselines.
   Shared GitHub runners are correctness/smoke and trend evidence, not a reliable
   source of hard throughput pass/fail thresholds.

## Current architecture and next optimization decisions

The initial server uses a capped, managed blocking worker per client, `TCP_NODELAY`,
a borrowing parser, buffered pipeline replies and a single `RwLock<HashMap>`.
The accept loop sleeps for up to 1 ms when idle; connection setup is untimed.
GET uses shared locking; SET/DEL use exclusive locking. Locks are released before
network I/O; multi-key DEL/EXISTS are atomic with respect to other commands.

This is a correctness/performance baseline, not the final scalability design.
Contention, per-request argument allocation, value copies, and thread-per-client
scaling are candidates to profile. Changing synchronization or I/O must retain
safe Rust, zero dependencies, command atomicity and all conformance tests.

References:
[Valkey benchmark pitfalls](https://valkey.io/topics/benchmark/),
[Redis benchmarking guidance](https://redis.io/docs/latest/operate/oss_and_stack/management/optimization/benchmarks/),
[Redis source build](https://github.com/redis/redis#build-redis-from-source),
[Valkey source build](https://github.com/valkey-io/valkey#building-valkey-using-makefile),
[taskset](https://man7.org/linux/man-pages/man1/taskset.1.html),
[Git diff no-index behavior](https://git-scm.com/docs/git-diff).
