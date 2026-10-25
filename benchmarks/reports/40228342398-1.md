# Current benchmark performance

Generated: 2026-10-25T20:05:31Z · [Benchmark run #8](https://github.com/macel94/rillcache/actions/runs/40228342398) · [Tested source `1f939a3`](https://github.com/macel94/rillcache/commit/1f939a352a5cbad624e7619f52659142f225b318)

**Benchmark job: success. Data: validated. Copilot review: completed.**

Benchmark attempt: 1; review attempt: 1. Preferred model: `gpt-6-luna` via **GitHub Copilot**.

[This report's permanent copy](https://github.com/macel94/rillcache/blob/main/benchmarks/reports/40228342398-1.md) · [Report history](https://github.com/macel94/rillcache/tree/main/benchmarks/reports) · [Methodology](https://github.com/macel94/rillcache/blob/main/docs/benchmarks.md)

## AI review

Actual model(s): **gpt-6-luna**. Selection: automatic-fallback. Authentication: workflow-token.

AI interpretation, **not measurements or a performance gate**. Verify claims against the deterministic tables below.

> Bottom line: In this Redis-compatible subset, Rillcache’s results depend on CPU budget and pipeline
> depth. They support workload-specific comparisons, not a general Redis or Valkey performance claim.
> 
> With inherited CPU affinity, median SET throughput was 133,536 commands/s at pipeline 1 and
> 1,653,329 at pipeline 16, versus Redis’s 58,369 and 590,512. Rillcache’s corresponding whole-batch
> p99s were 151.6 and 197.3 µs (Redis: 189.7 and 311.2 µs). These default-threading results use more
> than one logical CPU: Rillcache measured 198.8% CPU at pipeline 1 and 227.9% at pipeline 16.
> 
> Under a one-logical-CPU server budget, pipeline-1 SET throughput was lower for Rillcache (53,006 vs.
> Redis’s 58,090 commands/s), with similar whole-batch p99 (191.9 vs. 190.3 µs). At pipeline 16,
> Rillcache led on throughput (687,569 vs. 597,550), but its whole-batch p99 was higher (375.9 vs.
> 307.3 µs). Do not divide these batch p99s by pipeline depth to infer per-request latency.
> 
> Rillcache’s sampled RSS was about 3 MiB versus roughly 13 MiB for Redis and Valkey; these are
> approximate endpoints, not workload peaks. CPU is measured over an observation window, and zero idle
> ticks are quantized—not proof of zero cost. Rillcache also recorded more voluntary switches in some
> workloads (pipeline-16 SET: 0.055048/cmd vs. Redis’s 0.000007); counts cover matched thread
> lifetimes, not guaranteed process totals, and do not establish a cause. System-CPU shares also
> differed.
> 
> Uncertainty: The 180 native records passed validation, but this is a small subset, with three
> repetitions on a shared VM and unequal feature sets. Fixed-worker measurements are a separate group,
> not native or equal-per-core comparisons.
> 
> Next investigations:
> 
>  1. Repeat the equal-one-CPU pipeline-1 and pipeline-16 profiles with more repetitions and report
> run-to-run distributions, focusing on the pipeline-16 tail/throughput tradeoff.
>  2. Profile CPU and scheduling for equal-CPU runs, especially Rillcache’s higher switch counts and
> system-CPU share, to identify where measured time is spent without presuming causation.

## Measurements

**Data quality: complete.** 180/180 records validated across 4/4 complete profiles.


Hardware: AMD EPYC 7763 64-Core Processor. Rust 1.99.1 (58cf8f9c3 2026-10-13).

References: Redis 8.10.2 sha=00000000:0 malloc=jemalloc-5.3.0 bits=64 build=b3e81b0da4592727; Valkey 9.1.2 sha=00000000:0 malloc=jemalloc-5.3.0 bits=64 build=994aa6c4a01270e5.

### SET at a glance

Median commands/s and median of the three runs' **whole-batch p99** values (µs). Positive rate delta means Rillcache is faster in this workload only.

| Profile | Rillcache cmd/s | Redis | Valkey | Rillcache vs Redis | Rillcache p99 µs | Redis p99 | Valkey p99 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| pipeline-1 | 133536 | 58369 | 56539 | +128.8% | 151.6 | 189.7 | 196.6 |
| pipeline-16 | 1653329 | 590512 | 537040 | +180.0% | 197.3 | 311.2 | 344.1 |
| single-core-pipeline-1 | 53006 | 58090 | 57030 | -8.8% | 191.9 | 190.3 | 195.2 |
| single-core-pipeline-16 | 687569 | 597550 | 539862 | +15.1% | 375.9 | 307.3 | 341.1 |

CPU is relative to **one logical CPU**, so parallel servers can exceed 100%. Do not blend default-threading and single-core results or different pipelines. Shared VMs and unequal feature sets do not establish a general performance win or a cross-run regression.

<details>
<summary>pipeline-1: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 100000; clients: 8; pipeline: 1; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: inherited; client CPUs: inherited.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 133513 | 124075–134023 | 145.0 | 192.8 | 14.50 | 95.9 | 3.01 | 0.821010 | 0.193730 | unchanged |
| ping | redis | 62097 | 61788–62181 | 177.6 | 100.1 | 16.10 | 87.0 | 12.68 | 0.000110 | 0.000130 | unchanged |
| ping | valkey | 60442 | 60256–60569 | 182.0 | 100.3 | 16.60 | 86.8 | 12.70 | 0.000110 | 0.000080 | unchanged |
| set | rillcache | 133536 | 128116–134497 | 151.6 | 198.8 | 14.90 | 92.6 | 3.01 | 0.858400 | 0.154260 | unchanged |
| set | redis | 58369 | 57828–58401 | 189.7 | 100.0 | 17.20 | 81.4 | 12.68 | 0.000050 | 0.000070 | unchanged |
| set | valkey | 56539 | 56416–56706 | 196.6 | 99.8 | 17.60 | 77.8 | 12.70 | 0.000040 | 0.000060 | unchanged |
| get | rillcache | 120350 | 118455–134452 | 165.2 | 181.1 | 15.00 | 93.9 | 3.01 | 0.852760 | 0.162300 | unchanged |
| get | redis | 59655 | 59623–60077 | 184.4 | 99.6 | 16.70 | 86.2 | 12.68 | 0.000170 | 0.000070 | unchanged |
| get | valkey | 58363 | 58212–58468 | 189.8 | 99.8 | 17.10 | 80.7 | 12.70 | 0.000050 | 0.000080 | unchanged |
| set-nx-existing | rillcache | 124452 | 116191–127953 | 147.3 | 189.0 | 15.20 | 93.4 | 3.01 | 0.814210 | 0.200960 | unchanged |
| set-nx-existing | redis | 59324 | 58657–59447 | 186.2 | 99.8 | 16.80 | 82.7 | 12.68 | 0.000090 | 0.000060 | unchanged |
| set-nx-existing | valkey | 57999 | 57966–58195 | 192.6 | 100.3 | 17.30 | 80.9 | 12.70 | 0.000100 | 0.000080 | unchanged |
| set-xx-existing | rillcache | 126632 | 122519–130024 | 156.5 | 191.1 | 15.10 | 91.9 | 3.01 | 0.820050 | 0.194850 | unchanged |
| set-xx-existing | redis | 57894 | 57744–58344 | 191.7 | 99.9 | 17.30 | 80.3 | 12.66 | 0.000050 | 0.000060 | unchanged |
| set-xx-existing | valkey | 56622 | 56577–56661 | 196.7 | 99.7 | 17.60 | 78.5 | 12.63 | 0.000050 | 0.000080 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 2.43 | 2.43 | 936.8 | unchanged |
| redis | 1.001 | 0.000 | 13.20 | 13.20 | 10.0 | unchanged |
| valkey | 1.001 | 0.000 | 12.82 | 12.82 | 10.0 | unchanged |

</details>

<details>
<summary>pipeline-16: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 1000000; clients: 8; pipeline: 16; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: inherited; client CPUs: inherited.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 2025832 | 1847513–2066843 | 144.8 | 190.2 | 0.94 | 91.1 | 3.07 | 0.054546 | 0.008487 | unchanged |
| ping | redis | 789485 | 788578–793458 | 226.5 | 99.4 | 1.26 | 68.8 | 13.48 | 0.000004 | 0.000004 | unchanged |
| ping | valkey | 741718 | 739891–746168 | 243.3 | 100.0 | 1.35 | 62.2 | 12.86 | 0.000006 | 0.000005 | unchanged |
| set | rillcache | 1653329 | 1540850–1677510 | 197.3 | 227.9 | 1.38 | 72.8 | 3.07 | 0.055048 | 0.009019 | unchanged |
| set | redis | 590512 | 590186–591145 | 311.2 | 99.8 | 1.69 | 56.8 | 13.02 | 0.000007 | 0.000008 | unchanged |
| set | valkey | 537040 | 533967–537127 | 344.1 | 100.4 | 1.87 | 47.8 | 12.81 | 0.000008 | 0.000005 | unchanged |
| get | rillcache | 1936606 | 1929433–1946965 | 154.0 | 202.3 | 1.05 | 83.5 | 3.07 | 0.045229 | 0.018468 | unchanged |
| get | redis | 662272 | 661852–665859 | 273.8 | 99.9 | 1.51 | 60.9 | 13.02 | 0.000010 | 0.000004 | unchanged |
| get | valkey | 606468 | 602863–610988 | 301.2 | 100.0 | 1.64 | 54.9 | 12.81 | 0.000005 | 0.000007 | unchanged |
| set-nx-existing | rillcache | 1789074 | 1779467–1804489 | 159.1 | 212.7 | 1.19 | 77.3 | 3.07 | 0.047988 | 0.015630 | unchanged |
| set-nx-existing | redis | 619157 | 616396–619555 | 296.1 | 99.8 | 1.62 | 54.3 | 13.02 | 0.000005 | 0.000007 | unchanged |
| set-nx-existing | valkey | 589965 | 585699–592222 | 311.1 | 100.1 | 1.70 | 55.9 | 12.81 | 0.000002 | 0.000006 | unchanged |
| set-xx-existing | rillcache | 1672024 | 1663873–1686289 | 170.4 | 229.4 | 1.38 | 70.3 | 3.07 | 0.055901 | 0.008316 | unchanged |
| set-xx-existing | redis | 571568 | 569548–573086 | 321.8 | 100.3 | 1.75 | 57.1 | 13.02 | 0.000002 | 0.000009 | unchanged |
| set-xx-existing | valkey | 522877 | 522583–525072 | 354.1 | 99.8 | 1.91 | 47.1 | 12.81 | 0.000002 | 0.000006 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 1.000 | 2.42 | 2.42 | 934.7 | unchanged |
| redis | 1.001 | 0.000 | 13.45 | 13.45 | 10.0 | unchanged |
| valkey | 1.001 | 0.000 | 12.86 | 12.86 | 10.0 | unchanged |

</details>

<details>
<summary>single-core-pipeline-1: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 100000; clients: 8; pipeline: 1; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: 0; client CPUs: 1,2,3.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 55740 | 55464–55939 | 183.3 | 99.7 | 17.90 | 94.4 | 2.99 | 0.016920 | 1.003360 | unchanged |
| ping | redis | 62097 | 62082–62135 | 176.5 | 100.0 | 16.10 | 85.7 | 12.61 | 0.000060 | 0.000050 | unchanged |
| ping | valkey | 60463 | 60327–60627 | 182.2 | 100.1 | 16.60 | 86.7 | 12.74 | 0.000040 | 0.000060 | unchanged |
| set | rillcache | 53006 | 52476–53401 | 191.9 | 99.7 | 18.80 | 91.6 | 2.99 | 0.018080 | 1.001370 | unchanged |
| set | redis | 58090 | 58000–58240 | 190.3 | 100.1 | 17.20 | 79.8 | 12.61 | 0.000040 | 0.000060 | unchanged |
| set | valkey | 57030 | 56928–57039 | 195.2 | 99.8 | 17.50 | 78.9 | 12.74 | 0.000140 | 0.000050 | unchanged |
| get | rillcache | 53965 | 53556–54280 | 187.2 | 99.8 | 18.50 | 93.0 | 2.99 | 0.017440 | 1.003460 | unchanged |
| get | redis | 59740 | 59735–59753 | 184.3 | 100.3 | 16.80 | 85.1 | 12.61 | 0.000080 | 0.000040 | unchanged |
| get | valkey | 58431 | 58262–58432 | 188.9 | 100.5 | 17.20 | 81.3 | 12.74 | 0.000090 | 0.000080 | unchanged |
| set-nx-existing | rillcache | 52868 | 52723–53305 | 193.9 | 99.6 | 18.90 | 93.1 | 2.99 | 0.018670 | 1.004560 | unchanged |
| set-nx-existing | redis | 58770 | 58764–58976 | 187.8 | 99.9 | 17.00 | 82.4 | 12.61 | 0.000080 | 0.000040 | unchanged |
| set-nx-existing | valkey | 58229 | 58116–58255 | 189.7 | 100.1 | 17.20 | 81.4 | 12.74 | 0.000110 | 0.000050 | unchanged |
| set-xx-existing | rillcache | 52596 | 51407–53516 | 191.9 | 99.7 | 19.00 | 92.5 | 2.99 | 0.017740 | 1.004590 | unchanged |
| set-xx-existing | redis | 57867 | 57734–57886 | 190.9 | 100.1 | 17.30 | 80.3 | 12.59 | 0.000060 | 0.000030 | unchanged |
| set-xx-existing | valkey | 56656 | 56596–56831 | 195.5 | 100.0 | 17.60 | 78.0 | 12.68 | 0.000050 | 0.000070 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 1.000 | 2.41 | 2.41 | 934.8 | unchanged |
| redis | 1.001 | 0.000 | 13.13 | 13.13 | 10.0 | unchanged |
| valkey | 1.001 | 0.000 | 12.85 | 12.85 | 10.0 | unchanged |

</details>

<details>
<summary>single-core-pipeline-16: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 1000000; clients: 8; pipeline: 16; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: 0; client CPUs: 1,2,3.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 789463 | 785837–790859 | 326.3 | 99.6 | 1.26 | 92.1 | 3.07 | 0.063015 | 0.000626 | unchanged |
| ping | redis | 797868 | 794846–799289 | 224.6 | 99.9 | 1.25 | 69.3 | 13.31 | 0.000004 | 0.000002 | unchanged |
| ping | valkey | 746747 | 745773–746901 | 240.0 | 100.0 | 1.34 | 63.4 | 12.86 | 0.000003 | 0.000003 | unchanged |
| set | rillcache | 687569 | 687544–690384 | 375.9 | 100.1 | 1.45 | 80.0 | 3.07 | 0.063204 | 0.000899 | unchanged |
| set | redis | 597550 | 595947–599884 | 307.3 | 99.8 | 1.67 | 53.9 | 12.90 | 0.000002 | 0.000007 | unchanged |
| set | valkey | 539862 | 539558–540270 | 341.1 | 99.9 | 1.85 | 46.5 | 12.68 | 0.000006 | 0.000006 | unchanged |
| get | rillcache | 720559 | 719755–727054 | 351.1 | 100.0 | 1.39 | 85.6 | 3.07 | 0.062990 | 0.000800 | unchanged |
| get | redis | 668627 | 666080–670179 | 272.6 | 99.9 | 1.50 | 59.3 | 12.90 | 0.000002 | 0.000007 | unchanged |
| get | valkey | 612212 | 606647–613212 | 299.6 | 99.8 | 1.63 | 54.9 | 12.69 | 0.000011 | 0.000008 | unchanged |
| set-nx-existing | rillcache | 705730 | 701486–709622 | 363.9 | 99.5 | 1.41 | 82.5 | 3.07 | 0.063141 | 0.000822 | unchanged |
| set-nx-existing | redis | 623009 | 618945–626230 | 293.2 | 99.7 | 1.60 | 54.4 | 12.90 | 0.000006 | 0.000008 | unchanged |
| set-nx-existing | valkey | 595202 | 591716–596345 | 308.7 | 100.0 | 1.68 | 54.5 | 12.69 | 0.000003 | 0.000006 | unchanged |
| set-xx-existing | rillcache | 678988 | 678249–684234 | 378.2 | 99.8 | 1.47 | 78.9 | 3.07 | 0.063196 | 0.000897 | unchanged |
| set-xx-existing | redis | 577609 | 577522–579175 | 318.4 | 99.9 | 1.73 | 57.8 | 12.90 | 0.000005 | 0.000006 | unchanged |
| set-xx-existing | valkey | 526444 | 526342–527331 | 351.0 | 100.0 | 1.90 | 47.4 | 12.70 | 0.000010 | 0.000005 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 1.000 | 2.43 | 2.43 | 936.7 | unchanged |
| redis | 1.001 | 0.000 | 13.28 | 13.28 | 10.0 | unchanged |
| valkey | 1.001 | 0.000 | 12.86 | 12.86 | 10.0 | unchanged |

</details>

### Fixed-worker mode measurements

30/30 separate embedded records validated. 8 clients; 10000 commands/workload/run; pipeline 16; three repetitions. These share the client/server host and inherited CPU affinity; they are **not native competitor comparisons or equal-per-core scaling evidence**. No process CPU/RSS accounting was collected for this group. Fixed lanes have a 1 ms idle polling wait. They actively poll for at most 50 µs after progress before returning to idle parking; this trades active CPU work for fewer premature sleeps, not epoll readiness or free CPU. Multiple lanes also request a cooperative yield after 100 µs of continuous active passes when more than one client is admitted. Progress does not rearm that separate budget; single lanes/lone clients do not periodically yield. This is checked between bounded passes, not a hard deadline or an OS fairness guarantee.

| Workload | 1 worker median cmd/s | 1 worker min–max | 4 workers median cmd/s | 4 workers min–max | 1 worker batch p99 µs | 4 workers batch p99 µs |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| ping | 994463 | 993075–1000050 | 1217987 | 1181508–1451295 | 958.2 | 1099.6 |
| set | 784560 | 775796–832847 | 1101615 | 1075531–1151950 | 941.5 | 1091.8 |
| get | 865875 | 846707–877389 | 1421741 | 1086791–1431504 | 945.4 | 1053.6 |
| set-nx-existing | 875034 | 834592–925625 | 1205915 | 1174209–1238052 | 956.3 | 1130.5 |
| set-xx-existing | 844801 | 806502–873545 | 1188469 | 1154965–1204215 | 948.0 | 1013.3 |

### Interpretation limits

- Batch p99 is **not per-request p99**; never divide it by the pipeline length. Percentiles are not pooled across runs.
- CPU counters are tick-quantized; zero observed idle ticks do not prove zero CPU cost. CPU observation windows bracket, but are not identical to, request timers.
- RSS is an approximate endpoint sample, not allocator accounting or a workload peak; VmHWM includes startup, warmup and previous workloads.
- Context switches cover matched thread lifetimes only. Even unchanged endpoints cannot exclude threads born and destroyed between snapshots. PARTIAL coverage is not a process total.
- NX-existing measures rejected writes; XX-existing measures admitted overwrites. This is a small Redis-compatible subset, not equivalent functionality.
