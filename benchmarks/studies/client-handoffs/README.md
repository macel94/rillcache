# Default client I/O: closing the Valkey gap

**Final measured source:** [`0157177`](https://github.com/macel94/rillcache/commit/01571773e4aaca4c1765a226b1f7f1187e1fd33e).
Baseline: `6cea8cc`. Safe Rust, zero Rust dependencies, the normal release profile,
default managed per-client thread counts and benchmark CPU budgets are unchanged.

## Result in the latest reference CI cohort

The [validated current-reference report](../../reports/40245955560-1.md) has
**higher median throughput and lower whole-batch p99 than Valkey in all 20 tested
conditions**: five workloads × pipelines 1/16 × inherited/equal-one-CPU budgets.
This is a statement about that cohort, not all commands, traffic, hardware or
feature-equivalent Redis replacement performance. Inherited-budget Rillcache
uses multiple CPUs; those headline rates are not equal-per-core wins.

The harder **equal-one-logical-CPU** results are below. Server CPU and client CPUs
are disjoint. These are medians of three runs; batch p99 is not divided by the
pipeline length or pooled across runs.

### Pipeline 1 — equal one CPU

| Workload | Rillcache cmd/s | Valkey cmd/s | Rate gain | Rillcache batch p99 µs | Valkey batch p99 µs |
| --- | ---: | ---: | ---: | ---: | ---: |
| PING | 66551 | 61526 | +8.2% | 153.2 | 179.0 |
| SET | 63852 | 57685 | +10.7% | 159.3 | 191.2 |
| GET | 65143 | 59232 | +10.0% | 156.2 | 186.0 |
| NX-existing | 64525 | 59029 | +9.3% | 158.8 | 187.0 |
| XX-existing | 63762 | 57538 | +10.8% | 159.3 | 192.4 |

### Pipeline 16 — equal one CPU

| Workload | Rillcache cmd/s | Valkey cmd/s | Rate gain | Rillcache batch p99 µs | Valkey batch p99 µs |
| --- | ---: | ---: | ---: | ---: | ---: |
| PING | 1017672 | 755060 | +34.8% | 159.2 | 237.4 |
| SET | 845149 | 546508 | +54.6% | 190.8 | 336.0 |
| GET | 944431 | 621557 | +51.9% | 172.4 | 293.3 |
| NX-existing | 906362 | 601078 | +50.8% | 178.5 | 302.5 |
| XX-existing | 814324 | 534072 | +52.5% | 194.6 | 344.1 |

Eight clients, 1024 cyclic keys, 64-byte values, 1000 warmup commands/client;
100000 requests/workload/run at pipeline 1, 1000000 at pipeline 16. References:
Redis **8.10.2**, Valkey **9.1.2**, rechecked against upstream stable releases.
The complete report preserves spreads, Redis, CPU/RSS, idle and context coverage.
Its genuine **gpt-6-luna** assessment uses the workflow token and discloses the
automatic selection fallback. AI interpretation covers that reference cohort,
**not the separate A/B studies or local diagnostics**.

[Permanent raw reference inputs](reference-0157177/archive.json) retain 180 native
records, 12 idle samples and 30 separately validated embedded records. They passed
the Rust validator independently and regenerated the published measurements
byte-for-byte. An ordinary offline integration test checks this and wrong-source
rejection; publication artifacts also matched report/manifest bytes exactly.

## What changed, and why

The [previous reference cohort](../../reports/40230615995-1.md) had default
single-CPU pipeline-1 rates roughly 7.8–9.5% below Valkey, and worse pipeline-16
batch tails. PING ruled out the key map as the whole explanation. Fresh scoped
kernel profiles and syscall counts found receive-wait/scheduler work and roughly
one scheduling yield per short response, not a reason to weaken hashing or
remove timeouts. [Address-free diagnostics](diagnostics/kernel-summary.json)
remain separate from native timing; no host-global firewall/scheduler/perf policy
or unrelated process affinity was changed.

On a bind-time one-CPU hint, busy clients can briefly lend sockets/buffers to an
**existing** managed client thread while their own threads wait. Active I/O uses
the tested bounded fixed-worker parser/flush logic, amortizing scheduling across
a burst. No permanent worker is added, no benchmark mode is switched, and all
storage/multi-key/TTL/accounting semantics remain unchanged.

V2 starts a new epoch only after recent reply activity within 50 µs, or joins an
active epoch. Quiet borrowers return individually to blocking readers; the
coordinator keeps its own socket serviced. Epochs return all ownership after
50 µs without progress or a separate 100 ms maximum between bounded passes.
Thus busy peers cannot indefinitely retain a closed coordinator's admission slot.
Returned ready input is parsed before another read; partly sent replies retain
only their unsent suffix. Cloned-handle options are restored before owner I/O
resumes. One outstanding loan/client bounds channels; RAII handles late sends,
failed sends, cancellation and unwinding without stranded waiting workers.

See [design, costs and acceptance criteria](../../../docs/client-io-handoffs.md).
This is bounded active polling, not epoll, free CPU or a hard latency guarantee.
Unknown/multicore hints, explicit fixed modes and initially lone clients retain
their existing paths. At most one extra handle and bounded channel metadata are
needed per loan; default admission/thread-count bounds remain intact.

## Matched before/after evidence

- **Final v2:** [all 60 conditions](0157177/summary.md), [manifest/digests/order](0157177/manifest.json), [environment](0157177/environment.txt),
  [run 40245953698](https://github.com/macel94/rillcache/actions/runs/40245953698).
- **Initial v1:** [all 60 conditions](9062847/summary.md), [manifest](9062847/manifest.json),
  [its separate reference report](../../reports/40243686729-1.md).

Each A/B study has 360 native records in 72 JSON files: five workloads,
per-client/one/four-worker modes, inherited/one-CPU budgets, pipelines 1/16,
three alternating-order repetitions, the same frozen baseline client and normal
release builds. Every hash/record/order entry was independently checked and both
60-row summaries regenerated byte-for-byte. The two studies and the reference
cohort ran on **different runners**: compare before/after inside a cohort, not
absolute rates or the difference between v1/v2 percentage gains.

Final v2 default/one-CPU rates improved **14.2–18.0% at pipeline 1** and
**18.2–30.4% at pipeline 16** versus the frozen baseline, with lower batch p99
across those workloads. Representative SET results:

| Matched condition | Before → after cmd/s | Before → after batch p99 µs | Before → after CPU µs/cmd |
| --- | ---: | ---: | ---: |
| Default, one CPU, pipeline 1 | 55480 → 63947 (+15.3%) | 184.7 → 159.9 | 18.00 → 15.50 |
| Default, one CPU, pipeline 16 | 695291 → 822791 (+18.3%) | 372.6 → 195.1 | 1.44 → 1.21 |

Matched-thread SET switches/command fell **1.02330 → 0.03115** at pipeline 1 and
**0.06409 → 0.00247** at pipeline 16. These cover matched lifetimes, not guaranteed
process totals. CPU counters are quantized windows bracketing request timers,
not exact request-only costs. Fewer switches are supporting observations, not a
standalone proof of wall-time cause or a free-CPU claim.

### Regressions are retained

The final A/B cohort is **not an all-condition improvement**. Default/inherited
pipeline-16 GET fell **3.4%** (1857104 → 1793378); its batch p99 rose 161.3 →
179.9 µs. Inherited default PING pipeline-16 p99 rose 135.4 → 175.4 µs; inherited
SET pipeline-1 rate fell 1.3%, with p99 147.6 → 159.4 µs. Four-worker/inherited SET
pipeline-1 rate fell 3.9% and p99 rose 416.1 → 1051.8 µs. Other fixed-mode/tail
regressions remain in the complete table, even where handoffs are disabled.
Three shared-host runs cannot establish their exact cause or dismiss them as
noise. They remain profiling targets, not hidden by the winning reference cohort.

## Sparse and lifecycle controls

The first local paced diagnostic exposed extra sparse handoff work. V2 avoids
creating a lease for an isolated reply and returns quiet borrowers independently.
[Separate sparse syscall captures](diagnostics/sparse-syscalls/manifest.json)
after setup used 100 paced PINGs with two idle peers: baseline/v1/v2 recorded
**0 / 86 / 0** extra socket-handle duplication (`fcntl`) operations and
**0 / 172 / 0** socket-mode (`ioctl`) operations.
Tracing changes activation opportunities; these are instrumented counts, not
native speed or per-request cost measurements.

[Raw paced v1](diagnostics/paced-v1/manifest.json) and
[paced v2](diagnostics/paced-v2/manifest.json) diagnostics preserve the noisy and
slower samples too. Their highly variable VM latencies and coarse CPU windows,
which include idle time, support **no precise sparse speedup claim**. The locally
screened binaries and source labels/digests remain explicit; they are not clean
hosted release timing evidence. V2 local application bytes were matched to the
locally built committed source in the attached equivalence record.

Correctness includes all mode matrices, constrained default TCP, current-reference
wire checks, deterministic ownership/epoch/burst/quantum/partial-reply/cancellation
and unwind tests, 50 repeated ten-case ownership suites and 30 extra release TCP
suites. Native amd64/arm64 images passed fresh anonymous gates; the exact published
amd64 image and checksummed downloaded release were independently exercised in
all modes. Safe Rust, zero dependencies, buffer/admission limits, randomized
hashing, atomic writes, expiry accounting, backpressure and joined shutdown stay.

## Remaining performance work

Idle accept polling still wakes much more often than the references. Explicit
four-worker tails, mixed TTL/traffic, additional sizes/keyspaces, saturation and
controlled-hardware/separate-client replication still need investigation. A thin
image and a five-workload CI lead do not establish TLS, AUTH, durability, full
compatibility or [production readiness](../../../docs/production-readiness.md).
