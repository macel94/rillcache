# Current benchmark performance

Generated: 2026-10-25T20:34:09Z · [Benchmark run #9](https://github.com/macel94/rillcache/actions/runs/40230615995) · [Tested source `29bfb20`](https://github.com/macel94/rillcache/commit/29bfb20fd88880d73364a205d2940b76b1a8c297)

**Benchmark job: success. Data: validated. Copilot review: completed.**

Benchmark attempt: 1; review attempt: 1. Preferred model: `gpt-6-luna` via **GitHub Copilot**.

[This report's permanent copy](https://github.com/macel94/rillcache/blob/main/benchmarks/reports/40230615995-1.md) · [Report history](https://github.com/macel94/rillcache/tree/main/benchmarks/reports) · [Methodology](https://github.com/macel94/rillcache/blob/main/docs/benchmarks.md)

## AI review

Actual model(s): **gpt-6-luna**. Selection: automatic-fallback. Authentication: workflow-token.

AI interpretation, **not measurements or a performance gate**. Verify claims against the deterministic tables below.

> Bottom line: The validated native cohort (180 records) shows workload-specific tradeoffs, not a
> general Redis/Valkey victory. Rillcache’s strongest results are with pipelining; its single-core
> pipeline-1 SET result is slower than both references.
> 
> Equal one-logical-CPU budget: For SET at pipeline 1, Rillcache reached 52,406 cmd/s versus Redis
> 58,265 and Valkey 57,330; its whole-batch p99 was 198.9 µs, versus 189.7 and 192.9 µs. At pipeline
> 16, Rillcache reached 682,086 cmd/s—15.2% above Redis’s 592,110—but its whole-batch p99 was higher:
> 378.0 µs versus 307.7 µs. These are whole-batch percentiles, not per-request latency.
> 
> With default threading, SET throughput was 124,558 cmd/s at pipeline 1 and 1,702,547 at pipeline 16;
> the corresponding whole-batch p99s were 144.7 and 163.5 µs. Those comparisons do not share the
> equal-one-CPU budget: Rillcache used about 190% and 230% CPU, respectively, while the references
> were near 100%.
> 
> Resource and scheduling observations: Rillcache’s sampled RSS was about 3 MiB, versus roughly 13–15
> MiB for references, but RSS is an approximate endpoint sample, not workload peak or allocator
> accounting. CPU figures use their own observation windows; zero idle ticks are quantized, not proof
> of zero use. Rillcache showed more measured context switches per command, especially involuntary
> switches at single-core pipeline 1; these cover matched thread lifetimes, not guaranteed process
> totals, and do not establish cause. VmHWM is a lifetime peak.
> 
> Uncertainty and next investigations: Results are three-run medians on a shared VM with unequal
> feature sets; they do not establish cross-run changes or general Redis-compatible performance.
> NX-existing rejects writes, while XX-existing admits overwrites.
> 
>  1. Measure per-request latency distributions and repeat the single-core pipeline-16 SET case with
> longer runs to investigate its throughput advantage alongside higher batch p99.
>  2. Profile CPU scheduling and scaling across longer, repeated single-core and default-threading
> runs, focusing on the elevated context-switch counts and the differences between CPU observation and
> request-timing windows.

## Measurements

**Data quality: complete.** 180/180 records validated across 4/4 complete profiles.


Hardware: AMD EPYC 7763 64-Core Processor. Rust 1.99.1 (58cf8f9c3 2026-10-13).

References: Redis 8.10.2 sha=00000000:0 malloc=jemalloc-5.3.0 bits=64 build=bc430f685268c9d6; Valkey 9.1.2 sha=00000000:0 malloc=jemalloc-5.3.0 bits=64 build=976f5b735ee4df9.

### SET at a glance

Median commands/s and median of the three runs' **whole-batch p99** values (µs). Positive rate delta means Rillcache is faster in this workload only.

| Profile | Rillcache cmd/s | Redis | Valkey | Rillcache vs Redis | Rillcache p99 µs | Redis p99 | Valkey p99 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| pipeline-1 | 124558 | 58046 | 56974 | +114.6% | 144.7 | 190.1 | 194.9 |
| pipeline-16 | 1702547 | 597392 | 543808 | +185.0% | 163.5 | 306.7 | 338.7 |
| single-core-pipeline-1 | 52406 | 58265 | 57330 | -10.1% | 198.9 | 189.7 | 192.9 |
| single-core-pipeline-16 | 682086 | 592110 | 535087 | +15.2% | 378.0 | 307.7 | 344.0 |

CPU is relative to **one logical CPU**, so parallel servers can exceed 100%. Do not blend default-threading and single-core results or different pipelines. Shared VMs and unequal feature sets do not establish a general performance win or a cross-run regression.

<details>
<summary>pipeline-1: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 100000; clients: 8; pipeline: 1; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: inherited; client CPUs: inherited.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 129102 | 125078–130516 | 143.3 | 187.0 | 14.70 | 95.2 | 3.01 | 0.837410 | 0.172510 | unchanged |
| ping | redis | 61819 | 61490–62127 | 175.8 | 100.6 | 16.20 | 85.8 | 12.72 | 0.000090 | 0.000060 | unchanged |
| ping | valkey | 60264 | 59719–61397 | 183.2 | 100.0 | 16.60 | 86.5 | 12.68 | 0.000170 | 0.000390 | unchanged |
| set | rillcache | 124558 | 120698–130330 | 144.7 | 190.4 | 15.30 | 92.9 | 3.01 | 0.833720 | 0.179650 | unchanged |
| set | redis | 58046 | 57970–58056 | 190.1 | 99.8 | 17.20 | 80.2 | 12.72 | 0.000050 | 0.000090 | unchanged |
| set | valkey | 56974 | 56916–57093 | 194.9 | 99.9 | 17.50 | 78.4 | 12.68 | 0.000080 | 0.000070 | unchanged |
| get | rillcache | 124184 | 123769–124212 | 158.3 | 191.1 | 15.40 | 93.6 | 3.01 | 0.847510 | 0.174840 | unchanged |
| get | redis | 59760 | 59640–59867 | 183.9 | 100.2 | 16.80 | 86.8 | 12.72 | 0.000130 | 0.000060 | unchanged |
| get | valkey | 59203 | 58732–62371 | 186.6 | 99.4 | 16.80 | 83.4 | 12.68 | 0.000080 | 0.000080 | unchanged |
| set-nx-existing | rillcache | 124593 | 122332–125062 | 150.7 | 189.2 | 15.20 | 91.6 | 3.01 | 0.817090 | 0.195160 | unchanged |
| set-nx-existing | redis | 59014 | 58588–59139 | 186.7 | 99.9 | 16.90 | 81.9 | 12.72 | 0.000140 | 0.000070 | unchanged |
| set-nx-existing | valkey | 58277 | 58196–58795 | 189.3 | 99.9 | 17.10 | 80.2 | 12.68 | 0.000040 | 0.000060 | unchanged |
| set-xx-existing | rillcache | 124109 | 120567–125147 | 150.7 | 188.5 | 15.30 | 92.8 | 3.01 | 0.827500 | 0.188780 | unchanged |
| set-xx-existing | redis | 58154 | 58028–58484 | 190.8 | 99.8 | 17.20 | 80.2 | 12.70 | 0.000060 | 0.000120 | unchanged |
| set-xx-existing | valkey | 56889 | 56839–57499 | 194.7 | 100.0 | 17.60 | 80.1 | 12.58 | 0.000070 | 0.000070 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 1.000 | 2.43 | 2.43 | 936.8 | unchanged |
| redis | 1.001 | 0.000 | 13.24 | 13.24 | 10.0 | unchanged |
| valkey | 1.001 | 0.000 | 12.77 | 12.77 | 10.0 | unchanged |

</details>

<details>
<summary>pipeline-16: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 1000000; clients: 8; pipeline: 16; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: inherited; client CPUs: inherited.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 2024616 | 1975225–2116800 | 140.5 | 196.2 | 0.95 | 90.5 | 3.07 | 0.053081 | 0.009975 | unchanged |
| ping | redis | 795746 | 791993–800706 | 223.8 | 100.1 | 1.25 | 68.3 | 15.40 | 0.000005 | 0.000015 | unchanged |
| ping | valkey | 744540 | 739642–750064 | 240.9 | 99.7 | 1.34 | 61.9 | 12.77 | 0.000009 | 0.000005 | unchanged |
| set | rillcache | 1702547 | 1680288–1718500 | 163.5 | 229.6 | 1.36 | 72.8 | 3.07 | 0.056297 | 0.007940 | unchanged |
| set | redis | 597392 | 596738–600076 | 306.7 | 100.2 | 1.67 | 57.1 | 14.88 | 0.000005 | 0.000003 | unchanged |
| set | valkey | 543808 | 543041–545684 | 338.7 | 99.9 | 1.84 | 46.2 | 12.66 | 0.000005 | 0.000006 | unchanged |
| get | rillcache | 1803071 | 1748247–1874089 | 149.2 | 193.9 | 1.07 | 81.1 | 3.07 | 0.046933 | 0.016414 | unchanged |
| get | redis | 671291 | 661047–671410 | 270.3 | 99.8 | 1.49 | 62.8 | 14.88 | 0.000002 | 0.000007 | unchanged |
| get | valkey | 612975 | 612200–618367 | 298.3 | 99.9 | 1.63 | 54.6 | 12.66 | 0.000004 | 0.000006 | unchanged |
| set-nx-existing | rillcache | 1816663 | 1796032–1833448 | 160.4 | 215.9 | 1.18 | 76.3 | 3.07 | 0.051542 | 0.012012 | unchanged |
| set-nx-existing | redis | 618404 | 615809–621833 | 295.4 | 99.5 | 1.61 | 55.6 | 14.88 | 0.000008 | 0.000004 | unchanged |
| set-nx-existing | valkey | 596099 | 596045–597648 | 305.8 | 100.1 | 1.68 | 55.4 | 12.66 | 0.000005 | 0.000006 | unchanged |
| set-xx-existing | rillcache | 1694649 | 1680143–1698836 | 157.3 | 230.8 | 1.37 | 71.5 | 3.07 | 0.055347 | 0.008774 | unchanged |
| set-xx-existing | redis | 577914 | 573860–579676 | 317.5 | 100.0 | 1.73 | 59.0 | 14.88 | 0.000004 | 0.000005 | unchanged |
| set-xx-existing | valkey | 530226 | 529158–530785 | 347.6 | 99.8 | 1.88 | 45.7 | 12.66 | 0.000005 | 0.000008 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 1.000 | 2.43 | 2.43 | 936.8 | unchanged |
| redis | 1.001 | 0.000 | 15.37 | 15.37 | 10.0 | unchanged |
| valkey | 1.001 | 0.000 | 12.76 | 12.76 | 10.0 | unchanged |

</details>

<details>
<summary>single-core-pipeline-1: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 100000; clients: 8; pipeline: 1; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: 0; client CPUs: 1,2,3.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 55401 | 55290–55690 | 182.2 | 99.7 | 18.00 | 94.4 | 3.01 | 0.017010 | 1.003280 | unchanged |
| ping | redis | 62122 | 62101–62271 | 176.0 | 99.4 | 16.00 | 86.3 | 12.77 | 0.000120 | 0.000060 | unchanged |
| ping | valkey | 60514 | 60491–60698 | 180.9 | 99.5 | 16.40 | 86.6 | 14.69 | 0.000040 | 0.000100 | unchanged |
| set | rillcache | 52406 | 51863–52422 | 198.9 | 100.1 | 19.20 | 92.2 | 3.01 | 0.018230 | 1.002510 | unchanged |
| set | redis | 58265 | 58087–58380 | 189.7 | 100.2 | 17.20 | 80.2 | 12.77 | 0.000050 | 0.000050 | unchanged |
| set | valkey | 57330 | 57180–57377 | 192.9 | 100.0 | 17.50 | 77.1 | 14.69 | 0.000060 | 0.000050 | unchanged |
| get | rillcache | 53906 | 53886–54037 | 186.0 | 100.2 | 18.60 | 93.5 | 3.01 | 0.017270 | 1.003870 | unchanged |
| get | redis | 60188 | 59961–60369 | 182.8 | 99.6 | 16.60 | 87.3 | 12.77 | 0.000030 | 0.000060 | unchanged |
| get | valkey | 58453 | 58312–58613 | 188.6 | 100.2 | 17.10 | 81.8 | 14.69 | 0.000130 | 0.000060 | unchanged |
| set-nx-existing | rillcache | 53109 | 51933–53360 | 201.5 | 99.8 | 18.80 | 92.7 | 3.01 | 0.018690 | 1.003680 | unchanged |
| set-nx-existing | redis | 59098 | 59059–59331 | 186.8 | 100.2 | 16.90 | 83.5 | 12.77 | 0.000110 | 0.000050 | unchanged |
| set-nx-existing | valkey | 58446 | 58138–58524 | 189.1 | 100.0 | 17.10 | 81.2 | 14.69 | 0.000030 | 0.000070 | unchanged |
| set-xx-existing | rillcache | 51678 | 51508–52708 | 191.9 | 99.9 | 19.40 | 92.3 | 3.01 | 0.018400 | 1.003300 | unchanged |
| set-xx-existing | redis | 57869 | 57709–57906 | 191.2 | 100.1 | 17.30 | 80.3 | 12.77 | 0.000100 | 0.000070 | unchanged |
| set-xx-existing | valkey | 57092 | 56895–57152 | 194.3 | 100.0 | 17.50 | 77.3 | 14.61 | 0.000060 | 0.000080 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 1.000 | 2.42 | 2.42 | 937.8 | unchanged |
| redis | 1.001 | 0.000 | 13.30 | 13.30 | 10.0 | unchanged |
| valkey | 1.001 | 0.000 | 14.81 | 14.81 | 10.0 | unchanged |

</details>

<details>
<summary>single-core-pipeline-16: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 1000000; clients: 8; pipeline: 16; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: 0; client CPUs: 1,2,3.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 773455 | 770109–780095 | 333.3 | 99.0 | 1.28 | 93.8 | 3.07 | 0.062973 | 0.000707 | unchanged |
| ping | redis | 788119 | 785348–788340 | 227.4 | 100.1 | 1.27 | 70.9 | 15.43 | 0.000006 | 0.000003 | unchanged |
| ping | valkey | 742161 | 735405–747137 | 241.8 | 100.1 | 1.35 | 59.6 | 12.98 | 0.000005 | 0.000005 | unchanged |
| set | rillcache | 682086 | 677253–683489 | 378.0 | 99.7 | 1.46 | 81.1 | 3.07 | 0.063119 | 0.000932 | unchanged |
| set | redis | 592110 | 590890–597807 | 307.7 | 100.0 | 1.69 | 55.6 | 14.91 | 0.000008 | 0.000004 | unchanged |
| set | valkey | 535087 | 534498–538490 | 344.0 | 100.0 | 1.86 | 48.4 | 12.94 | 0.000007 | 0.000007 | unchanged |
| get | rillcache | 716805 | 709813–717348 | 358.0 | 100.3 | 1.40 | 85.0 | 3.07 | 0.062953 | 0.000822 | unchanged |
| get | redis | 663316 | 662196–663809 | 273.1 | 99.5 | 1.50 | 59.3 | 14.91 | 0.000005 | 0.000006 | unchanged |
| get | valkey | 600269 | 597629–601747 | 304.5 | 99.8 | 1.67 | 55.1 | 12.94 | 0.000006 | 0.000006 | unchanged |
| set-nx-existing | rillcache | 696362 | 690903–704577 | 372.6 | 100.1 | 1.44 | 81.9 | 3.07 | 0.063068 | 0.000887 | unchanged |
| set-nx-existing | redis | 615290 | 614046–619743 | 297.1 | 99.7 | 1.62 | 56.1 | 14.91 | 0.000009 | 0.000006 | unchanged |
| set-nx-existing | valkey | 588042 | 587770–595204 | 310.3 | 99.9 | 1.70 | 61.2 | 12.94 | 0.000007 | 0.000006 | unchanged |
| set-xx-existing | rillcache | 671477 | 670645–678537 | 383.0 | 99.7 | 1.48 | 79.9 | 3.07 | 0.063148 | 0.000921 | unchanged |
| set-xx-existing | redis | 574011 | 572341–574439 | 318.6 | 99.9 | 1.74 | 58.0 | 14.91 | 0.000001 | 0.000007 | unchanged |
| set-xx-existing | valkey | 525019 | 521774–526258 | 351.3 | 100.2 | 1.91 | 49.2 | 12.94 | 0.000004 | 0.000008 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 1.000 | 2.42 | 2.42 | 934.8 | unchanged |
| redis | 1.001 | 0.000 | 15.39 | 15.39 | 10.0 | unchanged |
| valkey | 1.001 | 0.000 | 12.97 | 12.97 | 10.0 | unchanged |

</details>

### Fixed-worker mode measurements

30/30 separate embedded records validated. 8 clients; 10000 commands/workload/run; pipeline 16; three repetitions. These share the client/server host and inherited CPU affinity; they are **not native competitor comparisons or equal-per-core scaling evidence**. No process CPU/RSS accounting was collected for this group. Fixed lanes have a 1 ms idle polling wait. They actively poll for at most 50 µs after progress before returning to idle parking; this trades active CPU work for fewer premature sleeps, not epoll readiness or free CPU. Multiple lanes also request a cooperative yield after 100 µs of continuous active passes when more than one client is admitted. Progress does not rearm that separate budget; single lanes/lone clients do not periodically yield. This is checked between bounded passes, not a hard deadline or an OS fairness guarantee. The v2 interval restarts after a yield/park returns, excluding that explicit wait; involuntary descheduling can still consume wall time.

| Workload | 1 worker median cmd/s | 1 worker min–max | 4 workers median cmd/s | 4 workers min–max | 1 worker batch p99 µs | 4 workers batch p99 µs |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| ping | 949958 | 944042–996700 | 1190640 | 1058027–1423905 | 889.3 | 1042.8 |
| set | 738244 | 720045–791591 | 1171682 | 1041388–1249973 | 958.0 | 1007.1 |
| get | 909578 | 863964–923461 | 1344094 | 1295463–1398730 | 949.9 | 1041.5 |
| set-nx-existing | 876241 | 786440–911268 | 1203327 | 1191051–1364448 | 961.1 | 1085.5 |
| set-xx-existing | 845483 | 788899–881671 | 1162408 | 1095661–1193740 | 954.4 | 970.5 |

### Interpretation limits

- Batch p99 is **not per-request p99**; never divide it by the pipeline length. Percentiles are not pooled across runs.
- CPU counters are tick-quantized; zero observed idle ticks do not prove zero CPU cost. CPU observation windows bracket, but are not identical to, request timers.
- RSS is an approximate endpoint sample, not allocator accounting or a workload peak; VmHWM includes startup, warmup and previous workloads.
- Context switches cover matched thread lifetimes only. Even unchanged endpoints cannot exclude threads born and destroyed between snapshots. PARTIAL coverage is not a process total.
- NX-existing measures rejected writes; XX-existing measures admitted overwrites. This is a small Redis-compatible subset, not equivalent functionality.
