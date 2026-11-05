# Current benchmark performance

Generated: 2026-11-05T23:40:14Z · [Benchmark run #15](https://github.com/macel94/rillcache/actions/runs/42129908609) · [Tested source `fd7dbd8`](https://github.com/macel94/rillcache/commit/fd7dbd81e8e5a02b5d4654e45960252a75aef0dd)

**Benchmark job: success. Data: validated. Copilot review: completed.**

Benchmark attempt: 1; review attempt: 1. Preferred model: `gpt-6-luna` via **GitHub Copilot**.

[This report's permanent copy](https://github.com/macel94/rillcache/blob/main/benchmarks/reports/42129908609-1.md) · [Report history](https://github.com/macel94/rillcache/tree/main/benchmarks/reports) · [Methodology](https://github.com/macel94/rillcache/blob/main/docs/benchmarks.md)

## AI review

Actual model(s): **gpt-6-luna**. Selection: automatic-fallback. Authentication: workflow-token.

AI interpretation, **not measurements or a performance gate**. Verify claims against the deterministic tables below.

> Bottom line: In this validated, narrow Redis-compatible workload set, Rillcache has higher median
> throughput and lower whole-batch p99 than Redis and Valkey on the reported SET profiles. This is not
> evidence of a general Redis/Valkey victory: the subset is limited, and NX-existing rejects writes
> while XX-existing admits overwrites.
> 
> Throughput and tail latency: At the default threading and pipeline 1, SET reaches 120,848 commands/s
> versus Redis’s 109,659 (+10.2%); whole-batch p99 is 76.7 µs versus 93.1 µs. At pipeline 16,
> throughput is 1.53M versus 1.14M (+34.2%), with batch p99 of 95.6 versus 161.7 µs. Under an equal
> one-logical-CPU budget, pipeline-1 SET is 118,522 versus 108,990 (+8.7%), and pipeline-16 is 1.56M
> versus 1.15M (+36.5%); corresponding batch p99s are 78.1 vs 93.3 µs and 95.4 vs 161.4 µs. These are
> batch percentiles, not per-request latency; pipeline length cannot be used to divide them.
> 
> CPU, memory, scheduling: CPU readings are near 100% of one logical CPU across these profiles; for
> pipeline-16 SET, reported CPU per command is 0.64 µs for Rillcache, 0.87 for Redis, and 0.98 for
> Valkey. CPU observation windows differ from request timers, and zero idle ticks are quantized.
> Approximate endpoint RSS is lower for Rillcache (about 5.9–6.0 MiB vs roughly 8.3–9.0 MiB), not a
> workload peak or allocator accounting; VmHWM is a lifetime peak. Switch counts are small, but cover
> matched thread lifetimes, not necessarily every process thread, and do not establish cause.
> 
> Uncertainty and next investigations: The 180 native records are validated across three repetitions,
> but shared-VM effects and limited workload shape constrain inference. Separately, fixed-worker
> mode’s four-worker SET/GET medians collapse to about 54k/44k commands/s from about 1.23M/1.43M with
> one worker, with much higher batch p99; it is not native competitor or equal-per-core evidence and
> has no process CPU/RSS accounting.
> 
>  1. Repeat fixed-worker tests with per-lane progress, queueing, and CPU measurements to locate the
> scaling collapse.
>  2. Extend native tests across key/value sizes, client counts, longer runs, and latency
> distributions to assess workload sensitivity and run-to-run uncertainty.

## Measurements

**Data quality: complete.** 180/180 records validated across 4/4 complete profiles.


Hardware: AMD EPYC 9V45 96-Core Processor. Rust 1.99.1 (58cf8f9c3 2026-10-13).

References: Redis 8.10.2 sha=00000000:0 malloc=jemalloc-5.3.0 bits=64 build=92967602054f1ae2; Valkey 9.1.2 sha=00000000:0 malloc=jemalloc-5.3.0 bits=64 build=b2862cde3e536ccd.

### SET at a glance

Median commands/s and median of the three runs' **whole-batch p99** values (µs). Positive rate delta means Rillcache is faster in this workload only.

| Profile | Rillcache cmd/s | Redis | Valkey | Rillcache vs Redis | Rillcache p99 µs | Redis p99 | Valkey p99 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| pipeline-1 | 120848 | 109659 | 106131 | +10.2% | 76.7 | 93.1 | 97.9 |
| pipeline-16 | 1531251 | 1140953 | 1025284 | +34.2% | 95.6 | 161.7 | 182.7 |
| single-core-pipeline-1 | 118522 | 108990 | 104518 | +8.7% | 78.1 | 93.3 | 200.9 |
| single-core-pipeline-16 | 1564608 | 1146249 | 1020435 | +36.5% | 95.4 | 161.4 | 183.3 |

CPU is relative to **one logical CPU**, so parallel servers can exceed 100%. Do not blend default-threading and single-core results or different pipelines. Shared VMs and unequal feature sets do not establish a general performance win or a cross-run regression.

<details>
<summary>pipeline-1: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 100000; clients: 8; pipeline: 1; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: inherited; client CPUs: inherited.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 124181 | 124111–124670 | 74.4 | 100.5 | 8.10 | 95.1 | 6.05 | 0.000010 | 0.000060 | unchanged |
| ping | redis | 112592 | 105469–114058 | 89.2 | 99.2 | 8.90 | 92.6 | 9.02 | 0.000180 | 0.000020 | unchanged |
| ping | valkey | 113268 | 109514–113312 | 88.9 | 99.6 | 8.80 | 90.9 | 8.37 | 0.000150 | 0.000030 | unchanged |
| set | rillcache | 120848 | 120836–120983 | 76.7 | 100.3 | 8.30 | 92.8 | 6.05 | 0.000010 | 0.000050 | unchanged |
| set | redis | 109659 | 106456–109928 | 93.1 | 100.0 | 9.10 | 90.1 | 9.03 | 0.000070 | 0.000060 | unchanged |
| set | valkey | 106131 | 105792–107374 | 97.9 | 99.8 | 9.40 | 86.2 | 8.39 | 0.000040 | 0.000040 | unchanged |
| get | rillcache | 120727 | 120193–122483 | 77.4 | 100.2 | 8.30 | 93.8 | 6.05 | 0.000010 | 0.000060 | unchanged |
| get | redis | 110168 | 110120–111014 | 92.3 | 99.1 | 9.00 | 92.2 | 9.04 | 0.000050 | 0.000040 | unchanged |
| get | valkey | 106818 | 106063–107404 | 96.1 | 99.9 | 9.40 | 87.2 | 8.39 | 0.000060 | 0.000040 | unchanged |
| set-nx-existing | rillcache | 121076 | 120286–121930 | 76.3 | 100.0 | 8.30 | 93.9 | 6.05 | 0.000010 | 0.000040 | unchanged |
| set-nx-existing | redis | 111207 | 106336–111402 | 91.3 | 100.1 | 9.00 | 90.0 | 9.05 | 0.000040 | 0.000050 | unchanged |
| set-nx-existing | valkey | 107379 | 106982–108143 | 95.8 | 100.5 | 9.30 | 88.2 | 8.39 | 0.000120 | 0.000040 | unchanged |
| set-xx-existing | rillcache | 119761 | 118540–120756 | 77.0 | 100.2 | 8.40 | 92.9 | 6.05 | 0.000010 | 0.000020 | unchanged |
| set-xx-existing | redis | 109539 | 108957–109581 | 92.8 | 100.7 | 9.20 | 89.1 | 9.02 | 0.000080 | 0.000060 | unchanged |
| set-xx-existing | valkey | 105281 | 104452–106483 | 98.9 | 100.1 | 9.50 | 86.3 | 8.38 | 0.000100 | 0.000040 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 4.95 | 4.95 | 99.0 | unchanged |
| redis | 1.000 | 0.000 | 8.82 | 8.82 | 10.0 | unchanged |
| valkey | 1.000 | 0.000 | 8.21 | 8.21 | 10.0 | unchanged |

</details>

<details>
<summary>pipeline-16: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 1000000; clients: 8; pipeline: 16; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: inherited; client CPUs: inherited.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 1899746 | 1897137–1918622 | 77.7 | 100.5 | 0.53 | 92.5 | 5.94 | 0.000001 | 0.000004 | unchanged |
| ping | redis | 1444736 | 1440487–1445356 | 119.7 | 99.7 | 0.69 | 76.8 | 8.98 | 0.000003 | 0.000004 | unchanged |
| ping | valkey | 1387145 | 1380180–1398060 | 126.5 | 100.7 | 0.73 | 72.6 | 8.43 | 0.000010 | 0.000003 | unchanged |
| set | rillcache | 1531251 | 1491557–1546493 | 95.6 | 99.0 | 0.64 | 73.1 | 5.94 | 0.000001 | 0.000004 | unchanged |
| set | redis | 1140953 | 1070676–1141263 | 161.7 | 99.3 | 0.87 | 61.3 | 8.97 | 0.000005 | 0.000005 | unchanged |
| set | valkey | 1025284 | 1023609–1027793 | 182.7 | 100.7 | 0.98 | 51.0 | 8.43 | 0.000002 | 0.000004 | unchanged |
| get | rillcache | 1680909 | 1679648–1688527 | 88.3 | 100.8 | 0.60 | 83.3 | 5.94 | 0.000001 | 0.000005 | unchanged |
| get | redis | 1251922 | 1247729–1271656 | 143.9 | 100.4 | 0.80 | 63.8 | 8.98 | 0.000002 | 0.000003 | unchanged |
| get | valkey | 1163056 | 1140306–1175497 | 157.3 | 100.3 | 0.87 | 61.4 | 8.43 | 0.000003 | 0.000002 | unchanged |
| set-nx-existing | rillcache | 1607878 | 1581956–1616610 | 92.1 | 100.2 | 0.63 | 77.8 | 5.94 | 0.000001 | 0.000003 | unchanged |
| set-nx-existing | redis | 1160111 | 1157136–1193519 | 159.1 | 100.6 | 0.87 | 59.8 | 8.98 | 0.000011 | 0.000004 | unchanged |
| set-nx-existing | valkey | 1091098 | 1090848–1155599 | 391.5 | 100.3 | 0.91 | 60.4 | 8.43 | 0.000005 | 0.000002 | unchanged |
| set-xx-existing | rillcache | 1522341 | 1490326–1563311 | 96.4 | 100.0 | 0.66 | 71.6 | 5.94 | 0.000001 | 0.000003 | unchanged |
| set-xx-existing | redis | 1097876 | 1061390–1105036 | 169.7 | 99.9 | 0.91 | 58.9 | 8.98 | 0.000004 | 0.000003 | unchanged |
| set-xx-existing | valkey | 997841 | 933207–999656 | 189.3 | 99.8 | 1.00 | 54.0 | 8.43 | 0.000006 | 0.000002 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 4.89 | 4.89 | 99.0 | unchanged |
| redis | 1.000 | 0.000 | 8.73 | 8.73 | 10.0 | unchanged |
| valkey | 1.000 | 0.000 | 8.24 | 8.24 | 10.0 | unchanged |

</details>

<details>
<summary>single-core-pipeline-1: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 100000; clients: 8; pipeline: 1; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: 0; client CPUs: 1,2,3.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 121907 | 121606–124061 | 76.1 | 100.0 | 8.20 | 96.3 | 5.98 | 0.000010 | 0.000030 | unchanged |
| ping | redis | 113708 | 113223–114190 | 88.3 | 100.0 | 8.80 | 92.0 | 8.82 | 0.000120 | 0.000020 | unchanged |
| ping | valkey | 109605 | 107896–112729 | 94.3 | 99.2 | 9.10 | 90.9 | 8.31 | 0.000100 | 0.000040 | unchanged |
| set | rillcache | 118522 | 118186–120784 | 78.1 | 100.4 | 8.50 | 92.9 | 5.98 | 0.000010 | 0.000050 | unchanged |
| set | redis | 108990 | 108460–109593 | 93.3 | 100.8 | 9.30 | 89.2 | 8.82 | 0.000130 | 0.000030 | unchanged |
| set | valkey | 104518 | 99260–106897 | 200.9 | 99.4 | 9.60 | 86.5 | 8.33 | 0.000130 | 0.000030 | unchanged |
| get | rillcache | 120340 | 120144–121443 | 77.1 | 100.8 | 8.30 | 92.9 | 5.98 | 0.000010 | 0.000050 | unchanged |
| get | redis | 111261 | 109926–111452 | 92.1 | 100.0 | 9.00 | 89.0 | 8.84 | 0.000110 | 0.000030 | unchanged |
| get | valkey | 109002 | 107842–109077 | 93.5 | 99.2 | 9.20 | 89.0 | 8.33 | 0.000130 | 0.000020 | unchanged |
| set-nx-existing | rillcache | 120654 | 119256–121065 | 76.5 | 99.3 | 8.30 | 95.1 | 5.98 | 0.000010 | 0.000050 | unchanged |
| set-nx-existing | redis | 110376 | 109692–110795 | 92.0 | 99.8 | 9.10 | 89.0 | 8.84 | 0.000070 | 0.000020 | unchanged |
| set-nx-existing | valkey | 108859 | 108393–108949 | 93.5 | 100.1 | 9.20 | 87.0 | 8.33 | 0.000020 | 0.000040 | unchanged |
| set-xx-existing | rillcache | 119212 | 118692–119598 | 77.5 | 100.9 | 8.50 | 92.9 | 5.98 | 0.000010 | 0.000020 | unchanged |
| set-xx-existing | redis | 109162 | 107247–110033 | 93.1 | 100.1 | 9.20 | 89.0 | 8.82 | 0.000160 | 0.000020 | unchanged |
| set-xx-existing | valkey | 105784 | 99749–106845 | 99.0 | 99.7 | 9.50 | 86.0 | 8.31 | 0.000320 | 0.000030 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 4.95 | 4.95 | 99.0 | unchanged |
| redis | 1.000 | 0.000 | 8.61 | 8.61 | 10.0 | unchanged |
| valkey | 1.000 | 0.000 | 8.16 | 8.16 | 10.0 | unchanged |

</details>

<details>
<summary>single-core-pipeline-16: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 1000000; clients: 8; pipeline: 16; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: 0; client CPUs: 1,2,3.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 1917211 | 1916964–1934385 | 76.9 | 99.7 | 0.52 | 92.3 | 6.02 | 0.000001 | 0.000002 | unchanged |
| ping | redis | 1458092 | 1449767–1458402 | 119.5 | 100.0 | 0.69 | 72.1 | 8.96 | 0.000009 | 0.000002 | unchanged |
| ping | valkey | 1383529 | 1382043–1400871 | 127.4 | 100.8 | 0.72 | 72.2 | 8.27 | 0.000002 | 0.000003 | unchanged |
| set | rillcache | 1564608 | 1528694–1572786 | 95.4 | 100.1 | 0.64 | 73.4 | 6.02 | 0.000001 | 0.000002 | unchanged |
| set | redis | 1146249 | 1141395–1155561 | 161.4 | 100.4 | 0.87 | 56.3 | 8.96 | 0.000006 | 0.000004 | unchanged |
| set | valkey | 1020435 | 1010251–1029677 | 183.3 | 100.0 | 0.98 | 49.0 | 8.26 | 0.000003 | 0.000003 | unchanged |
| get | rillcache | 1700449 | 1683461–1721226 | 86.6 | 99.8 | 0.59 | 83.1 | 6.02 | 0.000001 | 0.000004 | unchanged |
| get | redis | 1261223 | 1253036–1266418 | 142.7 | 100.2 | 0.80 | 63.3 | 8.96 | 0.000005 | 0.000003 | unchanged |
| get | valkey | 1163723 | 1145764–1165874 | 156.9 | 99.1 | 0.86 | 61.6 | 8.27 | 0.000003 | 0.000003 | unchanged |
| set-nx-existing | rillcache | 1600272 | 1575900–1627316 | 92.8 | 99.2 | 0.62 | 77.0 | 6.02 | 0.000001 | 0.000002 | unchanged |
| set-nx-existing | redis | 1187400 | 1179048–1193709 | 154.2 | 99.0 | 0.84 | 61.9 | 8.96 | 0.000005 | 0.000004 | unchanged |
| set-nx-existing | valkey | 1154700 | 1134637–1162950 | 159.5 | 99.3 | 0.86 | 59.3 | 8.27 | 0.000006 | 0.000002 | unchanged |
| set-xx-existing | rillcache | 1534857 | 1500144–1539513 | 95.6 | 99.7 | 0.65 | 69.2 | 6.02 | 0.000001 | 0.000002 | unchanged |
| set-xx-existing | redis | 1117397 | 1103943–1119010 | 166.0 | 100.5 | 0.90 | 56.7 | 8.96 | 0.000010 | 0.000003 | unchanged |
| set-xx-existing | valkey | 989795 | 958343–1003141 | 190.3 | 99.9 | 1.01 | 52.9 | 8.27 | 0.000008 | 0.000003 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 4.97 | 4.97 | 99.0 | unchanged |
| redis | 1.000 | 0.000 | 8.71 | 8.71 | 10.0 | unchanged |
| valkey | 1.000 | 0.000 | 8.07 | 8.07 | 10.0 | unchanged |

</details>

### Fixed-worker mode measurements

30/30 separate embedded records validated. 8 clients; 10000 commands/workload/run; pipeline 16; three repetitions. These share the client/server host and inherited CPU affinity; they are **not native competitor comparisons or equal-per-core scaling evidence**. No process CPU/RSS accounting was collected for this group. Fixed lanes have a 1 ms idle polling wait. They actively poll for at most 50 µs after progress before returning to idle parking; this trades active CPU work for fewer premature sleeps, not epoll readiness or free CPU. Multiple lanes also request a cooperative yield after 100 µs of continuous active passes when more than one client is admitted. Progress does not rearm that separate budget; single lanes/lone clients do not periodically yield. This is checked between bounded passes, not a hard deadline or an OS fairness guarantee. The v2 interval restarts after a yield/park returns, excluding that explicit wait; involuntary descheduling can still consume wall time.

| Workload | 1 worker median cmd/s | 1 worker min–max | 4 workers median cmd/s | 4 workers min–max | 1 worker batch p99 µs | 4 workers batch p99 µs |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| ping | 1492988 | 1439738–1580974 | 1817813 | 779561–2448322 | 986.4 | 1066.9 |
| set | 1227108 | 1177643–1285847 | 54057 | 52658–54186 | 1014.5 | 3332.4 |
| get | 1427239 | 1423864–1428253 | 43761 | 30102–53565 | 1010.3 | 14631.8 |
| set-nx-existing | 1373554 | 1367752–1386396 | 54892 | 25404–55012 | 1006.8 | 4834.2 |
| set-xx-existing | 1274268 | 1225853–1287123 | 54480 | 53729–56236 | 1022.9 | 3885.6 |

### Interpretation limits

- Batch p99 is **not per-request p99**; never divide it by the pipeline length. Percentiles are not pooled across runs.
- CPU counters are tick-quantized; zero observed idle ticks do not prove zero CPU cost. CPU observation windows bracket, but are not identical to, request timers.
- RSS is an approximate endpoint sample, not allocator accounting or a workload peak; VmHWM includes startup, warmup and previous workloads.
- Context switches cover matched thread lifetimes only. Even unchanged endpoints cannot exclude threads born and destroyed between snapshots. PARTIAL coverage is not a process total.
- NX-existing measures rejected writes; XX-existing measures admitted overwrites. This is a small Redis-compatible subset, not equivalent functionality.
