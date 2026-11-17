# Current benchmark performance

Generated: 2026-11-17T17:52:38Z · [Benchmark run #19](https://github.com/macel94/rillcache/actions/runs/44139137382) · [Tested source `9e9cb45`](https://github.com/macel94/rillcache/commit/9e9cb451a1f3da9be6ab23e61757fd041217f8b7)

**Benchmark job: success. Data: validated. Copilot review: completed.**

Benchmark attempt: 1; review attempt: 1. Preferred model: `gpt-6-luna` via **GitHub Copilot**.

[This report's permanent copy](https://github.com/macel94/rillcache/blob/main/benchmarks/reports/44139137382-1.md) · [Report history](https://github.com/macel94/rillcache/tree/main/benchmarks/reports) · [Methodology](https://github.com/macel94/rillcache/blob/main/docs/benchmarks.md)

## AI review

Actual model(s): **gpt-6-luna**. Selection: automatic-fallback. Authentication: workflow-token.

AI interpretation, **not measurements or a performance gate**. Verify claims against the deterministic tables below.

> Bottom line: In this validated cohort, Rillcache has higher throughput and lower whole-batch p99
> than Redis and Valkey on the measured workloads, including with the server limited to one logical
> CPU. This is workload-specific evidence, not a general Redis/Valkey comparison.
> 
> Equal-one-logical-CPU tradeoff: For SET, pipeline 1 reached 69,496 commands/s versus Redis’s 61,698
> and Valkey’s 60,547; batch p99 was 145.3 µs versus 182.3 and 185.5 µs. At pipeline 16, throughput
> was 832,786 versus 627,370 and 556,807 commands/s, with batch p99 of 188.1 µs versus 307.6 and 352.4
> µs. These are batch percentiles, not per-request latency; do not divide them by pipeline length.
> Default-threading results are a separate comparison.
> 
> Resource and scheduling observations: In the equal-CPU SET runs, CPU use was about 100% for all
> three; at pipeline 16, reported CPU was 1.21 µs/command for Rillcache, 1.59 for Redis, and 1.80 for
> Valkey. System time’s share varied by workload, so these counters alone do not explain throughput.
> Approximate pipeline-16 RSS endpoints were 6.54 MiB for Rillcache, 8.72 for Redis, and 8.30 for
> Valkey—not workload peaks or allocator accounting. Idle zero CPU ticks are quantized, not proof of
> zero cost. Context-switch counts cover matched thread lifetimes, not guaranteed process totals.
> 
> The separate fixed-worker measurements are not native competitor or equal-per-core evidence: four
> workers delivered only about 38k commands/s for SET/GET versus roughly 716k/862k with one worker,
> while ping rose from about 1.00m to 1.49m. No process CPU or RSS accounting was collected for this
> group.
> 
> Next investigations: (1) Repeat fixed-worker workloads with per-lane progress, queue/park timing,
> and CPU measurements to locate where four-worker SET/GET throughput collapses. (2) Expand native
> runs across key counts, value sizes, and client counts, reporting batch and per-request latency
> separately; the current 1,024-key, 64-byte, eight-client results cannot establish broader behavior.

## Measurements

**Data quality: complete.** 180/180 records validated across 4/4 complete profiles.


Hardware: AMD EPYC 7763 64-Core Processor. Rust 1.99.1 (58cf8f9c3 2026-10-13).

References: Redis 8.10.2 sha=00000000:0 malloc=jemalloc-5.3.0 bits=64 build=e9127d16a381b04f; Valkey 9.1.2 sha=00000000:0 malloc=jemalloc-5.3.0 bits=64 build=dea14fe646e932c7.

### SET at a glance

Median commands/s and median of the three runs' **whole-batch p99** values (µs). Positive rate delta means Rillcache is faster in this workload only.

| Profile | Rillcache cmd/s | Redis | Valkey | Rillcache vs Redis | Rillcache p99 µs | Redis p99 | Valkey p99 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| pipeline-1 | 70843 | 62450 | 61503 | +13.4% | 144.8 | 178.3 | 183.4 |
| pipeline-16 | 857024 | 618535 | 551726 | +38.6% | 186.9 | 315.2 | 357.2 |
| single-core-pipeline-1 | 69496 | 61698 | 60547 | +12.6% | 145.3 | 182.3 | 185.5 |
| single-core-pipeline-16 | 832786 | 627370 | 556807 | +32.7% | 188.1 | 307.6 | 352.4 |

CPU is relative to **one logical CPU**, so parallel servers can exceed 100%. Do not blend default-threading and single-core results or different pipelines. Shared VMs and unequal feature sets do not establish a general performance win or a cross-run regression.

<details>
<summary>pipeline-1: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 100000; clients: 8; pipeline: 1; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: inherited; client CPUs: inherited.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 72761 | 72596–73823 | 139.6 | 100.2 | 13.70 | 94.2 | 6.27 | 0.000020 | 0.000060 | unchanged |
| ping | redis | 64490 | 64301–66085 | 168.8 | 99.9 | 15.50 | 89.7 | 8.96 | 0.000030 | 0.000060 | unchanged |
| ping | valkey | 64783 | 63446–65258 | 169.5 | 99.8 | 15.40 | 86.7 | 8.25 | 0.000190 | 0.000050 | unchanged |
| set | rillcache | 70843 | 69332–70878 | 144.8 | 99.9 | 14.20 | 89.4 | 6.27 | 0.000010 | 0.000080 | unchanged |
| set | redis | 62450 | 61532–63084 | 178.3 | 99.9 | 16.00 | 83.8 | 8.96 | 0.000090 | 0.000060 | unchanged |
| set | valkey | 61503 | 61108–61812 | 183.4 | 100.2 | 16.30 | 80.4 | 8.27 | 0.000040 | 0.000060 | unchanged |
| get | rillcache | 73489 | 70606–73515 | 141.7 | 99.9 | 13.60 | 92.6 | 6.27 | 0.000010 | 0.000080 | unchanged |
| get | redis | 64295 | 64220–64443 | 170.5 | 99.6 | 15.50 | 86.5 | 8.96 | 0.000220 | 0.000110 | unchanged |
| get | valkey | 62861 | 61774–62953 | 177.2 | 99.9 | 15.90 | 83.0 | 8.27 | 0.000120 | 0.000070 | unchanged |
| set-nx-existing | rillcache | 70767 | 70672–71809 | 142.6 | 100.3 | 14.10 | 90.8 | 6.27 | 0.000010 | 0.000090 | unchanged |
| set-nx-existing | redis | 62696 | 62334–63469 | 178.4 | 99.7 | 15.90 | 85.0 | 8.96 | 0.000160 | 0.000090 | unchanged |
| set-nx-existing | valkey | 62954 | 61369–63182 | 177.7 | 99.4 | 15.80 | 82.2 | 8.27 | 0.000060 | 0.000070 | unchanged |
| set-xx-existing | rillcache | 69386 | 69266–69971 | 146.4 | 100.0 | 14.40 | 88.8 | 6.27 | 0.000010 | 0.000070 | unchanged |
| set-xx-existing | redis | 62276 | 61415–65351 | 180.0 | 99.6 | 16.00 | 83.8 | 8.96 | 0.000140 | 0.000050 | unchanged |
| set-xx-existing | valkey | 61336 | 59700–61399 | 184.5 | 100.1 | 16.30 | 80.4 | 8.27 | 0.000180 | 0.000060 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 5.02 | 5.02 | 99.0 | unchanged |
| redis | 1.001 | 0.000 | 8.76 | 8.76 | 10.0 | unchanged |
| valkey | 1.001 | 0.000 | 8.11 | 8.11 | 10.0 | unchanged |

</details>

<details>
<summary>pipeline-16: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 1000000; clients: 8; pipeline: 16; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: inherited; client CPUs: inherited.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 1153014 | 1144521–1162651 | 140.2 | 100.3 | 0.87 | 89.7 | 6.19 | 0.000001 | 0.000006 | unchanged |
| ping | redis | 842269 | 839882–850361 | 220.3 | 99.9 | 1.19 | 73.1 | 8.92 | 0.000004 | 0.000006 | unchanged |
| ping | valkey | 778791 | 775275–780559 | 242.3 | 99.9 | 1.28 | 65.6 | 8.26 | 0.000009 | 0.000011 | unchanged |
| set | rillcache | 857024 | 845950–864276 | 186.9 | 100.2 | 1.16 | 62.2 | 6.19 | 0.000001 | 0.000007 | unchanged |
| set | redis | 618535 | 618030–631387 | 315.2 | 99.6 | 1.61 | 59.6 | 8.92 | 0.000005 | 0.000006 | unchanged |
| set | valkey | 551726 | 550880–552041 | 357.2 | 100.2 | 1.82 | 41.4 | 8.25 | 0.000005 | 0.000006 | unchanged |
| get | rillcache | 1005761 | 996103–1013040 | 161.6 | 100.3 | 0.99 | 77.2 | 6.19 | 0.000001 | 0.000007 | unchanged |
| get | redis | 691817 | 684570–696411 | 276.8 | 99.2 | 1.45 | 60.7 | 8.92 | 0.000003 | 0.000005 | unchanged |
| get | valkey | 626875 | 622610–630336 | 310.5 | 100.2 | 1.59 | 61.6 | 8.25 | 0.000004 | 0.000004 | unchanged |
| set-nx-existing | rillcache | 928168 | 891104–949259 | 176.7 | 99.8 | 1.08 | 68.8 | 6.19 | 0.000001 | 0.000011 | unchanged |
| set-nx-existing | redis | 648608 | 645592–655856 | 297.7 | 99.9 | 1.54 | 50.7 | 8.92 | 0.000003 | 0.000006 | unchanged |
| set-nx-existing | valkey | 616227 | 613756–616760 | 316.9 | 100.0 | 1.63 | 62.0 | 8.26 | 0.000007 | 0.000005 | unchanged |
| set-xx-existing | rillcache | 842166 | 823578–849006 | 190.3 | 100.2 | 1.19 | 63.0 | 6.19 | 0.000001 | 0.000004 | unchanged |
| set-xx-existing | redis | 602259 | 601848–607258 | 323.3 | 100.2 | 1.65 | 56.4 | 8.92 | 0.000003 | 0.000006 | unchanged |
| set-xx-existing | valkey | 536176 | 535865–536741 | 367.0 | 100.2 | 1.87 | 40.1 | 8.26 | 0.000005 | 0.000005 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 4.83 | 4.83 | 99.0 | unchanged |
| redis | 1.000 | 0.000 | 8.70 | 8.70 | 10.0 | unchanged |
| valkey | 1.001 | 0.000 | 8.07 | 8.07 | 10.0 | unchanged |

</details>

<details>
<summary>single-core-pipeline-1: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 100000; clients: 8; pipeline: 1; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: 0; client CPUs: 1,2,3.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 73399 | 73369–73691 | 137.6 | 99.8 | 13.60 | 94.1 | 6.20 | 0.000010 | 0.000100 | unchanged |
| ping | redis | 64955 | 64851–65412 | 167.5 | 99.8 | 15.40 | 89.0 | 8.73 | 0.000080 | 0.000070 | unchanged |
| ping | valkey | 63591 | 63457–63712 | 173.3 | 100.2 | 15.80 | 87.3 | 8.13 | 0.000120 | 0.000090 | unchanged |
| set | rillcache | 69496 | 68300–69501 | 145.3 | 100.1 | 14.40 | 89.6 | 6.20 | 0.000020 | 0.000060 | unchanged |
| set | redis | 61698 | 61512–61900 | 182.3 | 100.2 | 16.30 | 82.6 | 8.73 | 0.000050 | 0.000070 | unchanged |
| set | valkey | 60547 | 60467–60622 | 185.5 | 99.9 | 16.50 | 81.2 | 8.13 | 0.000030 | 0.000050 | unchanged |
| get | rillcache | 70957 | 70610–71346 | 142.1 | 100.3 | 14.20 | 91.5 | 6.20 | 0.000010 | 0.000060 | unchanged |
| get | redis | 63249 | 62995–63667 | 174.5 | 99.5 | 15.80 | 86.7 | 8.73 | 0.000070 | 0.000060 | unchanged |
| get | valkey | 62194 | 62136–62249 | 178.2 | 100.0 | 16.10 | 84.4 | 8.14 | 0.000050 | 0.000050 | unchanged |
| set-nx-existing | rillcache | 70229 | 70210–70873 | 143.2 | 99.7 | 14.20 | 90.8 | 6.20 | 0.000020 | 0.000070 | unchanged |
| set-nx-existing | redis | 62036 | 61937–62109 | 179.7 | 100.3 | 16.20 | 84.0 | 8.73 | 0.000150 | 0.000060 | unchanged |
| set-nx-existing | valkey | 61926 | 61514–62156 | 181.2 | 99.7 | 16.10 | 83.9 | 8.14 | 0.000040 | 0.000060 | unchanged |
| set-xx-existing | rillcache | 68792 | 68770–69124 | 146.0 | 99.7 | 14.50 | 90.3 | 6.20 | 0.000010 | 0.000040 | unchanged |
| set-xx-existing | redis | 61220 | 61209–61327 | 183.6 | 99.7 | 16.30 | 77.9 | 8.73 | 0.000080 | 0.000060 | unchanged |
| set-xx-existing | valkey | 60364 | 59971–60425 | 185.9 | 100.2 | 16.60 | 79.0 | 8.14 | 0.000120 | 0.000060 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 4.99 | 4.99 | 99.0 | unchanged |
| redis | 1.001 | 0.000 | 8.54 | 8.54 | 9.0 | unchanged |
| valkey | 1.001 | 0.000 | 7.97 | 7.97 | 10.0 | unchanged |

</details>

<details>
<summary>single-core-pipeline-16: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 1000000; clients: 8; pipeline: 16; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: 0; client CPUs: 1,2,3.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 1136477 | 1126778–1137489 | 141.3 | 100.1 | 0.88 | 89.8 | 6.54 | 0.000002 | 0.000004 | unchanged |
| ping | redis | 849431 | 841674–849889 | 219.4 | 100.2 | 1.18 | 63.6 | 8.72 | 0.000009 | 0.000005 | unchanged |
| ping | valkey | 781334 | 768957–783244 | 240.9 | 99.9 | 1.28 | 66.9 | 8.30 | 0.000010 | 0.000005 | unchanged |
| set | rillcache | 832786 | 823328–832978 | 188.1 | 100.8 | 1.21 | 66.1 | 6.54 | 0.000001 | 0.000003 | unchanged |
| set | redis | 627370 | 624930–628345 | 307.6 | 99.9 | 1.59 | 66.2 | 8.72 | 0.000008 | 0.000006 | unchanged |
| set | valkey | 556807 | 551047–557182 | 352.4 | 100.2 | 1.80 | 45.6 | 8.30 | 0.000004 | 0.000008 | unchanged |
| get | rillcache | 952729 | 950402–985652 | 166.1 | 99.8 | 1.05 | 78.2 | 6.54 | 0.000001 | 0.000004 | unchanged |
| get | redis | 703621 | 700158–707749 | 271.9 | 99.9 | 1.42 | 65.2 | 8.73 | 0.000004 | 0.000006 | unchanged |
| get | valkey | 635117 | 632962–635664 | 303.8 | 99.7 | 1.57 | 52.2 | 8.30 | 0.000013 | 0.000007 | unchanged |
| set-nx-existing | rillcache | 904311 | 886827–933537 | 176.0 | 100.2 | 1.11 | 70.3 | 6.54 | 0.000001 | 0.000005 | unchanged |
| set-nx-existing | redis | 652861 | 652452–654775 | 296.6 | 99.9 | 1.53 | 50.3 | 8.72 | 0.000006 | 0.000005 | unchanged |
| set-nx-existing | valkey | 620920 | 620447–621895 | 311.7 | 99.9 | 1.61 | 64.0 | 8.30 | 0.000003 | 0.000006 | unchanged |
| set-xx-existing | rillcache | 823782 | 816923–835788 | 190.6 | 100.3 | 1.22 | 63.1 | 6.54 | 0.000001 | 0.000005 | unchanged |
| set-xx-existing | redis | 606085 | 605168–606555 | 322.1 | 100.0 | 1.65 | 58.8 | 8.72 | 0.000001 | 0.000008 | unchanged |
| set-xx-existing | valkey | 542831 | 542784–543971 | 362.4 | 99.9 | 1.84 | 39.1 | 8.30 | 0.000010 | 0.000007 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 5.12 | 5.12 | 99.0 | unchanged |
| redis | 1.001 | 0.000 | 8.49 | 8.49 | 10.0 | unchanged |
| valkey | 1.001 | 0.000 | 8.13 | 8.13 | 9.0 | unchanged |

</details>

### Fixed-worker mode measurements

30/30 separate embedded records validated. 8 clients; 10000 commands/workload/run; pipeline 16; three repetitions. These share the client/server host and inherited CPU affinity; they are **not native competitor comparisons or equal-per-core scaling evidence**. No process CPU/RSS accounting was collected for this group. Fixed lanes have a 1 ms idle polling wait. They actively poll for at most 50 µs after progress before returning to idle parking; this trades active CPU work for fewer premature sleeps, not epoll readiness or free CPU. Multiple lanes also request a cooperative yield after 100 µs of continuous active passes when more than one client is admitted. Progress does not rearm that separate budget; single lanes/lone clients do not periodically yield. This is checked between bounded passes, not a hard deadline or an OS fairness guarantee. The v2 interval restarts after a yield/park returns, excluding that explicit wait; involuntary descheduling can still consume wall time.

| Workload | 1 worker median cmd/s | 1 worker min–max | 4 workers median cmd/s | 4 workers min–max | 1 worker batch p99 µs | 4 workers batch p99 µs |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| ping | 1001849 | 986750–1026410 | 1492266 | 1464562–1505098 | 924.0 | 955.9 |
| set | 716058 | 707193–726927 | 38313 | 37621–38511 | 925.9 | 6265.0 |
| get | 861909 | 838571–879290 | 38443 | 37091–38727 | 946.7 | 6930.9 |
| set-nx-existing | 837806 | 832361–844391 | 38847 | 38487–38892 | 957.7 | 6122.0 |
| set-xx-existing | 766002 | 697658–785307 | 38331 | 37985–38625 | 951.8 | 6333.8 |

### Interpretation limits

- Batch p99 is **not per-request p99**; never divide it by the pipeline length. Percentiles are not pooled across runs.
- CPU counters are tick-quantized; zero observed idle ticks do not prove zero CPU cost. CPU observation windows bracket, but are not identical to, request timers.
- RSS is an approximate endpoint sample, not allocator accounting or a workload peak; VmHWM includes startup, warmup and previous workloads.
- Context switches cover matched thread lifetimes only. Even unchanged endpoints cannot exclude threads born and destroyed between snapshots. PARTIAL coverage is not a process total.
- NX-existing measures rejected writes; XX-existing measures admitted overwrites. This is a small Redis-compatible subset, not equivalent functionality.
