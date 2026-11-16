# Current benchmark performance

Generated: 2026-11-16T09:32:31Z · [Benchmark run #18](https://github.com/macel94/rillcache/actions/runs/43839258578) · [Tested source `868cf54`](https://github.com/macel94/rillcache/commit/868cf54df2131d768d760ce71ca30483199f48fb)

**Benchmark job: success. Data: validated. Copilot review: completed.**

Benchmark attempt: 1; review attempt: 1. Preferred model: `gpt-6-luna` via **GitHub Copilot**.

[This report's permanent copy](https://github.com/macel94/rillcache/blob/main/benchmarks/reports/43839258578-1.md) · [Report history](https://github.com/macel94/rillcache/tree/main/benchmarks/reports) · [Methodology](https://github.com/macel94/rillcache/blob/main/docs/benchmarks.md)

## AI review

Actual model(s): **gpt-6-luna**. Selection: automatic-fallback. Authentication: workflow-token.

AI interpretation, **not measurements or a performance gate**. Verify claims against the deterministic tables below.

> Bottom line: In this validated native cohort, Rillcache has higher median throughput and lower
> whole-batch p99 than Redis and Valkey on these workloads—not evidence of a general Redis/Valkey
> victory. At an equal one-logical-CPU server budget, SET pipeline 1 measured 234k commands/s versus
> Redis’s 187k, with batch p99 46.1 vs 67.3 µs. At pipeline 16, it measured 2.10m vs 1.31m commands/s,
> with batch p99 77.7 vs 150.9 µs. These p99 values are for whole batches, not per-request latency;
> do not divide them by pipeline length. NX-existing rejects writes, while XX-existing admits
> overwrites.
> 
> Resource and scheduling observations: In those equal-CPU SET runs, CPU use was about 99–101% for all
> three servers; CPU time was 4.2 vs 5.4 µs/command at pipeline 1 and 0.48 vs 0.76 at pipeline 16
> (Rillcache vs Redis). Pipeline-16 endpoint RSS was 6.29 vs 8.83 MiB; these are approximate samples,
> not workload peaks. Matched-lifetime voluntary switches were 0.000002 vs 0.000044 per command, but
> that counter does not establish process-wide totals or explain performance. Default-threading
> figures are a separate profile and must not be conflated with the equal-one-CPU results.
> 
> Uncertainty: Results cover three runs per workload on a shared VM and a limited Redis-compatible
> subset. CPU observation windows differ from request timers; zero idle CPU ticks are quantized. RSS
> is approximate, and VmHWM is a lifetime peak. The separate fixed-worker measurements are not native
> competitor or equal-per-core comparisons.
> 
> Next investigations:
> 
>  1. Repeat the equal-CPU native comparison under longer, isolated runs, collecting per-request
> latency distributions as well as batch p99 to test tail behavior and run-to-run stability.
>  2. Diagnose the separate fixed-worker mutation result: four workers measured 103k SET commands/s
> and 3.21 ms batch p99 versus one worker’s 1.67m and 1.02 ms. Profile lane contention, polling, and
> yield/park behavior before drawing conclusions about the cause.

## Measurements

**Data quality: complete.** 180/180 records validated across 4/4 complete profiles.


Hardware: Intel(R) Xeon(R) 6975P-C. Rust 1.99.1 (58cf8f9c3 2026-10-13).

References: Redis 8.10.2 sha=00000000:0 malloc=jemalloc-5.3.0 bits=64 build=294cf3a2a45c5b74; Valkey 9.1.2 sha=00000000:0 malloc=jemalloc-5.3.0 bits=64 build=645782e00d4a38db.

### SET at a glance

Median commands/s and median of the three runs' **whole-batch p99** values (µs). Positive rate delta means Rillcache is faster in this workload only.

| Profile | Rillcache cmd/s | Redis | Valkey | Rillcache vs Redis | Rillcache p99 µs | Redis p99 | Valkey p99 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| pipeline-1 | 233767 | 184752 | 175978 | +26.5% | 46.2 | 68.6 | 72.4 |
| pipeline-16 | 2118367 | 1330420 | 1145882 | +59.2% | 77.2 | 149.0 | 173.8 |
| single-core-pipeline-1 | 234219 | 187025 | 178025 | +25.2% | 46.1 | 67.3 | 70.6 |
| single-core-pipeline-16 | 2097866 | 1313131 | 1152237 | +59.8% | 77.7 | 150.9 | 172.6 |

CPU is relative to **one logical CPU**, so parallel servers can exceed 100%. Do not blend default-threading and single-core results or different pipelines. Shared VMs and unequal feature sets do not establish a general performance win or a cross-run regression.

<details>
<summary>pipeline-1: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 100000; clients: 8; pipeline: 1; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: inherited; client CPUs: inherited.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 254550 | 250939–255038 | 43.4 | 100.4 | 4.00 | 92.5 | 6.17 | 0.000010 | 0.000030 | unchanged |
| ping | redis | 200536 | 199054–200620 | 62.8 | 100.3 | 5.00 | 84.3 | 9.02 | 0.000350 | 0.000040 | unchanged |
| ping | valkey | 194001 | 164864–195252 | 65.0 | 99.5 | 5.20 | 82.7 | 8.28 | 0.000530 | 0.000060 | unchanged |
| set | rillcache | 233767 | 232999–234842 | 46.2 | 98.6 | 4.20 | 88.4 | 6.17 | 0.000010 | 0.000030 | unchanged |
| set | redis | 184752 | 184686–185479 | 68.6 | 99.7 | 5.40 | 83.3 | 9.02 | 0.000330 | 0.000030 | unchanged |
| set | valkey | 175978 | 175629–176167 | 72.4 | 98.6 | 5.60 | 80.4 | 8.28 | 0.000300 | 0.000080 | unchanged |
| get | rillcache | 242755 | 242242–243531 | 44.6 | 99.5 | 4.10 | 90.2 | 6.17 | 0.000010 | 0.000030 | unchanged |
| get | redis | 192891 | 191614–195327 | 65.5 | 101.5 | 5.20 | 84.6 | 9.02 | 0.000700 | 0.000020 | unchanged |
| get | valkey | 181355 | 181193–182414 | 70.9 | 99.7 | 5.50 | 80.0 | 8.29 | 0.000270 | 0.000050 | unchanged |
| set-nx-existing | rillcache | 239616 | 238007–239642 | 45.2 | 100.6 | 4.20 | 88.1 | 6.17 | 0.000010 | 0.000030 | unchanged |
| set-nx-existing | redis | 187624 | 187370–188816 | 67.3 | 99.3 | 5.30 | 81.1 | 9.02 | 0.000500 | 0.000050 | unchanged |
| set-nx-existing | valkey | 180810 | 180134–180843 | 70.4 | 100.8 | 5.60 | 80.4 | 8.28 | 0.000280 | 0.000170 | unchanged |
| set-xx-existing | rillcache | 232310 | 232160–234086 | 46.6 | 100.6 | 4.30 | 86.0 | 6.17 | 0.000010 | 0.000030 | unchanged |
| set-xx-existing | redis | 182948 | 182777–184426 | 69.2 | 98.8 | 5.40 | 85.2 | 9.02 | 0.000130 | 0.000090 | unchanged |
| set-xx-existing | valkey | 173833 | 172373–174309 | 73.2 | 99.1 | 5.70 | 75.4 | 8.28 | 0.000410 | 0.000060 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 4.88 | 4.88 | 99.0 | unchanged |
| redis | 1.000 | 0.000 | 8.77 | 8.77 | 10.0 | unchanged |
| valkey | 1.000 | 0.000 | 8.11 | 8.11 | 10.0 | unchanged |

</details>

<details>
<summary>pipeline-16: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 1000000; clients: 8; pipeline: 16; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: inherited; client CPUs: inherited.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 3641559 | 3629393–3731916 | 47.8 | 98.3 | 0.27 | 84.6 | 6.11 | 0.000002 | 0.000008 | unchanged |
| ping | redis | 1983993 | 1975539–1991538 | 98.9 | 100.7 | 0.51 | 58.0 | 8.95 | 0.000018 | 0.000003 | unchanged |
| ping | valkey | 1762801 | 1758873–1765128 | 112.5 | 100.4 | 0.57 | 50.9 | 8.46 | 0.000014 | 0.000001 | unchanged |
| set | rillcache | 2118367 | 2099572–2124297 | 77.2 | 99.8 | 0.47 | 50.0 | 6.11 | 0.000001 | 0.000002 | unchanged |
| set | redis | 1330420 | 1319377–1395773 | 149.0 | 98.9 | 0.74 | 39.4 | 8.95 | 0.000011 | 0.000002 | unchanged |
| set | valkey | 1145882 | 1145178–1146930 | 173.8 | 100.8 | 0.88 | 37.5 | 8.46 | 0.000010 | 0.000004 | unchanged |
| get | rillcache | 2675465 | 2624428–2714072 | 64.3 | 101.7 | 0.38 | 67.6 | 6.11 | 0.000001 | 0.000002 | unchanged |
| get | redis | 1503254 | 1499614–1508783 | 133.0 | 100.5 | 0.67 | 46.3 | 8.95 | 0.000023 | 0.000003 | unchanged |
| get | valkey | 1340533 | 1337088–1349196 | 149.3 | 99.2 | 0.74 | 44.6 | 8.46 | 0.000009 | 0.000004 | unchanged |
| set-nx-existing | rillcache | 2414923 | 2378995–2473283 | 70.0 | 99.9 | 0.42 | 61.9 | 6.11 | 0.000001 | 0.000004 | unchanged |
| set-nx-existing | redis | 1371103 | 1365874–1372681 | 144.6 | 101.1 | 0.74 | 40.5 | 8.95 | 0.000026 | 0.000006 | unchanged |
| set-nx-existing | valkey | 1290693 | 1286859–1294718 | 153.9 | 99.1 | 0.77 | 39.5 | 8.46 | 0.000011 | 0.000004 | unchanged |
| set-xx-existing | rillcache | 2090624 | 2057727–2092800 | 78.0 | 100.3 | 0.48 | 50.0 | 6.11 | 0.000001 | 0.000001 | unchanged |
| set-xx-existing | redis | 1273600 | 1272754–1275828 | 155.9 | 99.3 | 0.78 | 38.5 | 8.95 | 0.000011 | 0.000003 | unchanged |
| set-xx-existing | valkey | 1111808 | 1109002–1114644 | 179.7 | 100.3 | 0.90 | 36.7 | 8.46 | 0.000012 | 0.000004 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 4.82 | 4.82 | 99.0 | unchanged |
| redis | 1.000 | 0.000 | 8.70 | 8.70 | 10.0 | unchanged |
| valkey | 1.000 | 0.000 | 8.27 | 8.27 | 10.0 | unchanged |

</details>

<details>
<summary>single-core-pipeline-1: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 100000; clients: 8; pipeline: 1; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: 0; client CPUs: 1,2,3.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 256307 | 253547–258813 | 42.3 | 98.9 | 3.90 | 94.7 | 6.12 | 0.000010 | 0.000010 | unchanged |
| ping | redis | 202061 | 201714–202691 | 62.1 | 98.8 | 4.90 | 87.8 | 8.93 | 0.000340 | 0.000020 | unchanged |
| ping | valkey | 196571 | 195638–196847 | 64.1 | 99.7 | 5.10 | 84.3 | 8.41 | 0.000240 | 0.000020 | unchanged |
| set | rillcache | 234219 | 233114–236270 | 46.1 | 99.2 | 4.20 | 88.1 | 6.12 | 0.000010 | 0.000010 | unchanged |
| set | redis | 187025 | 186308–187667 | 67.3 | 100.6 | 5.40 | 83.3 | 8.93 | 0.000360 | 0.000020 | unchanged |
| set | valkey | 178025 | 177579–179045 | 70.6 | 100.2 | 5.60 | 78.6 | 8.41 | 0.000360 | 0.000020 | unchanged |
| get | rillcache | 243447 | 242777–243939 | 44.3 | 99.8 | 4.10 | 90.2 | 6.12 | 0.000010 | 0.000010 | unchanged |
| get | redis | 194899 | 194593–194905 | 64.6 | 99.4 | 5.10 | 82.4 | 8.93 | 0.000210 | 0.000020 | unchanged |
| get | valkey | 184774 | 184417–185266 | 68.2 | 101.4 | 5.50 | 79.2 | 8.41 | 0.000170 | 0.000010 | unchanged |
| set-nx-existing | rillcache | 239181 | 239095–241664 | 44.9 | 100.4 | 4.20 | 90.5 | 6.12 | 0.000010 | 0.000010 | unchanged |
| set-nx-existing | redis | 189762 | 185717–189786 | 66.8 | 100.5 | 5.30 | 81.1 | 8.93 | 0.000600 | 0.000010 | unchanged |
| set-nx-existing | valkey | 182515 | 182214–182555 | 68.9 | 100.4 | 5.50 | 80.4 | 8.40 | 0.000140 | 0.000020 | unchanged |
| set-xx-existing | rillcache | 235291 | 232739–235315 | 46.0 | 100.1 | 4.30 | 88.4 | 6.12 | 0.000010 | 0.000010 | unchanged |
| set-xx-existing | redis | 185753 | 185614–185826 | 67.5 | 100.2 | 5.40 | 79.2 | 8.93 | 0.000160 | 0.000010 | unchanged |
| set-xx-existing | valkey | 176155 | 175779–176846 | 71.6 | 100.4 | 5.70 | 78.6 | 8.41 | 0.000120 | 0.000020 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 4.88 | 4.88 | 99.0 | unchanged |
| redis | 1.000 | 0.000 | 8.70 | 8.70 | 10.0 | unchanged |
| valkey | 1.000 | 0.000 | 8.23 | 8.23 | 10.0 | unchanged |

</details>

<details>
<summary>single-core-pipeline-16: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 1000000; clients: 8; pipeline: 16; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: 0; client CPUs: 1,2,3.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 3661410 | 3608010–3708555 | 48.3 | 100.1 | 0.27 | 88.5 | 6.29 | 0.000001 | 0.000000 | unchanged |
| ping | redis | 1972393 | 1972314–1977640 | 99.3 | 98.8 | 0.50 | 54.9 | 8.83 | 0.000018 | 0.000003 | unchanged |
| ping | valkey | 1772379 | 1771144–1772741 | 110.9 | 100.9 | 0.57 | 50.0 | 8.24 | 0.000025 | 0.000003 | unchanged |
| set | rillcache | 2097866 | 2096880–2104645 | 77.7 | 100.6 | 0.48 | 52.1 | 6.29 | 0.000002 | 0.000001 | unchanged |
| set | redis | 1313131 | 1312358–1316728 | 150.9 | 100.0 | 0.76 | 40.3 | 8.83 | 0.000044 | 0.000002 | unchanged |
| set | valkey | 1152237 | 1147952–1153613 | 172.6 | 99.9 | 0.87 | 36.8 | 8.24 | 0.000012 | 0.000002 | unchanged |
| get | rillcache | 2653206 | 2637675–2656988 | 65.8 | 100.8 | 0.38 | 65.8 | 6.29 | 0.000001 | 0.000001 | unchanged |
| get | redis | 1483302 | 1482101–1484215 | 135.1 | 100.8 | 0.68 | 44.8 | 8.84 | 0.000013 | 0.000002 | unchanged |
| get | valkey | 1360262 | 1359191–1365215 | 147.1 | 99.3 | 0.73 | 41.1 | 8.23 | 0.000011 | 0.000003 | unchanged |
| set-nx-existing | rillcache | 2402726 | 2400635–2409329 | 70.3 | 100.8 | 0.42 | 63.4 | 6.29 | 0.000001 | 0.000002 | unchanged |
| set-nx-existing | redis | 1352958 | 1350637–1354270 | 146.5 | 100.1 | 0.74 | 39.2 | 8.84 | 0.000027 | 0.000004 | unchanged |
| set-nx-existing | valkey | 1294414 | 1293265–1297449 | 152.9 | 99.9 | 0.77 | 39.0 | 8.23 | 0.000017 | 0.000002 | unchanged |
| set-xx-existing | rillcache | 2064617 | 2059326–2076939 | 78.3 | 99.7 | 0.48 | 51.0 | 6.29 | 0.000001 | 0.000000 | unchanged |
| set-xx-existing | redis | 1256925 | 1255262–1262129 | 157.8 | 99.3 | 0.79 | 38.0 | 8.83 | 0.000020 | 0.000001 | unchanged |
| set-xx-existing | valkey | 1117979 | 1115647–1119760 | 177.3 | 99.5 | 0.89 | 36.0 | 8.23 | 0.000009 | 0.000002 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 4.89 | 4.89 | 99.0 | unchanged |
| redis | 1.000 | 0.000 | 8.57 | 8.57 | 10.0 | unchanged |
| valkey | 1.000 | 0.000 | 8.04 | 8.04 | 10.0 | unchanged |

</details>

### Fixed-worker mode measurements

30/30 separate embedded records validated. 8 clients; 10000 commands/workload/run; pipeline 16; three repetitions. These share the client/server host and inherited CPU affinity; they are **not native competitor comparisons or equal-per-core scaling evidence**. No process CPU/RSS accounting was collected for this group. Fixed lanes have a 1 ms idle polling wait. They actively poll for at most 50 µs after progress before returning to idle parking; this trades active CPU work for fewer premature sleeps, not epoll readiness or free CPU. Multiple lanes also request a cooperative yield after 100 µs of continuous active passes when more than one client is admitted. Progress does not rearm that separate budget; single lanes/lone clients do not periodically yield. This is checked between bounded passes, not a hard deadline or an OS fairness guarantee. The v2 interval restarts after a yield/park returns, excluding that explicit wait; involuntary descheduling can still consume wall time.

| Workload | 1 worker median cmd/s | 1 worker min–max | 4 workers median cmd/s | 4 workers min–max | 1 worker batch p99 µs | 4 workers batch p99 µs |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| ping | 2555421 | 2548746–2594615 | 3090603 | 2900295–3434441 | 1003.2 | 814.8 |
| set | 1672622 | 1531508–1699818 | 103129 | 97371–104668 | 1023.5 | 3211.0 |
| get | 1977931 | 1959735–2002194 | 100509 | 100275–100527 | 1017.0 | 3337.9 |
| set-nx-existing | 1859295 | 1835892–1933610 | 112817 | 110390–114369 | 1017.5 | 2616.4 |
| set-xx-existing | 1667111 | 1667018–1677052 | 107116 | 103352–108571 | 1020.3 | 2806.9 |

### Interpretation limits

- Batch p99 is **not per-request p99**; never divide it by the pipeline length. Percentiles are not pooled across runs.
- CPU counters are tick-quantized; zero observed idle ticks do not prove zero CPU cost. CPU observation windows bracket, but are not identical to, request timers.
- RSS is an approximate endpoint sample, not allocator accounting or a workload peak; VmHWM includes startup, warmup and previous workloads.
- Context switches cover matched thread lifetimes only. Even unchanged endpoints cannot exclude threads born and destroyed between snapshots. PARTIAL coverage is not a process total.
- NX-existing measures rejected writes; XX-existing measures admitted overwrites. This is a small Redis-compatible subset, not equivalent functionality.
