# Current benchmark performance

Generated: 2026-10-24T22:45:52Z · [Benchmark run #6](https://github.com/macel94/rillcache/actions/runs/40139425158) · [Tested source `8c42257`](https://github.com/macel94/rillcache/commit/8c42257b2b521834a0ae115306ed7e0fe4de7114)

**Benchmark job: success. Data: validated. Copilot review: completed.**

Benchmark attempt: 1; review attempt: 1. Preferred model: `gpt-6-luna` via **GitHub Copilot**.

[This report's permanent copy](https://github.com/macel94/rillcache/blob/main/benchmarks/reports/40139425158-1.md) · [Report history](https://github.com/macel94/rillcache/tree/main/benchmarks/reports) · [Methodology](https://github.com/macel94/rillcache/blob/main/docs/benchmarks.md)

## AI review

Actual model(s): **gpt-6-luna**. Selection: automatic-fallback. Authentication: workflow-token.

AI interpretation, **not measurements or a performance gate**. Verify claims against the deterministic tables below.

> Bottom line: In the 180-record native cohort, results depend on pipeline and CPU budget. With
> inherited threading, Rillcache’s SET throughput was about 1.9–2.0× Redis’s; under a one-logical-CPU
> budget, it was 12.3% lower at pipeline 1 and 12.8% higher at pipeline 16. This is workload-specific
> evidence, not a general Redis or Valkey victory.
> 
> Throughput and tail tradeoffs: At one CPU, pipeline-1 SET reached 75,338 commands/s versus Redis’s
> 85,866, with whole-batch p99 of 202.3 µs versus 121.7 µs. At pipeline 16, Rillcache reached 962,161
> versus 852,849 commands/s, but its whole-batch p99 was higher: 263.6 versus 218.9 µs. With inherited
> threading at pipeline 16, throughput was 1,762,046 versus 860,238 commands/s, and batch p99 was
> 176.5 versus 216.7 µs. Batch p99 is not per-request latency and should not be divided by pipeline
> length.
> 
> CPU, memory, scheduling: Inherited-threading SET at pipeline 16 used 251.8% of one logical CPU and
> 1.44 CPU µs/command, versus Redis at 100.3% and 1.17 µs/command; the throughput advantage therefore
> came with greater measured CPU use. At one CPU, Rillcache’s pipeline-1 SET used 13.2 CPU µs/command
> versus Redis’s 11.7. Approximate endpoint RSS was lower for Rillcache (about 3 MiB versus 13–15
> MiB), not a workload peak or allocator measure. Rillcache also recorded roughly 0.83 voluntary
> context switches per command at inherited-threading pipeline 1, versus near zero for Redis; counters
> cover matched thread lifetimes and do not establish cause. Idle zero CPU ticks are quantized, not
> proof of zero cost.
> 
> Uncertainty and next investigations: Results are medians of three runs on a shared VM; CPU
> observation windows differ from request timing, and RSS is approximate. NX-existing rejects writes;
> XX-existing admits overwrites, so these are distinct operations. First, profile equal-one-CPU
> pipeline-1 scheduling and CPU cost, given its lower throughput and higher switch counts. Second,
> investigate fixed-worker mode’s 4-worker throughput (for example, SET 509,564 versus 1-worker
> 1,035,255 commands/s) and higher batch p99; these embedded measurements are a separate group, not
> native or equal-per-core comparisons.

## Measurements

**Data quality: complete.** 180/180 records validated across 4/4 complete profiles.


Hardware: AMD EPYC 9V74 80-Core Processor. Rust 1.99.1 (58cf8f9c3 2026-10-13).

References: Redis 8.10.2 sha=00000000:0 malloc=jemalloc-5.3.0 bits=64 build=a82aa1e584000411; Valkey 9.1.2 sha=00000000:0 malloc=jemalloc-5.3.0 bits=64 build=3e5f6c202ff61563.

### SET at a glance

Median commands/s and median of the three runs' **whole-batch p99** values (µs). Positive rate delta means Rillcache is faster in this workload only.

| Profile | Rillcache cmd/s | Redis | Valkey | Rillcache vs Redis | Rillcache p99 µs | Redis p99 | Valkey p99 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| pipeline-1 | 164377 | 85962 | 85374 | +91.2% | 108.2 | 121.0 | 122.6 |
| pipeline-16 | 1762046 | 860238 | 778130 | +104.8% | 176.5 | 216.7 | 244.2 |
| single-core-pipeline-1 | 75338 | 85866 | 84131 | -12.3% | 202.3 | 121.7 | 125.4 |
| single-core-pipeline-16 | 962161 | 852849 | 773890 | +12.8% | 263.6 | 218.9 | 240.8 |

CPU is relative to **one logical CPU**, so parallel servers can exceed 100%. Do not blend default-threading and single-core results or different pipelines. Shared VMs and unequal feature sets do not establish a general performance win or a cross-run regression.

<details>
<summary>pipeline-1: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 100000; clients: 8; pipeline: 1; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: inherited; client CPUs: inherited.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 171413 | 170373–176285 | 113.4 | 190.1 | 11.10 | 94.4 | 3.05 | 0.835990 | 0.172540 | unchanged |
| ping | redis | 90136 | 89185–90620 | 114.5 | 99.9 | 11.10 | 90.2 | 15.45 | 0.000050 | 0.000050 | unchanged |
| ping | valkey | 88606 | 88346–88618 | 117.0 | 100.1 | 11.30 | 89.4 | 14.86 | 0.000090 | 0.000050 | unchanged |
| set | rillcache | 164377 | 161541–174015 | 108.2 | 188.9 | 11.40 | 91.3 | 3.05 | 0.827230 | 0.183710 | unchanged |
| set | redis | 85962 | 85755–86122 | 121.0 | 99.7 | 11.60 | 87.1 | 15.45 | 0.000080 | 0.000040 | unchanged |
| set | valkey | 85374 | 84191–85477 | 122.6 | 100.0 | 11.70 | 85.5 | 14.93 | 0.000160 | 0.000040 | unchanged |
| get | rillcache | 173484 | 167235–178574 | 98.7 | 194.1 | 11.20 | 92.1 | 3.05 | 0.854800 | 0.156200 | unchanged |
| get | redis | 87795 | 87350–87929 | 118.6 | 100.1 | 11.40 | 89.5 | 14.99 | 0.000090 | 0.000050 | unchanged |
| get | valkey | 87067 | 86001–88426 | 120.0 | 99.9 | 11.50 | 86.7 | 14.78 | 0.000130 | 0.000030 | unchanged |
| set-nx-existing | rillcache | 177473 | 166156–178188 | 99.8 | 197.6 | 11.20 | 93.1 | 3.05 | 0.850740 | 0.158750 | unchanged |
| set-nx-existing | redis | 87791 | 87278–90272 | 118.3 | 100.3 | 11.50 | 87.8 | 14.99 | 0.000160 | 0.000050 | unchanged |
| set-nx-existing | valkey | 85817 | 85518–86049 | 121.6 | 100.4 | 11.70 | 87.2 | 14.78 | 0.000090 | 0.000070 | unchanged |
| set-xx-existing | rillcache | 168350 | 165300–169900 | 107.8 | 193.2 | 11.40 | 90.6 | 3.05 | 0.828590 | 0.184890 | unchanged |
| set-xx-existing | redis | 86608 | 85970–86953 | 120.2 | 99.7 | 11.50 | 87.8 | 14.98 | 0.000100 | 0.000030 | unchanged |
| set-xx-existing | valkey | 84149 | 84138–84507 | 124.9 | 99.7 | 11.80 | 87.3 | 14.78 | 0.000060 | 0.000050 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 2.47 | 2.47 | 941.8 | unchanged |
| redis | 1.000 | 0.000 | 15.43 | 15.43 | 10.0 | unchanged |
| valkey | 1.001 | 0.000 | 14.86 | 14.86 | 10.0 | unchanged |

</details>

<details>
<summary>pipeline-16: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 1000000; clients: 8; pipeline: 16; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: inherited; client CPUs: inherited.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 2627166 | 2625256–2681243 | 131.1 | 191.4 | 0.72 | 88.9 | 3.09 | 0.048712 | 0.014946 | unchanged |
| ping | redis | 1129977 | 1126567–1139105 | 155.8 | 100.5 | 0.89 | 69.7 | 13.36 | 0.000017 | 0.000005 | unchanged |
| ping | valkey | 1081006 | 1079822–1096060 | 164.8 | 99.3 | 0.92 | 70.7 | 12.83 | 0.000010 | 0.000004 | unchanged |
| set | rillcache | 1762046 | 1714280–1762361 | 176.5 | 251.8 | 1.44 | 63.4 | 3.09 | 0.072458 | 0.010534 | unchanged |
| set | redis | 860238 | 855570–864517 | 216.7 | 100.3 | 1.17 | 56.0 | 13.36 | 0.000017 | 0.000005 | unchanged |
| set | valkey | 778130 | 769677–779026 | 244.2 | 100.3 | 1.29 | 51.2 | 12.86 | 0.000009 | 0.000003 | unchanged |
| get | rillcache | 2402210 | 2370475–2489806 | 123.8 | 206.3 | 0.86 | 73.3 | 3.10 | 0.041871 | 0.021906 | unchanged |
| get | redis | 965698 | 961175–965949 | 187.8 | 100.4 | 1.04 | 60.6 | 13.36 | 0.000012 | 0.000004 | unchanged |
| get | valkey | 892005 | 883340–892565 | 206.6 | 99.9 | 1.12 | 61.6 | 12.86 | 0.000008 | 0.000003 | unchanged |
| set-nx-existing | rillcache | 2322623 | 2309418–2339025 | 116.0 | 221.9 | 0.96 | 71.9 | 3.10 | 0.052800 | 0.010830 | unchanged |
| set-nx-existing | redis | 894957 | 889256–905646 | 206.6 | 100.5 | 1.12 | 60.2 | 13.36 | 0.000002 | 0.000007 | unchanged |
| set-nx-existing | valkey | 865642 | 858738–872200 | 212.7 | 100.4 | 1.16 | 56.9 | 12.77 | 0.000009 | 0.000003 | unchanged |
| set-xx-existing | rillcache | 1738039 | 1701449–1750738 | 175.1 | 253.5 | 1.45 | 64.6 | 3.10 | 0.072079 | 0.011310 | unchanged |
| set-xx-existing | redis | 834577 | 822410–836002 | 226.0 | 99.5 | 1.19 | 53.8 | 12.91 | 0.000009 | 0.000009 | unchanged |
| set-xx-existing | valkey | 750192 | 749408–750479 | 249.8 | 99.7 | 1.33 | 48.9 | 12.77 | 0.000005 | 0.000006 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 2.43 | 2.43 | 941.8 | unchanged |
| redis | 1.000 | 0.000 | 13.33 | 13.33 | 10.0 | unchanged |
| valkey | 1.001 | 0.000 | 12.82 | 12.82 | 10.0 | unchanged |

</details>

<details>
<summary>single-core-pipeline-1: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 100000; clients: 8; pipeline: 1; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: 0; client CPUs: 1,2,3.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 76862 | 76628–76972 | 198.1 | 99.3 | 12.90 | 95.4 | 3.05 | 1.003740 | 0.007850 | unchanged |
| ping | redis | 89558 | 89242–90207 | 119.9 | 100.1 | 11.20 | 90.2 | 13.45 | 0.000060 | 0.000040 | unchanged |
| ping | valkey | 88543 | 88306–88823 | 117.5 | 99.4 | 11.20 | 89.4 | 12.91 | 0.000110 | 0.000060 | unchanged |
| set | rillcache | 75338 | 75207–75445 | 202.3 | 99.5 | 13.20 | 93.2 | 3.05 | 1.004010 | 0.008060 | unchanged |
| set | redis | 85866 | 85743–86076 | 121.7 | 100.3 | 11.70 | 88.9 | 13.45 | 0.000060 | 0.000040 | unchanged |
| set | valkey | 84131 | 84128–84299 | 125.4 | 100.1 | 11.90 | 84.7 | 12.98 | 0.000030 | 0.000050 | unchanged |
| get | rillcache | 76014 | 75405–76044 | 200.5 | 99.6 | 13.10 | 93.2 | 3.05 | 1.003710 | 0.008070 | unchanged |
| get | redis | 87671 | 87613–87761 | 119.0 | 99.9 | 11.40 | 88.6 | 12.98 | 0.000130 | 0.000050 | unchanged |
| get | valkey | 86222 | 86200–86274 | 121.9 | 99.2 | 11.50 | 87.9 | 12.83 | 0.000040 | 0.000050 | unchanged |
| set-nx-existing | rillcache | 75751 | 75735–75987 | 200.8 | 99.9 | 13.20 | 93.1 | 3.05 | 1.003660 | 0.008160 | unchanged |
| set-nx-existing | redis | 86740 | 86630–86749 | 120.2 | 100.5 | 11.60 | 89.6 | 12.99 | 0.000030 | 0.000050 | unchanged |
| set-nx-existing | valkey | 85595 | 85570–85684 | 122.1 | 100.1 | 11.70 | 88.0 | 12.83 | 0.000020 | 0.000060 | unchanged |
| set-xx-existing | rillcache | 75280 | 75178–75316 | 202.8 | 99.9 | 13.30 | 91.7 | 3.05 | 1.003860 | 0.007940 | unchanged |
| set-xx-existing | redis | 85353 | 84845–85471 | 123.4 | 100.1 | 11.80 | 87.9 | 12.98 | 0.000060 | 0.000040 | unchanged |
| set-xx-existing | valkey | 83767 | 83689–83983 | 126.0 | 99.6 | 11.90 | 85.7 | 12.83 | 0.000040 | 0.000060 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 2.46 | 2.46 | 942.8 | unchanged |
| redis | 1.000 | 0.000 | 13.43 | 13.43 | 10.0 | unchanged |
| valkey | 1.001 | 0.000 | 12.91 | 12.91 | 10.0 | unchanged |

</details>

<details>
<summary>single-core-pipeline-16: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 1000000; clients: 8; pipeline: 16; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: 0; client CPUs: 1,2,3.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 1190292 | 1184462–1195672 | 206.0 | 100.4 | 0.84 | 89.3 | 3.09 | 0.062745 | 0.000524 | unchanged |
| ping | redis | 1124723 | 1121564–1128713 | 155.9 | 99.8 | 0.89 | 70.8 | 13.50 | 0.000006 | 0.000004 | unchanged |
| ping | valkey | 1077916 | 1048888–1078156 | 164.2 | 99.1 | 0.93 | 68.8 | 12.92 | 0.000005 | 0.000007 | unchanged |
| set | rillcache | 962161 | 953604–964339 | 263.6 | 99.1 | 1.04 | 73.8 | 3.09 | 0.062892 | 0.000787 | unchanged |
| set | redis | 852849 | 835692–855333 | 218.9 | 99.4 | 1.17 | 56.3 | 13.51 | 0.000007 | 0.000003 | unchanged |
| set | valkey | 773890 | 772513–775968 | 240.8 | 100.4 | 1.30 | 50.0 | 12.95 | 0.000009 | 0.000004 | unchanged |
| get | rillcache | 1051216 | 1047748–1052013 | 235.6 | 100.5 | 0.96 | 80.2 | 3.11 | 0.062744 | 0.000643 | unchanged |
| get | redis | 952646 | 948955–952933 | 189.9 | 100.5 | 1.06 | 64.2 | 13.51 | 0.000006 | 0.000004 | unchanged |
| get | valkey | 885081 | 884414–888166 | 207.2 | 100.0 | 1.13 | 58.4 | 12.95 | 0.000005 | 0.000003 | unchanged |
| set-nx-existing | rillcache | 1047358 | 1045492–1057326 | 235.3 | 100.4 | 0.95 | 80.0 | 3.11 | 0.062819 | 0.000615 | unchanged |
| set-nx-existing | redis | 886333 | 885832–890936 | 207.1 | 100.1 | 1.13 | 60.2 | 13.51 | 0.000004 | 0.000004 | unchanged |
| set-nx-existing | valkey | 861794 | 861594–862462 | 213.5 | 99.9 | 1.16 | 56.9 | 12.86 | 0.000014 | 0.000004 | unchanged |
| set-xx-existing | rillcache | 945399 | 918299–949014 | 267.1 | 100.0 | 1.06 | 72.6 | 3.11 | 0.062843 | 0.000799 | unchanged |
| set-xx-existing | redis | 825374 | 823527–826074 | 225.6 | 99.9 | 1.21 | 54.5 | 13.09 | 0.000006 | 0.000005 | unchanged |
| set-xx-existing | valkey | 750322 | 749961–752209 | 251.0 | 99.8 | 1.33 | 51.9 | 12.86 | 0.000007 | 0.000004 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 2.44 | 2.44 | 943.8 | unchanged |
| redis | 1.000 | 0.000 | 13.48 | 13.48 | 10.0 | unchanged |
| valkey | 1.001 | 0.000 | 12.91 | 12.91 | 10.0 | unchanged |

</details>

### Fixed-worker mode measurements

30/30 separate embedded records validated. 8 clients; 10000 commands/workload/run; pipeline 16; three repetitions. These share the client/server host and inherited CPU affinity; they are **not native competitor comparisons or equal-per-core scaling evidence**. No process CPU/RSS accounting was collected for this group. Fixed lanes have a 1 ms idle polling wait.

| Workload | 1 worker median cmd/s | 1 worker min–max | 4 workers median cmd/s | 4 workers min–max | 1 worker batch p99 µs | 4 workers batch p99 µs |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| ping | 1254705 | 1211017–1270281 | 232085 | 207833–351002 | 902.8 | 1107.4 |
| set | 1035255 | 984348–1055857 | 509564 | 420056–612689 | 909.9 | 1163.2 |
| get | 1137929 | 1085611–1138592 | 269288 | 236930–672860 | 901.5 | 1119.6 |
| set-nx-existing | 1090518 | 1067715–1129696 | 376366 | 326398–462565 | 912.3 | 1128.2 |
| set-xx-existing | 986128 | 962343–1057744 | 566345 | 534672–578846 | 907.2 | 1130.9 |

### Interpretation limits

- Batch p99 is **not per-request p99**; never divide it by the pipeline length. Percentiles are not pooled across runs.
- CPU counters are tick-quantized; zero observed idle ticks do not prove zero CPU cost. CPU observation windows bracket, but are not identical to, request timers.
- RSS is an approximate endpoint sample, not allocator accounting or a workload peak; VmHWM includes startup, warmup and previous workloads.
- Context switches cover matched thread lifetimes only. Even unchanged endpoints cannot exclude threads born and destroyed between snapshots. PARTIAL coverage is not a process total.
- NX-existing measures rejected writes; XX-existing measures admitted overwrites. This is a small Redis-compatible subset, not equivalent functionality.
