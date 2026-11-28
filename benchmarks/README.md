# Current benchmark performance

Generated: 2026-11-28T10:15:30Z · [Benchmark run #21](https://github.com/macel94/rillcache/actions/runs/46132823663) · [Tested source `2b1eb74`](https://github.com/macel94/rillcache/commit/2b1eb740ac7cd360f837e1d97831af1a0dff8f4c)

**Benchmark job: success. Data: validated. Copilot review: completed.**

Benchmark attempt: 1; review attempt: 1. Preferred model: `gpt-6-luna` via **GitHub Copilot**.

[This report's permanent copy](https://github.com/macel94/rillcache/blob/main/benchmarks/reports/46132823663-1.md) · [Report history](https://github.com/macel94/rillcache/tree/main/benchmarks/reports) · [Methodology](https://github.com/macel94/rillcache/blob/main/docs/benchmarks.md)

## AI review

Actual model(s): **gpt-6-luna**. Selection: automatic-fallback. Authentication: workflow-token.

AI interpretation, **not measurements or a performance gate**. Verify claims against the deterministic tables below.

> Bottom line: In this validated, narrow SET workload, Rillcache has higher median throughput and
> lower whole-batch p99 than Redis and Valkey in both default-threading and equal-one-logical-CPU
> profiles. This is not a general Redis/Valkey performance verdict.
> 
> Equal-CPU tradeoff: With one server logical CPU, pipeline-1 SET reached 208,899 commands/s versus
> Redis’s 178,170 and Valkey’s 168,501; whole-batch p99 was 53.7 µs versus 71.1 and 76.4 µs. At
> pipeline-16, rates were 1.762M versus 1.286M and 1.125M commands/s; batch p99 was 91.1 µs versus
> 155.5 and 176.9 µs. These are batch percentiles, not per-request latency; do not divide by pipeline
> size. Default-threading shows a similar pattern, but is a separate CPU-budget comparison.
> 
> CPU, memory, scheduling: Servers were near one logical CPU of measured use in these native runs;
> Rillcache SET used 4.8 CPU µs/command at pipeline-1 and 0.57 at pipeline-16 in the equal-CPU
> profiles. CPU windows only bracket request timing, and tick-quantized zero idle ticks would not
> prove zero cost. Approximate workload RSS endpoints were about 7.0 MiB for Rillcache, 9.0 for Redis,
> and 8.3–8.4 for Valkey; these are not peaks or allocator accounting. Matched-lifetime voluntary
> switches per command were lower for Rillcache, but are not guaranteed process totals. Idle CPU reads
> were 0.000% over one-second windows; lifetime peaks were close to endpoint RSS and include prior
> activity.
> 
> The separate fixed-worker cohort warrants caution: with four workers, SET was 87,883 commands/s
> versus 1.477M with one, and batch p99 was 3,530.9 versus 1,001.1 µs. This is not a native-competitor
> or equal-per-core comparison, and counters were not collected; the measurements alone do not
> establish why.
> 
> Next investigations: (1) Repeat fixed-worker measurements with per-lane progress, queueing, CPU, and
> scheduling telemetry to localize the four-worker SET/GET collapse. (2) Expand native equal-CPU
> tests across key cardinalities, value sizes, and mixed workloads to check whether the SET result
> generalizes beyond 1,024 keys and 64-byte values.

## Measurements

**Data quality: complete.** 180/180 records validated across 4/4 complete profiles.


Hardware: Intel(R) Xeon(R) 6975P-C. Rust 1.100.0 (0ef1bc386 2026-11-09).

References: Redis 8.10.2 sha=00000000:0 malloc=jemalloc-5.3.0 bits=64 build=3392ba6431793ae; Valkey 9.1.2 sha=00000000:0 malloc=jemalloc-5.3.0 bits=64 build=cc1fd8cf3dd82f50.

### SET at a glance

Median commands/s and median of the three runs' **whole-batch p99** values (µs). Positive rate delta means Rillcache is faster in this workload only.

| Profile | Rillcache cmd/s | Redis | Valkey | Rillcache vs Redis | Rillcache p99 µs | Redis p99 | Valkey p99 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| pipeline-1 | 212744 | 178771 | 168640 | +19.0% | 52.9 | 70.8 | 75.9 |
| pipeline-16 | 1784940 | 1277318 | 1124177 | +39.7% | 88.7 | 156.7 | 178.0 |
| single-core-pipeline-1 | 208899 | 178170 | 168501 | +17.2% | 53.7 | 71.1 | 76.4 |
| single-core-pipeline-16 | 1761556 | 1286384 | 1125149 | +36.9% | 91.1 | 155.5 | 176.9 |

CPU is relative to **one logical CPU**, so parallel servers can exceed 100%. Do not blend default-threading and single-core results or different pipelines. Shared VMs and unequal feature sets do not establish a general performance win or a cross-run regression.

<details>
<summary>pipeline-1: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 100000; clients: 8; pipeline: 1; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: inherited; client CPUs: inherited.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 229809 | 228171–232807 | 48.2 | 100.4 | 4.40 | 93.2 | 7.00 | 0.000010 | 0.000010 | unchanged |
| ping | redis | 191902 | 178127–193316 | 65.7 | 99.7 | 5.20 | 88.5 | 8.95 | 0.000560 | 0.000070 | unchanged |
| ping | valkey | 185125 | 183498–186431 | 68.0 | 99.0 | 5.40 | 84.9 | 8.27 | 0.000420 | 0.000130 | unchanged |
| set | rillcache | 212744 | 208362–212961 | 52.9 | 97.9 | 4.70 | 87.2 | 7.00 | 0.000010 | 0.000070 | unchanged |
| set | redis | 178771 | 177935–179531 | 70.8 | 99.6 | 5.60 | 81.8 | 8.95 | 0.000270 | 0.000140 | unchanged |
| set | valkey | 168640 | 168595–169343 | 75.9 | 99.5 | 5.90 | 81.4 | 8.27 | 0.000310 | 0.000080 | unchanged |
| get | rillcache | 220688 | 220385–224731 | 50.7 | 99.3 | 4.50 | 88.9 | 7.00 | 0.000010 | 0.000040 | unchanged |
| get | redis | 185801 | 185289–185820 | 68.2 | 100.0 | 5.40 | 86.8 | 8.95 | 0.000120 | 0.000090 | unchanged |
| get | valkey | 175298 | 174908–175796 | 72.8 | 99.7 | 5.70 | 82.5 | 8.27 | 0.000150 | 0.000040 | unchanged |
| set-nx-existing | rillcache | 216294 | 215569–220550 | 51.9 | 99.1 | 4.60 | 91.1 | 7.00 | 0.000010 | 0.000030 | unchanged |
| set-nx-existing | redis | 179775 | 179465–180885 | 70.2 | 99.5 | 5.50 | 81.8 | 8.95 | 0.000380 | 0.000040 | unchanged |
| set-nx-existing | valkey | 172221 | 172202–172654 | 74.1 | 99.9 | 5.80 | 79.3 | 8.27 | 0.000270 | 0.000070 | unchanged |
| set-xx-existing | rillcache | 209020 | 202128–209856 | 54.0 | 100.7 | 4.80 | 86.0 | 7.00 | 0.000010 | 0.000040 | unchanged |
| set-xx-existing | redis | 176532 | 157643–177035 | 71.9 | 99.3 | 5.70 | 82.5 | 8.95 | 0.000470 | 0.000040 | unchanged |
| set-xx-existing | valkey | 167359 | 167318–168019 | 76.4 | 99.1 | 5.90 | 78.0 | 8.27 | 0.000310 | 0.000100 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 5.62 | 5.62 | 99.0 | unchanged |
| redis | 1.000 | 0.000 | 8.70 | 8.70 | 10.0 | unchanged |
| valkey | 1.000 | 0.000 | 8.08 | 8.08 | 10.0 | unchanged |

</details>

<details>
<summary>pipeline-16: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 1000000; clients: 8; pipeline: 16; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: inherited; client CPUs: inherited.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 2804698 | 2741653–2854842 | 60.8 | 98.7 | 0.36 | 71.4 | 6.84 | 0.000001 | 0.000005 | unchanged |
| ping | redis | 1911620 | 1911056–1924504 | 103.6 | 99.3 | 0.52 | 56.6 | 9.00 | 0.000023 | 0.000003 | unchanged |
| ping | valkey | 1734529 | 1733083–1735787 | 114.4 | 100.6 | 0.58 | 52.5 | 8.30 | 0.000016 | 0.000005 | unchanged |
| set | rillcache | 1784940 | 1779301–1800647 | 88.7 | 99.6 | 0.56 | 43.6 | 6.84 | 0.000003 | 0.000005 | unchanged |
| set | redis | 1277318 | 1276813–1282042 | 156.7 | 99.6 | 0.78 | 39.7 | 9.00 | 0.000009 | 0.000072 | unchanged |
| set | valkey | 1124177 | 1123840–1151258 | 178.0 | 100.0 | 0.89 | 37.1 | 8.30 | 0.000013 | 0.000005 | unchanged |
| get | rillcache | 2133766 | 2114960–2146884 | 77.7 | 100.3 | 0.47 | 61.7 | 6.84 | 0.000001 | 0.000002 | unchanged |
| get | redis | 1451698 | 1446362–1465168 | 139.4 | 99.6 | 0.68 | 46.4 | 9.01 | 0.000007 | 0.000004 | unchanged |
| get | valkey | 1328071 | 1321776–1328142 | 153.5 | 100.9 | 0.76 | 44.7 | 8.30 | 0.000031 | 0.000004 | unchanged |
| set-nx-existing | rillcache | 2010220 | 2002579–2030718 | 80.2 | 100.5 | 0.50 | 52.9 | 6.84 | 0.000002 | 0.000003 | unchanged |
| set-nx-existing | redis | 1322002 | 1317307–1334891 | 151.7 | 98.8 | 0.75 | 44.0 | 9.00 | 0.000025 | 0.000003 | unchanged |
| set-nx-existing | valkey | 1268294 | 1263835–1276193 | 158.2 | 99.5 | 0.78 | 38.0 | 8.30 | 0.000011 | 0.000003 | unchanged |
| set-xx-existing | rillcache | 1784498 | 1769222–1790969 | 88.4 | 100.8 | 0.57 | 38.6 | 6.84 | 0.000001 | 0.000002 | unchanged |
| set-xx-existing | redis | 1222194 | 1221386–1230555 | 163.4 | 99.0 | 0.81 | 38.3 | 9.00 | 0.000018 | 0.000004 | unchanged |
| set-xx-existing | valkey | 1097008 | 1090691–1097651 | 183.7 | 99.8 | 0.91 | 34.1 | 8.30 | 0.000011 | 0.000003 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 5.45 | 5.45 | 99.0 | unchanged |
| redis | 1.000 | 0.000 | 8.75 | 8.75 | 10.0 | unchanged |
| valkey | 1.000 | 0.000 | 8.09 | 8.09 | 10.0 | unchanged |

</details>

<details>
<summary>single-core-pipeline-1: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 100000; clients: 8; pipeline: 1; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: 0; client CPUs: 1,2,3.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 226479 | 225743–228294 | 50.1 | 99.6 | 4.40 | 90.9 | 7.04 | 0.000010 | 0.000020 | unchanged |
| ping | redis | 191119 | 190274–191191 | 66.2 | 100.8 | 5.30 | 86.8 | 8.96 | 0.000250 | 0.000050 | unchanged |
| ping | valkey | 186711 | 186383–186952 | 68.0 | 100.8 | 5.40 | 85.2 | 8.32 | 0.000150 | 0.000020 | unchanged |
| set | rillcache | 208899 | 208668–208966 | 53.7 | 100.2 | 4.80 | 85.7 | 7.04 | 0.000010 | 0.000010 | unchanged |
| set | redis | 178170 | 178032–178908 | 71.1 | 99.7 | 5.60 | 82.5 | 8.96 | 0.000230 | 0.000030 | unchanged |
| set | valkey | 168501 | 168055–169098 | 76.4 | 99.7 | 5.90 | 78.0 | 8.32 | 0.000640 | 0.000040 | unchanged |
| get | rillcache | 217866 | 214919–218357 | 52.1 | 98.8 | 4.60 | 89.1 | 7.04 | 0.000010 | 0.000020 | unchanged |
| get | redis | 185071 | 183462–185540 | 68.0 | 100.9 | 5.50 | 83.6 | 8.97 | 0.000240 | 0.000030 | unchanged |
| get | valkey | 174444 | 174206–174790 | 74.2 | 101.1 | 5.80 | 79.3 | 8.32 | 0.000280 | 0.000020 | unchanged |
| set-nx-existing | rillcache | 214611 | 212929–215025 | 52.3 | 98.9 | 4.60 | 87.0 | 7.04 | 0.000010 | 0.000010 | unchanged |
| set-nx-existing | redis | 179712 | 179239–180108 | 70.2 | 98.8 | 5.50 | 80.4 | 8.97 | 0.000340 | 0.000030 | unchanged |
| set-nx-existing | valkey | 172395 | 172227–173024 | 74.6 | 99.9 | 5.80 | 79.3 | 8.32 | 0.000340 | 0.000040 | unchanged |
| set-xx-existing | rillcache | 207060 | 207036–207985 | 54.2 | 99.8 | 4.80 | 87.5 | 7.04 | 0.000010 | 0.000020 | unchanged |
| set-xx-existing | redis | 176645 | 175738–176999 | 72.1 | 100.1 | 5.70 | 80.7 | 8.96 | 0.000140 | 0.000030 | unchanged |
| set-xx-existing | valkey | 167315 | 167116–167511 | 77.2 | 100.2 | 6.00 | 78.0 | 8.32 | 0.000240 | 0.000010 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 5.53 | 5.53 | 99.0 | unchanged |
| redis | 1.000 | 0.000 | 8.71 | 8.71 | 10.0 | unchanged |
| valkey | 1.000 | 0.000 | 8.13 | 8.13 | 10.0 | unchanged |

</details>

<details>
<summary>single-core-pipeline-16: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 1000000; clients: 8; pipeline: 16; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: 0; client CPUs: 1,2,3.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 2784482 | 2750087–2786926 | 62.1 | 100.3 | 0.36 | 71.4 | 7.05 | 0.000001 | 0.000001 | unchanged |
| ping | redis | 1903698 | 1882135–1918434 | 104.5 | 99.7 | 0.53 | 58.5 | 8.82 | 0.000018 | 0.000001 | unchanged |
| ping | valkey | 1722215 | 1721271–1722782 | 115.7 | 99.9 | 0.58 | 53.4 | 8.40 | 0.000021 | 0.000005 | unchanged |
| set | rillcache | 1761556 | 1739626–1762004 | 91.1 | 100.4 | 0.57 | 43.1 | 7.05 | 0.000001 | 0.000001 | unchanged |
| set | redis | 1286384 | 1261572–1289292 | 155.5 | 99.3 | 0.77 | 40.3 | 8.82 | 0.000026 | 0.000002 | unchanged |
| set | valkey | 1125149 | 1123575–1128059 | 176.9 | 100.0 | 0.89 | 39.3 | 8.40 | 0.000007 | 0.000001 | unchanged |
| get | rillcache | 2101919 | 2087648–2117137 | 79.8 | 100.9 | 0.48 | 60.4 | 7.05 | 0.000001 | 0.000002 | unchanged |
| get | redis | 1440793 | 1428000–1453618 | 140.4 | 99.9 | 0.70 | 47.1 | 8.83 | 0.000023 | 0.000001 | unchanged |
| get | valkey | 1323576 | 1319841–1325799 | 153.2 | 100.3 | 0.76 | 44.0 | 8.40 | 0.000007 | 0.000004 | unchanged |
| set-nx-existing | rillcache | 1985009 | 1965872–1991232 | 82.2 | 99.2 | 0.50 | 56.0 | 7.05 | 0.000001 | 0.000001 | unchanged |
| set-nx-existing | redis | 1319853 | 1305063–1325834 | 150.6 | 100.5 | 0.76 | 46.1 | 8.82 | 0.000019 | 0.000004 | unchanged |
| set-nx-existing | valkey | 1266205 | 1260890–1270206 | 158.0 | 100.0 | 0.79 | 40.5 | 8.40 | 0.000010 | 0.000002 | unchanged |
| set-xx-existing | rillcache | 1730043 | 1727404–1730988 | 92.8 | 100.4 | 0.58 | 44.8 | 7.05 | 0.000001 | 0.000002 | unchanged |
| set-xx-existing | redis | 1228630 | 1223760–1236087 | 162.0 | 99.5 | 0.81 | 39.5 | 8.82 | 0.000034 | 0.000002 | unchanged |
| set-xx-existing | valkey | 1091967 | 1086590–1093800 | 182.7 | 99.9 | 0.92 | 34.8 | 8.40 | 0.000022 | 0.000003 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 5.70 | 5.70 | 99.0 | unchanged |
| redis | 1.000 | 0.000 | 8.57 | 8.57 | 10.0 | unchanged |
| valkey | 1.000 | 0.000 | 8.21 | 8.21 | 10.0 | unchanged |

</details>

### Fixed-worker mode measurements

30/30 separate embedded records validated. 8 clients; 10000 commands/workload/run; pipeline 16; three repetitions. These share the client/server host and inherited CPU affinity; they are **not native competitor comparisons or equal-per-core scaling evidence**. No process CPU/RSS accounting was collected for this group. Fixed lanes have a 1 ms idle polling wait. They actively poll for at most 50 µs after progress before returning to idle parking; this trades active CPU work for fewer premature sleeps, not epoll readiness or free CPU. Multiple lanes also request a cooperative yield after 100 µs of continuous active passes when more than one client is admitted. Progress does not rearm that separate budget; single lanes/lone clients do not periodically yield. This is checked between bounded passes, not a hard deadline or an OS fairness guarantee. The v2 interval restarts after a yield/park returns, excluding that explicit wait; involuntary descheduling can still consume wall time.

| Workload | 1 worker median cmd/s | 1 worker min–max | 4 workers median cmd/s | 4 workers min–max | 1 worker batch p99 µs | 4 workers batch p99 µs |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| ping | 2083945 | 2069413–2137883 | 2195751 | 2098759–2297467 | 971.7 | 1025.4 |
| set | 1476615 | 1350202–1489376 | 87883 | 82079–90394 | 1001.1 | 3530.9 |
| get | 1704910 | 1692416–1733858 | 87763 | 83807–89002 | 1013.7 | 3462.2 |
| set-nx-existing | 1606352 | 1330106–1609832 | 96120 | 93960–97322 | 996.4 | 3647.9 |
| set-xx-existing | 1439288 | 1437488–1446882 | 94400 | 92321–95684 | 1004.3 | 3510.2 |

### Interpretation limits

- Batch p99 is **not per-request p99**; never divide it by the pipeline length. Percentiles are not pooled across runs.
- CPU counters are tick-quantized; zero observed idle ticks do not prove zero CPU cost. CPU observation windows bracket, but are not identical to, request timers.
- RSS is an approximate endpoint sample, not allocator accounting or a workload peak; VmHWM includes startup, warmup and previous workloads.
- Context switches cover matched thread lifetimes only. Even unchanged endpoints cannot exclude threads born and destroyed between snapshots. PARTIAL coverage is not a process total.
- NX-existing measures rejected writes; XX-existing measures admitted overwrites. This is a small Redis-compatible subset, not equivalent functionality.
