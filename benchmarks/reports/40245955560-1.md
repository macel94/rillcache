# Current benchmark performance

Generated: 2026-10-25T23:51:41Z · [Benchmark run #11](https://github.com/macel94/rillcache/actions/runs/40245955560) · [Tested source `0157177`](https://github.com/macel94/rillcache/commit/01571773e4aaca4c1765a226b1f7f1187e1fd33e)

**Benchmark job: success. Data: validated. Copilot review: completed.**

Benchmark attempt: 1; review attempt: 1. Preferred model: `gpt-6-luna` via **GitHub Copilot**.

[This report's permanent copy](https://github.com/macel94/rillcache/blob/main/benchmarks/reports/40245955560-1.md) · [Report history](https://github.com/macel94/rillcache/tree/main/benchmarks/reports) · [Methodology](https://github.com/macel94/rillcache/blob/main/docs/benchmarks.md)

## AI review

Actual model(s): **gpt-6-luna**. Selection: automatic-fallback. Authentication: workflow-token.

AI interpretation, **not measurements or a performance gate**. Verify claims against the deterministic tables below.

> Bottom line: In this validated subset and these workloads, Rillcache’s median SET throughput and
> whole-batch p99 were better than Redis and Valkey’s in all four profiles. This is not a general
> Redis/Valkey victory: functionality is limited, and the shared-VM results cover three runs per
> workload.
> 
> Equal one-logical-CPU budget: SET throughput was 63,852 vs Redis’s 59,333 commands/s at pipeline 1
> (+7.6%), and 845,149 vs 599,325 at pipeline 16 (+41.0%). Rillcache’s median whole-batch p99 was also
> lower: 159.3 vs 186.2 µs and 190.8 vs 304.5 µs, respectively. These are batch percentiles, not
> per-request latency; do not divide them by pipeline length.
> 
> Default threading is a separate comparison: SET throughput was 127,264 vs 59,018 commands/s at
> pipeline 1, and 1,749,248 vs 598,577 at pipeline 16. Rillcache used 186.9% and 230.7% of one logical
> CPU in those runs, while Redis was near 100%; this is not an equal-CPU result. CPU counters use a
> different observation window than request timers.
> 
> Rillcache’s measured RSS endpoints were lower (about 3.1–3.4 MiB vs 12.7–14.8 MiB), but RSS is
> approximate; VmHWM is a lifetime peak, not workload memory. Rillcache also recorded more voluntary
> context switches per command (e.g. 0.0161 vs 0.00005 for equal-budget pipeline-1 SET); counts cover
> matched thread lifetimes, not guaranteed process totals, and do not establish cause. Idle zero CPU
> readings are tick-quantized. NX-existing rejects writes; XX-existing admits overwrites.
> 
> Uncertainty: Three repetitions and a shared VM limit generalization. The separate fixed-worker
> results are not native-competitor or equal-per-core evidence; four workers raised throughput over
> one worker but also raised whole-batch p99.
> 
> Next investigations: (1) Repeat equal-one-CPU tests across more client counts and pipelines,
> reporting run-level distributions and aligned CPU windows. (2) Profile scheduling and syscall costs
> under controlled conditions to investigate the higher context-switch counts and system-CPU share
> without inferring causation from counters alone.

## Measurements

**Data quality: complete.** 180/180 records validated across 4/4 complete profiles.


Hardware: AMD EPYC 7763 64-Core Processor. Rust 1.99.1 (58cf8f9c3 2026-10-13).

References: Redis 8.10.2 sha=00000000:0 malloc=jemalloc-5.3.0 bits=64 build=56002203148bc6a1; Valkey 9.1.2 sha=00000000:0 malloc=jemalloc-5.3.0 bits=64 build=b6035f9ab637e696.

### SET at a glance

Median commands/s and median of the three runs' **whole-batch p99** values (µs). Positive rate delta means Rillcache is faster in this workload only.

| Profile | Rillcache cmd/s | Redis | Valkey | Rillcache vs Redis | Rillcache p99 µs | Redis p99 | Valkey p99 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| pipeline-1 | 127264 | 59018 | 57299 | +115.6% | 143.7 | 189.1 | 193.2 |
| pipeline-16 | 1749248 | 598577 | 546624 | +192.2% | 151.1 | 305.9 | 335.5 |
| single-core-pipeline-1 | 63852 | 59333 | 57685 | +7.6% | 159.3 | 186.2 | 191.2 |
| single-core-pipeline-16 | 845149 | 599325 | 546508 | +41.0% | 190.8 | 304.5 | 336.0 |

CPU is relative to **one logical CPU**, so parallel servers can exceed 100%. Do not blend default-threading and single-core results or different pipelines. Shared VMs and unequal feature sets do not establish a general performance win or a cross-run regression.

<details>
<summary>pipeline-1: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 100000; clients: 8; pipeline: 1; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: inherited; client CPUs: inherited.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 129328 | 113589–134218 | 144.0 | 188.7 | 14.60 | 95.2 | 3.05 | 0.862840 | 0.152020 | unchanged |
| ping | redis | 62712 | 62651–63409 | 175.4 | 100.2 | 16.00 | 87.3 | 12.98 | 0.000090 | 0.000060 | unchanged |
| ping | valkey | 61579 | 61091–61596 | 179.0 | 100.2 | 16.30 | 84.0 | 12.75 | 0.000050 | 0.000840 | unchanged |
| set | rillcache | 127264 | 118837–132661 | 143.7 | 186.9 | 14.90 | 92.6 | 3.05 | 0.823410 | 0.189210 | unchanged |
| set | redis | 59018 | 58504–59226 | 189.1 | 100.0 | 17.00 | 81.9 | 12.98 | 0.000140 | 0.000070 | unchanged |
| set | valkey | 57299 | 57113–57607 | 193.2 | 100.2 | 17.40 | 78.2 | 12.75 | 0.000140 | 0.000080 | unchanged |
| get | rillcache | 129057 | 124365–135790 | 141.4 | 188.3 | 14.40 | 94.5 | 3.05 | 0.849870 | 0.160480 | unchanged |
| get | redis | 61009 | 60422–61374 | 181.0 | 99.7 | 16.50 | 84.6 | 12.98 | 0.000130 | 0.000080 | unchanged |
| get | valkey | 59047 | 58849–59295 | 186.8 | 100.0 | 16.90 | 83.4 | 12.75 | 0.000040 | 0.000080 | unchanged |
| set-nx-existing | rillcache | 131162 | 121037–131421 | 150.1 | 190.4 | 14.70 | 93.9 | 3.05 | 0.833820 | 0.178030 | unchanged |
| set-nx-existing | redis | 59752 | 59536–60907 | 184.9 | 100.0 | 16.70 | 85.0 | 12.98 | 0.000140 | 0.000110 | unchanged |
| set-nx-existing | valkey | 58720 | 58548–58865 | 187.9 | 99.5 | 17.00 | 81.2 | 12.75 | 0.000030 | 0.000070 | unchanged |
| set-xx-existing | rillcache | 130527 | 120056–134988 | 141.6 | 190.4 | 14.60 | 92.1 | 3.05 | 0.833930 | 0.183460 | unchanged |
| set-xx-existing | redis | 58455 | 58404–58782 | 189.1 | 99.8 | 17.00 | 80.0 | 12.96 | 0.000070 | 0.000080 | unchanged |
| set-xx-existing | valkey | 57135 | 56908–57364 | 194.7 | 99.6 | 17.50 | 79.0 | 12.66 | 0.000120 | 0.000080 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 2.46 | 2.46 | 933.8 | unchanged |
| redis | 1.000 | 0.000 | 13.50 | 13.50 | 10.0 | unchanged |
| valkey | 1.001 | 0.000 | 12.86 | 12.86 | 10.0 | unchanged |

</details>

<details>
<summary>pipeline-16: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 1000000; clients: 8; pipeline: 16; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: inherited; client CPUs: inherited.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 1977719 | 1839739–2165366 | 146.1 | 183.7 | 0.93 | 91.2 | 3.08 | 0.051708 | 0.011770 | unchanged |
| ping | redis | 800193 | 799326–804538 | 223.3 | 100.0 | 1.25 | 65.6 | 13.38 | 0.000011 | 0.000004 | unchanged |
| ping | valkey | 753276 | 749492–755571 | 238.0 | 99.7 | 1.33 | 65.4 | 12.82 | 0.000002 | 0.000008 | unchanged |
| set | rillcache | 1749248 | 1731767–1749286 | 151.1 | 230.7 | 1.32 | 70.5 | 3.08 | 0.055725 | 0.008501 | unchanged |
| set | redis | 598577 | 595946–598831 | 305.9 | 99.9 | 1.67 | 53.9 | 12.86 | 0.000003 | 0.000011 | unchanged |
| set | valkey | 546624 | 545904–547735 | 335.5 | 100.2 | 1.83 | 45.7 | 12.75 | 0.000003 | 0.000007 | unchanged |
| get | rillcache | 1904934 | 1872930–2032995 | 151.9 | 199.8 | 1.05 | 81.1 | 3.08 | 0.047821 | 0.016526 | unchanged |
| get | redis | 667294 | 664259–668931 | 271.4 | 100.3 | 1.50 | 61.1 | 12.86 | 0.000002 | 0.000007 | unchanged |
| get | valkey | 618816 | 617883–621373 | 293.5 | 99.6 | 1.61 | 52.2 | 12.75 | 0.000005 | 0.000007 | unchanged |
| set-nx-existing | rillcache | 1808140 | 1765258–1828831 | 164.4 | 209.5 | 1.16 | 76.7 | 3.08 | 0.049489 | 0.014275 | unchanged |
| set-nx-existing | redis | 623084 | 617615–623399 | 292.8 | 100.3 | 1.61 | 50.9 | 12.86 | 0.000005 | 0.000008 | unchanged |
| set-nx-existing | valkey | 599128 | 596756–602642 | 304.4 | 100.0 | 1.67 | 52.4 | 12.75 | 0.000003 | 0.000007 | unchanged |
| set-xx-existing | rillcache | 1707688 | 1701801–1720276 | 162.6 | 227.8 | 1.34 | 69.2 | 3.08 | 0.055047 | 0.009124 | unchanged |
| set-xx-existing | redis | 578261 | 576818–582488 | 317.8 | 100.0 | 1.73 | 56.1 | 12.86 | 0.000002 | 0.000013 | unchanged |
| set-xx-existing | valkey | 532559 | 530467–536970 | 345.2 | 99.9 | 1.87 | 45.0 | 12.75 | 0.000004 | 0.000008 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 2.43 | 2.43 | 933.8 | unchanged |
| redis | 1.001 | 0.000 | 13.35 | 13.35 | 10.0 | unchanged |
| valkey | 1.001 | 0.000 | 12.82 | 12.82 | 10.0 | unchanged |

</details>

<details>
<summary>single-core-pipeline-1: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 100000; clients: 8; pipeline: 1; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: 0; client CPUs: 1,2,3.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 66551 | 66146–67100 | 153.2 | 99.8 | 15.00 | 94.7 | 3.39 | 0.015220 | 0.014560 | unchanged |
| ping | redis | 63225 | 63190–63421 | 173.9 | 99.9 | 15.80 | 86.2 | 12.82 | 0.000020 | 0.000040 | unchanged |
| ping | valkey | 61526 | 61507–61647 | 179.0 | 99.8 | 16.20 | 85.2 | 14.61 | 0.000060 | 0.000060 | unchanged |
| set | rillcache | 63852 | 63822–64196 | 159.3 | 100.2 | 15.70 | 91.7 | 3.39 | 0.016100 | 0.015270 | unchanged |
| set | redis | 59333 | 59160–59343 | 186.2 | 100.0 | 16.90 | 81.5 | 12.82 | 0.000050 | 0.000030 | unchanged |
| set | valkey | 57685 | 57635–57801 | 191.2 | 99.7 | 17.30 | 79.3 | 14.61 | 0.000030 | 0.000050 | unchanged |
| get | rillcache | 65143 | 64632–65167 | 156.2 | 99.6 | 15.40 | 92.9 | 3.39 | 0.015690 | 0.014930 | unchanged |
| get | redis | 61162 | 61161–61290 | 180.1 | 99.9 | 16.30 | 85.3 | 12.82 | 0.000020 | 0.000050 | unchanged |
| get | valkey | 59232 | 59118–59286 | 186.0 | 99.9 | 16.90 | 83.3 | 14.61 | 0.000080 | 0.000060 | unchanged |
| set-nx-existing | rillcache | 64525 | 64314–65036 | 158.8 | 99.5 | 15.40 | 92.2 | 3.40 | 0.016070 | 0.015280 | unchanged |
| set-nx-existing | redis | 59970 | 59959–60021 | 183.8 | 99.5 | 16.60 | 84.9 | 12.82 | 0.000090 | 0.000040 | unchanged |
| set-nx-existing | valkey | 59029 | 59006–59054 | 187.0 | 100.3 | 17.00 | 81.2 | 14.61 | 0.000050 | 0.000050 | unchanged |
| set-xx-existing | rillcache | 63762 | 63704–64344 | 159.3 | 100.1 | 15.70 | 91.7 | 3.40 | 0.016460 | 0.015480 | unchanged |
| set-xx-existing | redis | 58725 | 58684–58839 | 188.0 | 100.0 | 17.00 | 80.0 | 12.80 | 0.000100 | 0.000060 | unchanged |
| set-xx-existing | valkey | 57538 | 57383–57556 | 192.4 | 100.1 | 17.40 | 77.0 | 14.59 | 0.000080 | 0.000070 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 1.000 | 2.47 | 2.47 | 933.8 | unchanged |
| redis | 1.001 | 0.000 | 13.34 | 13.34 | 10.0 | unchanged |
| valkey | 1.001 | 0.000 | 14.79 | 14.79 | 10.0 | unchanged |

</details>

<details>
<summary>single-core-pipeline-16: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 1000000; clients: 8; pipeline: 16; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: 0; client CPUs: 1,2,3.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 1017672 | 1014187–1020361 | 159.2 | 99.3 | 0.98 | 89.8 | 3.41 | 0.001024 | 0.000968 | unchanged |
| ping | redis | 804339 | 802359–806317 | 222.0 | 99.7 | 1.24 | 64.5 | 13.23 | 0.000009 | 0.000005 | unchanged |
| ping | valkey | 755060 | 745578–755227 | 237.4 | 99.6 | 1.33 | 64.7 | 12.77 | 0.000003 | 0.000006 | unchanged |
| set | rillcache | 845149 | 831406–845655 | 190.8 | 99.7 | 1.19 | 73.7 | 3.41 | 0.001215 | 0.001151 | unchanged |
| set | redis | 599325 | 598934–600246 | 304.5 | 100.6 | 1.68 | 52.4 | 12.71 | 0.000005 | 0.000006 | unchanged |
| set | valkey | 546508 | 544802–548439 | 336.0 | 99.8 | 1.82 | 45.1 | 12.66 | 0.000007 | 0.000004 | unchanged |
| get | rillcache | 944431 | 942484–945373 | 172.4 | 99.8 | 1.06 | 81.1 | 3.41 | 0.001075 | 0.001014 | unchanged |
| get | redis | 671037 | 667108–672722 | 271.0 | 99.5 | 1.49 | 63.1 | 12.71 | 0.000008 | 0.000004 | unchanged |
| get | valkey | 621557 | 620826–622481 | 293.3 | 99.9 | 1.61 | 53.1 | 12.66 | 0.000007 | 0.000003 | unchanged |
| set-nx-existing | rillcache | 906362 | 905064–908891 | 178.5 | 100.6 | 1.11 | 76.6 | 3.41 | 0.001121 | 0.001063 | unchanged |
| set-nx-existing | redis | 624624 | 624213–627161 | 291.7 | 99.7 | 1.59 | 50.9 | 12.71 | 0.000003 | 0.000006 | unchanged |
| set-nx-existing | valkey | 601078 | 600060–603887 | 302.5 | 99.6 | 1.66 | 53.3 | 12.66 | 0.000005 | 0.000004 | unchanged |
| set-xx-existing | rillcache | 814324 | 808365–830115 | 194.6 | 99.6 | 1.22 | 73.6 | 3.41 | 0.001279 | 0.001192 | unchanged |
| set-xx-existing | redis | 580017 | 579921–582061 | 315.8 | 100.1 | 1.72 | 55.2 | 12.71 | 0.000006 | 0.000006 | unchanged |
| set-xx-existing | valkey | 534072 | 532058–534645 | 344.1 | 99.4 | 1.86 | 45.2 | 12.66 | 0.000007 | 0.000003 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 2.47 | 2.47 | 933.8 | unchanged |
| redis | 1.001 | 0.000 | 13.20 | 13.20 | 10.0 | unchanged |
| valkey | 1.001 | 0.000 | 12.77 | 12.77 | 10.0 | unchanged |

</details>

### Fixed-worker mode measurements

30/30 separate embedded records validated. 8 clients; 10000 commands/workload/run; pipeline 16; three repetitions. These share the client/server host and inherited CPU affinity; they are **not native competitor comparisons or equal-per-core scaling evidence**. No process CPU/RSS accounting was collected for this group. Fixed lanes have a 1 ms idle polling wait. They actively poll for at most 50 µs after progress before returning to idle parking; this trades active CPU work for fewer premature sleeps, not epoll readiness or free CPU. Multiple lanes also request a cooperative yield after 100 µs of continuous active passes when more than one client is admitted. Progress does not rearm that separate budget; single lanes/lone clients do not periodically yield. This is checked between bounded passes, not a hard deadline or an OS fairness guarantee. The v2 interval restarts after a yield/park returns, excluding that explicit wait; involuntary descheduling can still consume wall time.

| Workload | 1 worker median cmd/s | 1 worker min–max | 4 workers median cmd/s | 4 workers min–max | 1 worker batch p99 µs | 4 workers batch p99 µs |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| ping | 990183 | 979019–1011977 | 1224634 | 1170639–1326129 | 926.3 | 1097.1 |
| set | 810263 | 753458–875515 | 1128844 | 1052178–1256219 | 952.2 | 1141.0 |
| get | 874142 | 871270–883300 | 1256825 | 1126215–1340890 | 952.4 | 1056.4 |
| set-nx-existing | 860076 | 794813–920889 | 1217839 | 1124266–1247627 | 945.7 | 1098.9 |
| set-xx-existing | 800196 | 728623–810413 | 1094579 | 1078918–1183210 | 951.2 | 1119.2 |

### Interpretation limits

- Batch p99 is **not per-request p99**; never divide it by the pipeline length. Percentiles are not pooled across runs.
- CPU counters are tick-quantized; zero observed idle ticks do not prove zero CPU cost. CPU observation windows bracket, but are not identical to, request timers.
- RSS is an approximate endpoint sample, not allocator accounting or a workload peak; VmHWM includes startup, warmup and previous workloads.
- Context switches cover matched thread lifetimes only. Even unchanged endpoints cannot exclude threads born and destroyed between snapshots. PARTIAL coverage is not a process total.
- NX-existing measures rejected writes; XX-existing measures admitted overwrites. This is a small Redis-compatible subset, not equivalent functionality.
