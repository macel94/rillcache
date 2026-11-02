# Current benchmark performance

Generated: 2026-11-02T16:09:22Z · [Benchmark run #14](https://github.com/macel94/rillcache/actions/runs/41477114469) · [Tested source `9692e07`](https://github.com/macel94/rillcache/commit/9692e074780dc9667f14962875f959fa1c2af348)

**Benchmark job: success. Data: validated. Copilot review: completed.**

Benchmark attempt: 1; review attempt: 1. Preferred model: `gpt-6-luna` via **GitHub Copilot**.

[This report's permanent copy](https://github.com/macel94/rillcache/blob/main/benchmarks/reports/41477114469-1.md) · [Report history](https://github.com/macel94/rillcache/tree/main/benchmarks/reports) · [Methodology](https://github.com/macel94/rillcache/blob/main/docs/benchmarks.md)

## AI review

Actual model(s): **gpt-6-luna**. Selection: automatic-fallback. Authentication: workflow-token.

AI interpretation, **not measurements or a performance gate**. Verify claims against the deterministic tables below.

> Bottom line: In this validated native cohort (180 records), Rillcache’s SET throughput is higher in
> the measured workloads, including under an equal one-logical-CPU budget. This is a workload-specific
> result, not a general Redis or Valkey victory.
> 
> At equal CPU, SET throughput was 119,105 vs. Redis’s 110,504 commands/s at pipeline 1 (+7.8%), and
> 1,509,416 vs. 1,132,680 at pipeline 16 (+33.3%). Median whole-batch p99 was 77.4 vs. 91.3 µs and
> 97.3 vs. 162.9 µs, respectively. These are batch latencies—not per-request p99—and should not be
> divided by pipeline length. Default-threading results are separate: SET was +6.3% at pipeline 1 and
> +31.8% at pipeline 16 versus Redis.
> 
> Resource observations: Native workload CPU was generally around one logical CPU; for pipeline-16
> SET, CPU was 99.6% for Redis and 99.6% for Rillcache in the single-core profile, with reported CPU
> time of 0.88 vs. 0.66 µs/command. CPU windows do not exactly match request timers. Rillcache’s
> approximate RSS endpoints were lower (5.86 vs. 8.79 MiB for that profile), but these are not
> workload peaks; VmHWM is a lifetime peak. Zero idle CPU ticks are quantized, not proof of zero cost.
> Scheduling counts cover matched thread lifetimes only; for example, single-core pipeline-1 SET
> voluntary switches were similar (0.00029 vs. 0.00026 per command), without establishing process-wide
> totals or causation.
> 
> Uncertainty and next investigations: Results are three-run medians on a shared VM, with workload-
> and pipeline-specific variability; the subset is not equivalent to full Redis/Valkey functionality.
> First, investigate the separate fixed-worker measurements: four workers’ SET throughput was 53,990
> vs. 1,281,104 commands/s for one worker, with batch p99 3,831 vs. 1,010 µs. This group lacks process
> CPU/RSS accounting and is not equal-per-core evidence. Second, repeat native equal-CPU runs with
> per-request latency distributions and more repetitions, especially pipeline-16 ping, whose
> default-threading range was 1.75–2.61 million commands/s.

## Measurements

**Data quality: complete.** 180/180 records validated across 4/4 complete profiles.


Hardware: AMD EPYC 9V45 96-Core Processor. Rust 1.99.1 (58cf8f9c3 2026-10-13).

References: Redis 8.10.2 sha=00000000:0 malloc=jemalloc-5.3.0 bits=64 build=33de5ea66c226878; Valkey 9.1.2 sha=00000000:0 malloc=jemalloc-5.3.0 bits=64 build=dc615266b03fe270.

### SET at a glance

Median commands/s and median of the three runs' **whole-batch p99** values (µs). Positive rate delta means Rillcache is faster in this workload only.

| Profile | Rillcache cmd/s | Redis | Valkey | Rillcache vs Redis | Rillcache p99 µs | Redis p99 | Valkey p99 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| pipeline-1 | 117182 | 110191 | 106825 | +6.3% | 78.5 | 92.1 | 96.9 |
| pipeline-16 | 1526766 | 1158289 | 1030536 | +31.8% | 97.4 | 158.7 | 181.5 |
| single-core-pipeline-1 | 119105 | 110504 | 107512 | +7.8% | 77.4 | 91.3 | 95.2 |
| single-core-pipeline-16 | 1509416 | 1132680 | 1036223 | +33.3% | 97.3 | 162.9 | 181.0 |

CPU is relative to **one logical CPU**, so parallel servers can exceed 100%. Do not blend default-threading and single-core results or different pipelines. Shared VMs and unequal feature sets do not establish a general performance win or a cross-run regression.

<details>
<summary>pipeline-1: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 100000; clients: 8; pipeline: 1; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: inherited; client CPUs: inherited.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 139341 | 123199–141509 | 68.9 | 119.8 | 8.60 | 95.3 | 6.46 | 0.085850 | 0.014260 | unchanged |
| ping | redis | 111925 | 111737–113704 | 93.1 | 100.5 | 9.00 | 92.2 | 8.93 | 0.000060 | 0.000030 | unchanged |
| ping | valkey | 112235 | 112114–112975 | 90.2 | 99.4 | 8.80 | 91.0 | 8.27 | 0.000170 | 0.000060 | unchanged |
| set | rillcache | 117182 | 115329–119761 | 78.5 | 100.6 | 8.60 | 94.0 | 6.46 | 0.002980 | 0.000060 | unchanged |
| set | redis | 110191 | 109719–110205 | 92.1 | 100.2 | 9.10 | 90.0 | 8.93 | 0.000060 | 0.000030 | unchanged |
| set | valkey | 106825 | 106291–107340 | 96.9 | 99.9 | 9.40 | 86.0 | 8.28 | 0.000150 | 0.000040 | unchanged |
| get | rillcache | 119947 | 112686–120351 | 77.1 | 99.9 | 8.30 | 95.2 | 6.46 | 0.002860 | 0.000050 | unchanged |
| get | redis | 110609 | 107090–111734 | 91.7 | 99.5 | 9.00 | 91.0 | 8.94 | 0.000240 | 0.000050 | unchanged |
| get | valkey | 108514 | 107507–108936 | 93.6 | 99.9 | 9.20 | 89.1 | 8.29 | 0.000130 | 0.000050 | unchanged |
| set-nx-existing | rillcache | 119676 | 118564–119830 | 77.4 | 100.5 | 8.40 | 94.0 | 6.46 | 0.002830 | 0.000040 | unchanged |
| set-nx-existing | redis | 106135 | 103097–111839 | 221.9 | 99.5 | 9.50 | 89.6 | 8.95 | 0.000260 | 0.000050 | unchanged |
| set-nx-existing | valkey | 108375 | 103527–108720 | 93.8 | 99.7 | 9.20 | 88.0 | 8.29 | 0.000110 | 0.000050 | unchanged |
| set-xx-existing | rillcache | 117555 | 116626–119481 | 78.6 | 99.9 | 8.50 | 91.8 | 6.46 | 0.002540 | 0.000070 | unchanged |
| set-xx-existing | redis | 109422 | 108265–109854 | 92.4 | 99.5 | 9.10 | 89.2 | 8.93 | 0.000030 | 0.000040 | unchanged |
| set-xx-existing | valkey | 107600 | 107082–107882 | 95.6 | 99.2 | 9.20 | 88.0 | 8.27 | 0.000100 | 0.000040 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 5.11 | 5.11 | 395.9 | unchanged |
| redis | 1.000 | 0.000 | 8.72 | 8.72 | 10.0 | unchanged |
| valkey | 1.000 | 0.000 | 8.11 | 8.11 | 10.0 | unchanged |

</details>

<details>
<summary>pipeline-16: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 1000000; clients: 8; pipeline: 16; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: inherited; client CPUs: inherited.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 2177772 | 1753792–2606808 | 72.0 | 121.9 | 0.56 | 92.9 | 6.06 | 0.003636 | 0.002044 | unchanged |
| ping | redis | 1451854 | 1439476–1452599 | 120.7 | 99.3 | 0.69 | 73.5 | 8.91 | 0.000004 | 0.000003 | unchanged |
| ping | valkey | 1384859 | 1255299–1420263 | 128.5 | 98.3 | 0.71 | 72.5 | 8.43 | 0.000008 | 0.000004 | unchanged |
| set | rillcache | 1526766 | 1521611–1538922 | 97.4 | 99.2 | 0.65 | 69.2 | 6.12 | 0.000223 | 0.000006 | unchanged |
| set | redis | 1158289 | 1154978–1162620 | 158.7 | 99.6 | 0.86 | 55.8 | 8.91 | 0.000003 | 0.000005 | unchanged |
| set | valkey | 1030536 | 1014929–1032850 | 181.5 | 99.1 | 0.96 | 51.5 | 8.41 | 0.000006 | 0.000006 | unchanged |
| get | rillcache | 1679927 | 1668477–1728698 | 87.8 | 100.2 | 0.60 | 83.3 | 6.12 | 0.000232 | 0.000003 | unchanged |
| get | redis | 1265685 | 1263524–1266457 | 142.4 | 98.7 | 0.78 | 62.8 | 8.92 | 0.000003 | 0.000004 | unchanged |
| get | valkey | 1167949 | 1117242–1183969 | 157.7 | 100.6 | 0.86 | 62.6 | 8.43 | 0.000017 | 0.000005 | unchanged |
| set-nx-existing | rillcache | 1606444 | 1546754–1625171 | 92.3 | 100.5 | 0.63 | 77.8 | 6.12 | 0.000200 | 0.000005 | unchanged |
| set-nx-existing | redis | 1196023 | 1166689–1196668 | 153.2 | 100.4 | 0.85 | 58.8 | 8.92 | 0.000005 | 0.000005 | unchanged |
| set-nx-existing | valkey | 1130510 | 1101672–1168035 | 179.5 | 100.4 | 0.89 | 57.3 | 8.43 | 0.000003 | 0.000003 | unchanged |
| set-xx-existing | rillcache | 1523201 | 1521580–1528169 | 97.7 | 99.0 | 0.65 | 69.7 | 6.12 | 0.000197 | 0.000005 | unchanged |
| set-xx-existing | redis | 1121192 | 1110540–1126502 | 165.6 | 99.9 | 0.90 | 58.9 | 8.92 | 0.000002 | 0.000004 | unchanged |
| set-xx-existing | valkey | 1008007 | 1005118–1009837 | 186.0 | 99.9 | 0.99 | 51.5 | 8.43 | 0.000003 | 0.000007 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 4.84 | 4.84 | 395.9 | unchanged |
| redis | 1.000 | 0.000 | 8.66 | 8.66 | 10.0 | unchanged |
| valkey | 1.000 | 0.000 | 8.23 | 8.23 | 10.0 | unchanged |

</details>

<details>
<summary>single-core-pipeline-1: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 100000; clients: 8; pipeline: 1; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: 0; client CPUs: 1,2,3.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 122995 | 122930–123399 | 75.1 | 99.9 | 8.10 | 95.1 | 5.81 | 0.000080 | 0.000040 | unchanged |
| ping | redis | 114307 | 111939–114587 | 87.9 | 99.6 | 8.70 | 90.8 | 8.83 | 0.000140 | 0.000030 | unchanged |
| ping | valkey | 113568 | 113137–113739 | 88.5 | 99.9 | 8.80 | 88.6 | 8.16 | 0.000080 | 0.000040 | unchanged |
| set | rillcache | 119105 | 118065–121323 | 77.4 | 99.2 | 8.30 | 93.9 | 5.81 | 0.000290 | 0.000050 | unchanged |
| set | redis | 110504 | 108196–111355 | 91.3 | 99.5 | 9.00 | 89.1 | 8.85 | 0.000260 | 0.000050 | unchanged |
| set | valkey | 107512 | 104003–107851 | 95.2 | 100.3 | 9.40 | 87.1 | 8.17 | 0.000090 | 0.000040 | unchanged |
| get | rillcache | 120796 | 120515–122011 | 76.5 | 100.0 | 8.30 | 95.2 | 5.81 | 0.000480 | 0.000040 | unchanged |
| get | redis | 111981 | 111864–112234 | 89.8 | 99.9 | 8.90 | 90.0 | 8.87 | 0.000100 | 0.000030 | unchanged |
| get | valkey | 109115 | 105508–109345 | 95.8 | 100.6 | 9.20 | 87.0 | 8.17 | 0.000170 | 0.000050 | unchanged |
| set-nx-existing | rillcache | 121203 | 119313–121392 | 76.0 | 100.6 | 8.30 | 92.9 | 5.81 | 0.000410 | 0.000030 | unchanged |
| set-nx-existing | redis | 110971 | 105874–111530 | 91.0 | 99.8 | 9.00 | 88.9 | 8.87 | 0.000090 | 0.000040 | unchanged |
| set-nx-existing | valkey | 108921 | 108591–109425 | 93.1 | 100.2 | 9.20 | 87.0 | 8.17 | 0.000190 | 0.000040 | unchanged |
| set-xx-existing | rillcache | 119292 | 118464–119669 | 77.1 | 100.2 | 8.40 | 91.8 | 5.81 | 0.000420 | 0.000050 | unchanged |
| set-xx-existing | redis | 109980 | 109794–110007 | 91.8 | 101.0 | 9.20 | 89.1 | 8.84 | 0.000100 | 0.000030 | unchanged |
| set-xx-existing | valkey | 107022 | 106642–107548 | 97.4 | 100.0 | 9.30 | 86.2 | 8.16 | 0.000170 | 0.000050 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 4.77 | 4.77 | 99.0 | unchanged |
| redis | 1.000 | 0.000 | 8.63 | 8.63 | 10.0 | unchanged |
| valkey | 1.000 | 0.000 | 7.99 | 7.99 | 10.0 | unchanged |

</details>

<details>
<summary>single-core-pipeline-16: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 1000000; clients: 8; pipeline: 16; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: 0; client CPUs: 1,2,3.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 1943000 | 1912739–1950715 | 76.0 | 99.4 | 0.51 | 94.1 | 5.86 | 0.000011 | 0.000002 | unchanged |
| ping | redis | 1418437 | 1352528–1447142 | 123.2 | 99.8 | 0.69 | 72.5 | 8.80 | 0.000008 | 0.000003 | unchanged |
| ping | valkey | 1405046 | 1279479–1415436 | 125.0 | 99.8 | 0.71 | 73.2 | 8.33 | 0.000012 | 0.000003 | unchanged |
| set | rillcache | 1509416 | 1484553–1545250 | 97.3 | 99.6 | 0.66 | 72.3 | 5.86 | 0.000017 | 0.000003 | unchanged |
| set | redis | 1132680 | 1058948–1158632 | 162.9 | 99.6 | 0.88 | 58.9 | 8.79 | 0.000006 | 0.000004 | unchanged |
| set | valkey | 1036223 | 978986–1036581 | 181.0 | 99.5 | 0.96 | 50.0 | 8.32 | 0.000006 | 0.000003 | unchanged |
| get | rillcache | 1706589 | 1699995–1709587 | 86.6 | 100.7 | 0.59 | 81.4 | 5.86 | 0.000017 | 0.000003 | unchanged |
| get | redis | 1257516 | 1229138–1268100 | 145.6 | 100.1 | 0.81 | 63.3 | 8.80 | 0.000005 | 0.000003 | unchanged |
| get | valkey | 1173591 | 1099425–1177316 | 155.5 | 100.0 | 0.86 | 60.0 | 8.33 | 0.000006 | 0.000006 | unchanged |
| set-nx-existing | rillcache | 1598664 | 1551344–1613260 | 93.2 | 99.3 | 0.62 | 78.1 | 5.86 | 0.000002 | 0.000004 | unchanged |
| set-nx-existing | redis | 1181899 | 1119423–1198706 | 155.8 | 99.6 | 0.85 | 63.9 | 8.80 | 0.000005 | 0.000004 | unchanged |
| set-nx-existing | valkey | 1166298 | 1157381–1174632 | 157.8 | 99.8 | 0.85 | 58.8 | 8.33 | 0.000003 | 0.000002 | unchanged |
| set-xx-existing | rillcache | 1495616 | 1444760–1518132 | 97.6 | 100.2 | 0.67 | 68.7 | 5.86 | 0.000028 | 0.000004 | unchanged |
| set-xx-existing | redis | 1107110 | 1029505–1125104 | 167.0 | 99.8 | 0.90 | 55.6 | 8.80 | 0.000002 | 0.000004 | unchanged |
| set-xx-existing | valkey | 1001246 | 999950–1004224 | 188.0 | 100.0 | 1.00 | 52.5 | 8.33 | 0.000004 | 0.000005 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 4.82 | 4.82 | 99.0 | unchanged |
| redis | 1.000 | 0.000 | 8.54 | 8.54 | 10.0 | unchanged |
| valkey | 1.000 | 0.000 | 8.14 | 8.14 | 10.0 | unchanged |

</details>

### Fixed-worker mode measurements

30/30 separate embedded records validated. 8 clients; 10000 commands/workload/run; pipeline 16; three repetitions. These share the client/server host and inherited CPU affinity; they are **not native competitor comparisons or equal-per-core scaling evidence**. No process CPU/RSS accounting was collected for this group. Fixed lanes have a 1 ms idle polling wait. They actively poll for at most 50 µs after progress before returning to idle parking; this trades active CPU work for fewer premature sleeps, not epoll readiness or free CPU. Multiple lanes also request a cooperative yield after 100 µs of continuous active passes when more than one client is admitted. Progress does not rearm that separate budget; single lanes/lone clients do not periodically yield. This is checked between bounded passes, not a hard deadline or an OS fairness guarantee. The v2 interval restarts after a yield/park returns, excluding that explicit wait; involuntary descheduling can still consume wall time.

| Workload | 1 worker median cmd/s | 1 worker min–max | 4 workers median cmd/s | 4 workers min–max | 1 worker batch p99 µs | 4 workers batch p99 µs |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| ping | 1540801 | 1488205–1595865 | 1649785 | 1335234–2068128 | 1012.9 | 1055.0 |
| set | 1281104 | 1143591–1334995 | 53990 | 53451–54442 | 1010.4 | 3831.0 |
| get | 1413038 | 1406743–1440641 | 53440 | 53391–53980 | 1012.6 | 3355.3 |
| set-nx-existing | 1375489 | 1352576–1395144 | 55245 | 53910–56782 | 1015.8 | 3295.4 |
| set-xx-existing | 1310209 | 1228370–1325165 | 54531 | 53708–55429 | 1019.9 | 3296.8 |

### Interpretation limits

- Batch p99 is **not per-request p99**; never divide it by the pipeline length. Percentiles are not pooled across runs.
- CPU counters are tick-quantized; zero observed idle ticks do not prove zero CPU cost. CPU observation windows bracket, but are not identical to, request timers.
- RSS is an approximate endpoint sample, not allocator accounting or a workload peak; VmHWM includes startup, warmup and previous workloads.
- Context switches cover matched thread lifetimes only. Even unchanged endpoints cannot exclude threads born and destroyed between snapshots. PARTIAL coverage is not a process total.
- NX-existing measures rejected writes; XX-existing measures admitted overwrites. This is a small Redis-compatible subset, not equivalent functionality.
