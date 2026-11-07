# Current benchmark performance

Generated: 2026-11-06T00:03:04Z · [Benchmark run #16](https://github.com/macel94/rillcache/actions/runs/42132312138) · [Tested source `fd7dbd8`](https://github.com/macel94/rillcache/commit/fd7dbd81e8e5a02b5d4654e45960252a75aef0dd)

**Benchmark job: success. Data: validated. Copilot review: completed.**

Benchmark attempt: 1; review attempt: 1. Preferred model: `gpt-6-luna` via **GitHub Copilot**.

[This report's permanent copy](https://github.com/macel94/rillcache/blob/main/benchmarks/reports/42132312138-1.md) · [Report history](https://github.com/macel94/rillcache/tree/main/benchmarks/reports) · [Methodology](https://github.com/macel94/rillcache/blob/main/docs/benchmarks.md)

## AI review

Actual model(s): **gpt-6-luna**. Selection: automatic-fallback. Authentication: workflow-token.

AI interpretation, **not measurements or a performance gate**. Verify claims against the deterministic tables below.

> Bottom line: In this validated cohort, Rillcache led the tested SET workloads, but this is
> workload-specific evidence—not a general Redis or Valkey victory. At an equal one-logical-CPU
> budget, pipeline-1 throughput was 70,046 commands/s versus Redis’s 61,969 and Valkey’s 60,457;
> pipeline-16 was 857,181 versus 619,809 and 552,278. The corresponding whole-batch p99s were 142.8 µs
> versus 179.0/184.7 µs and 184.7 µs versus 313.1/354.9 µs. These are batch latencies, not
> per-request p99s; pipeline length cannot be used to divide them. Default-threading results are
> separate: SET pipeline-16 was 857,662 commands/s, not an equal-core comparison.
> 
> Resource observations: CPU was near 100% of one logical CPU in these tests; pipeline-16 SET reported
> 1.16 CPU µs/command for Rillcache versus 1.62/1.81 for Redis/Valkey. CPU counters use their own
> observation windows, and zero idle ticks are quantized. Approximate RSS endpoints for equal-core
> pipeline-1 SET were 5.57 MiB versus 8.68/8.36 MiB—not workload peaks or allocator accounting; VmHWM
> is a lifetime peak. Matched-thread context-switch rates were small, and thread endpoints were
> unchanged, but these counts are not guaranteed process totals and do not establish why throughput
> differs. NX-existing rejects writes; XX-existing admits overwrites.
> 
> Uncertainty: Results cover a small Redis-compatible subset, 1024 cyclic keys, 64-byte values, and
> three runs per workload on a shared VM. The 180 native records were validated, but the evidence does
> not establish behavior under different workloads, contention, or feature sets. Fixed-worker
> measurements are a separate group: four workers showed roughly 39k commands/s for SET and batch p99s
> around 5.8 ms, versus 721k and 972 µs with one worker; they are not native comparisons or
> equal-per-core evidence.
> 
> Next investigations:
> 
>  1. Diagnose and repeat the fixed-worker multi-worker SET/GET cases, recording process CPU,
> queueing, and lane-level progress; the throughput and batch-p99 gap is large and unexplained by
> these measurements.
>  2. Broaden native tests across key/value sizes, contention, and mixed workloads, with per-request
> latency distributions alongside batch p99, to test whether these SET results generalize.

## Measurements

**Data quality: complete.** 180/180 records validated across 4/4 complete profiles.


Hardware: AMD EPYC 7763 64-Core Processor. Rust 1.99.1 (58cf8f9c3 2026-10-13).

References: Redis 8.10.2 sha=00000000:0 malloc=jemalloc-5.3.0 bits=64 build=f1c4c8526bba86e1; Valkey 9.1.2 sha=00000000:0 malloc=jemalloc-5.3.0 bits=64 build=f5584e78f5ec61b0.

### SET at a glance

Median commands/s and median of the three runs' **whole-batch p99** values (µs). Positive rate delta means Rillcache is faster in this workload only.

| Profile | Rillcache cmd/s | Redis | Valkey | Rillcache vs Redis | Rillcache p99 µs | Redis p99 | Valkey p99 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| pipeline-1 | 70598 | 62128 | 60413 | +13.6% | 141.6 | 179.5 | 185.4 |
| pipeline-16 | 857662 | 622730 | 548973 | +37.7% | 183.9 | 311.0 | 356.9 |
| single-core-pipeline-1 | 70046 | 61969 | 60457 | +13.0% | 142.8 | 179.0 | 184.7 |
| single-core-pipeline-16 | 857181 | 619809 | 552278 | +38.3% | 184.7 | 313.1 | 354.9 |

CPU is relative to **one logical CPU**, so parallel servers can exceed 100%. Do not blend default-threading and single-core results or different pipelines. Shared VMs and unequal feature sets do not establish a general performance win or a cross-run regression.

<details>
<summary>pipeline-1: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 100000; clients: 8; pipeline: 1; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: inherited; client CPUs: inherited.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 73868 | 73494–74070 | 135.5 | 99.9 | 13.50 | 95.6 | 5.57 | 0.000010 | 0.000040 | unchanged |
| ping | redis | 65113 | 64629–65212 | 167.2 | 99.8 | 15.40 | 88.3 | 8.84 | 0.000050 | 0.000080 | unchanged |
| ping | valkey | 63419 | 63028–63470 | 172.2 | 99.5 | 15.70 | 87.9 | 8.22 | 0.000030 | 0.000060 | unchanged |
| set | rillcache | 70598 | 70128–70617 | 141.6 | 100.2 | 14.20 | 91.4 | 5.57 | 0.000010 | 0.000080 | unchanged |
| set | redis | 62128 | 61926–63984 | 179.5 | 100.0 | 16.10 | 82.1 | 8.85 | 0.000100 | 0.000050 | unchanged |
| set | valkey | 60413 | 60353–60691 | 185.4 | 100.2 | 16.60 | 80.1 | 8.22 | 0.000080 | 0.000060 | unchanged |
| get | rillcache | 72112 | 72077–72143 | 139.4 | 100.2 | 13.90 | 93.5 | 5.57 | 0.000010 | 0.000050 | unchanged |
| get | redis | 63392 | 63359–63471 | 172.9 | 99.5 | 15.70 | 86.6 | 8.85 | 0.000030 | 0.000060 | unchanged |
| get | valkey | 61941 | 61840–62023 | 178.1 | 99.7 | 16.10 | 83.9 | 8.22 | 0.000020 | 0.000060 | unchanged |
| set-nx-existing | rillcache | 71204 | 71152–71556 | 140.4 | 100.3 | 14.10 | 91.5 | 5.57 | 0.000010 | 0.000080 | unchanged |
| set-nx-existing | redis | 62533 | 62511–63018 | 176.3 | 100.0 | 15.90 | 84.9 | 8.85 | 0.000060 | 0.000060 | unchanged |
| set-nx-existing | valkey | 61605 | 61554–61719 | 179.1 | 99.8 | 16.20 | 84.0 | 8.22 | 0.000040 | 0.000040 | unchanged |
| set-xx-existing | rillcache | 70291 | 69814–70400 | 142.0 | 99.8 | 14.20 | 90.8 | 5.57 | 0.000020 | 0.000050 | unchanged |
| set-xx-existing | redis | 61591 | 61453–61882 | 180.3 | 100.2 | 16.20 | 82.7 | 8.85 | 0.000010 | 0.000050 | unchanged |
| set-xx-existing | valkey | 60238 | 60156–60426 | 185.3 | 100.3 | 16.60 | 79.0 | 8.22 | 0.000090 | 0.000060 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 4.36 | 4.36 | 99.0 | unchanged |
| redis | 1.001 | 0.000 | 8.65 | 8.65 | 10.0 | unchanged |
| valkey | 1.001 | 0.000 | 8.07 | 8.07 | 10.0 | unchanged |

</details>

<details>
<summary>pipeline-16: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 1000000; clients: 8; pipeline: 16; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: inherited; client CPUs: inherited.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 1147185 | 1145807–1155098 | 139.5 | 99.7 | 0.87 | 90.8 | 5.55 | 0.000001 | 0.000004 | unchanged |
| ping | redis | 830859 | 819339–844479 | 221.6 | 99.9 | 1.21 | 75.2 | 8.98 | 0.000003 | 0.000006 | unchanged |
| ping | valkey | 775782 | 774524–776129 | 241.7 | 100.0 | 1.29 | 64.3 | 8.18 | 0.000005 | 0.000004 | unchanged |
| set | rillcache | 857662 | 856780–881104 | 183.9 | 100.2 | 1.17 | 60.7 | 5.55 | 0.000001 | 0.000010 | unchanged |
| set | redis | 622730 | 621598–623608 | 311.0 | 100.4 | 1.61 | 65.4 | 8.98 | 0.000002 | 0.000005 | unchanged |
| set | valkey | 548973 | 548296–550148 | 356.9 | 99.8 | 1.82 | 44.2 | 8.17 | 0.000004 | 0.000007 | unchanged |
| get | rillcache | 992287 | 983081–995799 | 160.1 | 99.6 | 1.01 | 76.2 | 5.55 | 0.000001 | 0.000006 | unchanged |
| get | redis | 694674 | 693273–697190 | 274.5 | 100.0 | 1.44 | 62.8 | 8.98 | 0.000002 | 0.000005 | unchanged |
| get | valkey | 633015 | 631320–634445 | 302.3 | 100.6 | 1.59 | 63.9 | 8.17 | 0.000005 | 0.000004 | unchanged |
| set-nx-existing | rillcache | 926519 | 923996–941611 | 169.9 | 99.8 | 1.07 | 68.9 | 5.55 | 0.000002 | 0.000004 | unchanged |
| set-nx-existing | redis | 647028 | 646699–649487 | 298.8 | 100.3 | 1.55 | 47.7 | 8.98 | 0.000004 | 0.000004 | unchanged |
| set-nx-existing | valkey | 616060 | 614394–616613 | 313.2 | 99.9 | 1.62 | 64.2 | 8.18 | 0.000008 | 0.000005 | unchanged |
| set-xx-existing | rillcache | 831535 | 827646–859162 | 188.4 | 99.8 | 1.20 | 64.1 | 5.55 | 0.000001 | 0.000012 | unchanged |
| set-xx-existing | redis | 600521 | 599758–602293 | 323.8 | 100.0 | 1.66 | 57.8 | 8.98 | 0.000005 | 0.000007 | unchanged |
| set-xx-existing | valkey | 535464 | 531458–535684 | 366.1 | 100.2 | 1.88 | 36.9 | 8.18 | 0.000003 | 0.000007 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 4.44 | 4.44 | 99.0 | unchanged |
| redis | 1.000 | 0.000 | 8.76 | 8.76 | 10.0 | unchanged |
| valkey | 1.001 | 0.000 | 7.98 | 7.98 | 10.0 | unchanged |

</details>

<details>
<summary>single-core-pipeline-1: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 100000; clients: 8; pipeline: 1; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: 0; client CPUs: 1,2,3.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 73472 | 73293–73878 | 135.9 | 99.9 | 13.60 | 94.9 | 5.57 | 0.000010 | 0.000040 | unchanged |
| ping | redis | 65058 | 65005–65123 | 166.6 | 100.3 | 15.40 | 89.0 | 8.68 | 0.000050 | 0.000070 | unchanged |
| ping | valkey | 63613 | 63542–63834 | 171.3 | 99.6 | 15.60 | 86.6 | 8.36 | 0.000070 | 0.000060 | unchanged |
| set | rillcache | 70046 | 69769–70190 | 142.8 | 99.8 | 14.30 | 90.8 | 5.57 | 0.000010 | 0.000060 | unchanged |
| set | redis | 61969 | 61952–61974 | 179.0 | 100.3 | 16.20 | 84.0 | 8.68 | 0.000080 | 0.000040 | unchanged |
| set | valkey | 60457 | 60374–60483 | 184.7 | 99.8 | 16.50 | 77.6 | 8.36 | 0.000090 | 0.000050 | unchanged |
| get | rillcache | 72054 | 71509–72421 | 139.2 | 100.1 | 13.90 | 93.5 | 5.57 | 0.000010 | 0.000090 | unchanged |
| get | redis | 63463 | 63111–63478 | 172.8 | 99.7 | 15.80 | 86.6 | 8.68 | 0.000030 | 0.000050 | unchanged |
| get | valkey | 61749 | 61743–61787 | 178.7 | 100.0 | 16.20 | 84.6 | 8.37 | 0.000090 | 0.000050 | unchanged |
| set-nx-existing | rillcache | 71186 | 70679–71670 | 140.5 | 100.4 | 14.10 | 90.8 | 5.57 | 0.000010 | 0.000070 | unchanged |
| set-nx-existing | redis | 62722 | 62692–62809 | 175.9 | 100.3 | 16.00 | 85.6 | 8.69 | 0.000040 | 0.000070 | unchanged |
| set-nx-existing | valkey | 61606 | 61505–61638 | 179.1 | 99.8 | 16.20 | 84.0 | 8.37 | 0.000050 | 0.000050 | unchanged |
| set-xx-existing | rillcache | 70206 | 69823–70422 | 142.6 | 100.0 | 14.30 | 90.2 | 5.57 | 0.000010 | 0.000060 | unchanged |
| set-xx-existing | redis | 61670 | 61593–61796 | 180.3 | 99.9 | 16.20 | 82.7 | 8.69 | 0.000020 | 0.000050 | unchanged |
| set-xx-existing | valkey | 60182 | 60122–60263 | 185.4 | 99.9 | 16.60 | 79.5 | 8.37 | 0.000030 | 0.000030 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 4.46 | 4.46 | 99.0 | unchanged |
| redis | 1.001 | 0.000 | 8.49 | 8.49 | 10.0 | unchanged |
| valkey | 1.001 | 0.000 | 8.22 | 8.22 | 10.0 | unchanged |

</details>

<details>
<summary>single-core-pipeline-16: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 1000000; clients: 8; pipeline: 16; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: 0; client CPUs: 1,2,3.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 1136719 | 1132341–1140907 | 139.9 | 99.6 | 0.88 | 90.8 | 5.45 | 0.000002 | 0.000004 | unchanged |
| ping | redis | 833009 | 831316–833051 | 220.9 | 99.9 | 1.20 | 75.0 | 8.99 | 0.000005 | 0.000004 | unchanged |
| ping | valkey | 777443 | 776886–778716 | 241.0 | 99.6 | 1.28 | 63.3 | 8.26 | 0.000003 | 0.000005 | unchanged |
| set | rillcache | 857181 | 854305–857280 | 184.7 | 99.4 | 1.16 | 65.5 | 5.45 | 0.000001 | 0.000005 | unchanged |
| set | redis | 619809 | 618987–620266 | 313.1 | 100.3 | 1.62 | 63.6 | 8.99 | 0.000007 | 0.000008 | unchanged |
| set | valkey | 552278 | 551697–552555 | 354.9 | 99.9 | 1.81 | 44.8 | 8.25 | 0.000002 | 0.000011 | unchanged |
| get | rillcache | 966331 | 964771–984995 | 161.2 | 100.3 | 1.03 | 77.5 | 5.45 | 0.000001 | 0.000005 | unchanged |
| get | redis | 694023 | 690238–695040 | 274.6 | 99.9 | 1.44 | 63.2 | 8.99 | 0.000003 | 0.000009 | unchanged |
| get | valkey | 634228 | 634147–635000 | 302.3 | 99.7 | 1.57 | 65.0 | 8.25 | 0.000003 | 0.000009 | unchanged |
| set-nx-existing | rillcache | 931462 | 915597–945472 | 169.5 | 99.8 | 1.07 | 71.6 | 5.45 | 0.000001 | 0.000003 | unchanged |
| set-nx-existing | redis | 646939 | 646590–651099 | 299.1 | 100.2 | 1.55 | 45.2 | 8.99 | 0.000002 | 0.000007 | unchanged |
| set-nx-existing | valkey | 614590 | 614384–614902 | 314.4 | 100.2 | 1.63 | 62.3 | 8.26 | 0.000004 | 0.000008 | unchanged |
| set-xx-existing | rillcache | 839923 | 819174–845669 | 185.4 | 99.9 | 1.20 | 64.2 | 5.45 | 0.000001 | 0.000007 | unchanged |
| set-xx-existing | redis | 599406 | 598879–601132 | 324.5 | 100.0 | 1.67 | 56.6 | 8.99 | 0.000001 | 0.000007 | unchanged |
| set-xx-existing | valkey | 537319 | 537195–537411 | 364.8 | 99.9 | 1.86 | 36.6 | 8.26 | 0.000010 | 0.000009 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 4.34 | 4.34 | 99.0 | unchanged |
| redis | 1.001 | 0.000 | 8.76 | 8.76 | 10.0 | unchanged |
| valkey | 1.001 | 0.000 | 8.07 | 8.07 | 10.0 | unchanged |

</details>

### Fixed-worker mode measurements

30/30 separate embedded records validated. 8 clients; 10000 commands/workload/run; pipeline 16; three repetitions. These share the client/server host and inherited CPU affinity; they are **not native competitor comparisons or equal-per-core scaling evidence**. No process CPU/RSS accounting was collected for this group. Fixed lanes have a 1 ms idle polling wait. They actively poll for at most 50 µs after progress before returning to idle parking; this trades active CPU work for fewer premature sleeps, not epoll readiness or free CPU. Multiple lanes also request a cooperative yield after 100 µs of continuous active passes when more than one client is admitted. Progress does not rearm that separate budget; single lanes/lone clients do not periodically yield. This is checked between bounded passes, not a hard deadline or an OS fairness guarantee. The v2 interval restarts after a yield/park returns, excluding that explicit wait; involuntary descheduling can still consume wall time.

| Workload | 1 worker median cmd/s | 1 worker min–max | 4 workers median cmd/s | 4 workers min–max | 1 worker batch p99 µs | 4 workers batch p99 µs |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| ping | 978752 | 949895–1033529 | 1349273 | 1313210–1460279 | 949.6 | 1069.9 |
| set | 721329 | 703234–816950 | 38976 | 38041–39113 | 971.6 | 5813.5 |
| get | 839224 | 821397–880902 | 38795 | 38165–39057 | 957.3 | 6280.9 |
| set-nx-existing | 805665 | 776566–859080 | 39006 | 38342–39135 | 965.6 | 6129.5 |
| set-xx-existing | 717328 | 716470–720717 | 39066 | 38774–39381 | 961.3 | 6303.3 |

### Interpretation limits

- Batch p99 is **not per-request p99**; never divide it by the pipeline length. Percentiles are not pooled across runs.
- CPU counters are tick-quantized; zero observed idle ticks do not prove zero CPU cost. CPU observation windows bracket, but are not identical to, request timers.
- RSS is an approximate endpoint sample, not allocator accounting or a workload peak; VmHWM includes startup, warmup and previous workloads.
- Context switches cover matched thread lifetimes only. Even unchanged endpoints cannot exclude threads born and destroyed between snapshots. PARTIAL coverage is not a process total.
- NX-existing measures rejected writes; XX-existing measures admitted overwrites. This is a small Redis-compatible subset, not equivalent functionality.
