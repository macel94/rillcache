# Current benchmark performance

Generated: 2026-11-10T16:37:06Z · [Benchmark run #17](https://github.com/macel94/rillcache/actions/runs/42854338669) · [Tested source `4209859`](https://github.com/macel94/rillcache/commit/42098598d2e3cf8fabef1cf32f06d529eb69b0f5)

**Benchmark job: success. Data: validated. Copilot review: completed.**

Benchmark attempt: 1; review attempt: 1. Preferred model: `gpt-6-luna` via **GitHub Copilot**.

[This report's permanent copy](https://github.com/macel94/rillcache/blob/main/benchmarks/reports/42854338669-1.md) · [Report history](https://github.com/macel94/rillcache/tree/main/benchmarks/reports) · [Methodology](https://github.com/macel94/rillcache/blob/main/docs/benchmarks.md)

## AI review

Actual model(s): **gpt-6-luna**. Selection: automatic-fallback. Authentication: workflow-token.

AI interpretation, **not measurements or a performance gate**. Verify claims against the deterministic tables below.

> Bottom line: In this validated native cohort of 180 records, Rillcache had higher SET throughput and
> lower whole-batch p99 than Redis and Valkey in the measured workloads. This is workload-specific
> evidence, not a general Redis/Valkey victory.
> 
> Equal-one-logical-CPU: With the server pinned to one CPU, pipeline-1 SET reached 96,714 commands/s
> versus Redis’s 88,206 (+9.6%); whole-batch p99 was 96.5 µs versus 118.1 µs. At pipeline-16,
> throughput was 1,187,145 versus 879,861 (+34.9%), with batch p99 of 124.5 versus 217.0 µs. These are
> batch percentiles—not per-request latency, and they should not be divided by pipeline length.
> Rillcache’s CPU use was near 100% in both cases; at pipeline-16 it used 0.85 CPU µs/command versus
> Redis’s 1.13. Default-threading results are separate: do not treat them as equal-CPU comparisons.
> 
> Resource and scheduling observations: At single-core pipeline-16, endpoint RSS was about 5.96 MiB
> for Rillcache, 9.04 for Redis, and 8.34 for Valkey; RSS is approximate, while VmHWM is a lifetime
> peak. Rillcache showed fewer voluntary switches per command in these rows, but matched
> thread-lifetime counters are not guaranteed process totals and do not establish why throughput
> differs. Idle CPU readings of zero are tick-quantized; they do not prove zero cost.
> 
> Uncertainty: Three runs per workload, shared-VM conditions, observation-window differences, and the
> subset’s limited functionality constrain interpretation. NX-existing rejects writes; XX-existing
> admits overwrites.
> 
> Next investigations: (1) Measure per-request latency distributions and repeat across client counts
> and pipeline depths; current batch p99 rises from 96.5 µs at pipeline-1 to 124.5 µs at pipeline-16.
> (2) Separately diagnose fixed-worker behavior: four-worker SET was about 49,722 commands/s with
> 4,379.6 µs batch p99, versus one worker’s 973,883 and 965.7 µs. That embedded group is not a native
> competitor or equal-per-core comparison and lacks CPU/RSS accounting; add those measurements while
> examining queueing and worker coordination.

## Measurements

**Data quality: complete.** 180/180 records validated across 4/4 complete profiles.


Hardware: AMD EPYC 9V74 80-Core Processor. Rust 1.99.1 (58cf8f9c3 2026-10-13).

References: Redis 8.10.2 sha=00000000:0 malloc=jemalloc-5.3.0 bits=64 build=560571108c09b049; Valkey 9.1.2 sha=00000000:0 malloc=jemalloc-5.3.0 bits=64 build=fdee4c72b466a6c8.

### SET at a glance

Median commands/s and median of the three runs' **whole-batch p99** values (µs). Positive rate delta means Rillcache is faster in this workload only.

| Profile | Rillcache cmd/s | Redis | Valkey | Rillcache vs Redis | Rillcache p99 µs | Redis p99 | Valkey p99 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| pipeline-1 | 97258 | 88692 | 85394 | +9.7% | 96.8 | 117.7 | 122.3 |
| pipeline-16 | 1191438 | 878023 | 769265 | +35.7% | 125.4 | 217.8 | 252.9 |
| single-core-pipeline-1 | 96714 | 88206 | 85653 | +9.6% | 96.5 | 118.1 | 122.1 |
| single-core-pipeline-16 | 1187145 | 879861 | 760977 | +34.9% | 124.5 | 217.0 | 254.8 |

CPU is relative to **one logical CPU**, so parallel servers can exceed 100%. Do not blend default-threading and single-core results or different pipelines. Shared VMs and unequal feature sets do not establish a general performance win or a cross-run regression.

<details>
<summary>pipeline-1: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 100000; clients: 8; pipeline: 1; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: inherited; client CPUs: inherited.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 101185 | 101128–102240 | 93.3 | 99.1 | 9.80 | 95.9 | 5.82 | 0.000010 | 0.000060 | unchanged |
| ping | redis | 91607 | 91427–91709 | 111.0 | 99.8 | 10.90 | 90.8 | 8.94 | 0.000190 | 0.000050 | unchanged |
| ping | valkey | 90098 | 88756–90240 | 113.9 | 100.1 | 11.10 | 90.1 | 8.28 | 0.000110 | 0.000080 | unchanged |
| set | rillcache | 97258 | 96604–98883 | 96.8 | 99.5 | 10.20 | 91.3 | 5.82 | 0.000010 | 0.000050 | unchanged |
| set | redis | 88692 | 88064–88711 | 117.7 | 99.5 | 11.30 | 86.8 | 8.95 | 0.000160 | 0.000050 | unchanged |
| set | valkey | 85394 | 84601–85510 | 122.3 | 99.8 | 11.70 | 88.9 | 8.30 | 0.000550 | 0.000040 | unchanged |
| get | rillcache | 99296 | 98423–99965 | 94.8 | 99.4 | 10.00 | 94.0 | 5.82 | 0.000010 | 0.000050 | unchanged |
| get | redis | 89523 | 88847–89886 | 115.0 | 99.7 | 11.20 | 88.4 | 8.94 | 0.000070 | 0.000030 | unchanged |
| get | valkey | 87081 | 86369–87231 | 120.1 | 100.1 | 11.50 | 87.0 | 8.29 | 0.000040 | 0.000060 | unchanged |
| set-nx-existing | rillcache | 99079 | 98432–99136 | 95.0 | 100.1 | 10.10 | 92.1 | 5.82 | 0.000010 | 0.000060 | unchanged |
| set-nx-existing | redis | 89069 | 88679–89424 | 116.2 | 99.7 | 11.20 | 87.6 | 8.94 | 0.000320 | 0.000030 | unchanged |
| set-nx-existing | valkey | 85822 | 85647–86092 | 120.8 | 99.3 | 11.60 | 86.2 | 8.29 | 0.000140 | 0.000060 | unchanged |
| set-xx-existing | rillcache | 98291 | 97533–99359 | 95.9 | 100.2 | 10.20 | 91.0 | 5.82 | 0.000010 | 0.000040 | unchanged |
| set-xx-existing | redis | 88281 | 88010–88388 | 117.8 | 99.8 | 11.30 | 85.8 | 8.94 | 0.000390 | 0.000050 | unchanged |
| set-xx-existing | valkey | 85481 | 84987–85521 | 122.9 | 100.2 | 11.80 | 86.3 | 8.28 | 0.000080 | 0.000050 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 4.66 | 4.66 | 99.0 | unchanged |
| redis | 1.000 | 0.000 | 8.73 | 8.73 | 10.0 | unchanged |
| valkey | 1.001 | 0.000 | 8.12 | 8.12 | 10.0 | unchanged |

</details>

<details>
<summary>pipeline-16: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 1000000; clients: 8; pipeline: 16; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: inherited; client CPUs: inherited.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 1572733 | 1560758–1574163 | 95.8 | 99.1 | 0.63 | 93.7 | 5.89 | 0.000001 | 0.000009 | unchanged |
| ping | redis | 1163308 | 1126148–1170292 | 153.5 | 100.6 | 0.87 | 70.1 | 8.98 | 0.000004 | 0.000004 | unchanged |
| ping | valkey | 1109782 | 1107894–1110020 | 164.0 | 100.8 | 0.91 | 68.1 | 8.45 | 0.000006 | 0.000003 | unchanged |
| set | rillcache | 1191438 | 1187902–1195815 | 125.4 | 98.9 | 0.83 | 69.9 | 5.89 | 0.000001 | 0.000011 | unchanged |
| set | redis | 878023 | 864589–879669 | 217.8 | 100.3 | 1.14 | 55.3 | 9.00 | 0.000004 | 0.000004 | unchanged |
| set | valkey | 769265 | 765859–772194 | 252.9 | 99.6 | 1.30 | 47.7 | 8.45 | 0.000014 | 0.000006 | unchanged |
| get | rillcache | 1379877 | 1369490–1383848 | 109.4 | 100.7 | 0.73 | 78.1 | 5.89 | 0.000001 | 0.000004 | unchanged |
| get | redis | 977880 | 954376–980246 | 190.7 | 99.9 | 1.03 | 61.5 | 9.01 | 0.000002 | 0.000005 | unchanged |
| get | valkey | 901855 | 899945–902504 | 209.5 | 100.1 | 1.11 | 61.3 | 8.46 | 0.000024 | 0.000004 | unchanged |
| set-nx-existing | rillcache | 1317039 | 1293057–1329966 | 115.1 | 99.5 | 0.75 | 72.0 | 5.89 | 0.000001 | 0.000005 | unchanged |
| set-nx-existing | redis | 906670 | 906307–910156 | 208.9 | 99.7 | 1.10 | 61.5 | 9.02 | 0.000005 | 0.000006 | unchanged |
| set-nx-existing | valkey | 873669 | 872740–874896 | 218.0 | 100.4 | 1.15 | 53.9 | 8.45 | 0.000010 | 0.000003 | unchanged |
| set-xx-existing | rillcache | 1174941 | 1160545–1191784 | 129.6 | 99.9 | 0.85 | 67.1 | 5.89 | 0.000001 | 0.000009 | unchanged |
| set-xx-existing | redis | 849802 | 846296–850345 | 225.5 | 100.2 | 1.18 | 53.4 | 8.98 | 0.000005 | 0.000005 | unchanged |
| set-xx-existing | valkey | 748586 | 728411–749017 | 260.0 | 100.3 | 1.34 | 41.8 | 8.45 | 0.000013 | 0.000005 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 4.80 | 4.80 | 99.0 | unchanged |
| redis | 1.000 | 0.000 | 8.76 | 8.76 | 10.0 | unchanged |
| valkey | 1.000 | 0.000 | 8.26 | 8.26 | 10.0 | unchanged |

</details>

<details>
<summary>single-core-pipeline-1: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 100000; clients: 8; pipeline: 1; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: 0; client CPUs: 1,2,3.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 99686 | 99547–102348 | 93.5 | 99.3 | 9.90 | 96.0 | 5.94 | 0.000010 | 0.000030 | unchanged |
| ping | redis | 91579 | 91317–91802 | 111.4 | 99.8 | 10.90 | 90.8 | 8.92 | 0.000120 | 0.000050 | unchanged |
| ping | valkey | 89954 | 89867–90020 | 113.7 | 100.6 | 11.20 | 90.2 | 8.27 | 0.000180 | 0.000040 | unchanged |
| set | rillcache | 96714 | 95126–98974 | 96.5 | 99.9 | 10.30 | 92.4 | 5.94 | 0.000010 | 0.000060 | unchanged |
| set | redis | 88206 | 87900–88252 | 118.1 | 99.7 | 11.30 | 87.6 | 8.93 | 0.000070 | 0.000040 | unchanged |
| set | valkey | 85653 | 85646–85751 | 122.1 | 100.2 | 11.70 | 83.8 | 8.30 | 0.000050 | 0.000050 | unchanged |
| get | rillcache | 97680 | 97630–99576 | 95.0 | 100.5 | 10.20 | 93.2 | 5.94 | 0.000020 | 0.000040 | unchanged |
| get | redis | 89194 | 89075–89412 | 115.4 | 99.7 | 11.20 | 90.2 | 8.92 | 0.000090 | 0.000060 | unchanged |
| get | valkey | 87207 | 86054–87282 | 119.4 | 99.8 | 11.50 | 87.0 | 8.28 | 0.000090 | 0.000030 | unchanged |
| set-nx-existing | rillcache | 96665 | 96474–97945 | 96.5 | 99.9 | 10.30 | 92.3 | 5.94 | 0.000010 | 0.000030 | unchanged |
| set-nx-existing | redis | 88528 | 88484–89010 | 116.7 | 99.1 | 11.20 | 89.3 | 8.92 | 0.000080 | 0.000030 | unchanged |
| set-nx-existing | valkey | 86691 | 86665–87033 | 120.5 | 100.5 | 11.60 | 85.3 | 8.28 | 0.000020 | 0.000030 | unchanged |
| set-xx-existing | rillcache | 95998 | 95212–97488 | 97.4 | 99.8 | 10.40 | 91.3 | 5.94 | 0.000010 | 0.000040 | unchanged |
| set-xx-existing | redis | 87744 | 87575–87931 | 118.8 | 99.8 | 11.40 | 86.7 | 8.92 | 0.000070 | 0.000040 | unchanged |
| set-xx-existing | valkey | 85414 | 84961–85457 | 122.6 | 100.2 | 11.80 | 87.3 | 8.27 | 0.000090 | 0.000030 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 4.80 | 4.80 | 99.0 | unchanged |
| redis | 1.000 | 0.000 | 8.71 | 8.71 | 10.0 | unchanged |
| valkey | 1.001 | 0.000 | 8.11 | 8.11 | 10.0 | unchanged |

</details>

<details>
<summary>single-core-pipeline-16: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 1000000; clients: 8; pipeline: 16; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: 0; client CPUs: 1,2,3.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 1589294 | 1585371–1589719 | 94.1 | 100.1 | 0.63 | 93.7 | 5.96 | 0.000001 | 0.000001 | unchanged |
| ping | redis | 1164166 | 1159109–1165294 | 153.3 | 100.1 | 0.86 | 72.1 | 9.02 | 0.000006 | 0.000003 | unchanged |
| ping | valkey | 1109190 | 1102334–1110755 | 163.2 | 99.2 | 0.90 | 69.7 | 8.32 | 0.000005 | 0.000003 | unchanged |
| set | rillcache | 1187145 | 1176245–1202787 | 124.5 | 100.0 | 0.85 | 70.6 | 5.96 | 0.000001 | 0.000006 | unchanged |
| set | redis | 879861 | 879464–882051 | 217.0 | 99.4 | 1.13 | 57.5 | 9.04 | 0.000016 | 0.000003 | unchanged |
| set | valkey | 760977 | 758824–761235 | 254.8 | 99.7 | 1.31 | 45.8 | 8.34 | 0.000017 | 0.000003 | unchanged |
| get | rillcache | 1357093 | 1347328–1398341 | 110.3 | 99.3 | 0.73 | 78.1 | 5.96 | 0.000001 | 0.000001 | unchanged |
| get | redis | 980689 | 954635–980936 | 190.1 | 99.0 | 1.01 | 59.4 | 9.04 | 0.000010 | 0.000005 | unchanged |
| get | valkey | 903964 | 902225–905738 | 208.6 | 100.1 | 1.11 | 59.5 | 8.34 | 0.000011 | 0.000001 | unchanged |
| set-nx-existing | rillcache | 1296556 | 1279855–1327665 | 114.6 | 100.9 | 0.77 | 72.4 | 5.96 | 0.000001 | 0.000002 | unchanged |
| set-nx-existing | redis | 908705 | 887864–910717 | 207.9 | 99.9 | 1.10 | 60.9 | 9.05 | 0.000002 | 0.000003 | unchanged |
| set-nx-existing | valkey | 872458 | 869449–875338 | 218.2 | 100.0 | 1.15 | 53.9 | 8.32 | 0.000012 | 0.000003 | unchanged |
| set-xx-existing | rillcache | 1179200 | 1168121–1180719 | 126.1 | 100.4 | 0.86 | 67.4 | 5.96 | 0.000001 | 0.000004 | unchanged |
| set-xx-existing | redis | 850861 | 848320–851779 | 224.4 | 100.4 | 1.18 | 51.3 | 9.02 | 0.000004 | 0.000003 | unchanged |
| set-xx-existing | valkey | 739572 | 739445–739717 | 262.7 | 100.6 | 1.36 | 53.7 | 8.32 | 0.000009 | 0.000002 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 4.92 | 4.92 | 99.0 | unchanged |
| redis | 1.000 | 0.000 | 8.80 | 8.80 | 10.0 | unchanged |
| valkey | 1.000 | 0.000 | 8.15 | 8.15 | 10.0 | unchanged |

</details>

### Fixed-worker mode measurements

30/30 separate embedded records validated. 8 clients; 10000 commands/workload/run; pipeline 16; three repetitions. These share the client/server host and inherited CPU affinity; they are **not native competitor comparisons or equal-per-core scaling evidence**. No process CPU/RSS accounting was collected for this group. Fixed lanes have a 1 ms idle polling wait. They actively poll for at most 50 µs after progress before returning to idle parking; this trades active CPU work for fewer premature sleeps, not epoll readiness or free CPU. Multiple lanes also request a cooperative yield after 100 µs of continuous active passes when more than one client is admitted. Progress does not rearm that separate budget; single lanes/lone clients do not periodically yield. This is checked between bounded passes, not a hard deadline or an OS fairness guarantee. The v2 interval restarts after a yield/park returns, excluding that explicit wait; involuntary descheduling can still consume wall time.

| Workload | 1 worker median cmd/s | 1 worker min–max | 4 workers median cmd/s | 4 workers min–max | 1 worker batch p99 µs | 4 workers batch p99 µs |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| ping | 1321320 | 1124090–1382228 | 1773467 | 1382316–2111296 | 955.9 | 1068.8 |
| set | 973883 | 917774–1036561 | 49722 | 49039–50062 | 965.7 | 4379.6 |
| get | 1200768 | 1181329–1221614 | 49319 | 49038–50781 | 957.8 | 4153.5 |
| set-nx-existing | 1125055 | 1101214–1149942 | 49670 | 49538–50187 | 945.5 | 4346.5 |
| set-xx-existing | 1037940 | 1027496–1063003 | 49432 | 48831–50365 | 945.1 | 4648.7 |

### Interpretation limits

- Batch p99 is **not per-request p99**; never divide it by the pipeline length. Percentiles are not pooled across runs.
- CPU counters are tick-quantized; zero observed idle ticks do not prove zero CPU cost. CPU observation windows bracket, but are not identical to, request timers.
- RSS is an approximate endpoint sample, not allocator accounting or a workload peak; VmHWM includes startup, warmup and previous workloads.
- Context switches cover matched thread lifetimes only. Even unchanged endpoints cannot exclude threads born and destroyed between snapshots. PARTIAL coverage is not a process total.
- NX-existing measures rejected writes; XX-existing measures admitted overwrites. This is a small Redis-compatible subset, not equivalent functionality.
