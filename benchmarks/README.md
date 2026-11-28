# Current benchmark performance

Generated: 2026-11-28T14:13:51Z · [Benchmark run #26](https://github.com/macel94/rillcache/actions/runs/46152369773) · [Tested source `f0b5409`](https://github.com/macel94/rillcache/commit/f0b540945557cb5d4d553acab27a2a76d48d1cce)

**Benchmark job: success. Data: validated. Copilot review: completed.**

Benchmark attempt: 1; review attempt: 1. Preferred model: `gpt-6-luna` via **GitHub Copilot**.

[This report's permanent copy](https://github.com/macel94/rillcache/blob/main/benchmarks/reports/46152369773-1.md) · [Report history](https://github.com/macel94/rillcache/tree/main/benchmarks/reports) · [Methodology](https://github.com/macel94/rillcache/blob/main/docs/benchmarks.md)

## AI review

Actual model(s): **gpt-6-luna**. Selection: automatic-fallback. Authentication: workflow-token.

AI interpretation, **not measurements or a performance gate**. Verify claims against the deterministic tables below.

> Bottom line: In this validated, 180-record native cohort, Rillcache led the tested Redis and Valkey
> implementations on the measured workloads, but this is not evidence of a general Redis/Valkey
> performance win. Results are workload- and setup-specific.
> 
> Equal-one-logical-CPU tradeoff: For SET, pipeline 1 reached 218,102 commands/s versus Redis’s
> 181,490 (+20.2%); pipeline 16 reached 1.81M versus 1.32M (+37.5%). The corresponding whole-batch
> p99s were 51.4 vs 70.0 µs and 87.3 vs 152.2 µs—not per-request latency. Default-threading results
> are a separate comparison: 218,542 vs 184,147 commands/s at pipeline 1, and 1.81M vs 1.30M at
> pipeline 16. Do not treat those as equal-one-CPU results. NX-existing rejects writes; XX-existing
> admits overwrites.
> 
> Resource and scheduling observations: CPU measurements cluster around 100% of one logical CPU; SET
> CPU cost was 4.6 vs Redis’s 5.5 µs/command at pipeline 1, and 0.55 vs 0.77 at pipeline 16. These
> counters use a separate observation window, and zero idle ticks are quantized. Approximate RSS
> endpoints were lower for Rillcache (about 6.9 MiB vs 8.9 MiB for Redis in the single-core profiles);
> they are not workload peaks, while VmHWM is a lifetime peak. Matched-thread voluntary switches per
> command were also lower in these samples, but counters do not establish why or guarantee process
> totals.
> 
> Uncertainty and next investigations: Three repetitions and narrow 1,024-key, 64-byte workloads limit
> generalization; whole-batch percentiles are not pooled across runs. Fixed-worker results are a
> separate group without CPU/RSS accounting: four workers delivered about 101k SET/s versus 1.45M with
> one worker, with batch p99 3,384 vs 1,026 µs. Investigate (1) repeat that fixed-worker write/GET
> scaling test with contention and scheduling instrumentation, and (2) broaden native tests to larger
> values, keyspaces, and client counts, reporting per-request latency separately from batch p99.

## Measurements

**Data quality: complete.** 180/180 records validated across 4/4 complete profiles.


Hardware: Intel(R) Xeon(R) 6975P-C. Rust 1.100.0 (0ef1bc386 2026-11-09).

References: Redis 8.10.2 sha=00000000:0 malloc=jemalloc-5.3.0 bits=64 build=2ae49b6f80c754c; Valkey 9.1.2 sha=00000000:0 malloc=jemalloc-5.3.0 bits=64 build=e45e90479e40ae41.

### SET at a glance

Median commands/s and median of the three runs' **whole-batch p99** values (µs). Positive rate delta means Rillcache is faster in this workload only.

| Profile | Rillcache cmd/s | Redis | Valkey | Rillcache vs Redis | Rillcache p99 µs | Redis p99 | Valkey p99 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| pipeline-1 | 218542 | 184147 | 172910 | +18.7% | 51.4 | 68.5 | 75.2 |
| pipeline-16 | 1810972 | 1300704 | 1122989 | +39.2% | 87.4 | 154.2 | 178.6 |
| single-core-pipeline-1 | 218102 | 181490 | 172814 | +20.2% | 51.4 | 70.0 | 74.9 |
| single-core-pipeline-16 | 1813594 | 1319063 | 1142483 | +37.5% | 87.3 | 152.2 | 175.9 |

CPU is relative to **one logical CPU**, so parallel servers can exceed 100%. Do not blend default-threading and single-core results or different pipelines. Shared VMs and unequal feature sets do not establish a general performance win or a cross-run regression.

<details>
<summary>pipeline-1: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 100000; clients: 8; pipeline: 1; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: inherited; client CPUs: inherited.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 239338 | 237450–240798 | 46.4 | 100.5 | 4.20 | 90.5 | 6.75 | 0.000010 | 0.000020 | unchanged |
| ping | redis | 197747 | 192677–198908 | 63.6 | 99.4 | 5.10 | 84.3 | 9.07 | 0.000520 | 0.000200 | unchanged |
| ping | valkey | 191615 | 182089–192416 | 65.2 | 100.0 | 5.30 | 82.7 | 8.42 | 0.000340 | 0.000110 | unchanged |
| set | rillcache | 218542 | 217395–220044 | 51.4 | 101.2 | 4.60 | 84.8 | 6.75 | 0.000010 | 0.000040 | unchanged |
| set | redis | 184147 | 183674–184498 | 68.5 | 99.6 | 5.40 | 81.5 | 9.07 | 0.000160 | 0.000050 | unchanged |
| set | valkey | 172910 | 170530–173250 | 75.2 | 98.9 | 5.80 | 77.2 | 8.42 | 0.000460 | 0.000050 | unchanged |
| get | rillcache | 231936 | 231595–231947 | 48.2 | 99.7 | 4.30 | 88.4 | 6.75 | 0.000010 | 0.000030 | unchanged |
| get | redis | 189269 | 188918–189323 | 66.6 | 100.3 | 5.30 | 83.0 | 9.07 | 0.000560 | 0.000100 | unchanged |
| get | valkey | 179443 | 170975–179979 | 72.3 | 98.9 | 5.50 | 81.0 | 8.42 | 0.000880 | 0.000080 | unchanged |
| set-nx-existing | rillcache | 225247 | 224780–225493 | 50.3 | 99.2 | 4.40 | 90.7 | 6.75 | 0.000010 | 0.000040 | unchanged |
| set-nx-existing | redis | 186150 | 185102–187230 | 68.0 | 100.5 | 5.40 | 83.3 | 9.07 | 0.000250 | 0.000030 | unchanged |
| set-nx-existing | valkey | 176877 | 175869–176902 | 72.7 | 100.8 | 5.70 | 79.3 | 8.42 | 0.000420 | 0.000070 | unchanged |
| set-xx-existing | rillcache | 219421 | 219123–219503 | 51.2 | 100.8 | 4.60 | 84.4 | 6.75 | 0.000010 | 0.000020 | unchanged |
| set-xx-existing | redis | 181517 | 168326–183027 | 69.2 | 100.6 | 5.50 | 81.8 | 9.07 | 0.000260 | 0.000050 | unchanged |
| set-xx-existing | valkey | 171314 | 170947–171646 | 75.2 | 99.5 | 5.80 | 76.3 | 8.42 | 0.000270 | 0.000040 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 5.36 | 5.36 | 99.0 | unchanged |
| redis | 1.000 | 0.000 | 8.82 | 8.82 | 10.0 | unchanged |
| valkey | 1.000 | 0.000 | 8.23 | 8.23 | 10.0 | unchanged |

</details>

<details>
<summary>pipeline-16: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 1000000; clients: 8; pipeline: 16; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: inherited; client CPUs: inherited.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 2842588 | 2815018–2901305 | 61.4 | 98.6 | 0.35 | 71.4 | 6.86 | 0.000001 | 0.000006 | unchanged |
| ping | redis | 1931193 | 1879702–1945344 | 103.0 | 99.2 | 0.52 | 57.7 | 8.89 | 0.000042 | 0.000015 | unchanged |
| ping | valkey | 1749769 | 1744029–1760369 | 114.2 | 100.3 | 0.57 | 52.6 | 8.41 | 0.000020 | 0.000004 | unchanged |
| set | rillcache | 1810972 | 1808662–1811935 | 87.4 | 99.5 | 0.55 | 41.8 | 6.86 | 0.000001 | 0.000001 | unchanged |
| set | redis | 1300704 | 1294929–1305236 | 154.2 | 99.7 | 0.77 | 41.6 | 8.89 | 0.000010 | 0.000004 | unchanged |
| set | valkey | 1122989 | 1115444–1129742 | 178.6 | 99.4 | 0.89 | 38.2 | 8.41 | 0.000010 | 0.000003 | unchanged |
| get | rillcache | 2181538 | 2172733–2190123 | 76.3 | 99.9 | 0.46 | 60.0 | 6.86 | 0.000001 | 0.000003 | unchanged |
| get | redis | 1466173 | 1466104–1474219 | 137.5 | 99.7 | 0.68 | 44.8 | 8.89 | 0.000024 | 0.000003 | unchanged |
| get | valkey | 1330399 | 1325572–1330695 | 153.4 | 99.8 | 0.75 | 41.3 | 8.41 | 0.000007 | 0.000002 | unchanged |
| set-nx-existing | rillcache | 2043788 | 2038085–2045752 | 79.2 | 100.1 | 0.49 | 55.1 | 6.86 | 0.000001 | 0.000002 | unchanged |
| set-nx-existing | redis | 1341017 | 1339100–1348933 | 149.1 | 99.2 | 0.74 | 43.2 | 8.89 | 0.000026 | 0.000003 | unchanged |
| set-nx-existing | valkey | 1270285 | 1268153–1275391 | 158.5 | 100.2 | 0.79 | 40.5 | 8.41 | 0.000022 | 0.000003 | unchanged |
| set-xx-existing | rillcache | 1777919 | 1777294–1799973 | 88.2 | 101.3 | 0.57 | 42.9 | 6.86 | 0.000001 | 0.000002 | unchanged |
| set-xx-existing | redis | 1246044 | 1241594–1251297 | 161.1 | 99.7 | 0.80 | 43.8 | 8.89 | 0.000025 | 0.000006 | unchanged |
| set-xx-existing | valkey | 1092128 | 1091042–1096610 | 183.5 | 100.5 | 0.92 | 33.7 | 8.41 | 0.000018 | 0.000006 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 5.51 | 5.51 | 99.0 | unchanged |
| redis | 1.000 | 0.000 | 8.63 | 8.63 | 10.0 | unchanged |
| valkey | 1.000 | 0.000 | 8.21 | 8.21 | 10.0 | unchanged |

</details>

<details>
<summary>single-core-pipeline-1: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 100000; clients: 8; pipeline: 1; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: 0; client CPUs: 1,2,3.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 237090 | 236780–237589 | 46.6 | 97.4 | 4.10 | 92.7 | 6.96 | 0.000010 | 0.000030 | unchanged |
| ping | redis | 196292 | 196240–196436 | 64.2 | 100.1 | 5.10 | 86.3 | 8.87 | 0.000330 | 0.000010 | unchanged |
| ping | valkey | 192365 | 192010–193208 | 65.2 | 100.4 | 5.20 | 83.0 | 8.25 | 0.000370 | 0.000010 | unchanged |
| set | rillcache | 218102 | 216334–218455 | 51.4 | 100.3 | 4.60 | 87.0 | 6.96 | 0.000010 | 0.000010 | unchanged |
| set | redis | 181490 | 180535–182898 | 70.0 | 100.6 | 5.50 | 81.8 | 8.87 | 0.000410 | 0.000020 | unchanged |
| set | valkey | 172814 | 172811–172926 | 74.9 | 100.2 | 5.80 | 77.6 | 8.25 | 0.000330 | 0.000020 | unchanged |
| get | rillcache | 225963 | 225253–226273 | 49.4 | 99.4 | 4.40 | 88.6 | 6.96 | 0.000010 | 0.000010 | unchanged |
| get | redis | 189007 | 188601–189823 | 66.4 | 100.1 | 5.30 | 86.8 | 8.85 | 0.000460 | 0.000020 | unchanged |
| get | valkey | 179482 | 179170–180834 | 71.3 | 99.4 | 5.50 | 80.4 | 8.25 | 0.000200 | 0.000030 | unchanged |
| set-nx-existing | rillcache | 222004 | 221628–223922 | 50.4 | 99.7 | 4.50 | 86.7 | 6.96 | 0.000010 | 0.000010 | unchanged |
| set-nx-existing | redis | 184696 | 183783–185298 | 68.0 | 99.7 | 5.40 | 81.5 | 8.86 | 0.000290 | 0.000030 | unchanged |
| set-nx-existing | valkey | 177886 | 177644–178440 | 72.2 | 101.2 | 5.70 | 80.7 | 8.25 | 0.000280 | 0.000020 | unchanged |
| set-xx-existing | rillcache | 215359 | 215135–216665 | 51.8 | 101.1 | 4.70 | 85.1 | 6.96 | 0.000010 | 0.000020 | unchanged |
| set-xx-existing | redis | 180711 | 179317–181907 | 70.7 | 100.0 | 5.50 | 80.0 | 8.86 | 0.000130 | 0.000010 | unchanged |
| set-xx-existing | valkey | 171557 | 171354–172913 | 75.2 | 100.3 | 5.80 | 79.3 | 8.25 | 0.000380 | 0.000020 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 5.53 | 5.53 | 99.0 | unchanged |
| redis | 1.000 | 0.000 | 8.62 | 8.62 | 10.0 | unchanged |
| valkey | 1.000 | 0.000 | 8.07 | 8.07 | 10.0 | unchanged |

</details>

<details>
<summary>single-core-pipeline-16: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 1000000; clients: 8; pipeline: 16; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: 0; client CPUs: 1,2,3.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 2842224 | 2833709–2888068 | 60.0 | 99.4 | 0.35 | 70.6 | 6.92 | 0.000001 | 0.000001 | unchanged |
| ping | redis | 1941441 | 1941127–1952962 | 102.1 | 100.9 | 0.52 | 61.5 | 8.82 | 0.000014 | 0.000001 | unchanged |
| ping | valkey | 1758161 | 1750219–1761356 | 113.9 | 99.7 | 0.57 | 51.8 | 8.39 | 0.000011 | 0.000004 | unchanged |
| set | rillcache | 1813594 | 1805884–1824076 | 87.3 | 99.7 | 0.55 | 43.6 | 6.92 | 0.000001 | 0.000001 | unchanged |
| set | redis | 1319063 | 1316489–1321176 | 152.2 | 98.9 | 0.75 | 44.0 | 8.82 | 0.000010 | 0.000001 | unchanged |
| set | valkey | 1142483 | 1138334–1144423 | 175.9 | 99.4 | 0.87 | 39.1 | 8.39 | 0.000026 | 0.000002 | unchanged |
| get | rillcache | 2170903 | 2170013–2177295 | 76.2 | 102.0 | 0.47 | 59.6 | 6.92 | 0.000002 | 0.000001 | unchanged |
| get | redis | 1489254 | 1485185–1498114 | 135.7 | 99.5 | 0.67 | 49.3 | 8.82 | 0.000009 | 0.000001 | unchanged |
| get | valkey | 1340135 | 1333379–1349107 | 151.9 | 99.8 | 0.74 | 43.2 | 8.39 | 0.000010 | 0.000002 | unchanged |
| set-nx-existing | rillcache | 2043588 | 2041992–2044708 | 78.8 | 100.2 | 0.49 | 53.1 | 6.92 | 0.000001 | 0.000000 | unchanged |
| set-nx-existing | redis | 1353358 | 1351642–1364586 | 148.2 | 99.6 | 0.73 | 42.5 | 8.82 | 0.000013 | 0.000002 | unchanged |
| set-nx-existing | valkey | 1278393 | 1278379–1286982 | 157.6 | 101.0 | 0.79 | 40.5 | 8.39 | 0.000012 | 0.000001 | unchanged |
| set-xx-existing | rillcache | 1773784 | 1762667–1783283 | 89.3 | 99.3 | 0.56 | 44.8 | 6.92 | 0.000003 | 0.000002 | unchanged |
| set-xx-existing | redis | 1264004 | 1256909–1264964 | 159.0 | 99.9 | 0.79 | 42.5 | 8.82 | 0.000011 | 0.000003 | unchanged |
| set-xx-existing | valkey | 1106318 | 1105898–1108300 | 181.2 | 100.7 | 0.91 | 35.6 | 8.39 | 0.000007 | 0.000001 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 5.43 | 5.43 | 99.0 | unchanged |
| redis | 1.000 | 0.000 | 8.56 | 8.56 | 10.0 | unchanged |
| valkey | 1.000 | 0.000 | 8.20 | 8.20 | 10.0 | unchanged |

</details>

### Fixed-worker mode measurements

30/30 separate embedded records validated. 8 clients; 10000 commands/workload/run; pipeline 16; three repetitions. These share the client/server host and inherited CPU affinity; they are **not native competitor comparisons or equal-per-core scaling evidence**. No process CPU/RSS accounting was collected for this group. Fixed lanes have a 1 ms idle polling wait. They actively poll for at most 50 µs after progress before returning to idle parking; this trades active CPU work for fewer premature sleeps, not epoll readiness or free CPU. Multiple lanes also request a cooperative yield after 100 µs of continuous active passes when more than one client is admitted. Progress does not rearm that separate budget; single lanes/lone clients do not periodically yield. This is checked between bounded passes, not a hard deadline or an OS fairness guarantee. The v2 interval restarts after a yield/park returns, excluding that explicit wait; involuntary descheduling can still consume wall time.

| Workload | 1 worker median cmd/s | 1 worker min–max | 4 workers median cmd/s | 4 workers min–max | 1 worker batch p99 µs | 4 workers batch p99 µs |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| ping | 2141408 | 2096916–2190608 | 2721875 | 2574356–3061382 | 1029.4 | 908.4 |
| set | 1449267 | 1362220–1515107 | 100818 | 97609–104819 | 1025.6 | 3384.1 |
| get | 1766754 | 1753194–1778001 | 99297 | 94790–104538 | 1035.8 | 3701.1 |
| set-nx-existing | 1669115 | 1636175–1669720 | 112325 | 109822–114937 | 1027.3 | 2787.0 |
| set-xx-existing | 1477345 | 1473285–1501469 | 102552 | 98907–103637 | 1025.0 | 3241.5 |

### Interpretation limits

- Batch p99 is **not per-request p99**; never divide it by the pipeline length. Percentiles are not pooled across runs.
- CPU counters are tick-quantized; zero observed idle ticks do not prove zero CPU cost. CPU observation windows bracket, but are not identical to, request timers.
- RSS is an approximate endpoint sample, not allocator accounting or a workload peak; VmHWM includes startup, warmup and previous workloads.
- Context switches cover matched thread lifetimes only. Even unchanged endpoints cannot exclude threads born and destroyed between snapshots. PARTIAL coverage is not a process total.
- NX-existing measures rejected writes; XX-existing measures admitted overwrites. This is a small Redis-compatible subset, not equivalent functionality.
