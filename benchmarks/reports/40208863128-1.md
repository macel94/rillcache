# Current benchmark performance

Generated: 2026-10-25T16:00:28Z · [Benchmark run #7](https://github.com/macel94/rillcache/actions/runs/40208863128) · [Tested source `d57cb2e`](https://github.com/macel94/rillcache/commit/d57cb2ed0e077eb7d33109d4deee989d534daa4c)

**Benchmark job: success. Data: validated. Copilot review: completed.**

Benchmark attempt: 1; review attempt: 1. Preferred model: `gpt-6-luna` via **GitHub Copilot**.

[This report's permanent copy](https://github.com/macel94/rillcache/blob/main/benchmarks/reports/40208863128-1.md) · [Report history](https://github.com/macel94/rillcache/tree/main/benchmarks/reports) · [Methodology](https://github.com/macel94/rillcache/blob/main/docs/benchmarks.md)

## AI review

Actual model(s): **gpt-6-luna**. Selection: automatic-fallback. Authentication: workflow-token.

AI interpretation, **not measurements or a performance gate**. Verify claims against the deterministic tables below.

> Bottom line: In this validated cohort, Rillcache’s results depend strongly on pipeline depth and CPU
> budget. Default-threading SET throughput was higher in these workloads, but that is not an
> equal-CPU comparison or evidence of a general Redis/Valkey win.
> 
> Equal one-logical-CPU budget: With pipeline 1, SET throughput was lower for Rillcache (49,392
> commands/s) than Redis (59,411) and Valkey (58,135), with higher whole-batch p99 (324.3 µs versus
> 185.6 and 190.4). At pipeline 16, Rillcache led SET throughput (681,750 versus 600,540 and 542,984),
> while its whole-batch p99 remained higher than both (378.2 µs versus 304.7 and 338.0). These are
> batch percentiles, not per-request latency.
> 
> CPU, memory, scheduling: Under inherited affinity, Rillcache used about 1.9–2.3 logical CPUs in the
> measured workloads, while Redis and Valkey were near one; do not conflate this with the single-core
> profile. Rillcache’s sampled RSS was about 2.9–3.0 MiB versus roughly 12.8–14.7 MiB for competitors,
> but RSS is an approximate endpoint, and VmHWM is a lifetime peak. Rillcache also recorded more
> voluntary context switches per command (for example, SET pipeline 1: 0.838 versus 0.00015 for
> Redis); these counters cover matched thread lifetimes and do not establish a cause. Idle CPU
> readings of zero are tick-quantized, not proof of zero cost.
> 
> Uncertainty: Three runs per workload, shared-VM conditions, differing feature sets, approximate
> counters, and observation windows limit inference. Fixed-worker results are a separate embedded
> group: four workers raised SET throughput from 764,510 to 979,402 commands/s, while whole-batch p99
> rose from 951.1 to 1,231.0 µs; there is no CPU/RSS accounting for that group.
> 
> Next investigations: (1) Repeat equal-one-CPU pipeline-1 and pipeline-16 tests with more repetitions
> and controlled host conditions, focusing on the pipeline-1 SET throughput/tail gap and pipeline-16
> tail tradeoff. (2) Profile Rillcache’s high context-switch counts and system-CPU share against
> matched workloads, then measure fixed-worker CPU use alongside throughput and batch p99.

## Measurements

**Data quality: complete.** 180/180 records validated across 4/4 complete profiles.


Hardware: AMD EPYC 7763 64-Core Processor. Rust 1.99.1 (58cf8f9c3 2026-10-13).

References: Redis 8.10.2 sha=00000000:0 malloc=jemalloc-5.3.0 bits=64 build=30d92547d31a122f; Valkey 9.1.2 sha=00000000:0 malloc=jemalloc-5.3.0 bits=64 build=5da062dc450e1425.

### SET at a glance

Median commands/s and median of the three runs' **whole-batch p99** values (µs). Positive rate delta means Rillcache is faster in this workload only.

| Profile | Rillcache cmd/s | Redis | Valkey | Rillcache vs Redis | Rillcache p99 µs | Redis p99 | Valkey p99 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| pipeline-1 | 126030 | 58389 | 57310 | +115.8% | 136.0 | 189.1 | 192.7 |
| pipeline-16 | 1688206 | 593941 | 541534 | +184.2% | 163.1 | 307.9 | 340.0 |
| single-core-pipeline-1 | 49392 | 59411 | 58135 | -16.9% | 324.3 | 185.6 | 190.4 |
| single-core-pipeline-16 | 681750 | 600540 | 542984 | +13.5% | 378.2 | 304.7 | 338.0 |

CPU is relative to **one logical CPU**, so parallel servers can exceed 100%. Do not blend default-threading and single-core results or different pipelines. Shared VMs and unequal feature sets do not establish a general performance win or a cross-run regression.

<details>
<summary>pipeline-1: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 100000; clients: 8; pipeline: 1; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: inherited; client CPUs: inherited.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 131144 | 125013–132203 | 145.8 | 188.9 | 14.60 | 95.2 | 2.93 | 0.827360 | 0.182650 | unchanged |
| ping | redis | 62540 | 61831–62672 | 176.8 | 100.1 | 16.00 | 85.6 | 12.77 | 0.000040 | 0.000110 | unchanged |
| ping | valkey | 61257 | 60906–61373 | 180.0 | 100.0 | 16.30 | 85.9 | 14.65 | 0.000110 | 0.000090 | unchanged |
| set | rillcache | 126030 | 123560–126590 | 136.0 | 191.0 | 15.10 | 92.2 | 2.93 | 0.838460 | 0.173890 | unchanged |
| set | redis | 58389 | 58359–59064 | 189.1 | 99.8 | 17.10 | 81.3 | 12.77 | 0.000150 | 0.000080 | unchanged |
| set | valkey | 57310 | 57307–57698 | 192.7 | 99.7 | 17.40 | 77.0 | 14.65 | 0.000100 | 0.000100 | unchanged |
| get | rillcache | 131727 | 124288–134360 | 140.1 | 193.5 | 14.70 | 93.9 | 2.93 | 0.849350 | 0.164140 | unchanged |
| get | redis | 60094 | 59654–60459 | 184.1 | 99.7 | 16.60 | 84.3 | 12.77 | 0.000100 | 0.000380 | unchanged |
| get | valkey | 58855 | 58846–59357 | 187.4 | 99.7 | 16.90 | 82.7 | 14.65 | 0.000260 | 0.000050 | unchanged |
| set-nx-existing | rillcache | 133103 | 130147–134862 | 140.4 | 194.0 | 14.90 | 93.3 | 2.93 | 0.823780 | 0.188190 | unchanged |
| set-nx-existing | redis | 59320 | 58706–59478 | 186.8 | 100.2 | 16.90 | 82.2 | 12.77 | 0.000140 | 0.000050 | unchanged |
| set-nx-existing | valkey | 58765 | 58653–59130 | 187.8 | 99.7 | 17.00 | 82.4 | 14.65 | 0.000090 | 0.000110 | unchanged |
| set-xx-existing | rillcache | 122127 | 115853–126866 | 150.7 | 185.5 | 15.20 | 93.4 | 2.93 | 0.832930 | 0.183080 | unchanged |
| set-xx-existing | redis | 58284 | 57695–64482 | 190.0 | 99.8 | 17.10 | 80.3 | 12.77 | 0.000180 | 0.000070 | unchanged |
| set-xx-existing | valkey | 57121 | 56601–57561 | 194.2 | 99.6 | 17.60 | 78.0 | 14.58 | 0.000150 | 0.000100 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 1.000 | 2.35 | 2.35 | 932.8 | unchanged |
| redis | 1.001 | 0.000 | 13.30 | 13.30 | 10.0 | unchanged |
| valkey | 1.001 | 0.000 | 14.77 | 14.77 | 10.0 | unchanged |

</details>

<details>
<summary>pipeline-16: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 1000000; clients: 8; pipeline: 16; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: inherited; client CPUs: inherited.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 2016515 | 1854186–2127060 | 144.6 | 187.3 | 0.94 | 91.4 | 2.98 | 0.053554 | 0.010020 | unchanged |
| ping | redis | 794016 | 789895–799268 | 224.6 | 99.9 | 1.26 | 69.0 | 13.34 | 0.000004 | 0.000007 | unchanged |
| ping | valkey | 752185 | 744306–753121 | 238.5 | 100.4 | 1.34 | 63.4 | 12.87 | 0.000019 | 0.000010 | unchanged |
| set | rillcache | 1688206 | 1538941–1705563 | 163.1 | 227.7 | 1.35 | 70.9 | 2.98 | 0.052009 | 0.012310 | unchanged |
| set | redis | 593941 | 593328–599114 | 307.9 | 100.0 | 1.68 | 53.9 | 12.82 | 0.000004 | 0.000006 | unchanged |
| set | valkey | 541534 | 540379–542825 | 340.0 | 100.2 | 1.85 | 45.4 | 12.76 | 0.000004 | 0.000007 | unchanged |
| get | rillcache | 1929676 | 1891253–1944490 | 146.3 | 207.8 | 1.09 | 81.7 | 2.98 | 0.050650 | 0.012456 | unchanged |
| get | redis | 665088 | 663222–668035 | 273.0 | 100.1 | 1.50 | 62.0 | 12.82 | 0.000004 | 0.000005 | unchanged |
| get | valkey | 604348 | 604111–605062 | 301.5 | 100.3 | 1.66 | 53.6 | 12.76 | 0.000004 | 0.000007 | unchanged |
| set-nx-existing | rillcache | 1730536 | 1693349–1824433 | 173.9 | 205.7 | 1.19 | 76.5 | 2.98 | 0.047472 | 0.015982 | unchanged |
| set-nx-existing | redis | 620224 | 618755–622493 | 294.1 | 99.6 | 1.61 | 54.7 | 12.82 | 0.000003 | 0.000007 | unchanged |
| set-nx-existing | valkey | 597444 | 597206–598604 | 304.7 | 100.3 | 1.68 | 53.6 | 12.76 | 0.000006 | 0.000007 | unchanged |
| set-xx-existing | rillcache | 1691236 | 1688937–1707998 | 147.6 | 231.4 | 1.37 | 69.1 | 2.98 | 0.056080 | 0.008223 | unchanged |
| set-xx-existing | redis | 576186 | 575467–579979 | 318.0 | 99.7 | 1.73 | 56.9 | 12.82 | 0.000002 | 0.000005 | unchanged |
| set-xx-existing | valkey | 530402 | 530323–531270 | 347.3 | 99.9 | 1.88 | 44.4 | 12.76 | 0.000004 | 0.000009 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 1.000 | 2.34 | 2.34 | 936.8 | unchanged |
| redis | 1.001 | 0.000 | 13.30 | 13.30 | 10.0 | unchanged |
| valkey | 1.001 | 0.000 | 12.86 | 12.86 | 10.0 | unchanged |

</details>

<details>
<summary>single-core-pipeline-1: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 100000; clients: 8; pipeline: 1; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: 0; client CPUs: 1,2,3.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 51627 | 51177–51736 | 311.5 | 99.8 | 19.30 | 95.4 | 2.93 | 1.007230 | 0.010180 | unchanged |
| ping | redis | 63272 | 63114–63430 | 172.7 | 100.2 | 15.80 | 85.4 | 12.95 | 0.000050 | 0.000070 | unchanged |
| ping | valkey | 61646 | 61585–61816 | 177.7 | 100.1 | 16.20 | 84.6 | 12.78 | 0.000090 | 0.000060 | unchanged |
| set | rillcache | 49392 | 49209–49592 | 324.3 | 99.9 | 20.20 | 93.6 | 2.93 | 1.007950 | 0.010680 | unchanged |
| set | redis | 59411 | 59256–59496 | 185.6 | 100.4 | 16.90 | 82.2 | 12.95 | 0.000100 | 0.000080 | unchanged |
| set | valkey | 58135 | 58060–58221 | 190.4 | 100.1 | 17.20 | 77.9 | 12.78 | 0.000100 | 0.000050 | unchanged |
| get | rillcache | 50506 | 49628–50690 | 317.9 | 100.0 | 19.80 | 93.9 | 2.93 | 1.007870 | 0.010020 | unchanged |
| get | redis | 61326 | 61060–61598 | 179.4 | 99.9 | 16.30 | 85.3 | 12.95 | 0.000060 | 0.000070 | unchanged |
| get | valkey | 59347 | 59021–59388 | 185.3 | 100.3 | 16.90 | 82.8 | 12.79 | 0.000040 | 0.000050 | unchanged |
| set-nx-existing | rillcache | 49797 | 49605–49918 | 323.2 | 100.2 | 20.10 | 93.6 | 2.93 | 1.008040 | 0.010320 | unchanged |
| set-nx-existing | redis | 60167 | 59816–60553 | 183.2 | 99.8 | 16.60 | 85.5 | 12.95 | 0.000030 | 0.000070 | unchanged |
| set-nx-existing | valkey | 59372 | 59302–59387 | 185.7 | 99.7 | 16.80 | 84.1 | 12.79 | 0.000040 | 0.000050 | unchanged |
| set-xx-existing | rillcache | 49244 | 48969–49350 | 327.6 | 99.9 | 20.30 | 93.1 | 2.93 | 1.007890 | 0.010790 | unchanged |
| set-xx-existing | redis | 59106 | 58667–59315 | 186.6 | 99.7 | 16.90 | 80.4 | 12.93 | 0.000040 | 0.000080 | unchanged |
| set-xx-existing | valkey | 57935 | 57861–57963 | 191.0 | 100.2 | 17.30 | 79.2 | 12.74 | 0.000060 | 0.000040 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 1.000 | 2.34 | 2.34 | 931.8 | unchanged |
| redis | 1.001 | 0.000 | 13.45 | 13.45 | 10.0 | unchanged |
| valkey | 1.001 | 0.000 | 12.93 | 12.93 | 10.0 | unchanged |

</details>

<details>
<summary>single-core-pipeline-16: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 1000000; clients: 8; pipeline: 16; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: 0; client CPUs: 1,2,3.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 781773 | 779069–785440 | 330.3 | 99.2 | 1.27 | 92.2 | 2.99 | 0.062972 | 0.000697 | unchanged |
| ping | redis | 796817 | 789042–800320 | 224.9 | 100.2 | 1.26 | 66.9 | 13.34 | 0.000006 | 0.000011 | unchanged |
| ping | valkey | 748496 | 748302–749221 | 239.5 | 99.6 | 1.33 | 63.4 | 14.78 | 0.000010 | 0.000005 | unchanged |
| set | rillcache | 681750 | 678183–681773 | 378.2 | 100.2 | 1.47 | 81.0 | 2.99 | 0.063140 | 0.000902 | unchanged |
| set | redis | 600540 | 600363–601250 | 304.7 | 99.7 | 1.66 | 54.2 | 12.82 | 0.000006 | 0.000006 | unchanged |
| set | valkey | 542984 | 542737–544296 | 338.0 | 100.1 | 1.84 | 44.6 | 14.60 | 0.000004 | 0.000007 | unchanged |
| get | rillcache | 717985 | 716566–719658 | 358.6 | 99.8 | 1.39 | 84.9 | 2.99 | 0.062935 | 0.000863 | unchanged |
| get | redis | 672617 | 672265–672630 | 269.5 | 100.1 | 1.49 | 64.9 | 12.82 | 0.000006 | 0.000004 | unchanged |
| get | valkey | 612525 | 609494–614184 | 296.9 | 99.5 | 1.62 | 53.7 | 14.61 | 0.000005 | 0.000005 | unchanged |
| set-nx-existing | rillcache | 699803 | 698226–704693 | 369.5 | 99.8 | 1.43 | 82.5 | 2.99 | 0.063107 | 0.000885 | unchanged |
| set-nx-existing | redis | 626455 | 625201–627863 | 290.1 | 100.2 | 1.60 | 54.4 | 12.82 | 0.000003 | 0.000005 | unchanged |
| set-nx-existing | valkey | 598921 | 597028–599360 | 304.6 | 99.7 | 1.67 | 53.6 | 14.61 | 0.000004 | 0.000005 | unchanged |
| set-xx-existing | rillcache | 670104 | 668127–671298 | 387.5 | 100.0 | 1.49 | 79.2 | 2.99 | 0.063159 | 0.000934 | unchanged |
| set-xx-existing | redis | 581179 | 580943–581797 | 314.5 | 99.5 | 1.71 | 55.6 | 12.82 | 0.000003 | 0.000006 | unchanged |
| set-xx-existing | valkey | 529681 | 528950–530282 | 347.8 | 100.1 | 1.89 | 47.1 | 14.61 | 0.000006 | 0.000005 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 1.000 | 2.34 | 2.34 | 934.8 | unchanged |
| redis | 1.001 | 0.000 | 13.31 | 13.31 | 10.0 | unchanged |
| valkey | 1.001 | 0.000 | 14.76 | 14.76 | 10.0 | unchanged |

</details>

### Fixed-worker mode measurements

30/30 separate embedded records validated. 8 clients; 10000 commands/workload/run; pipeline 16; three repetitions. These share the client/server host and inherited CPU affinity; they are **not native competitor comparisons or equal-per-core scaling evidence**. No process CPU/RSS accounting was collected for this group. Fixed lanes have a 1 ms idle polling wait. They actively poll for at most 50 µs after progress before returning to idle parking; this trades active CPU work for fewer premature sleeps, not epoll readiness or free CPU.

| Workload | 1 worker median cmd/s | 1 worker min–max | 4 workers median cmd/s | 4 workers min–max | 1 worker batch p99 µs | 4 workers batch p99 µs |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| ping | 926752 | 882649–989755 | 1168617 | 1058641–1210874 | 937.8 | 1151.4 |
| set | 764510 | 755552–831712 | 979402 | 851274–1035033 | 951.1 | 1231.0 |
| get | 843909 | 812225–871466 | 1033108 | 961655–1057120 | 948.6 | 1558.2 |
| set-nx-existing | 828140 | 798523–839879 | 1001915 | 931390–1049150 | 925.0 | 1268.6 |
| set-xx-existing | 776270 | 768449–830841 | 965764 | 933228–1133559 | 951.3 | 1193.6 |

### Interpretation limits

- Batch p99 is **not per-request p99**; never divide it by the pipeline length. Percentiles are not pooled across runs.
- CPU counters are tick-quantized; zero observed idle ticks do not prove zero CPU cost. CPU observation windows bracket, but are not identical to, request timers.
- RSS is an approximate endpoint sample, not allocator accounting or a workload peak; VmHWM includes startup, warmup and previous workloads.
- Context switches cover matched thread lifetimes only. Even unchanged endpoints cannot exclude threads born and destroyed between snapshots. PARTIAL coverage is not a process total.
- NX-existing measures rejected writes; XX-existing measures admitted overwrites. This is a small Redis-compatible subset, not equivalent functionality.
