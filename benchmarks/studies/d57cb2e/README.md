# Hot-path study: results and tradeoffs

[Tested source `d57cb2e`](https://github.com/macel94/rillcache/commit/d57cb2ed0e077eb7d33109d4deee989d534daa4c)
versus `3dca0c7` · [Successful native A/B run](https://github.com/macel94/rillcache/actions/runs/40208861043).
The archived [generated table](summary.md) is byte-identical to its artifact.
All 72 JSON hashes and 360 records were independently revalidated and the entire
60-row table regenerated before archiving. [Environment](environment.txt),
[manifest/binary hashes/order](manifest.json), and raw JSON are retained here.

The same frozen baseline Rust client measured eight clients, 1024 cyclic keys,
64-byte values, pipelines 1/16, 20,000/1,000,000 requests, 1000 warmups/client and
three repetitions. Both builds used the same toolchain/normal release profile.
Only one owned native server ran at a time. Shared-runner timings are not stable
performance gates or controlled-hardware guarantees. Modes are separate groups;
mode order was not fully counterbalanced.

## What improved—and what did not

- **Default per-client, inherited budget, pipeline 16:** SET 1,200,278 → 1,690,316
  commands/s (**+40.8%**), CPU 2.00 → 1.38 µs/command, whole-batch p99 261.6 →
  154.1 µs. Existing XX throughput rose **34.1%**.
- **Default per-client, one CPU:** all five median rates rose **0.8–2.9%** at
  pipeline 1 and **2.9–8.3%** at pipeline 16. Small deltas require controlled
  repetition; they do not establish a universal improvement.
- **One worker, one CPU:** store-workload pipeline-16 rates rose **7.7–14.7%**;
  batch p99 and sampled CPU/command also improved. This is an execution-mode
  result, not a direct comparison against Redis on another runner.
- **Four workers:** fewer premature 1 ms sleeps produced large throughput gains,
  but **four workers sharing one CPU worsened pipeline-1 batch p99**. SET was
  8,467 → 57,968 commands/s, while p99 rose **1,207.5 → 4,399.5 µs**. All five
  pipeline-1 tails in that group rose to roughly 4.4 ms; pipeline-16 tails also
  worsened. Per-command CPU often fell, but serving more commands increases active
  CPU use: bounded polling is not free CPU or kernel readiness notification.
- **Measured regressions remain visible:** default/inherited pipeline-1 existing
  XX fell **5.1%**, with p99 161.2 → 168.6 µs; default/inherited pipeline-16 GET
  fell **0.8%**, with p99 166.6 → 178.1 µs. Some other tails also worsened despite
  higher rates. Inspect every row and its spread, not only these selected results.

For a one-CPU deployment, start evaluation with `--threads 1`, not an assumption
that four workers improve latency: the candidate's one-worker SET group achieved
60,633 commands/s with 177.2 µs batch p99, versus 57,968/4,399.5 for four workers.
This cohort is not a universal tuning recommendation. The existing per-client
**default remains unchanged**; test real clients, sizes, TTLs and CPU quotas.

## Diagnostic instruction counts

Separate local captures monitor server **user-space retired instructions**, not
kernel work or wall time. Three alternating repetitions per build/workload,
500,000 requests, eight clients, pipeline 16, 1024 keys, 64-byte values and 100
warmups/client; the server had one logical CPU and clients other allowed CPUs.
Counts cover the **whole client invocation**, including setup/warmup/verification
and server background work, not only the request timer. They must not be divided
into a purported pure per-request cost. Normal release flags and overflow checks
were retained. [Manifest and 30 raw counter files](pmu/manifest.json) preserve
binary hashes, compiler, perf version, scope and 99%+ counter-running coverage.
Instrumented TCP timing was intentionally not copied into this archive.

| Workload | Before median user instructions | After median | Reduction |
| --- | ---: | ---: | ---: |
| PING | 263431606 | 179229804 | 31.96% |
| SET | 1423986554 | 775409436 | 45.55% |
| GET | 863258845 | 475587057 | 44.91% |
| NX existing | 904364191 | 648351430 | 28.31% |
| XX existing | 1511306492 | 857190254 | 43.28% |

## Current references and remaining bottleneck

The independent [Redis 8.10.2 / Valkey 9.1.2 report](../../reports/40208863128-1.md)
contains 180 native records, 12 idle samples and 30 separately validated embedded
mode records, plus genuine **gpt-6-luna** interpretation through the workflow
token (actual model disclosed; automatic selection fallback). That AI assessment
covers its own reference cohort, **not this separate A/B study**.

Default-mode equal-one-CPU SET remains **16.9% behind Redis at pipeline 1**
(49,392 versus 59,411 commands/s; batch p99 324.3 versus 185.6 µs). At pipeline 16
it is 13.5% ahead in rate but still worse in batch p99. Single-CPU pipeline-1
Rillcache CPU was over 93% system time for the store workloads, with about one
matched-thread voluntary context switch/command. This identifies I/O/scheduling
as the remaining major target; removing user-space allocations cannot remove
that kernel cost. These counters do not by themselves prove its exact cause.

Next work: native fixed-worker/reference comparisons on the same host and CPU
budget, explicit oversubscribed-worker tail/fairness experiments, kernel/client
profiling and a deliberate safe readiness boundary. Keep idle, shutdown,
backpressure and atomic multi-key guarantees; no unsafe shortcut, dependency,
weaker hashing or general production/performance claim follows from these results.

Validation: 116 tests per mode in debug/release, current-reference corpus in all
modes, 30 extra release TCP suites, package/checksum/link/workflow checks, native
amd64/arm64 container tests and credential-free public pulls. Image publication
is not deployment or production readiness. See [design and reproduction](../../../docs/hot-paths.md).
