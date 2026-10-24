# Current benchmark performance

Generated: 2026-10-24T19:59:31Z · [Benchmark run #4](https://github.com/macel94/rillcache/actions/runs/40126874559) · [Tested source `2a4b590`](https://github.com/macel94/rillcache/commit/2a4b590f8b8dcd23a39090b3b1ce6e2ed0582926)

**Benchmark job: success. Data: validated. Luna review: unavailable: configure the COPILOT\_GITHUB\_TOKEN repository secret (fine-grained personal token, Copilot Requests permission).**

Benchmark attempt: 1; review attempt: 2. Requested model: `gpt-6-luna` via **GitHub Copilot**.

[This report's permanent copy](https://github.com/macel94/rillcache/blob/main/benchmarks/reports/40126874559-2.md) · [Report history](https://github.com/macel94/rillcache/tree/main/benchmarks/reports) · [Methodology](https://github.com/macel94/rillcache/blob/main/docs/benchmarks.md)

## AI review

**No AI assessment was accepted.** unavailable: configure the COPILOT\_GITHUB\_TOKEN repository secret (fine-grained personal token, Copilot Requests permission). The numeric tables remain independently generated; no other model is substituted.


## Measurements

**Data quality: complete.** 180/180 records validated across 4/4 complete profiles.


Hardware: AMD EPYC 9V45 96-Core Processor. Rust 1.99.1 (58cf8f9c3 2026-10-13).

References: Redis 8.10.2 sha=00000000:0 malloc=jemalloc-5.3.0 bits=64 build=9f59c4119f0e2634; Valkey 9.1.2 sha=00000000:0 malloc=jemalloc-5.3.0 bits=64 build=9501c3c5c2b33eb1.

### SET at a glance

Median commands/s and median of the three runs' **whole-batch p99** values (µs). Positive rate delta means Rillcache is faster in this workload only.

| Profile | Rillcache cmd/s | Redis | Valkey | Rillcache vs Redis | Rillcache p99 µs | Redis p99 | Valkey p99 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| pipeline-1 | 192765 | 105476 | 103973 | +82.8% | 94.6 | 97.3 | 98.0 |
| pipeline-16 | 2549071 | 1106594 | 1018262 | +130.4% | 110.9 | 161.2 | 175.0 |
| single-core-pipeline-1 | 90932 | 105269 | 104551 | -13.6% | 163.1 | 96.9 | 97.8 |
| single-core-pipeline-16 | 1308343 | 1106135 | 1007025 | +18.3% | 182.6 | 161.2 | 179.2 |

CPU is relative to **one logical CPU**, so parallel servers can exceed 100%. Do not blend default-threading and single-core results or different pipelines. Shared VMs and unequal feature sets do not establish a general performance win or a cross-run regression.

<details>
<summary>pipeline-1: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 100000; clients: 8; pipeline: 1; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: inherited; client CPUs: inherited.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 206867 | 205928–213895 | 88.9 | 194.3 | 9.30 | 94.6 | 2.91 | 0.806010 | 0.204520 | unchanged |
| ping | redis | 109347 | 107702–109655 | 93.7 | 100.1 | 9.20 | 91.3 | 13.41 | 0.000160 | 0.000060 | unchanged |
| ping | valkey | 107503 | 107319–108761 | 94.5 | 99.9 | 9.30 | 91.3 | 12.92 | 0.000200 | 0.000030 | unchanged |
| set | rillcache | 192765 | 190457–202257 | 94.6 | 188.4 | 9.60 | 92.6 | 2.91 | 0.833940 | 0.180260 | unchanged |
| set | redis | 105476 | 104479–106644 | 97.3 | 99.2 | 9.40 | 89.4 | 13.41 | 0.000040 | 0.000040 | unchanged |
| set | valkey | 103973 | 103932–104139 | 98.0 | 100.8 | 9.70 | 89.7 | 12.92 | 0.000160 | 0.000030 | unchanged |
| get | rillcache | 192368 | 189105–204920 | 90.2 | 179.5 | 9.40 | 93.7 | 2.91 | 0.847440 | 0.166470 | unchanged |
| get | redis | 105966 | 105685–106779 | 97.0 | 99.6 | 9.40 | 90.3 | 13.41 | 0.000090 | 0.000040 | unchanged |
| get | valkey | 106011 | 105403–106050 | 95.9 | 99.6 | 9.40 | 88.5 | 12.92 | 0.000220 | 0.000020 | unchanged |
| set-nx-existing | rillcache | 203843 | 203491–207113 | 78.8 | 193.5 | 9.50 | 94.7 | 2.91 | 0.875510 | 0.131980 | unchanged |
| set-nx-existing | redis | 105661 | 105045–106834 | 96.4 | 99.8 | 9.50 | 90.2 | 12.86 | 0.000030 | 0.000050 | unchanged |
| set-nx-existing | valkey | 105067 | 105048–105098 | 97.2 | 99.8 | 9.50 | 88.4 | 12.92 | 0.000060 | 0.000030 | unchanged |
| set-xx-existing | rillcache | 195574 | 184398–196241 | 87.1 | 184.3 | 9.60 | 93.8 | 2.91 | 0.838040 | 0.173970 | unchanged |
| set-xx-existing | redis | 105376 | 104062–106015 | 97.8 | 99.9 | 9.50 | 88.4 | 12.86 | 0.000110 | 0.000050 | unchanged |
| set-xx-existing | valkey | 103608 | 103437–105464 | 98.2 | 100.3 | 9.70 | 89.7 | 12.75 | 0.000060 | 0.001270 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 2.38 | 2.38 | 909.7 | unchanged |
| redis | 1.000 | 0.000 | 13.38 | 13.38 | 10.0 | unchanged |
| valkey | 1.000 | 0.000 | 12.92 | 12.92 | 10.0 | unchanged |

</details>

<details>
<summary>pipeline-16: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 1000000; clients: 8; pipeline: 16; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: inherited; client CPUs: inherited.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 3063261 | 2978959–3068958 | 98.9 | 190.0 | 0.62 | 90.3 | 2.95 | 0.051959 | 0.012025 | unchanged |
| ping | redis | 1395300 | 1378391–1399746 | 122.9 | 99.3 | 0.71 | 74.6 | 15.21 | 0.000021 | 0.000006 | unchanged |
| ping | valkey | 1358458 | 1342078–1360414 | 126.7 | 99.3 | 0.73 | 71.2 | 12.81 | 0.000007 | 0.000003 | unchanged |
| set | rillcache | 2549071 | 2380808–2583683 | 110.9 | 213.9 | 0.84 | 71.9 | 2.95 | 0.046010 | 0.017783 | unchanged |
| set | redis | 1106594 | 1101988–1131024 | 161.2 | 99.5 | 0.90 | 61.4 | 15.20 | 0.000007 | 0.000006 | unchanged |
| set | valkey | 1018262 | 1017333–1020292 | 175.0 | 99.8 | 0.98 | 53.1 | 12.81 | 0.000002 | 0.000007 | unchanged |
| get | rillcache | 2667976 | 2629583–2823808 | 100.7 | 189.1 | 0.69 | 79.7 | 2.96 | 0.047571 | 0.016339 | unchanged |
| get | redis | 1213442 | 1205513–1222806 | 144.1 | 100.2 | 0.83 | 65.1 | 15.21 | 0.000002 | 0.000002 | unchanged |
| get | valkey | 1131814 | 1118135–1144654 | 155.8 | 100.7 | 0.89 | 61.8 | 12.81 | 0.000010 | 0.000004 | unchanged |
| set-nx-existing | rillcache | 2957587 | 2795904–2958602 | 102.9 | 206.8 | 0.70 | 80.0 | 2.96 | 0.047234 | 0.016438 | unchanged |
| set-nx-existing | redis | 1136321 | 1101030–1143928 | 159.2 | 99.5 | 0.88 | 61.1 | 15.21 | 0.000018 | 0.000005 | unchanged |
| set-nx-existing | valkey | 1133585 | 1131979–1145207 | 155.6 | 99.6 | 0.88 | 59.1 | 12.81 | 0.000004 | 0.000004 | unchanged |
| set-xx-existing | rillcache | 2617182 | 2437730–2623639 | 109.9 | 220.1 | 0.88 | 73.8 | 2.96 | 0.044666 | 0.019295 | unchanged |
| set-xx-existing | redis | 1076023 | 1017944–1099476 | 167.7 | 100.0 | 0.93 | 56.5 | 15.21 | 0.000009 | 0.000007 | unchanged |
| set-xx-existing | valkey | 992641 | 992294–1003026 | 180.2 | 100.2 | 1.00 | 52.5 | 12.81 | 0.000005 | 0.000005 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 2.34 | 2.34 | 943.8 | unchanged |
| redis | 1.000 | 0.000 | 15.17 | 15.17 | 10.0 | unchanged |
| valkey | 1.000 | 0.000 | 12.81 | 12.81 | 10.0 | unchanged |

</details>

<details>
<summary>single-core-pipeline-1: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 100000; clients: 8; pipeline: 1; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: 0; client CPUs: 1,2,3.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 91942 | 88435–92154 | 161.0 | 100.4 | 11.00 | 96.4 | 2.91 | 1.004150 | 0.006530 | unchanged |
| ping | redis | 108711 | 104608–108896 | 93.0 | 99.3 | 9.20 | 93.4 | 13.27 | 0.000080 | 0.000070 | unchanged |
| ping | valkey | 108002 | 107935–108234 | 93.8 | 99.5 | 9.20 | 91.3 | 12.89 | 0.000110 | 0.000050 | unchanged |
| set | rillcache | 90932 | 88311–91237 | 163.1 | 99.7 | 11.00 | 95.4 | 2.91 | 1.003820 | 0.006810 | unchanged |
| set | redis | 105269 | 102007–105572 | 96.9 | 100.0 | 9.50 | 89.5 | 13.27 | 0.000040 | 0.000050 | unchanged |
| set | valkey | 104551 | 103979–104578 | 97.8 | 100.4 | 9.60 | 88.5 | 12.89 | 0.000060 | 0.000060 | unchanged |
| get | rillcache | 90954 | 90795–91475 | 162.2 | 99.7 | 10.90 | 94.5 | 2.91 | 1.003710 | 0.006880 | unchanged |
| get | redis | 106829 | 105507–107382 | 95.1 | 99.8 | 9.30 | 90.3 | 13.28 | 0.000030 | 0.000050 | unchanged |
| get | valkey | 105890 | 105772–106059 | 97.3 | 99.4 | 9.40 | 90.4 | 12.89 | 0.000030 | 0.000090 | unchanged |
| set-nx-existing | rillcache | 91089 | 91031–91242 | 162.5 | 99.4 | 10.90 | 95.4 | 2.91 | 1.003940 | 0.006450 | unchanged |
| set-nx-existing | redis | 105990 | 104554–106182 | 98.3 | 99.8 | 9.50 | 90.5 | 12.73 | 0.000080 | 0.000100 | unchanged |
| set-nx-existing | valkey | 105649 | 105531–105777 | 96.4 | 100.2 | 9.50 | 89.4 | 12.89 | 0.000030 | 0.000050 | unchanged |
| set-xx-existing | rillcache | 90962 | 90548–91033 | 163.4 | 99.2 | 10.90 | 95.4 | 2.91 | 1.003470 | 0.006970 | unchanged |
| set-xx-existing | redis | 101581 | 100078–104669 | 101.1 | 100.0 | 9.80 | 88.0 | 12.73 | 0.000050 | 0.000040 | unchanged |
| set-xx-existing | valkey | 103806 | 100009–104067 | 98.4 | 99.9 | 9.60 | 90.0 | 12.72 | 0.000020 | 0.000060 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 2.37 | 2.37 | 944.8 | unchanged |
| redis | 1.000 | 0.000 | 13.25 | 13.25 | 10.0 | unchanged |
| valkey | 1.000 | 0.000 | 12.89 | 12.89 | 10.0 | unchanged |

</details>

<details>
<summary>single-core-pipeline-16: all five workloads, CPU, memory and scheduling</summary>

Requests/workload/run: 1000000; clients: 8; pipeline: 16; cyclic keys: 1024; value bytes: 64; warmup/client: 1000; repetitions: 3. Server CPUs: 0; client CPUs: 1,2,3.

| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | rillcache | 1419345 | 1380638–1424907 | 167.3 | 99.7 | 0.70 | 91.8 | 3.00 | 0.062787 | 0.000400 | unchanged |
| ping | redis | 1387746 | 1374543–1388829 | 123.0 | 100.3 | 0.73 | 74.0 | 13.48 | 0.000003 | 0.000004 | unchanged |
| ping | valkey | 1356656 | 1351296–1356806 | 126.4 | 100.3 | 0.74 | 71.6 | 12.97 | 0.000005 | 0.000002 | unchanged |
| set | rillcache | 1308343 | 1297360–1309096 | 182.6 | 99.8 | 0.77 | 81.8 | 3.00 | 0.062810 | 0.000507 | unchanged |
| set | redis | 1106135 | 1099869–1108612 | 161.2 | 100.1 | 0.91 | 57.1 | 13.47 | 0.000008 | 0.000005 | unchanged |
| set | valkey | 1007025 | 1005009–1007469 | 179.2 | 99.7 | 0.99 | 53.5 | 12.97 | 0.000003 | 0.000005 | unchanged |
| get | rillcache | 1318707 | 1312912–1323612 | 181.1 | 99.7 | 0.76 | 84.2 | 3.02 | 0.062742 | 0.000513 | unchanged |
| get | redis | 1208648 | 1208285–1212369 | 145.4 | 100.3 | 0.83 | 63.9 | 13.47 | 0.000003 | 0.000002 | unchanged |
| get | valkey | 1137122 | 1136656–1140171 | 155.7 | 100.3 | 0.88 | 59.8 | 12.97 | 0.000005 | 0.000004 | unchanged |
| set-nx-existing | rillcache | 1332543 | 1330230–1340088 | 178.4 | 99.1 | 0.74 | 85.1 | 3.02 | 0.062808 | 0.000463 | unchanged |
| set-nx-existing | redis | 1144880 | 1140738–1147712 | 154.6 | 100.7 | 0.88 | 59.1 | 13.47 | 0.000002 | 0.000006 | unchanged |
| set-nx-existing | valkey | 1114726 | 1110874–1115583 | 159.7 | 99.2 | 0.89 | 59.6 | 12.97 | 0.000003 | 0.000005 | unchanged |
| set-xx-existing | rillcache | 1293536 | 1290176–1304493 | 185.3 | 100.6 | 0.78 | 80.8 | 3.02 | 0.062847 | 0.000512 | unchanged |
| set-xx-existing | redis | 1076628 | 1075710–1078134 | 165.7 | 100.0 | 0.93 | 63.4 | 13.47 | 0.000003 | 0.000003 | unchanged |
| set-xx-existing | valkey | 976855 | 973499–977642 | 184.3 | 99.7 | 1.02 | 54.9 | 12.97 | 0.000001 | 0.000005 | unchanged |

| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| rillcache | 1.000 | 0.000 | 2.39 | 2.39 | 944.8 | unchanged |
| redis | 1.000 | 0.000 | 13.44 | 13.44 | 10.0 | unchanged |
| valkey | 1.000 | 0.000 | 12.97 | 12.97 | 10.0 | unchanged |

</details>

### Interpretation limits

- Batch p99 is **not per-request p99**; never divide it by the pipeline length. Percentiles are not pooled across runs.
- CPU counters are tick-quantized; zero observed idle ticks do not prove zero CPU cost. CPU observation windows bracket, but are not identical to, request timers.
- RSS is an approximate endpoint sample, not allocator accounting or a workload peak; VmHWM includes startup, warmup and previous workloads.
- Context switches cover matched thread lifetimes only. Even unchanged endpoints cannot exclude threads born and destroyed between snapshots. PARTIAL coverage is not a process total.
- NX-existing measures rejected writes; XX-existing measures admitted overwrites. This is a small Redis-compatible subset, not equivalent functionality.
