# Current benchmark performance

Generated: 2026-11-28T16:25:42Z · [Benchmark run #27](https://github.com/macel94/rillcache/actions/runs/46163657327) · [Tested source `4c2c247`](https://github.com/macel94/rillcache/commit/4c2c24718584a0f95ee208abf191230f663893e1)

**Benchmark job: success. Data: validated. Copilot review: completed.**

Benchmark attempt: 1; review attempt: 1. Preferred model: `gpt-6-luna` via **GitHub Copilot**.

[This report's permanent copy](https://github.com/macel94/rillcache/blob/main/benchmarks/reports/46163657327-1.md) · [Report history](https://github.com/macel94/rillcache/tree/main/benchmarks/reports) · [Methodology](https://github.com/macel94/rillcache/blob/main/docs/benchmarks.md)

## AI review

Actual model(s): **mai-code-2-flash**. Selection: automatic-fallback. Authentication: workflow-token.

AI interpretation, **not measurements or a performance gate**. Verify claims against the deterministic tables below.

> Bottom line: Rillcache led SET throughput and whole-batch p99 in both native modes and under one-CPU
> affinity; fixed-worker results are a separate 30-record group and support no native-cohort
> comparison. This is a workload-, feature-, and configuration-specific result, not a general
> Redis/Valkey or Rust conclusion.
> 
> Throughput and tail tradeoffs
> 
> Native SET comparisons (180-record cohort):
> 
> ┌───────────────────────┬──────────────┬───────────┬─────────┬─────────────────────────────────┐
> │ Budget                │ Rillcache    │ Redis     │ Valkey  │ Rillcache SET p99               │
> ├───────────────────────┼──────────────┼───────────┼─────────┼─────────────────────────────────┤
> │ Default, pipeline 1   │ 130,622      │ 119,008   │ 118,526 │ 74.3 µs; Redis 105.5; Valkey    │
> │                       │ cmd/s        │           │         │ 106.8                           │
> ├───────────────────────┼──────────────┼───────────┼─────────┼─────────────────────────────────┤
> │ Default, pipeline 16  │ 1,336,057    │ 1,092,980 │ 976,177 │ 113.0 µs; Redis 187.4; Valkey   │
> │                       │ cmd/s        │           │         │ 209.3                           │
> ├───────────────────────┼──────────────┼───────────┼─────────┼─────────────────────────────────┤
> │ 1 logical CPU,        │ 127,487      │ 119,852   │ 112,701 │ 76.1 µs; Redis 106.3; Valkey    │
> │ pipeline 1            │ cmd/s        │           │         │ 111.1                           │
> ├───────────────────────┼──────────────┼───────────┼─────────┼─────────────────────────────────┤
> │ 1 logical CPU,        │ 1,339,039    │ 1,081,841 │ 968,617 │ 117.6 µs; Redis 189.7; Valkey   │
> │ pipeline 16           │ cmd/s        │           │         │ 210.9                           │
> └───────────────────────┴──────────────┴───────────┴─────────┴─────────────────────────────────┘
> 
> Rillcache’s SET gain was 6.4–9.8% at pipeline 1 and 22.2–23.8% at pipeline 16, with lower median
> whole-batch p99 in every SET profile. These are batch percentiles; 117.6 µs is not a per-request
> latency, and it must not be divided by pipeline 16. Default-threading and equal-one-CPU results
> remain separate. NX-existing rejected writes and XX-existing admitted overwrites; they are not
> identical operations.
> 
> CPU, memory, and scheduling
> 
> Server CPU percentages remained near 99–101% of the observation CPU window, so these measurements
> are consistent with saturated or approximately equal single-CPU demand. Rillcache SET used 7.6–7.9
> CPU µs/cmd at pipeline 1 versus 8.3–8.9 µs/cmd for the references; pipeline-16 values were 0.75–0.76
> versus 0.92–1.03 µs/cmd. Idle 1-second CPU readbacks were zero and cannot establish zero idle cost
> because counters are tick-quantized.
> 
> Active-RSS endpoints were approximately 6.86–7.23 MiB for Rillcache versus 8.80–9.11 MiB for Redis
> and 8.23–8.38 MiB for Valkey. RSS is endpoint-only, not a workload peak. Context-switch rates were
> generally lower for Rillcache, but matched-lifetime switching data and unchanged thread endpoints do
> not prove process-total or thread-lifecycle identity.
> 
> Fixed-worker mode
> 
> The embedded fixed-worker group was fast for ping—1.507M cmd/s at one worker and 2.076M at four—but
> SET, GET, and existing-key operations fell sharply at four workers, from 1.106–1.137M to 21.9–22.9k
> cmd/s, with whole-batch p99 rising to 19.9–28.3 ms. It does not provide native or equal-core
> evidence.
> 
> Next investigations:
> 
>  1. Repeat SET/GET/NX/XX with 1, 2, 4, and 8 fixed lanes while recording CPU and RSS, to locate
> whether the four-lane degradation is workload-, coordination-, or scheduling-specific.
>  2. Expand native equal-core runs across short, large, and mixed values plus larger keyspaces,
> retaining batch and request-level percentiles, since the current 64-byte/1024-key SET sample cannot
> establish broader tail behavior.

## Measurements

**Data quality: complete.** 180/180 records validated across 4/4 complete profiles.


Hardware: Intel(R) Xeon(R) 6973P-C. Rust 1.100.0 (0ef1bc386 2026-11-09).

References: Redis 8.10.2 sha=00000000:0 malloc=jemalloc-5.3.0 bits=64 build=a32e75510c13794b; Valkey 9.1.2 sha=00000000:0 malloc=jemalloc-5.3.0 bits=64 build=2abd0cfa8da125d9.

### SET at a glance

Median commands/s and median of the three runs' **whole-batch p99** values (µs). Positive rate delta means Rillcache is faster in this workload only.

| Profile | Rillcache cmd/s | Redis | Valkey | Rillcache vs Redis | Rillcache p99 µs | Redis p99 | Valkey p99 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| pipeline-1 | 130622 | 119008 | 118526 | +9.8% | 74.3 | 105.5 | 106.8 |
| pipeline-16 | 1336057 | 1092980 | 976177 | +22.2% | 113.0 | 187.4 | 209.3 |
| single-core-pipeline-1 | 127487 | 119852 | 112701 | +6.4% | 76.1 | 106.3 | 111.1 |
| single-core-pipeline-16 | 1339039 | 1081841 | 968617 | +23.8% | 117.6 | 189.7 | 210.9 |

CPU is relative to **one logical CPU**, so parallel servers can exceed 100%. Do not blend default-threading and single-core results or different pipelines. Shared VMs and unequal feature sets do not establish a general performance win or a cross-run regression.

<details>
<summary>pipeline-1: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 100000; clients: 8; pipeline: 1; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: inherited; client CPUs: inherited.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 139307 | 138824–141343 | 68.7 | 100.3 | 7.20 | 90.3 | 6.86 | 0.000010 | 0.000050 | unchanged |
| ping | redis | 125255 | 119783–125618 | 103.5 | 99.4 | 8.00 | 86.3 | 9.06 | 0.000260 | 0.000050 | unchanged |
| ping | valkey | 122998 | 122714–123495 | 102.7 | 99.6 | 8.10 | 87.7 | 8.33 | 0.000240 | 0.000040 | unchanged |
| set | rillcache | 130622 | 130500–130764 | 74.3 | 99.4 | 7.60 | 84.4 | 6.86 | 0.000020 | 0.000040 | unchanged |
| set | redis | 119008 | 117096–121423 | 105.5 | 99.5 | 8.40 | 84.1 | 9.08 | 0.000210 | 0.000030 | unchanged |
| set | valkey | 118526 | 117271–125297 | 106.8 | 99.5 | 8.40 | 80.0 | 8.36 | 0.000620 | 0.000060 | unchanged |
| get | rillcache | 133086 | 128146–135925 | 75.0 | 100.6 | 7.40 | 86.5 | 6.86 | 0.000010 | 0.000060 | unchanged |
| get | redis | 123303 | 122412–124123 | 103.9 | 99.1 | 8.00 | 83.8 | 9.10 | 0.000400 | 0.000030 | unchanged |
| get | valkey | 117670 | 113921–119753 | 106.8 | 100.0 | 8.50 | 83.5 | 8.36 | 0.000200 | 0.000060 | unchanged |
| set-nx-existing | rillcache | 131264 | 131210–133083 | 75.2 | 99.8 | 7.60 | 84.0 | 6.86 | 0.000010 | 0.000090 | unchanged |
| set-nx-existing | redis | 120954 | 107245–147767 | 103.0 | 99.7 | 8.30 | 82.8 | 9.11 | 0.000320 | 0.000050 | unchanged |
| set-nx-existing | valkey | 115648 | 112152–119940 | 108.3 | 99.5 | 8.60 | 81.9 | 8.36 | 0.000130 | 0.000040 | unchanged |
| set-xx-existing | rillcache | 127840 | 127068–134419 | 78.8 | 100.4 | 7.90 | 84.8 | 6.86 | 0.000010 | 0.000070 | unchanged |
| set-xx-existing | redis | 120172 | 119272–120378 | 105.8 | 99.0 | 8.30 | 81.0 | 9.07 | 0.000380 | 0.000050 | unchanged |
| set-xx-existing | valkey | 116991 | 116295–117431 | 106.6 | 100.6 | 8.60 | 80.2 | 8.34 | 0.000130 | 0.000050 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 5.62 | 5.62 | 99.0 | unchanged |
| redis | 1.000 | 0.000 | 8.87 | 8.87 | 10.0 | unchanged |
| valkey | 1.000 | 0.000 | 8.18 | 8.18 | 10.0 | unchanged |

</details>

<details>
<summary>pipeline-16: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 1000000; clients: 8; pipeline: 16; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: inherited; client CPUs: inherited.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 1945924 | 1912592–1960022 | 78.2 | 101.2 | 0.52 | 78.8 | 6.96 | 0.000001 | 0.000004 | unchanged |
| ping | redis | 1451205 | 1448366–1475496 | 141.2 | 101.6 | 0.69 | 66.7 | 9.02 | 0.000035 | 0.000003 | unchanged |
| ping | valkey | 1359263 | 1357977–1395569 | 150.9 | 100.5 | 0.74 | 63.9 | 8.34 | 0.000006 | 0.000003 | unchanged |
| set | rillcache | 1336057 | 1318980–1348661 | 113.0 | 100.2 | 0.75 | 60.0 | 6.96 | 0.000002 | 0.000003 | unchanged |
| set | redis | 1092980 | 1078906–1093549 | 187.4 | 99.4 | 0.92 | 53.8 | 9.00 | 0.000021 | 0.000005 | unchanged |
| set | valkey | 976177 | 973543–980208 | 209.3 | 100.3 | 1.03 | 49.0 | 8.37 | 0.000008 | 0.000005 | unchanged |
| get | rillcache | 1612918 | 1597571–1631512 | 96.4 | 99.0 | 0.61 | 69.4 | 6.96 | 0.000001 | 0.000004 | unchanged |
| get | redis | 1196740 | 1193028–1200282 | 171.8 | 100.5 | 0.84 | 61.4 | 9.01 | 0.000008 | 0.000003 | unchanged |
| get | valkey | 1110846 | 1053761–1114736 | 184.9 | 100.0 | 0.90 | 54.4 | 8.38 | 0.000004 | 0.000007 | unchanged |
| set-nx-existing | rillcache | 1480290 | 1402162–1583248 | 103.9 | 99.2 | 0.67 | 65.7 | 6.96 | 0.000002 | 0.000004 | unchanged |
| set-nx-existing | redis | 1119275 | 1116365–1127174 | 183.0 | 100.5 | 0.90 | 55.6 | 9.02 | 0.000007 | 0.000006 | unchanged |
| set-nx-existing | valkey | 1068988 | 1065695–1069285 | 192.1 | 99.4 | 0.93 | 52.7 | 8.38 | 0.000008 | 0.000006 | unchanged |
| set-xx-existing | rillcache | 1308602 | 1271587–1311519 | 115.2 | 100.7 | 0.77 | 61.0 | 6.96 | 0.000002 | 0.000004 | unchanged |
| set-xx-existing | redis | 1053460 | 1048735–1055846 | 194.0 | 99.6 | 0.95 | 48.4 | 9.02 | 0.000026 | 0.000003 | unchanged |
| set-xx-existing | valkey | 953394 | 952517–955084 | 214.3 | 100.1 | 1.05 | 44.8 | 8.38 | 0.000009 | 0.000004 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 5.54 | 5.54 | 99.0 | unchanged |
| redis | 1.000 | 0.000 | 8.76 | 8.76 | 10.0 | unchanged |
| valkey | 1.000 | 0.000 | 8.18 | 8.18 | 10.0 | unchanged |

</details>

<details>
<summary>single-core-pipeline-1: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 100000; clients: 8; pipeline: 1; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: 0; client CPUs: 1,2,3.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 136075 | 133932–136424 | 70.5 | 100.4 | 7.40 | 89.0 | 6.90 | 0.000010 | 0.000050 | unchanged |
| ping | redis | 123659 | 122775–124948 | 102.8 | 100.1 | 8.10 | 85.2 | 8.80 | 0.000270 | 0.000050 | unchanged |
| ping | valkey | 122674 | 109643–123039 | 103.1 | 99.7 | 8.20 | 85.4 | 8.23 | 0.000350 | 0.000040 | unchanged |
| set | rillcache | 127487 | 127007–127998 | 76.1 | 100.7 | 7.90 | 86.1 | 6.90 | 0.000040 | 0.000040 | unchanged |
| set | redis | 119852 | 119584–120224 | 106.3 | 99.8 | 8.30 | 80.7 | 8.81 | 0.000110 | 0.000030 | unchanged |
| set | valkey | 112701 | 106223–116407 | 111.1 | 100.3 | 8.90 | 82.0 | 8.23 | 0.000220 | 0.000040 | unchanged |
| get | rillcache | 133028 | 131798–133343 | 74.2 | 101.1 | 7.60 | 88.2 | 6.90 | 0.000010 | 0.000040 | unchanged |
| get | redis | 122135 | 108576–122342 | 104.1 | 99.1 | 8.10 | 83.7 | 8.83 | 0.000200 | 0.000050 | unchanged |
| get | valkey | 118891 | 112530–119925 | 106.5 | 99.8 | 8.40 | 81.9 | 8.25 | 0.000100 | 0.000040 | unchanged |
| set-nx-existing | rillcache | 131045 | 130513–131814 | 73.9 | 100.2 | 7.60 | 85.9 | 6.90 | 0.000010 | 0.000050 | unchanged |
| set-nx-existing | redis | 116614 | 108894–119846 | 107.4 | 100.3 | 8.60 | 81.4 | 8.84 | 0.000170 | 0.000040 | unchanged |
| set-nx-existing | valkey | 116110 | 108423–118721 | 108.7 | 100.8 | 8.70 | 81.0 | 8.25 | 0.000250 | 0.000040 | unchanged |
| set-xx-existing | rillcache | 128685 | 125703–128719 | 75.8 | 99.1 | 7.70 | 84.4 | 6.90 | 0.000010 | 0.000040 | unchanged |
| set-xx-existing | redis | 118533 | 105649–119176 | 107.3 | 100.1 | 8.40 | 82.1 | 8.80 | 0.000340 | 0.000040 | unchanged |
| set-xx-existing | valkey | 115819 | 115730–116485 | 108.7 | 99.6 | 8.60 | 79.1 | 8.23 | 0.000240 | 0.000040 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 5.54 | 5.54 | 99.0 | unchanged |
| redis | 1.000 | 0.000 | 8.59 | 8.59 | 10.0 | unchanged |
| valkey | 1.000 | 0.000 | 8.06 | 8.06 | 10.0 | unchanged |

</details>

<details>
<summary>single-core-pipeline-16: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 1000000; clients: 8; pipeline: 16; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: 0; client CPUs: 1,2,3.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 1883350 | 1860459–1894218 | 84.7 | 100.4 | 0.54 | 81.5 | 7.23 | 0.000001 | 0.000001 | unchanged |
| ping | redis | 1442402 | 1439534–1443461 | 142.7 | 100.9 | 0.70 | 67.1 | 8.86 | 0.000026 | 0.000004 | unchanged |
| ping | valkey | 1335883 | 1335627–1336579 | 153.8 | 100.2 | 0.75 | 64.0 | 8.23 | 0.000004 | 0.000003 | unchanged |
| set | rillcache | 1339039 | 1279289–1351914 | 117.6 | 100.0 | 0.76 | 57.9 | 7.23 | 0.000007 | 0.000004 | unchanged |
| set | redis | 1081841 | 1080144–1083408 | 189.7 | 100.6 | 0.93 | 54.8 | 8.84 | 0.000014 | 0.000004 | unchanged |
| set | valkey | 968617 | 961921–970525 | 210.9 | 99.9 | 1.03 | 47.6 | 8.26 | 0.000015 | 0.000004 | unchanged |
| get | rillcache | 1592940 | 1587369–1610742 | 99.2 | 100.0 | 0.63 | 68.3 | 7.23 | 0.000001 | 0.000003 | unchanged |
| get | redis | 1185137 | 1177771–1187876 | 173.8 | 100.7 | 0.85 | 61.2 | 8.86 | 0.000014 | 0.000004 | unchanged |
| get | valkey | 1098858 | 1098042–1100663 | 186.4 | 98.9 | 0.90 | 57.1 | 8.27 | 0.000019 | 0.000003 | unchanged |
| set-nx-existing | rillcache | 1463206 | 1450914–1540934 | 105.6 | 100.1 | 0.68 | 64.3 | 7.23 | 0.000008 | 0.000003 | unchanged |
| set-nx-existing | redis | 1108208 | 1105571–1109724 | 184.8 | 101.0 | 0.91 | 57.8 | 8.86 | 0.000015 | 0.000002 | unchanged |
| set-nx-existing | valkey | 1061353 | 1038918–1061476 | 193.0 | 99.7 | 0.95 | 50.0 | 8.27 | 0.000007 | 0.000004 | unchanged |
| set-xx-existing | rillcache | 1283835 | 1259495–1286282 | 119.1 | 100.3 | 0.78 | 59.7 | 7.23 | 0.000002 | 0.000002 | unchanged |
| set-xx-existing | redis | 1048849 | 1047198–1048963 | 195.6 | 99.6 | 0.95 | 48.4 | 8.86 | 0.000023 | 0.000005 | unchanged |
| set-xx-existing | valkey | 943218 | 942386–946610 | 216.9 | 100.0 | 1.06 | 44.3 | 8.27 | 0.000007 | 0.000006 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 5.64 | 5.64 | 99.0 | unchanged |
| redis | 1.000 | 0.000 | 8.60 | 8.60 | 10.0 | unchanged |
| valkey | 1.000 | 0.000 | 8.08 | 8.08 | 10.0 | unchanged |

</details>

### Fixed-worker mode measurements

30/30 separate embedded records validated. 8 clients; 10000 commands/workload/run; pipeline 16; three repetitions. These share the client/server host and inherited CPU affinity; they are **not native competitor comparisons or equal-per-core scaling evidence**. No process CPU/RSS accounting was collected for this group. Fixed lanes have a 1 ms idle polling wait. They actively poll for at most 50 µs after progress before returning to idle parking; this trades active CPU work for fewer premature sleeps, not epoll readiness or free CPU. Multiple lanes also request a cooperative yield after 100 µs of continuous active passes when more than one client is admitted. Progress does not rearm that separate budget; single lanes/lone clients do not periodically yield. This is checked between bounded passes, not a hard deadline or an OS fairness guarantee. The v2 interval restarts after a yield/park returns, excluding that explicit wait; involuntary descheduling can still consume wall time.

| Workload | 1 worker median cmd/s | 1 worker min–max | 4 workers median cmd/s | 4 workers min–max | 1 worker batch p99 µs | 4 workers batch p99 µs |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| ping | 1507206 | 1431064–1577032 | 2075743 | 2042281–2524612 | 983.0 | 946.9 |
| set | 1136477 | 1122302–1164945 | 22881 | 22462–32330 | 997.9 | 19866.2 |
| get | 1259345 | 1257750–1386894 | 22983 | 21972–42720 | 989.6 | 28297.0 |
| set-nx-existing | 1210175 | 1189208–1245096 | 22213 | 22156–23065 | 988.8 | 26054.2 |
| set-xx-existing | 1106700 | 1027837–1119424 | 21901 | 21768–23072 | 975.2 | 22246.8 |

### Interpretation limits

- Batch p99 is **not per-request p99**; never divide it by the pipeline length. Percentiles are not pooled across runs.
- CPU counters are tick-quantized; zero observed idle ticks do not prove zero CPU cost. CPU observation windows bracket, but are not identical to, request timers.
- RSS is an approximate endpoint sample, not allocator accounting or a workload peak; VmHWM includes startup, warmup and previous workloads.
- Context switches cover matched thread lifetimes only. Even unchanged endpoints cannot exclude threads born and destroyed between snapshots. PARTIAL coverage is not a process total.
- NX-existing measures rejected writes; XX-existing measures admitted overwrites. This is a small Redis-compatible subset, not equivalent functionality.
