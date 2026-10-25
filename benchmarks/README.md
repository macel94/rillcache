# Current benchmark performance

Generated: 2026-10-25T23:21:55Z · [Benchmark run #10](https://github.com/macel94/rillcache/actions/runs/40243686729) · [Tested source `9062847`](https://github.com/macel94/rillcache/commit/90628470f8fa487f64666705da46008564a08ba4)

**Benchmark job: success. Data: validated. Copilot review: completed.**

Benchmark attempt: 1; review attempt: 1. Preferred model: `gpt-6-luna` via **GitHub Copilot**.

[This report's permanent copy](https://github.com/macel94/rillcache/blob/main/benchmarks/reports/40243686729-1.md) · [Report history](https://github.com/macel94/rillcache/tree/main/benchmarks/reports) · [Methodology](https://github.com/macel94/rillcache/blob/main/docs/benchmarks.md)

## AI review

Actual model(s): **gpt-6-luna**. Selection: automatic-fallback. Authentication: workflow-token.

AI interpretation, **not measurements or a performance gate**. Verify claims against the deterministic tables below.

> Bottom line: In this validated native cohort (180 records), Rillcache leads the tested SET workloads
> on throughput and whole-batch p99, including under a one-logical-CPU budget. That is evidence about
> these workloads—not a general Redis or Valkey victory.
> 
> Equal-CPU tradeoff: With one logical CPU and pipeline 1, SET reached 64,125 commands/s versus
> Redis’s 58,413 (+9.8%); whole-batch p99 was 159.5 µs versus 188.8 µs. At pipeline 16, throughput was
> 825,822 versus 588,784 (+40.3%), and batch p99 was 193.5 versus 310.7 µs. These p99 values describe
> batches, not individual requests; they should not be divided by pipeline length. Default-threading
> results are a separate budget: for pipeline-16 SET, Rillcache used 229.4% of one logical CPU, so
> those rates are not equal-CPU comparisons.
> 
> Resource and scheduling observations: In equal-CPU SET runs, reported CPU was near 100% for all
> servers; Rillcache’s CPU time was 15.6 versus 17.1 µs/command at pipeline 1, and 1.21 versus 1.70 at
> pipeline 16. Approximate RSS endpoints were 3.24–3.29 MiB for Rillcache versus 12.88–13.05 MiB for
> Redis. Rillcache also recorded more voluntary and involuntary context switches per command. These
> counters cover matched thread lifetimes, not guaranteed process totals, and do not establish
> causation. Idle CPU readings of zero are tick-quantized; RSS endpoints are approximate, while VmHWM
> is a lifetime peak.
> 
> Uncertainty and next investigations: Three repetitions per workload, shared-VM conditions, and this
> limited command/value/key mix constrain inference. First, test representative mixed workloads and
> broader key/value sizes to see whether the throughput and batch-tail pattern persists. Second,
> profile equal-CPU runs’ scheduling and system-CPU costs, investigating the higher switch counts
> without assuming they explain the results. The embedded fixed-worker measurements are a separate
> group, not native competitor or equal-per-core evidence.

## Measurements

**Data quality: complete.** 180/180 records validated across 4/4 complete profiles.


Hardware: AMD EPYC 7763 64-Core Processor. Rust 1.99.1 (58cf8f9c3 2026-10-13).

References: Redis 8.10.2 sha=00000000:0 malloc=jemalloc-5.3.0 bits=64 build=9454cdbedf63a976; Valkey 9.1.2 sha=00000000:0 malloc=jemalloc-5.3.0 bits=64 build=53f5e8ee3f98b003.

### SET at a glance

Median commands/s and median of the three runs' **whole-batch p99** values (µs). Positive rate delta means Rillcache is faster in this workload only.

| Profile | Rillcache cmd/s | Redis | Valkey | Rillcache vs Redis | Rillcache p99 µs | Redis p99 | Valkey p99 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| pipeline-1 | 122350 | 58532 | 56851 | +109.0% | 202.4 | 188.9 | 194.0 |
| pipeline-16 | 1688443 | 580913 | 528303 | +190.7% | 166.9 | 315.7 | 348.3 |
| single-core-pipeline-1 | 64125 | 58413 | 57829 | +9.8% | 159.5 | 188.8 | 190.7 |
| single-core-pipeline-16 | 825822 | 588784 | 539833 | +40.3% | 193.5 | 310.7 | 339.9 |

CPU is relative to **one logical CPU**, so parallel servers can exceed 100%. Do not blend default-threading and single-core results or different pipelines. Shared VMs and unequal feature sets do not establish a general performance win or a cross-run regression.

<details>
<summary>pipeline-1: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 100000; clients: 8; pipeline: 1; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: inherited; client CPUs: inherited.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 124210 | 122381–130617 | 162.1 | 184.7 | 14.80 | 94.6 | 3.05 | 0.812270 | 0.203200 | unchanged |
| ping | redis | 62462 | 62449–62660 | 175.1 | 99.9 | 16.00 | 86.3 | 13.09 | 0.000040 | 0.000090 | unchanged |
| ping | valkey | 60441 | 60337–60460 | 180.7 | 99.7 | 16.50 | 87.3 | 14.75 | 0.000110 | 0.000410 | unchanged |
| set | rillcache | 122350 | 117673–126006 | 202.4 | 182.2 | 14.90 | 91.9 | 3.05 | 0.762460 | 0.251010 | unchanged |
| set | redis | 58532 | 58360–58833 | 188.9 | 99.8 | 17.10 | 80.1 | 13.09 | 0.000110 | 0.000050 | unchanged |
| set | valkey | 56851 | 56706–56922 | 194.0 | 99.6 | 17.50 | 78.3 | 14.75 | 0.000160 | 0.000450 | unchanged |
| get | rillcache | 119525 | 110965–121023 | 198.2 | 180.2 | 15.40 | 94.6 | 3.05 | 0.829940 | 0.185580 | unchanged |
| get | redis | 60071 | 59724–60284 | 182.4 | 99.7 | 16.60 | 86.1 | 13.10 | 0.000050 | 0.000070 | unchanged |
| get | valkey | 58084 | 57773–59557 | 189.1 | 99.9 | 17.20 | 82.1 | 14.75 | 0.000090 | 0.000130 | unchanged |
| set-nx-existing | rillcache | 128333 | 123047–131323 | 167.1 | 193.6 | 14.90 | 92.7 | 3.05 | 0.798180 | 0.217260 | unchanged |
| set-nx-existing | redis | 59550 | 58957–59759 | 184.8 | 100.6 | 16.90 | 82.8 | 13.10 | 0.000080 | 0.000070 | unchanged |
| set-nx-existing | valkey | 58620 | 58398–58942 | 188.0 | 99.6 | 17.00 | 82.4 | 14.75 | 0.000050 | 0.000080 | unchanged |
| set-xx-existing | rillcache | 118853 | 118263–121690 | 175.9 | 184.8 | 15.40 | 92.8 | 3.05 | 0.795750 | 0.220350 | unchanged |
| set-xx-existing | redis | 58557 | 57589–59194 | 190.2 | 99.5 | 17.00 | 79.2 | 13.08 | 0.000020 | 0.000950 | unchanged |
| set-xx-existing | valkey | 56444 | 56302–57259 | 196.0 | 100.2 | 17.70 | 78.7 | 14.67 | 0.000130 | 0.000040 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 1.000 | 2.46 | 2.46 | 936.8 | unchanged |
| redis | 1.001 | 0.000 | 13.61 | 13.61 | 10.0 | unchanged |
| valkey | 1.001 | 0.000 | 14.86 | 14.86 | 10.0 | unchanged |

</details>

<details>
<summary>pipeline-16: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 1000000; clients: 8; pipeline: 16; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: inherited; client CPUs: inherited.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 1842403 | 1827235–2053079 | 157.4 | 189.5 | 1.02 | 90.2 | 3.11 | 0.051693 | 0.011388 | unchanged |
| ping | redis | 780553 | 771022–781038 | 228.6 | 99.9 | 1.29 | 70.5 | 13.57 | 0.000008 | 0.000008 | unchanged |
| ping | valkey | 729631 | 729541–730718 | 245.3 | 100.7 | 1.38 | 60.1 | 12.83 | 0.000004 | 0.000008 | unchanged |
| set | rillcache | 1688443 | 1651842–1699475 | 166.9 | 229.4 | 1.36 | 71.3 | 3.11 | 0.055072 | 0.009127 | unchanged |
| set | redis | 580913 | 579177–582259 | 315.7 | 99.9 | 1.72 | 56.4 | 13.04 | 0.000004 | 0.000007 | unchanged |
| set | valkey | 528303 | 528294–529765 | 348.3 | 99.8 | 1.89 | 46.8 | 12.83 | 0.000006 | 0.000008 | unchanged |
| get | rillcache | 1782957 | 1762556–1805253 | 164.3 | 200.7 | 1.13 | 84.1 | 3.11 | 0.049932 | 0.014109 | unchanged |
| get | redis | 646122 | 644940–650337 | 280.2 | 99.5 | 1.54 | 55.2 | 13.04 | 0.000005 | 0.000005 | unchanged |
| get | valkey | 599420 | 596441–600242 | 304.4 | 100.1 | 1.67 | 55.7 | 12.83 | 0.000004 | 0.000011 | unchanged |
| set-nx-existing | rillcache | 1795709 | 1706722–1839982 | 155.3 | 215.0 | 1.18 | 75.0 | 3.11 | 0.049940 | 0.013937 | unchanged |
| set-nx-existing | redis | 608119 | 606264–608807 | 301.0 | 100.0 | 1.65 | 54.5 | 13.04 | 0.000001 | 0.000007 | unchanged |
| set-nx-existing | valkey | 582835 | 580566–582946 | 314.0 | 99.8 | 1.72 | 57.6 | 12.83 | 0.000007 | 0.000007 | unchanged |
| set-xx-existing | rillcache | 1683040 | 1660387–1689776 | 162.6 | 232.0 | 1.38 | 69.6 | 3.11 | 0.056642 | 0.007393 | unchanged |
| set-xx-existing | redis | 563783 | 563001–566937 | 325.9 | 99.8 | 1.77 | 52.5 | 13.05 | 0.000003 | 0.000010 | unchanged |
| set-xx-existing | valkey | 516433 | 516190–517655 | 357.8 | 99.6 | 1.93 | 45.6 | 12.83 | 0.000003 | 0.000007 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 2.47 | 2.47 | 933.8 | unchanged |
| redis | 1.001 | 0.000 | 13.54 | 13.54 | 10.0 | unchanged |
| valkey | 1.001 | 0.000 | 12.88 | 12.88 | 10.0 | unchanged |

</details>

<details>
<summary>single-core-pipeline-1: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 100000; clients: 8; pipeline: 1; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: 0; client CPUs: 1,2,3.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 66093 | 65867–66598 | 155.4 | 100.4 | 15.20 | 94.7 | 3.24 | 0.014110 | 0.014110 | unchanged |
| ping | redis | 61974 | 61950–62110 | 175.9 | 99.7 | 16.10 | 86.3 | 13.05 | 0.000080 | 0.000090 | unchanged |
| ping | valkey | 61377 | 61161–61547 | 178.3 | 100.3 | 16.40 | 86.0 | 12.71 | 0.000130 | 0.000070 | unchanged |
| set | rillcache | 64125 | 64079–64297 | 159.5 | 99.9 | 15.60 | 92.3 | 3.24 | 0.014520 | 0.014520 | unchanged |
| set | redis | 58413 | 58240–58698 | 188.8 | 99.9 | 17.10 | 81.4 | 13.05 | 0.000020 | 0.000100 | unchanged |
| set | valkey | 57829 | 57681–58085 | 190.7 | 100.0 | 17.30 | 79.2 | 12.71 | 0.000060 | 0.000060 | unchanged |
| get | rillcache | 64820 | 64522–65206 | 157.9 | 99.3 | 15.30 | 94.1 | 3.25 | 0.014380 | 0.014360 | unchanged |
| get | redis | 60130 | 60124–60313 | 183.0 | 99.8 | 16.60 | 86.7 | 13.06 | 0.000030 | 0.000090 | unchanged |
| get | valkey | 59071 | 59012–59266 | 186.3 | 99.8 | 16.90 | 82.4 | 12.71 | 0.000040 | 0.000060 | unchanged |
| set-nx-existing | rillcache | 64238 | 64138–64263 | 159.4 | 99.6 | 15.50 | 91.7 | 3.25 | 0.014580 | 0.014550 | unchanged |
| set-nx-existing | redis | 59070 | 59016–59317 | 187.0 | 99.8 | 16.90 | 83.4 | 13.06 | 0.000060 | 0.000120 | unchanged |
| set-nx-existing | valkey | 58993 | 58791–59041 | 186.9 | 99.7 | 16.90 | 81.1 | 12.71 | 0.000030 | 0.000090 | unchanged |
| set-xx-existing | rillcache | 64003 | 63644–64502 | 160.4 | 99.9 | 15.60 | 91.7 | 3.26 | 0.014620 | 0.014530 | unchanged |
| set-xx-existing | redis | 58023 | 58007–58108 | 189.9 | 100.3 | 17.30 | 79.2 | 13.05 | 0.000040 | 0.000100 | unchanged |
| set-xx-existing | valkey | 57687 | 57605–57697 | 191.2 | 99.8 | 17.30 | 77.3 | 12.65 | 0.000100 | 0.000080 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 2.47 | 2.47 | 933.8 | unchanged |
| redis | 1.001 | 0.000 | 13.57 | 13.57 | 10.0 | unchanged |
| valkey | 1.001 | 0.000 | 12.84 | 12.84 | 10.0 | unchanged |

</details>

<details>
<summary>single-core-pipeline-16: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 1000000; clients: 8; pipeline: 16; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: 0; client CPUs: 1,2,3.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 1019665 | 1015340–1024684 | 161.4 | 99.3 | 0.97 | 90.7 | 3.27 | 0.000918 | 0.000912 | unchanged |
| ping | redis | 790710 | 781107–790895 | 226.1 | 99.6 | 1.26 | 69.8 | 13.34 | 0.000004 | 0.000004 | unchanged |
| ping | valkey | 742218 | 737087–744459 | 242.1 | 100.2 | 1.35 | 63.0 | 14.77 | 0.000005 | 0.000005 | unchanged |
| set | rillcache | 825822 | 805112–830198 | 193.5 | 99.8 | 1.21 | 75.8 | 3.29 | 0.001145 | 0.001136 | unchanged |
| set | redis | 588784 | 587020–590550 | 310.7 | 100.1 | 1.70 | 56.7 | 12.88 | 0.000004 | 0.000006 | unchanged |
| set | valkey | 539833 | 536278–541076 | 339.9 | 100.1 | 1.85 | 47.6 | 14.67 | 0.000002 | 0.000008 | unchanged |
| get | rillcache | 925404 | 913419–928420 | 176.1 | 99.5 | 1.08 | 81.7 | 3.29 | 0.001011 | 0.001004 | unchanged |
| get | redis | 659484 | 654777–663719 | 274.1 | 99.6 | 1.51 | 58.3 | 12.88 | 0.000008 | 0.000004 | unchanged |
| get | valkey | 610282 | 610190–612059 | 297.8 | 100.0 | 1.64 | 54.3 | 14.67 | 0.000003 | 0.000008 | unchanged |
| set-nx-existing | rillcache | 885611 | 874753–888539 | 185.1 | 100.0 | 1.13 | 76.5 | 3.31 | 0.001061 | 0.001070 | unchanged |
| set-nx-existing | redis | 614791 | 614517–621519 | 295.2 | 100.0 | 1.62 | 54.0 | 12.88 | 0.000003 | 0.000006 | unchanged |
| set-nx-existing | valkey | 594951 | 593263–596141 | 306.4 | 100.1 | 1.68 | 55.4 | 14.67 | 0.000006 | 0.000006 | unchanged |
| set-xx-existing | rillcache | 795282 | 793249–810994 | 199.7 | 99.7 | 1.25 | 74.8 | 3.32 | 0.001174 | 0.001171 | unchanged |
| set-xx-existing | redis | 570712 | 561847–575678 | 322.8 | 100.0 | 1.75 | 54.9 | 12.88 | 0.000006 | 0.000008 | unchanged |
| set-xx-existing | valkey | 528532 | 528103–529694 | 348.3 | 100.1 | 1.89 | 48.1 | 14.67 | 0.000006 | 0.000007 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 1.000 | 2.46 | 2.46 | 936.8 | unchanged |
| redis | 1.001 | 0.000 | 13.31 | 13.31 | 10.0 | unchanged |
| valkey | 1.001 | 0.000 | 14.77 | 14.77 | 10.0 | unchanged |

</details>

### Fixed-worker mode measurements

30/30 separate embedded records validated. 8 clients; 10000 commands/workload/run; pipeline 16; three repetitions. These share the client/server host and inherited CPU affinity; they are **not native competitor comparisons or equal-per-core scaling evidence**. No process CPU/RSS accounting was collected for this group. Fixed lanes have a 1 ms idle polling wait. They actively poll for at most 50 µs after progress before returning to idle parking; this trades active CPU work for fewer premature sleeps, not epoll readiness or free CPU. Multiple lanes also request a cooperative yield after 100 µs of continuous active passes when more than one client is admitted. Progress does not rearm that separate budget; single lanes/lone clients do not periodically yield. This is checked between bounded passes, not a hard deadline or an OS fairness guarantee. The v2 interval restarts after a yield/park returns, excluding that explicit wait; involuntary descheduling can still consume wall time.

| Workload | 1 worker median cmd/s | 1 worker min–max | 4 workers median cmd/s | 4 workers min–max | 1 worker batch p99 µs | 4 workers batch p99 µs |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| ping | 906817 | 901239–995843 | 1312016 | 1149425–1339354 | 937.9 | 1126.4 |
| set | 805995 | 781918–854233 | 1116917 | 1068060–1181128 | 944.5 | 1134.7 |
| get | 845862 | 845715–875601 | 1313591 | 1269038–1355177 | 931.4 | 1117.3 |
| set-nx-existing | 815265 | 812079–839497 | 1230320 | 1228898–1393291 | 949.0 | 1094.2 |
| set-xx-existing | 801735 | 746313–843762 | 1093342 | 1059419–1176066 | 942.6 | 1098.9 |

### Interpretation limits

- Batch p99 is **not per-request p99**; never divide it by the pipeline length. Percentiles are not pooled across runs.
- CPU counters are tick-quantized; zero observed idle ticks do not prove zero CPU cost. CPU observation windows bracket, but are not identical to, request timers.
- RSS is an approximate endpoint sample, not allocator accounting or a workload peak; VmHWM includes startup, warmup and previous workloads.
- Context switches cover matched thread lifetimes only. Even unchanged endpoints cannot exclude threads born and destroyed between snapshots. PARTIAL coverage is not a process total.
- NX-existing measures rejected writes; XX-existing measures admitted overwrites. This is a small Redis-compatible subset, not equivalent functionality.
