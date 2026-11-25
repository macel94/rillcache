# Current benchmark performance

Generated: 2026-11-24T19:40:54Z · [Benchmark run #20](https://github.com/macel94/rillcache/actions/runs/45470853663) · [Tested source `34f6b78`](https://github.com/macel94/rillcache/commit/34f6b78adb767112c63125a0c22d2c0a5fb08337)

**Benchmark job: success. Data: validated. Copilot review: completed.**

Benchmark attempt: 1; review attempt: 1. Preferred model: `gpt-6-luna` via **GitHub Copilot**.

[This report's permanent copy](https://github.com/macel94/rillcache/blob/main/benchmarks/reports/45470853663-1.md) · [Report history](https://github.com/macel94/rillcache/tree/main/benchmarks/reports) · [Methodology](https://github.com/macel94/rillcache/blob/main/docs/benchmarks.md)

## AI review

Actual model(s): **gpt-6-luna**. Selection: automatic-fallback. Authentication: workflow-token.

AI interpretation, **not measurements or a performance gate**. Verify claims against the deterministic tables below.

> Bottom line: In this validated subset, Rillcache has higher median throughput and lower whole-batch
> p99 than Redis and Valkey in the reported SET workloads. This is workload-specific evidence, not a
> general Redis/Valkey victory.
> 
> At one logical CPU, pipeline-1 SET reached 117,466 commands/s (+5.9% vs Redis), with median
> whole-batch p99 of 77.9 µs vs 91.4 µs. At pipeline-16, it reached 1,519,613 commands/s (+31.4%),
> with batch p99 of 97.4 µs vs 159.4 µs. These are batch-level percentiles, not per-request latency;
> do not divide them by pipeline length. In the inherited-affinity profile, pipeline-16 SET was
> 1,494,761 commands/s (+49.3%) with 98.7 µs batch p99 vs Redis’s 313.7 µs. Keep these profiles
> distinct: the latter is not a controlled equal-one-CPU budget.
> 
> CPU was near 100% of one logical CPU in both profiles; in single-core pipeline-16 SET, reported CPU
> time was 0.66 vs Redis’s 0.86 µs/command. Rillcache’s workload RSS endpoints were about 6.7–6.9 MiB
> vs Redis’s 8.7–8.9 MiB, but these approximate samples are not workload peaks; VmHWM is a lifetime
> peak. Idle CPU reads of zero are tick-quantized. Context-switch counts cover matched thread
> lifetimes, not guaranteed process totals; the lower reported voluntary switches per command do not
> establish why throughput differs.
> 
> Uncertainty and next investigations: The 180 native records are validated, but shared-VM effects,
> limited workloads and a small Redis-compatible feature set constrain generalization. NX-existing
> rejects writes; XX-existing admits overwrites. First, investigate the separate fixed-worker group’s
> sharp 4-worker drop for SET (1,229,541 to 53,870 commands/s) and GET (1,416,745 to 56,759), with
> higher batch p99s—without treating it as a native competitor comparison. Second, repeat
> representative workloads under controlled CPU budgets and longer runs, collecting per-request
> latency distributions and CPU accounting to test whether these throughput and tail-latency patterns
> hold.

## Measurements

**Data quality: complete.** 180/180 records validated across 4/4 complete profiles.


Hardware: AMD EPYC 9V45 96-Core Processor. Rust 1.100.0 (0ef1bc386 2026-11-09).

References: Redis 8.10.2 sha=00000000:0 malloc=jemalloc-5.3.0 bits=64 build=a89eb8ae3d95efc2; Valkey 9.1.2 sha=00000000:0 malloc=jemalloc-5.3.0 bits=64 build=e8ce83e22145bef8.

### SET at a glance

Median commands/s and median of the three runs' **whole-batch p99** values (µs). Positive rate delta means Rillcache is faster in this workload only.

| Profile | Rillcache cmd/s | Redis | Valkey | Rillcache vs Redis | Rillcache p99 µs | Redis p99 | Valkey p99 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| pipeline-1 | 118643 | 111136 | 107474 | +6.8% | 77.7 | 91.7 | 95.2 |
| pipeline-16 | 1494761 | 1001104 | 1017943 | +49.3% | 98.7 | 313.7 | 185.4 |
| single-core-pipeline-1 | 117466 | 110886 | 107872 | +5.9% | 77.9 | 91.4 | 94.5 |
| single-core-pipeline-16 | 1519613 | 1156437 | 1026491 | +31.4% | 97.4 | 159.4 | 182.4 |

CPU is relative to **one logical CPU**, so parallel servers can exceed 100%. Do not blend default-threading and single-core results or different pipelines. Shared VMs and unequal feature sets do not establish a general performance win or a cross-run regression.

<details>
<summary>pipeline-1: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 100000; clients: 8; pipeline: 1; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: inherited; client CPUs: inherited.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 122457 | 122306–122630 | 75.3 | 100.3 | 8.20 | 96.3 | 6.81 | 0.000010 | 0.000040 | unchanged |
| ping | redis | 113571 | 112749–113916 | 88.7 | 99.1 | 8.70 | 90.8 | 8.85 | 0.000100 | 0.000040 | unchanged |
| ping | valkey | 112643 | 111822–113764 | 89.0 | 99.5 | 8.80 | 92.0 | 8.17 | 0.000350 | 0.000050 | unchanged |
| set | rillcache | 118643 | 118208–119490 | 77.7 | 100.4 | 8.40 | 94.0 | 6.81 | 0.000010 | 0.000040 | unchanged |
| set | redis | 111136 | 110297–111150 | 91.7 | 100.3 | 9.10 | 90.0 | 8.85 | 0.000060 | 0.000030 | unchanged |
| set | valkey | 107474 | 107188–108121 | 95.2 | 99.7 | 9.30 | 87.0 | 8.20 | 0.000310 | 0.000040 | unchanged |
| get | rillcache | 121160 | 120655–121421 | 76.2 | 100.5 | 8.30 | 93.9 | 6.81 | 0.000010 | 0.000040 | unchanged |
| get | redis | 111720 | 105841–112236 | 90.4 | 99.5 | 8.90 | 91.0 | 8.86 | 0.000220 | 0.000050 | unchanged |
| get | valkey | 108848 | 102527–110301 | 92.9 | 100.3 | 9.10 | 89.8 | 8.21 | 0.000050 | 0.000050 | unchanged |
| set-nx-existing | rillcache | 120557 | 120508–121856 | 76.4 | 98.8 | 8.20 | 95.1 | 6.81 | 0.000010 | 0.000040 | unchanged |
| set-nx-existing | redis | 110350 | 110315–110970 | 92.9 | 100.4 | 9.10 | 87.8 | 8.86 | 0.000280 | 0.000040 | unchanged |
| set-nx-existing | valkey | 109873 | 109801–109882 | 92.9 | 99.9 | 9.10 | 89.0 | 8.21 | 0.000160 | 0.000070 | unchanged |
| set-xx-existing | rillcache | 118076 | 117831–118626 | 77.9 | 99.6 | 8.40 | 92.9 | 6.81 | 0.000010 | 0.000040 | unchanged |
| set-xx-existing | redis | 109440 | 107916–109682 | 92.9 | 99.6 | 9.10 | 89.0 | 8.85 | 0.000070 | 0.000030 | unchanged |
| set-xx-existing | valkey | 106707 | 106620–106839 | 95.8 | 100.3 | 9.40 | 86.2 | 8.18 | 0.000180 | 0.000050 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 5.57 | 5.57 | 99.0 | unchanged |
| redis | 1.000 | 0.000 | 8.63 | 8.63 | 10.0 | unchanged |
| valkey | 1.000 | 0.000 | 8.02 | 8.02 | 10.0 | unchanged |

</details>

<details>
<summary>pipeline-16: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 1000000; clients: 8; pipeline: 16; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: inherited; client CPUs: inherited.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 1894937 | 1890267–1907978 | 77.6 | 100.4 | 0.53 | 92.5 | 6.75 | 0.000001 | 0.000004 | unchanged |
| ping | redis | 1444936 | 1444413–1450601 | 120.4 | 99.7 | 0.69 | 73.9 | 8.93 | 0.000010 | 0.000003 | unchanged |
| ping | valkey | 1407978 | 1403559–1415850 | 124.9 | 99.6 | 0.71 | 71.8 | 8.38 | 0.000009 | 0.000002 | unchanged |
| set | rillcache | 1494761 | 1482123–1531031 | 98.7 | 99.5 | 0.67 | 71.6 | 6.75 | 0.000001 | 0.000008 | unchanged |
| set | redis | 1001104 | 956115–1153560 | 313.7 | 100.3 | 1.01 | 61.5 | 8.91 | 0.000007 | 0.000002 | unchanged |
| set | valkey | 1017943 | 1012646–1023575 | 185.4 | 99.7 | 0.98 | 52.0 | 8.37 | 0.000003 | 0.000003 | unchanged |
| get | rillcache | 1676472 | 1669833–1719184 | 87.8 | 100.6 | 0.60 | 80.0 | 6.75 | 0.000001 | 0.000005 | unchanged |
| get | redis | 1175246 | 1158848–1228390 | 188.0 | 99.9 | 0.85 | 67.1 | 8.92 | 0.000006 | 0.000003 | unchanged |
| get | valkey | 1178504 | 1171440–1180899 | 154.8 | 100.3 | 0.85 | 59.5 | 8.38 | 0.000002 | 0.000001 | unchanged |
| set-nx-existing | rillcache | 1603916 | 1584677–1650784 | 91.9 | 99.4 | 0.62 | 76.7 | 6.75 | 0.000001 | 0.000007 | unchanged |
| set-nx-existing | redis | 1176093 | 1174928–1198410 | 157.5 | 100.6 | 0.85 | 61.2 | 8.93 | 0.000008 | 0.000004 | unchanged |
| set-nx-existing | valkey | 1163860 | 1161636–1167164 | 157.7 | 100.3 | 0.86 | 58.1 | 8.38 | 0.000005 | 0.000002 | unchanged |
| set-xx-existing | rillcache | 1520190 | 1500705–1526305 | 98.0 | 99.0 | 0.65 | 70.8 | 6.75 | 0.000001 | 0.000006 | unchanged |
| set-xx-existing | redis | 1118217 | 1115265–1122611 | 165.7 | 99.2 | 0.89 | 58.4 | 8.93 | 0.000007 | 0.000005 | unchanged |
| set-xx-existing | valkey | 996665 | 946891–997140 | 190.2 | 100.3 | 1.01 | 51.5 | 8.38 | 0.000003 | 0.000002 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 5.44 | 5.44 | 99.0 | unchanged |
| redis | 1.000 | 0.000 | 8.67 | 8.67 | 10.0 | unchanged |
| valkey | 1.000 | 0.000 | 8.18 | 8.18 | 10.0 | unchanged |

</details>

<details>
<summary>single-core-pipeline-1: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 100000; clients: 8; pipeline: 1; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: 0; client CPUs: 1,2,3.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 122073 | 121832–122570 | 75.4 | 99.9 | 8.20 | 95.1 | 6.72 | 0.000010 | 0.000040 | unchanged |
| ping | redis | 113790 | 113004–114113 | 89.3 | 99.4 | 8.80 | 92.0 | 8.68 | 0.000220 | 0.000060 | unchanged |
| ping | valkey | 113487 | 113390–113695 | 88.6 | 100.0 | 8.80 | 89.9 | 8.28 | 0.000230 | 0.000060 | unchanged |
| set | rillcache | 117466 | 117259–118498 | 77.9 | 99.7 | 8.50 | 92.9 | 6.72 | 0.000010 | 0.000030 | unchanged |
| set | redis | 110886 | 110713–110931 | 91.4 | 100.7 | 9.10 | 89.0 | 8.69 | 0.000060 | 0.000040 | unchanged |
| set | valkey | 107872 | 107834–108636 | 94.5 | 100.3 | 9.30 | 86.2 | 8.30 | 0.000080 | 0.000050 | unchanged |
| get | rillcache | 118665 | 116268–119362 | 77.7 | 99.7 | 8.40 | 94.2 | 6.72 | 0.000010 | 0.000030 | unchanged |
| get | redis | 112298 | 111932–112501 | 89.5 | 100.1 | 8.90 | 89.9 | 8.71 | 0.000220 | 0.000050 | unchanged |
| get | valkey | 109617 | 109427–109996 | 93.0 | 99.0 | 9.00 | 89.0 | 8.30 | 0.000460 | 0.000060 | unchanged |
| set-nx-existing | rillcache | 117279 | 116807–118097 | 77.8 | 100.4 | 8.60 | 92.9 | 6.72 | 0.000010 | 0.000030 | unchanged |
| set-nx-existing | redis | 111676 | 111108–111748 | 90.5 | 100.5 | 9.00 | 90.0 | 8.71 | 0.000020 | 0.000030 | unchanged |
| set-nx-existing | valkey | 108998 | 108492–109039 | 93.2 | 99.2 | 9.10 | 89.0 | 8.30 | 0.000270 | 0.000070 | unchanged |
| set-xx-existing | rillcache | 116737 | 116399–117944 | 78.6 | 99.1 | 8.50 | 94.0 | 6.72 | 0.000010 | 0.000050 | unchanged |
| set-xx-existing | redis | 110608 | 110459–110636 | 91.8 | 100.6 | 9.10 | 89.0 | 8.69 | 0.000070 | 0.000060 | unchanged |
| set-xx-existing | valkey | 107846 | 107399–108142 | 94.9 | 100.5 | 9.30 | 86.2 | 8.29 | 0.000230 | 0.000050 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 5.46 | 5.46 | 99.0 | unchanged |
| redis | 1.000 | 0.000 | 8.48 | 8.48 | 10.0 | unchanged |
| valkey | 1.000 | 0.000 | 8.13 | 8.13 | 10.0 | unchanged |

</details>

<details>
<summary>single-core-pipeline-16: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 1000000; clients: 8; pipeline: 16; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: 0; client CPUs: 1,2,3.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 1886102 | 1884509–1896372 | 78.9 | 98.6 | 0.52 | 94.2 | 6.86 | 0.000001 | 0.000003 | unchanged |
| ping | redis | 1455248 | 1454984–1458148 | 119.4 | 100.4 | 0.69 | 72.5 | 8.88 | 0.000012 | 0.000003 | unchanged |
| ping | valkey | 1409537 | 1406267–1410375 | 124.6 | 99.8 | 0.71 | 71.4 | 8.32 | 0.000007 | 0.000003 | unchanged |
| set | rillcache | 1519613 | 1506987–1520465 | 97.4 | 99.4 | 0.66 | 70.8 | 6.86 | 0.000001 | 0.000003 | unchanged |
| set | redis | 1156437 | 1155682–1159434 | 159.4 | 99.7 | 0.86 | 57.0 | 8.88 | 0.000004 | 0.000003 | unchanged |
| set | valkey | 1026491 | 1024694–1028751 | 182.4 | 100.6 | 0.98 | 52.0 | 8.31 | 0.000013 | 0.000005 | unchanged |
| get | rillcache | 1697713 | 1677836–1706717 | 87.6 | 100.1 | 0.59 | 80.0 | 6.86 | 0.000001 | 0.000002 | unchanged |
| get | redis | 1265077 | 1251616–1271531 | 142.7 | 100.1 | 0.79 | 61.2 | 8.87 | 0.000025 | 0.000002 | unchanged |
| get | valkey | 1178712 | 1171008–1179918 | 155.3 | 99.5 | 0.85 | 57.6 | 8.32 | 0.000022 | 0.000003 | unchanged |
| set-nx-existing | rillcache | 1608572 | 1593450–1613351 | 91.6 | 100.4 | 0.63 | 75.8 | 6.86 | 0.000001 | 0.000003 | unchanged |
| set-nx-existing | redis | 1197136 | 1193918–1198429 | 152.9 | 99.4 | 0.83 | 61.9 | 8.88 | 0.000019 | 0.000003 | unchanged |
| set-nx-existing | valkey | 1161918 | 1159738–1175315 | 158.4 | 99.9 | 0.85 | 58.8 | 8.32 | 0.000008 | 0.000005 | unchanged |
| set-xx-existing | rillcache | 1490992 | 1476871–1521747 | 98.4 | 99.9 | 0.67 | 70.1 | 6.86 | 0.000001 | 0.000003 | unchanged |
| set-xx-existing | redis | 1127060 | 1126666–1127772 | 163.9 | 100.2 | 0.89 | 58.4 | 8.88 | 0.000027 | 0.000003 | unchanged |
| set-xx-existing | valkey | 1002903 | 996158–1003760 | 188.2 | 100.3 | 1.00 | 52.0 | 8.32 | 0.000005 | 0.000004 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 5.57 | 5.57 | 99.0 | unchanged |
| redis | 1.000 | 0.000 | 8.62 | 8.62 | 10.0 | unchanged |
| valkey | 1.000 | 0.000 | 8.13 | 8.13 | 10.0 | unchanged |

</details>

### Fixed-worker mode measurements

30/30 separate embedded records validated. 8 clients; 10000 commands/workload/run; pipeline 16; three repetitions. These share the client/server host and inherited CPU affinity; they are **not native competitor comparisons or equal-per-core scaling evidence**. No process CPU/RSS accounting was collected for this group. Fixed lanes have a 1 ms idle polling wait. They actively poll for at most 50 µs after progress before returning to idle parking; this trades active CPU work for fewer premature sleeps, not epoll readiness or free CPU. Multiple lanes also request a cooperative yield after 100 µs of continuous active passes when more than one client is admitted. Progress does not rearm that separate budget; single lanes/lone clients do not periodically yield. This is checked between bounded passes, not a hard deadline or an OS fairness guarantee. The v2 interval restarts after a yield/park returns, excluding that explicit wait; involuntary descheduling can still consume wall time.

| Workload | 1 worker median cmd/s | 1 worker min–max | 4 workers median cmd/s | 4 workers min–max | 1 worker batch p99 µs | 4 workers batch p99 µs |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| ping | 1535715 | 1524514–1566108 | 1958462 | 1664985–2090012 | 1011.5 | 1080.0 |
| set | 1229541 | 1132529–1267342 | 53870 | 53660–54888 | 1015.7 | 3398.6 |
| get | 1416745 | 1361365–1418214 | 56759 | 56489–57488 | 1016.4 | 3173.4 |
| set-nx-existing | 1360666 | 1348142–1458675 | 57184 | 55524–57787 | 1020.1 | 3142.0 |
| set-xx-existing | 1286435 | 1285904–1288398 | 55590 | 37612–56582 | 1015.9 | 3265.9 |

### Interpretation limits

- Batch p99 is **not per-request p99**; never divide it by the pipeline length. Percentiles are not pooled across runs.
- CPU counters are tick-quantized; zero observed idle ticks do not prove zero CPU cost. CPU observation windows bracket, but are not identical to, request timers.
- RSS is an approximate endpoint sample, not allocator accounting or a workload peak; VmHWM includes startup, warmup and previous workloads.
- Context switches cover matched thread lifetimes only. Even unchanged endpoints cannot exclude threads born and destroyed between snapshots. PARTIAL coverage is not a process total.
- NX-existing measures rejected writes; XX-existing measures admitted overwrites. This is a small Redis-compatible subset, not equivalent functionality.
