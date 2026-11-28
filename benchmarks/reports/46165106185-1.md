# Current benchmark performance

Generated: 2026-11-28T16:45:34Z · [Benchmark run #28](https://github.com/macel94/rillcache/actions/runs/46165106185) · [Tested source `4c2c247`](https://github.com/macel94/rillcache/commit/4c2c24718584a0f95ee208abf191230f663893e1)

**Benchmark job: success. Data: validated. Copilot review: completed.**

Benchmark attempt: 1; review attempt: 1. Preferred model: `gpt-6-luna` via **GitHub Copilot**.

[This report's permanent copy](https://github.com/macel94/rillcache/blob/main/benchmarks/reports/46165106185-1.md) · [Report history](https://github.com/macel94/rillcache/tree/main/benchmarks/reports) · [Methodology](https://github.com/macel94/rillcache/blob/main/docs/benchmarks.md)

## AI review

Actual model(s): **gpt-6-luna**. Selection: automatic-fallback. Authentication: workflow-token.

AI interpretation, **not measurements or a performance gate**. Verify claims against the deterministic tables below.

> Bottom line: In this validated native cohort (180 records), Rillcache recorded higher throughput and
> lower whole-batch p99 than Redis and Valkey in the measured workloads. This is a workload-specific
> result, not a general Redis/Valkey victory.
> 
> At an equal one-logical-CPU budget (server pinned to CPU 0), SET pipeline 1 reached 95,238
> commands/s (+10.6% vs Redis), with batch p99 99.3 µs vs 120.7 µs for Redis and 125.5 µs for Valkey.
> At pipeline 16, it reached 1,088,081 commands/s (+24.9% vs Redis), with batch p99 135.5 µs vs 218.8
> and 255.0 µs. These are whole-batch percentiles, not per-request latency; do not divide by pipeline
> length. Default-threading results are separate: SET pipeline 16 was +29.0% vs Redis, not an
> equal-core comparison.
> 
> Resource and scheduling observations: CPU was near 100% of one logical CPU in these tests; SET CPU
> time was 10.5 µs/command at pipeline 1 and 0.92 µs at pipeline 16 for Rillcache. Approximate RSS
> endpoints were lower (6.92 vs Redis 8.99 MiB at single-core pipeline 1; 7.25 vs 9.02 MiB at pipeline
> 16), but these are not workload peaks; VmHWM is a lifetime peak. Scheduling counters were small,
> but cover matched thread lifetimes, not guaranteed process totals. Zero idle CPU ticks are
> quantized, not proof of zero cost.
> 
> Uncertainty: Shared-VM conditions, different feature sets, three repetitions, and CPU observation
> windows that do not exactly match request timers limit interpretation. NX-existing rejects writes;
> XX-existing admits overwrites.
> 
> Next investigations: (1) Expand equal-core runs across key/value sizes and mixed command workloads
> to test whether the throughput and tail-latency pattern holds beyond these settings. (2) Diagnose
> the separate fixed-worker group’s four-worker SET/GET results (~49–50k commands/s vs ~1.0M with one
> worker, with batch p99 rising to 3.6–4.5 ms); collect CPU/RSS and scheduling data there before
> interpreting the cause.

## Measurements

**Data quality: complete.** 180/180 records validated across 4/4 complete profiles.


Hardware: AMD EPYC 9V74 80-Core Processor. Rust 1.100.0 (0ef1bc386 2026-11-09).

References: Redis 8.10.2 sha=00000000:0 malloc=jemalloc-5.3.0 bits=64 build=38a25c0993daabc5; Valkey 9.1.2 sha=00000000:0 malloc=jemalloc-5.3.0 bits=64 build=539adeb5859231af.

### SET at a glance

Median commands/s and median of the three runs' **whole-batch p99** values (µs). Positive rate delta means Rillcache is faster in this workload only.

| Profile | Rillcache cmd/s | Redis | Valkey | Rillcache vs Redis | Rillcache p99 µs | Redis p99 | Valkey p99 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| pipeline-1 | 95021 | 85915 | 84107 | +10.6% | 98.9 | 121.0 | 125.3 |
| pipeline-16 | 1121275 | 869048 | 761231 | +29.0% | 133.9 | 217.1 | 254.7 |
| single-core-pipeline-1 | 95238 | 86077 | 83765 | +10.6% | 99.3 | 120.7 | 125.5 |
| single-core-pipeline-16 | 1088081 | 870944 | 760848 | +24.9% | 135.5 | 218.8 | 255.0 |

CPU is relative to **one logical CPU**, so parallel servers can exceed 100%. Do not blend default-threading and single-core results or different pipelines. Shared VMs and unequal feature sets do not establish a general performance win or a cross-run regression.

<details>
<summary>pipeline-1: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 100000; clients: 8; pipeline: 1; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: inherited; client CPUs: inherited.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 97935 | 97889–98628 | 96.0 | 99.6 | 10.10 | 95.0 | 7.03 | 0.000010 | 0.000060 | unchanged |
| ping | redis | 88962 | 88567–89274 | 114.6 | 100.0 | 11.30 | 90.9 | 8.89 | 0.000070 | 0.000270 | unchanged |
| ping | valkey | 88148 | 87365–93171 | 117.1 | 99.6 | 11.30 | 90.3 | 8.35 | 0.000090 | 0.000050 | unchanged |
| set | rillcache | 95021 | 94915–95442 | 98.9 | 99.6 | 10.50 | 90.5 | 7.03 | 0.000010 | 0.000080 | unchanged |
| set | redis | 85915 | 85712–86215 | 121.0 | 99.6 | 11.60 | 86.3 | 8.91 | 0.000060 | 0.000050 | unchanged |
| set | valkey | 84107 | 83578–88426 | 125.3 | 99.9 | 11.90 | 87.4 | 8.37 | 0.000170 | 0.000070 | unchanged |
| get | rillcache | 95833 | 94804–96265 | 98.1 | 99.6 | 10.40 | 93.3 | 7.03 | 0.000010 | 0.000060 | unchanged |
| get | redis | 87242 | 86980–88258 | 118.2 | 100.3 | 11.50 | 87.8 | 8.89 | 0.000310 | 0.000070 | unchanged |
| get | valkey | 89871 | 84873–90388 | 116.4 | 100.6 | 11.20 | 88.3 | 8.35 | 0.000160 | 0.000060 | unchanged |
| set-nx-existing | rillcache | 95281 | 95170–95531 | 98.4 | 100.0 | 10.50 | 93.3 | 7.03 | 0.000010 | 0.000040 | unchanged |
| set-nx-existing | redis | 86640 | 85939–90791 | 119.6 | 100.5 | 11.60 | 86.2 | 8.89 | 0.000160 | 0.000040 | unchanged |
| set-nx-existing | valkey | 85232 | 84888–88210 | 122.9 | 99.7 | 11.70 | 88.5 | 8.35 | 0.000080 | 0.000050 | unchanged |
| set-xx-existing | rillcache | 94732 | 94625–95111 | 99.2 | 99.3 | 10.50 | 91.4 | 7.03 | 0.000010 | 0.000060 | unchanged |
| set-xx-existing | redis | 87428 | 85896–89973 | 117.7 | 99.8 | 11.50 | 87.9 | 8.89 | 0.000100 | 0.000040 | unchanged |
| set-xx-existing | valkey | 87313 | 85702–87758 | 123.3 | 100.0 | 11.50 | 83.3 | 8.34 | 0.000210 | 0.000110 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 5.63 | 5.63 | 99.0 | unchanged |
| redis | 1.000 | 0.000 | 8.69 | 8.69 | 10.0 | unchanged |
| valkey | 1.000 | 0.000 | 8.19 | 8.19 | 10.0 | unchanged |

</details>

<details>
<summary>pipeline-16: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 1000000; clients: 8; pipeline: 16; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: inherited; client CPUs: inherited.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 1466939 | 1451382–1509918 | 102.9 | 101.1 | 0.69 | 82.1 | 6.79 | 0.000001 | 0.000003 | unchanged |
| ping | redis | 1169773 | 1147959–1170743 | 152.5 | 99.5 | 0.85 | 69.3 | 8.91 | 0.000008 | 0.000004 | unchanged |
| ping | valkey | 1085422 | 1082933–1129726 | 167.1 | 99.6 | 0.92 | 67.7 | 8.47 | 0.000004 | 0.000003 | unchanged |
| set | rillcache | 1121275 | 1105206–1140982 | 133.9 | 99.4 | 0.89 | 64.0 | 6.79 | 0.000001 | 0.000005 | unchanged |
| set | redis | 869048 | 866118–882332 | 217.1 | 99.7 | 1.14 | 55.2 | 8.92 | 0.000013 | 0.000003 | unchanged |
| set | valkey | 761231 | 758202–779939 | 254.7 | 100.5 | 1.32 | 50.0 | 8.47 | 0.000004 | 0.000007 | unchanged |
| get | rillcache | 1272161 | 1260189–1273992 | 119.0 | 100.5 | 0.79 | 73.1 | 6.79 | 0.000001 | 0.000003 | unchanged |
| get | redis | 966510 | 962768–985962 | 192.0 | 100.1 | 1.04 | 63.4 | 8.93 | 0.000005 | 0.000004 | unchanged |
| get | valkey | 885914 | 881822–913318 | 213.7 | 99.6 | 1.13 | 59.6 | 8.48 | 0.000016 | 0.000009 | unchanged |
| set-nx-existing | rillcache | 1202814 | 1192295–1216489 | 125.0 | 99.7 | 0.82 | 68.3 | 6.79 | 0.000001 | 0.000003 | unchanged |
| set-nx-existing | redis | 909275 | 891843–913039 | 206.1 | 99.9 | 1.10 | 60.6 | 8.94 | 0.000007 | 0.000003 | unchanged |
| set-nx-existing | valkey | 881489 | 856537–887909 | 214.6 | 99.6 | 1.13 | 56.2 | 8.47 | 0.000006 | 0.000004 | unchanged |
| set-xx-existing | rillcache | 1102539 | 1069673–1106667 | 138.0 | 99.6 | 0.92 | 64.1 | 6.79 | 0.000001 | 0.000003 | unchanged |
| set-xx-existing | redis | 849738 | 831490–852646 | 223.5 | 100.2 | 1.18 | 52.5 | 8.91 | 0.000003 | 0.000003 | unchanged |
| set-xx-existing | valkey | 752965 | 738226–755188 | 256.2 | 99.6 | 1.33 | 46.2 | 8.47 | 0.000005 | 0.000007 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 5.60 | 5.60 | 99.0 | unchanged |
| redis | 1.000 | 0.000 | 8.68 | 8.68 | 10.0 | unchanged |
| valkey | 1.000 | 0.000 | 8.28 | 8.28 | 10.0 | unchanged |

</details>

<details>
<summary>single-core-pipeline-1: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 100000; clients: 8; pipeline: 1; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: 0; client CPUs: 1,2,3.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 98011 | 97881–98323 | 95.7 | 100.3 | 10.20 | 94.1 | 6.92 | 0.000010 | 0.000030 | unchanged |
| ping | redis | 88993 | 88757–89737 | 114.1 | 100.5 | 11.20 | 91.2 | 8.97 | 0.000140 | 0.000090 | unchanged |
| ping | valkey | 88251 | 88243–88440 | 116.7 | 100.6 | 11.40 | 89.5 | 8.16 | 0.000100 | 0.000050 | unchanged |
| set | rillcache | 95238 | 94621–95332 | 99.3 | 100.0 | 10.50 | 91.4 | 6.92 | 0.000010 | 0.000060 | unchanged |
| set | redis | 86077 | 85929–86270 | 120.7 | 100.0 | 11.60 | 87.1 | 8.99 | 0.000110 | 0.000060 | unchanged |
| set | valkey | 83765 | 83676–83825 | 125.5 | 99.5 | 11.90 | 86.4 | 8.18 | 0.000140 | 0.000070 | unchanged |
| get | rillcache | 96549 | 95395–96934 | 97.3 | 100.4 | 10.40 | 93.3 | 6.92 | 0.000010 | 0.000050 | unchanged |
| get | redis | 87732 | 87506–87763 | 118.2 | 100.0 | 11.40 | 87.8 | 8.97 | 0.000100 | 0.000050 | unchanged |
| get | valkey | 85426 | 85202–85514 | 121.8 | 99.2 | 11.60 | 88.8 | 8.16 | 0.000180 | 0.000050 | unchanged |
| set-nx-existing | rillcache | 95309 | 94123–96428 | 98.2 | 100.1 | 10.50 | 92.4 | 6.92 | 0.000010 | 0.000040 | unchanged |
| set-nx-existing | redis | 86660 | 86397–86934 | 119.8 | 99.6 | 11.50 | 87.0 | 8.97 | 0.000060 | 0.000070 | unchanged |
| set-nx-existing | valkey | 85065 | 84984–85117 | 123.0 | 99.6 | 11.70 | 86.4 | 8.16 | 0.000070 | 0.000070 | unchanged |
| set-xx-existing | rillcache | 94455 | 94217–95634 | 99.6 | 99.8 | 10.60 | 90.6 | 6.92 | 0.000010 | 0.000060 | unchanged |
| set-xx-existing | redis | 85417 | 85159–85847 | 121.7 | 100.5 | 11.80 | 87.3 | 8.97 | 0.000090 | 0.000060 | unchanged |
| set-xx-existing | valkey | 83477 | 83404–83618 | 126.4 | 100.3 | 12.00 | 86.7 | 8.16 | 0.000090 | 0.000050 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 5.55 | 5.55 | 99.0 | unchanged |
| redis | 1.000 | 0.000 | 8.77 | 8.77 | 10.0 | unchanged |
| valkey | 1.000 | 0.000 | 8.02 | 8.02 | 10.0 | unchanged |

</details>

<details>
<summary>single-core-pipeline-16: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 1000000; clients: 8; pipeline: 16; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: 0; client CPUs: 1,2,3.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 1442425 | 1438896–1458119 | 103.6 | 100.7 | 0.70 | 82.9 | 7.25 | 0.000001 | 0.000001 | unchanged |
| ping | redis | 1144562 | 1139446–1145570 | 155.6 | 100.8 | 0.88 | 73.9 | 9.00 | 0.000004 | 0.000002 | unchanged |
| ping | valkey | 1081727 | 1078986–1082352 | 168.8 | 100.6 | 0.93 | 64.5 | 8.40 | 0.000010 | 0.000003 | unchanged |
| set | rillcache | 1088081 | 1083245–1123159 | 135.5 | 100.1 | 0.92 | 65.2 | 7.25 | 0.000001 | 0.000004 | unchanged |
| set | redis | 870944 | 867571–873770 | 218.8 | 100.1 | 1.15 | 51.3 | 9.02 | 0.000006 | 0.000003 | unchanged |
| set | valkey | 760848 | 743473–762057 | 255.0 | 99.6 | 1.32 | 47.0 | 8.41 | 0.000014 | 0.000004 | unchanged |
| get | rillcache | 1247683 | 1246291–1300977 | 118.6 | 100.2 | 0.80 | 74.0 | 7.25 | 0.000001 | 0.000004 | unchanged |
| get | redis | 971190 | 968528–974939 | 190.8 | 100.0 | 1.03 | 62.1 | 9.03 | 0.000002 | 0.000003 | unchanged |
| get | valkey | 881295 | 879289–884307 | 214.8 | 99.9 | 1.13 | 55.3 | 8.41 | 0.000003 | 0.000007 | unchanged |
| set-nx-existing | rillcache | 1196691 | 1160315–1199164 | 124.6 | 99.8 | 0.84 | 69.9 | 7.25 | 0.000001 | 0.000004 | unchanged |
| set-nx-existing | redis | 900638 | 899780–901787 | 210.4 | 99.8 | 1.11 | 58.6 | 9.03 | 0.000008 | 0.000004 | unchanged |
| set-nx-existing | valkey | 858550 | 854734–859915 | 221.9 | 100.0 | 1.17 | 49.6 | 8.40 | 0.000004 | 0.000003 | unchanged |
| set-xx-existing | rillcache | 1091891 | 1080317–1112418 | 137.2 | 100.1 | 0.92 | 61.1 | 7.25 | 0.000001 | 0.000004 | unchanged |
| set-xx-existing | redis | 841265 | 840070–842221 | 227.3 | 100.1 | 1.19 | 61.7 | 9.00 | 0.000004 | 0.000005 | unchanged |
| set-xx-existing | valkey | 740548 | 738720–740655 | 262.3 | 99.9 | 1.35 | 56.3 | 8.40 | 0.000008 | 0.000005 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 5.71 | 5.71 | 99.0 | unchanged |
| redis | 1.000 | 0.000 | 8.78 | 8.78 | 10.0 | unchanged |
| valkey | 1.000 | 0.000 | 8.23 | 8.23 | 10.0 | unchanged |

</details>

### Fixed-worker mode measurements

30/30 separate embedded records validated. 8 clients; 10000 commands/workload/run; pipeline 16; three repetitions. These share the client/server host and inherited CPU affinity; they are **not native competitor comparisons or equal-per-core scaling evidence**. No process CPU/RSS accounting was collected for this group. Fixed lanes have a 1 ms idle polling wait. They actively poll for at most 50 µs after progress before returning to idle parking; this trades active CPU work for fewer premature sleeps, not epoll readiness or free CPU. Multiple lanes also request a cooperative yield after 100 µs of continuous active passes when more than one client is admitted. Progress does not rearm that separate budget; single lanes/lone clients do not periodically yield. This is checked between bounded passes, not a hard deadline or an OS fairness guarantee. The v2 interval restarts after a yield/park returns, excluding that explicit wait; involuntary descheduling can still consume wall time.

| Workload | 1 worker median cmd/s | 1 worker min–max | 4 workers median cmd/s | 4 workers min–max | 1 worker batch p99 µs | 4 workers batch p99 µs |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| ping | 1249218 | 1232620–1308995 | 1939294 | 1796109–2014829 | 940.7 | 874.1 |
| set | 1019961 | 969010–1046238 | 49024 | 48359–49850 | 945.5 | 3619.8 |
| get | 1153856 | 1143646–1187888 | 49816 | 48898–50560 | 957.2 | 4536.1 |
| set-nx-existing | 1142354 | 1091785–1151185 | 49999 | 49835–50171 | 954.0 | 3558.0 |
| set-xx-existing | 988838 | 948530–1084593 | 49763 | 49141–50010 | 957.1 | 3619.1 |

### Interpretation limits

- Batch p99 is **not per-request p99**; never divide it by the pipeline length. Percentiles are not pooled across runs.
- CPU counters are tick-quantized; zero observed idle ticks do not prove zero CPU cost. CPU observation windows bracket, but are not identical to, request timers.
- RSS is an approximate endpoint sample, not allocator accounting or a workload peak; VmHWM includes startup, warmup and previous workloads.
- Context switches cover matched thread lifetimes only. Even unchanged endpoints cannot exclude threads born and destroyed between snapshots. PARTIAL coverage is not a process total.
- NX-existing measures rejected writes; XX-existing measures admitted overwrites. This is a small Redis-compatible subset, not equivalent functionality.
