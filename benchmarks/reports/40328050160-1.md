# Current benchmark performance

Generated: 2026-10-26T13:15:21Z · [Benchmark run #12](https://github.com/macel94/rillcache/actions/runs/40328050160) · [Tested source `37c46e7`](https://github.com/macel94/rillcache/commit/37c46e7ec1200dce038165e101a7eb542694f0e4)

**Benchmark job: success. Data: validated. Copilot review: completed.**

Benchmark attempt: 1; review attempt: 1. Preferred model: `gpt-6-luna` via **GitHub Copilot**.

[This report's permanent copy](https://github.com/macel94/rillcache/blob/main/benchmarks/reports/40328050160-1.md) · [Report history](https://github.com/macel94/rillcache/tree/main/benchmarks/reports) · [Methodology](https://github.com/macel94/rillcache/blob/main/docs/benchmarks.md)

## AI review

Actual model(s): **gpt-6-luna**. Selection: automatic-fallback. Authentication: workflow-token.

AI interpretation, **not measurements or a performance gate**. Verify claims against the deterministic tables below.

> Bottom line: In this validated cohort, Rillcache led these tested SET workloads on throughput and
> whole-batch p99, including under an equal one-logical-CPU budget. This is evidence about a small
> Redis-compatible subset and these specific workloads—not a general Redis or Valkey victory.
> 
> Equal-CPU tradeoff: At pipeline 1, Rillcache delivered 131,190 commands/s versus Redis’s 119,869
> (+9.4%), with whole-batch p99 of 74.3 versus 105.6 µs. At pipeline 16, it reached 1,519,054 versus
> 956,986 commands/s (+58.7%), and batch p99 was 99.2 versus 208.8 µs. These percentiles are for the
> batch, not per-request latency; they should not be divided by pipeline length.
> 
> CPU, memory, scheduling: Do not conflate those equal-CPU results with inherited-affinity runs:
> default-threading SET throughput was 258,704 and 2,119,873 commands/s at pipelines 1 and 16, while
> Rillcache used 178.4% and 253.4% of one logical CPU. In the equal-CPU pipeline-16 SET run, CPU was
> about 100% for all three servers; Rillcache’s reported CPU time was 0.66 µs/command versus Redis’s
> 1.05. RSS endpoint samples were lower for Rillcache (3.73 vs 11.58 MiB), but are approximate, not
> workload peaks; lifetime VmHWM is a different measure. Rillcache also recorded more voluntary
> switches per command than Redis in these samples. Counters alone do not establish why; matched
> thread lifetimes are not guaranteed process totals.
> 
> Uncertainty: Results cover three runs per workload on a shared VM; CPU ticks are quantized, and
> observation windows differ from request timers. NX-existing rejects writes, whereas XX-existing
> admits overwrites. The fixed-worker figures are a separate embedded group, not native competitor or
> equal-per-core evidence.
> 
> Next investigations: (1) Repeat equal-CPU tests across hosts and longer runs, reporting run-level
> variation and batch-p99 distributions. (2) Test broader key/value sizes and read/write mixes while
> profiling CPU and scheduling, to see whether throughput, tail latency, and switch-count patterns
> hold beyond these cases.

## Measurements

**Data quality: complete.** 180/180 records validated across 4/4 complete profiles.


Hardware: INTEL(R) XEON(R) PLATINUM 8573C. Rust 1.99.1 (58cf8f9c3 2026-10-13).

References: Redis 8.10.2 sha=00000000:0 malloc=jemalloc-5.3.0 bits=64 build=e64601ceff6e527b; Valkey 9.1.2 sha=00000000:0 malloc=jemalloc-5.3.0 bits=64 build=cdef507fedbc4d7a.

### SET at a glance

Median commands/s and median of the three runs' **whole-batch p99** values (µs). Positive rate delta means Rillcache is faster in this workload only.

| Profile | Rillcache cmd/s | Redis | Valkey | Rillcache vs Redis | Rillcache p99 µs | Redis p99 | Valkey p99 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| pipeline-1 | 258704 | 120797 | 115532 | +114.2% | 71.7 | 101.8 | 110.3 |
| pipeline-16 | 2119873 | 957636 | 860297 | +121.4% | 145.9 | 208.8 | 231.0 |
| single-core-pipeline-1 | 131190 | 119869 | 116349 | +9.4% | 74.3 | 105.6 | 108.2 |
| single-core-pipeline-16 | 1519054 | 956986 | 854324 | +58.7% | 99.2 | 208.8 | 233.2 |

CPU is relative to **one logical CPU**, so parallel servers can exceed 100%. Do not blend default-threading and single-core results or different pipelines. Shared VMs and unequal feature sets do not establish a general performance win or a cross-run regression.

<details>
<summary>pipeline-1: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 100000; clients: 8; pipeline: 1; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: inherited; client CPUs: inherited.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 253770 | 249422–254494 | 67.4 | 177.0 | 7.10 | 93.1 | 3.25 | 0.821740 | 0.192760 | unchanged |
| ping | redis | 126425 | 125917–127570 | 96.1 | 100.7 | 7.90 | 86.1 | 11.39 | 0.000240 | 0.000050 | unchanged |
| ping | valkey | 122637 | 119006–124888 | 104.2 | 99.9 | 8.10 | 85.0 | 10.84 | 0.000470 | 0.000080 | unchanged |
| set | rillcache | 258704 | 248692–273823 | 71.7 | 178.4 | 6.80 | 89.7 | 3.25 | 0.782900 | 0.230080 | unchanged |
| set | redis | 120797 | 99075–120837 | 101.8 | 100.2 | 8.30 | 81.9 | 11.39 | 0.000070 | 0.000050 | unchanged |
| set | valkey | 115532 | 114399–116501 | 110.3 | 99.5 | 8.60 | 80.2 | 10.94 | 0.000170 | 0.000050 | unchanged |
| get | rillcache | 245336 | 227760–322098 | 71.4 | 171.6 | 6.90 | 91.4 | 3.25 | 0.830040 | 0.180220 | unchanged |
| get | redis | 119073 | 100597–122051 | 109.6 | 100.6 | 8.50 | 82.9 | 11.39 | 0.000190 | 0.000050 | unchanged |
| get | valkey | 113475 | 98744–115935 | 115.1 | 99.8 | 8.80 | 80.7 | 10.95 | 0.000580 | 0.000080 | unchanged |
| set-nx-existing | rillcache | 271385 | 243708–285078 | 65.3 | 176.3 | 6.50 | 89.4 | 3.25 | 0.806440 | 0.207270 | unchanged |
| set-nx-existing | redis | 101544 | 99966–105933 | 124.1 | 99.6 | 9.80 | 81.6 | 10.85 | 0.000170 | 0.000070 | unchanged |
| set-nx-existing | valkey | 119539 | 99284–120967 | 102.7 | 99.2 | 8.30 | 81.9 | 10.95 | 0.000350 | 0.000030 | unchanged |
| set-xx-existing | rillcache | 247930 | 247311–263866 | 66.9 | 168.5 | 6.70 | 89.7 | 3.25 | 0.787130 | 0.228190 | unchanged |
| set-xx-existing | redis | 115891 | 105660–118176 | 111.9 | 99.3 | 8.70 | 82.8 | 10.85 | 0.000070 | 0.000050 | unchanged |
| set-xx-existing | valkey | 113280 | 105904–116954 | 114.0 | 99.5 | 8.70 | 79.8 | 10.73 | 0.000130 | 0.000080 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 2.61 | 2.61 | 944.9 | unchanged |
| redis | 1.000 | 0.000 | 11.37 | 11.37 | 10.0 | unchanged |
| valkey | 1.000 | 0.000 | 10.82 | 10.82 | 10.0 | unchanged |

</details>

<details>
<summary>pipeline-16: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 1000000; clients: 8; pipeline: 16; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: inherited; client CPUs: inherited.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 4199976 | 3715266–4935090 | 69.6 | 174.4 | 0.41 | 87.8 | 3.23 | 0.044618 | 0.018338 | unchanged |
| ping | redis | 1290659 | 1275653–1438840 | 152.9 | 99.5 | 0.77 | 62.8 | 11.44 | 0.000036 | 0.000015 | unchanged |
| ping | valkey | 1194557 | 1183187–1341766 | 166.7 | 100.3 | 0.84 | 61.9 | 10.98 | 0.000022 | 0.000004 | unchanged |
| set | rillcache | 2119873 | 2113100–2159826 | 145.9 | 253.4 | 1.20 | 58.8 | 3.23 | 0.089855 | 0.015972 | unchanged |
| set | redis | 957636 | 933556–962161 | 208.8 | 99.6 | 1.04 | 43.9 | 11.45 | 0.000008 | 0.000007 | unchanged |
| set | valkey | 860297 | 855703–861000 | 231.0 | 99.8 | 1.16 | 38.5 | 11.07 | 0.000004 | 0.000007 | unchanged |
| get | rillcache | 3538494 | 3444693–4062297 | 70.6 | 219.2 | 0.61 | 66.1 | 3.23 | 0.046044 | 0.017849 | unchanged |
| get | redis | 1059903 | 1059526–1072316 | 186.7 | 99.6 | 0.94 | 51.6 | 11.45 | 0.000009 | 0.000003 | unchanged |
| get | valkey | 974169 | 963688–977037 | 202.4 | 99.6 | 1.02 | 47.1 | 11.07 | 0.000012 | 0.000005 | unchanged |
| set-nx-existing | rillcache | 3419677 | 3242116–3536072 | 93.3 | 242.9 | 0.72 | 63.4 | 3.23 | 0.063461 | 0.007488 | unchanged |
| set-nx-existing | redis | 1005991 | 992671–1010132 | 198.4 | 100.0 | 0.99 | 48.5 | 11.45 | 0.000014 | 0.000007 | unchanged |
| set-nx-existing | valkey | 945629 | 945314–953792 | 208.7 | 100.2 | 1.06 | 43.8 | 11.07 | 0.000006 | 0.000005 | unchanged |
| set-xx-existing | rillcache | 2115628 | 2092593–2117489 | 143.9 | 257.2 | 1.23 | 60.6 | 3.23 | 0.089171 | 0.015675 | unchanged |
| set-xx-existing | redis | 923895 | 921955–926293 | 216.1 | 99.8 | 1.08 | 40.7 | 11.45 | 0.000007 | 0.000006 | unchanged |
| set-xx-existing | valkey | 836102 | 832012–841078 | 237.0 | 100.1 | 1.19 | 37.8 | 10.93 | 0.000009 | 0.000005 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 2.58 | 2.58 | 944.9 | unchanged |
| redis | 1.000 | 0.000 | 11.41 | 11.41 | 10.0 | unchanged |
| valkey | 1.000 | 0.000 | 10.95 | 10.95 | 10.0 | unchanged |

</details>

<details>
<summary>single-core-pipeline-1: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 100000; clients: 8; pipeline: 1; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: 0; client CPUs: 1,2,3.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 135235 | 133799–138317 | 80.2 | 100.3 | 7.40 | 90.7 | 3.66 | 0.007870 | 0.007390 | unchanged |
| ping | redis | 125786 | 122557–126565 | 99.9 | 100.6 | 8.00 | 87.5 | 11.41 | 0.000260 | 0.000040 | unchanged |
| ping | valkey | 123137 | 122324–123669 | 102.2 | 99.7 | 8.10 | 84.0 | 10.95 | 0.000060 | 0.000030 | unchanged |
| set | rillcache | 131190 | 129782–132734 | 74.3 | 101.0 | 7.70 | 87.2 | 3.66 | 0.008130 | 0.007730 | unchanged |
| set | redis | 119869 | 119179–120282 | 105.6 | 100.1 | 8.40 | 83.1 | 11.42 | 0.000070 | 0.000040 | unchanged |
| set | valkey | 116349 | 102669–118549 | 108.2 | 100.0 | 8.60 | 79.1 | 11.05 | 0.000090 | 0.000030 | unchanged |
| get | rillcache | 132273 | 129703–134387 | 85.8 | 100.5 | 7.60 | 89.6 | 3.68 | 0.007950 | 0.007510 | unchanged |
| get | redis | 119744 | 118440–122365 | 105.4 | 100.6 | 8.40 | 84.1 | 11.42 | 0.000150 | 0.000020 | unchanged |
| get | valkey | 119856 | 118293–121546 | 106.5 | 99.4 | 8.30 | 83.1 | 11.06 | 0.000130 | 0.000030 | unchanged |
| set-nx-existing | rillcache | 132805 | 129696–134477 | 74.0 | 100.8 | 7.60 | 88.2 | 3.68 | 0.008070 | 0.007610 | unchanged |
| set-nx-existing | redis | 119685 | 119356–121423 | 106.0 | 100.2 | 8.30 | 82.1 | 11.42 | 0.000030 | 0.000050 | unchanged |
| set-nx-existing | valkey | 117794 | 114460–118159 | 108.4 | 99.6 | 8.50 | 83.3 | 11.06 | 0.000050 | 0.000030 | unchanged |
| set-xx-existing | rillcache | 127394 | 127344–127827 | 88.1 | 99.7 | 7.80 | 88.6 | 3.68 | 0.008250 | 0.007860 | unchanged |
| set-xx-existing | redis | 115263 | 107871–118847 | 113.9 | 100.3 | 8.70 | 79.6 | 10.96 | 0.000210 | 0.000030 | unchanged |
| set-xx-existing | valkey | 107429 | 95554–115182 | 123.0 | 99.9 | 9.30 | 78.5 | 10.82 | 0.000170 | 0.000030 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 2.64 | 2.64 | 945.9 | unchanged |
| redis | 1.000 | 0.000 | 11.40 | 11.40 | 10.0 | unchanged |
| valkey | 1.000 | 0.000 | 10.93 | 10.93 | 10.0 | unchanged |

</details>

<details>
<summary>single-core-pipeline-16: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 1000000; clients: 8; pipeline: 16; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: 0; client CPUs: 1,2,3.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 1948895 | 1894908–1959916 | 84.8 | 99.9 | 0.51 | 83.0 | 3.73 | 0.000550 | 0.000522 | unchanged |
| ping | redis | 1279920 | 1274368–1294351 | 156.1 | 99.8 | 0.78 | 64.1 | 11.58 | 0.000020 | 0.000003 | unchanged |
| ping | valkey | 1197247 | 1181958–1202008 | 167.4 | 100.4 | 0.84 | 57.1 | 10.89 | 0.000012 | 0.000005 | unchanged |
| set | rillcache | 1519054 | 1508086–1524429 | 99.2 | 100.6 | 0.66 | 64.2 | 3.73 | 0.000705 | 0.000665 | unchanged |
| set | redis | 956986 | 956941–958405 | 208.8 | 100.5 | 1.05 | 43.8 | 11.58 | 0.000003 | 0.000006 | unchanged |
| set | valkey | 854324 | 848598–859055 | 233.2 | 99.6 | 1.16 | 37.1 | 10.98 | 0.000010 | 0.000003 | unchanged |
| get | rillcache | 1600947 | 1576892–1605947 | 101.4 | 100.8 | 0.63 | 71.4 | 3.73 | 0.000670 | 0.000635 | unchanged |
| get | redis | 1064919 | 1057364–1064927 | 187.1 | 100.1 | 0.94 | 52.1 | 11.58 | 0.000010 | 0.000003 | unchanged |
| get | valkey | 967063 | 952860–971451 | 206.2 | 100.0 | 1.04 | 43.8 | 10.98 | 0.000014 | 0.000003 | unchanged |
| set-nx-existing | rillcache | 1539151 | 1521862–1545820 | 102.2 | 98.9 | 0.65 | 68.8 | 3.73 | 0.000674 | 0.000644 | unchanged |
| set-nx-existing | redis | 994003 | 987230–994573 | 199.9 | 100.4 | 1.01 | 46.5 | 11.58 | 0.000007 | 0.000006 | unchanged |
| set-nx-existing | valkey | 942468 | 935448–946874 | 211.3 | 100.3 | 1.07 | 42.5 | 10.98 | 0.000004 | 0.000004 | unchanged |
| set-xx-existing | rillcache | 1472015 | 1416099–1498405 | 107.1 | 100.4 | 0.68 | 62.7 | 3.74 | 0.000717 | 0.000699 | unchanged |
| set-xx-existing | redis | 920054 | 918141–923459 | 216.7 | 100.1 | 1.09 | 42.1 | 11.58 | 0.000007 | 0.000004 | unchanged |
| set-xx-existing | valkey | 833163 | 826749–838950 | 239.2 | 100.0 | 1.20 | 37.2 | 10.77 | 0.000019 | 0.000004 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 2.62 | 2.62 | 945.8 | unchanged |
| redis | 1.000 | 0.000 | 11.54 | 11.54 | 10.0 | unchanged |
| valkey | 1.000 | 0.000 | 10.86 | 10.86 | 10.0 | unchanged |

</details>

### Fixed-worker mode measurements

30/30 separate embedded records validated. 8 clients; 10000 commands/workload/run; pipeline 16; three repetitions. These share the client/server host and inherited CPU affinity; they are **not native competitor comparisons or equal-per-core scaling evidence**. No process CPU/RSS accounting was collected for this group. Fixed lanes have a 1 ms idle polling wait. They actively poll for at most 50 µs after progress before returning to idle parking; this trades active CPU work for fewer premature sleeps, not epoll readiness or free CPU. Multiple lanes also request a cooperative yield after 100 µs of continuous active passes when more than one client is admitted. Progress does not rearm that separate budget; single lanes/lone clients do not periodically yield. This is checked between bounded passes, not a hard deadline or an OS fairness guarantee. The v2 interval restarts after a yield/park returns, excluding that explicit wait; involuntary descheduling can still consume wall time.

| Workload | 1 worker median cmd/s | 1 worker min–max | 4 workers median cmd/s | 4 workers min–max | 1 worker batch p99 µs | 4 workers batch p99 µs |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| ping | 1562868 | 1293858–1582775 | 1701752 | 1408843–1899092 | 933.2 | 1089.4 |
| set | 1368042 | 1111060–1377292 | 1619962 | 1381133–1634824 | 964.0 | 1099.3 |
| get | 1294720 | 1114221–1422082 | 1724968 | 1405033–1749610 | 977.6 | 1074.6 |
| set-nx-existing | 1118936 | 1115770–1428758 | 1650640 | 1355341–1949941 | 980.7 | 980.6 |
| set-xx-existing | 1094937 | 1074134–1363639 | 1383816 | 1121944–1635428 | 985.3 | 1054.1 |

### Interpretation limits

- Batch p99 is **not per-request p99**; never divide it by the pipeline length. Percentiles are not pooled across runs.
- CPU counters are tick-quantized; zero observed idle ticks do not prove zero CPU cost. CPU observation windows bracket, but are not identical to, request timers.
- RSS is an approximate endpoint sample, not allocator accounting or a workload peak; VmHWM includes startup, warmup and previous workloads.
- Context switches cover matched thread lifetimes only. Even unchanged endpoints cannot exclude threads born and destroyed between snapshots. PARTIAL coverage is not a process total.
- NX-existing measures rejected writes; XX-existing measures admitted overwrites. This is a small Redis-compatible subset, not equivalent functionality.
