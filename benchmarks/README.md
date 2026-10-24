# Current benchmark performance

Generated: 2026-10-24T21:12:38Z · [Benchmark run #5](https://github.com/macel94/rillcache/actions/runs/40132487946) · [Tested source `3969a2c`](https://github.com/macel94/rillcache/commit/3969a2cd1504d002593a777d5882f0c689f8416e)

**Benchmark job: success. Data: validated. Copilot review: completed.**

Benchmark attempt: 1; review attempt: 1. Preferred model: `gpt-6-luna` via **GitHub Copilot**.

[This report's permanent copy](https://github.com/macel94/rillcache/blob/main/benchmarks/reports/40132487946-1.md) · [Report history](https://github.com/macel94/rillcache/tree/main/benchmarks/reports) · [Methodology](https://github.com/macel94/rillcache/blob/main/docs/benchmarks.md)

## AI review

Actual model(s): **gpt-6-luna**. Selection: automatic-fallback. Authentication: workflow-token.

AI interpretation, **not measurements or a performance gate**. Verify claims against the deterministic tables below.

> Bottom line: Results are workload- and CPU-budget-specific. With inherited CPU affinity, Rillcache’s
> SET throughput was 131k commands/s at pipeline 1 and 1.64m at pipeline 16, versus Redis at 58k and
> 589k. Under a one-logical-CPU server budget, pipeline-1 SET was lower (49k vs Redis 59k), while
> pipeline-16 SET was higher (674k vs 601k). These measurements do not establish a general Redis or
> Valkey victory.
> 
> Tradeoffs: Rillcache’s whole-batch p99 was lower in the inherited-CPU SET profiles: 144 µs vs Redis
> 191 µs at pipeline 1, and 162 µs vs 311 µs at pipeline 16. With one logical CPU, its batch p99 was
> higher: 328 µs vs 189 µs at pipeline 1 and 382 µs vs 305 µs at pipeline 16. These are batch
> percentiles, not per-request latency; pipeline length cannot be used to derive request p99.
> 
> Resource observations: Default-profile CPU can exceed 100% because it is relative to one logical
> CPU: Rillcache used 196.5% for pipeline-1 SET and 232.4% for pipeline-16 SET. In the matched
> one-core profiles, CPU was about 100%; Rillcache’s reported CPU time per SET was 20.4 µs at pipeline
> 1 and 1.48 µs at pipeline 16, versus Redis’s 17.0 and 1.66 µs. Sampled Rillcache RSS was about 2.8
> MiB versus roughly 12.6–13.5 MiB for Redis, but RSS is approximate endpoint data, not workload peak
> or allocator accounting. Rillcache also showed more voluntary context switches per command; these
> cover matched thread lifetimes and do not establish a cause. Zero idle CPU ticks are quantized
> observations, not proof of zero CPU use.
> 
> Uncertainty and next investigations: Three runs per profile on a shared VM, unequal feature sets,
> and differing CPU affinity limit generalization; NX-existing rejects writes, while XX-existing
> admits overwrites. Next: (1) repeat paired, randomized runs with controlled CPU placement and report
> run-level spread; (2) profile pipeline-1 scheduling and batch-tail behavior, where Rillcache’s
> one-core SET throughput and batch p99 both trail the references, without inferring causation from
> switch counters.

## Measurements

**Data quality: complete.** 180/180 records validated across 4/4 complete profiles.


Hardware: AMD EPYC 7763 64-Core Processor. Rust 1.99.1 (58cf8f9c3 2026-10-13).

References: Redis 8.10.2 sha=00000000:0 malloc=jemalloc-5.3.0 bits=64 build=57b923a6b72a5205; Valkey 9.1.2 sha=00000000:0 malloc=jemalloc-5.3.0 bits=64 build=3ab378e28aed4927.

### SET at a glance

Median commands/s and median of the three runs' **whole-batch p99** values (µs). Positive rate delta means Rillcache is faster in this workload only.

| Profile | Rillcache cmd/s | Redis | Valkey | Rillcache vs Redis | Rillcache p99 µs | Redis p99 | Valkey p99 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| pipeline-1 | 131110 | 58203 | 57162 | +125.3% | 144.4 | 190.5 | 194.7 |
| pipeline-16 | 1643485 | 588788 | 543495 | +179.1% | 161.7 | 311.2 | 339.7 |
| single-core-pipeline-1 | 49145 | 58955 | 57676 | -16.6% | 327.6 | 188.6 | 192.1 |
| single-core-pipeline-16 | 673784 | 601174 | 546832 | +12.1% | 381.9 | 305.3 | 336.9 |

CPU is relative to **one logical CPU**, so parallel servers can exceed 100%. Do not blend default-threading and single-core results or different pipelines. Shared VMs and unequal feature sets do not establish a general performance win or a cross-run regression.

<details>
<summary>pipeline-1: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 100000; clients: 8; pipeline: 1; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: inherited; client CPUs: inherited.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 124811 | 121937–130003 | 146.0 | 188.9 | 15.00 | 94.7 | 2.80 | 0.843780 | 0.172950 | unchanged |
| ping | redis | 62666 | 62624–62779 | 176.8 | 100.2 | 16.00 | 85.6 | 13.04 | 0.000110 | 0.000070 | unchanged |
| ping | valkey | 60166 | 60074–60610 | 182.5 | 99.7 | 16.60 | 87.3 | 12.71 | 0.000160 | 0.000050 | unchanged |
| set | rillcache | 131110 | 124469–131577 | 144.4 | 196.5 | 15.00 | 92.7 | 2.80 | 0.821460 | 0.198980 | unchanged |
| set | redis | 58203 | 57708–58933 | 190.5 | 99.8 | 17.20 | 80.3 | 13.04 | 0.000120 | 0.000080 | unchanged |
| set | valkey | 57162 | 57015–57916 | 194.7 | 99.8 | 17.50 | 80.0 | 12.71 | 0.000100 | 0.000100 | unchanged |
| get | rillcache | 132116 | 122396–132576 | 143.2 | 198.0 | 15.00 | 91.9 | 2.80 | 0.837800 | 0.172380 | unchanged |
| get | redis | 59969 | 59670–60427 | 184.3 | 100.2 | 16.60 | 84.9 | 13.04 | 0.000180 | 0.000070 | unchanged |
| get | valkey | 58408 | 58092–58575 | 189.8 | 99.6 | 17.00 | 81.2 | 12.71 | 0.000100 | 0.000090 | unchanged |
| set-nx-existing | rillcache | 125215 | 114212–135344 | 146.4 | 187.7 | 15.00 | 93.1 | 2.80 | 0.837760 | 0.173150 | unchanged |
| set-nx-existing | redis | 59407 | 58325–59489 | 186.7 | 99.8 | 16.80 | 82.6 | 13.04 | 0.000050 | 0.000060 | unchanged |
| set-nx-existing | valkey | 58262 | 58215–60813 | 190.1 | 100.1 | 17.20 | 80.2 | 12.71 | 0.000080 | 0.000130 | unchanged |
| set-xx-existing | rillcache | 127189 | 121411–132498 | 147.5 | 194.4 | 15.30 | 92.6 | 2.80 | 0.835410 | 0.180970 | unchanged |
| set-xx-existing | redis | 57738 | 57308–58321 | 191.9 | 99.9 | 17.30 | 79.8 | 12.70 | 0.000170 | 0.000110 | unchanged |
| set-xx-existing | valkey | 57536 | 56556–57623 | 193.0 | 100.1 | 17.40 | 79.3 | 12.68 | 0.000050 | 0.000050 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 1.000 | 2.26 | 2.26 | 936.8 | unchanged |
| redis | 1.001 | 0.000 | 13.22 | 13.22 | 10.0 | unchanged |
| valkey | 1.001 | 0.000 | 12.86 | 12.86 | 10.0 | unchanged |

</details>

<details>
<summary>pipeline-16: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 1000000; clients: 8; pipeline: 16; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: inherited; client CPUs: inherited.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 2028462 | 1822963–2121574 | 158.6 | 194.5 | 0.96 | 89.4 | 2.84 | 0.049861 | 0.013828 | unchanged |
| ping | redis | 788443 | 776403–800318 | 231.9 | 99.3 | 1.26 | 69.3 | 13.49 | 0.000036 | 0.000012 | unchanged |
| ping | valkey | 746020 | 741806–752150 | 243.5 | 99.9 | 1.34 | 61.5 | 12.83 | 0.000006 | 0.000006 | unchanged |
| set | rillcache | 1643485 | 1638087–1658306 | 161.7 | 232.4 | 1.41 | 68.8 | 2.84 | 0.055654 | 0.008766 | unchanged |
| set | redis | 588788 | 585508–590331 | 311.2 | 99.5 | 1.70 | 56.8 | 12.97 | 0.000005 | 0.000009 | unchanged |
| set | valkey | 543495 | 542529–546278 | 339.7 | 99.9 | 1.84 | 47.3 | 12.70 | 0.000006 | 0.000007 | unchanged |
| get | rillcache | 1687471 | 1642315–1726311 | 159.9 | 200.0 | 1.16 | 76.7 | 2.86 | 0.042426 | 0.020957 | unchanged |
| get | redis | 660301 | 655831–660339 | 275.6 | 99.7 | 1.51 | 56.3 | 12.97 | 0.000002 | 0.000008 | unchanged |
| get | valkey | 614776 | 612811–617690 | 298.0 | 100.5 | 1.63 | 53.0 | 12.70 | 0.000005 | 0.000007 | unchanged |
| set-nx-existing | rillcache | 1833584 | 1766416–1870850 | 157.6 | 210.6 | 1.16 | 77.3 | 2.86 | 0.050409 | 0.013566 | unchanged |
| set-nx-existing | redis | 608171 | 607271–610567 | 300.5 | 100.2 | 1.65 | 53.9 | 12.97 | 0.000005 | 0.000009 | unchanged |
| set-nx-existing | valkey | 594546 | 593916–600333 | 309.0 | 99.9 | 1.68 | 54.8 | 12.70 | 0.000008 | 0.000007 | unchanged |
| set-xx-existing | rillcache | 1645296 | 1626943–1651303 | 166.4 | 235.0 | 1.43 | 69.2 | 2.86 | 0.056711 | 0.007849 | unchanged |
| set-xx-existing | redis | 562895 | 560232–563587 | 327.0 | 100.2 | 1.78 | 49.4 | 12.97 | 0.000006 | 0.000006 | unchanged |
| set-xx-existing | valkey | 529942 | 526931–532704 | 348.4 | 100.1 | 1.89 | 47.4 | 12.70 | 0.000004 | 0.000009 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 1.000 | 2.24 | 2.24 | 936.8 | unchanged |
| redis | 1.001 | 0.000 | 13.46 | 13.46 | 10.0 | unchanged |
| valkey | 1.001 | 0.000 | 12.83 | 12.83 | 10.0 | unchanged |

</details>

<details>
<summary>single-core-pipeline-1: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 100000; clients: 8; pipeline: 1; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: 0; client CPUs: 1,2,3.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 50812 | 50684–51169 | 316.6 | 100.1 | 19.70 | 94.4 | 2.79 | 1.006940 | 0.010720 | unchanged |
| ping | redis | 62565 | 62503–62722 | 174.9 | 100.0 | 16.00 | 85.0 | 12.62 | 0.000260 | 0.000060 | unchanged |
| ping | valkey | 61057 | 60988–61379 | 179.4 | 99.5 | 16.30 | 86.0 | 12.61 | 0.000050 | 0.000060 | unchanged |
| set | rillcache | 49145 | 49050–49190 | 327.6 | 100.0 | 20.40 | 93.6 | 2.79 | 1.007620 | 0.010910 | unchanged |
| set | redis | 58955 | 58701–58962 | 188.6 | 100.2 | 17.00 | 80.5 | 12.62 | 0.000150 | 0.000050 | unchanged |
| set | valkey | 57676 | 57577–57941 | 192.1 | 99.8 | 17.30 | 78.6 | 12.61 | 0.000080 | 0.000050 | unchanged |
| get | rillcache | 49344 | 49329–49706 | 327.1 | 100.1 | 20.30 | 93.5 | 2.79 | 1.007380 | 0.010880 | unchanged |
| get | redis | 60460 | 60370–60564 | 182.0 | 99.7 | 16.50 | 86.1 | 12.62 | 0.000120 | 0.000060 | unchanged |
| get | valkey | 59119 | 58739–59154 | 187.1 | 99.3 | 16.90 | 84.5 | 12.61 | 0.000030 | 0.000090 | unchanged |
| set-nx-existing | rillcache | 49639 | 49472–49666 | 323.0 | 99.8 | 20.10 | 93.5 | 2.79 | 1.007500 | 0.010920 | unchanged |
| set-nx-existing | redis | 59477 | 59401–59563 | 185.4 | 100.4 | 16.90 | 84.6 | 12.62 | 0.000020 | 0.000060 | unchanged |
| set-nx-existing | valkey | 58870 | 58863–58913 | 187.9 | 100.1 | 17.00 | 81.8 | 12.61 | 0.000160 | 0.000060 | unchanged |
| set-xx-existing | rillcache | 49036 | 48928–49226 | 326.7 | 99.8 | 20.40 | 93.1 | 2.79 | 1.008010 | 0.010650 | unchanged |
| set-xx-existing | redis | 58316 | 58233–58389 | 190.4 | 99.8 | 17.10 | 80.7 | 12.59 | 0.000060 | 0.000070 | unchanged |
| set-xx-existing | valkey | 57590 | 57554–57604 | 192.9 | 100.2 | 17.40 | 78.3 | 12.54 | 0.000140 | 0.000090 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 1.000 | 2.26 | 2.26 | 934.7 | unchanged |
| redis | 1.001 | 0.000 | 13.11 | 13.11 | 10.0 | unchanged |
| valkey | 1.001 | 0.000 | 12.75 | 12.75 | 10.0 | unchanged |

</details>

<details>
<summary>single-core-pipeline-16: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 1000000; clients: 8; pipeline: 16; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: 0; client CPUs: 1,2,3.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 763699 | 761493–769022 | 336.0 | 99.9 | 1.31 | 90.8 | 2.86 | 0.062980 | 0.000751 | unchanged |
| ping | redis | 802709 | 801267–802962 | 222.7 | 99.5 | 1.24 | 66.9 | 13.14 | 0.000008 | 0.000007 | unchanged |
| ping | valkey | 748288 | 746994–749876 | 239.9 | 100.1 | 1.34 | 61.2 | 14.77 | 0.000006 | 0.000005 | unchanged |
| set | rillcache | 673784 | 673283–677080 | 381.9 | 100.2 | 1.48 | 79.1 | 2.86 | 0.063111 | 0.000964 | unchanged |
| set | redis | 601174 | 600118–601175 | 305.3 | 99.8 | 1.66 | 51.8 | 12.62 | 0.000005 | 0.000008 | unchanged |
| set | valkey | 546832 | 545544–546919 | 336.9 | 100.0 | 1.83 | 45.1 | 14.62 | 0.000004 | 0.000006 | unchanged |
| get | rillcache | 685250 | 683078–685898 | 373.9 | 100.0 | 1.46 | 81.5 | 2.88 | 0.062970 | 0.000939 | unchanged |
| get | redis | 669464 | 666147–669841 | 271.5 | 99.8 | 1.49 | 63.1 | 12.62 | 0.000008 | 0.000004 | unchanged |
| get | valkey | 604884 | 603771–605105 | 302.2 | 100.2 | 1.66 | 53.3 | 14.62 | 0.000005 | 0.000007 | unchanged |
| set-nx-existing | rillcache | 700174 | 695074–702968 | 369.3 | 99.8 | 1.43 | 82.4 | 2.88 | 0.063086 | 0.000901 | unchanged |
| set-nx-existing | redis | 628845 | 628172–628949 | 290.2 | 99.9 | 1.59 | 53.8 | 12.62 | 0.000004 | 0.000006 | unchanged |
| set-nx-existing | valkey | 597835 | 597122–598248 | 305.2 | 99.9 | 1.67 | 54.8 | 14.62 | 0.000005 | 0.000006 | unchanged |
| set-xx-existing | rillcache | 670011 | 660339–670330 | 387.7 | 100.3 | 1.50 | 77.2 | 2.88 | 0.063184 | 0.000956 | unchanged |
| set-xx-existing | redis | 580754 | 580182–582130 | 316.0 | 100.3 | 1.73 | 57.2 | 12.62 | 0.000002 | 0.000007 | unchanged |
| set-xx-existing | valkey | 534096 | 533123–534206 | 344.6 | 99.9 | 1.87 | 45.7 | 14.62 | 0.000005 | 0.000007 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 2.26 | 2.26 | 936.8 | unchanged |
| redis | 1.001 | 0.000 | 13.11 | 13.11 | 10.0 | unchanged |
| valkey | 1.001 | 0.000 | 14.77 | 14.77 | 10.0 | unchanged |

</details>

### Interpretation limits

- Batch p99 is **not per-request p99**; never divide it by the pipeline length. Percentiles are not pooled across runs.
- CPU counters are tick-quantized; zero observed idle ticks do not prove zero CPU cost. CPU observation windows bracket, but are not identical to, request timers.
- RSS is an approximate endpoint sample, not allocator accounting or a workload peak; VmHWM includes startup, warmup and previous workloads.
- Context switches cover matched thread lifetimes only. Even unchanged endpoints cannot exclude threads born and destroyed between snapshots. PARTIAL coverage is not a process total.
- NX-existing measures rejected writes; XX-existing measures admitted overwrites. This is a small Redis-compatible subset, not equivalent functionality.
