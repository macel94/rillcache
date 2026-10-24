# CI benchmark review

## Bootstrap: ed15e2f

Actual GitHub evidence, not local timings:

- [Fast CI run 40098539251](https://github.com/macel94/rillcache/actions/runs/40098539251):
  successful; the `make ci` step took about 3 seconds, whole job about 21 seconds.
- [Comparison run 40098536127](https://github.com/macel94/rillcache/actions/runs/40098536127):
  successful; whole job about 5 minutes 40 seconds, including native reference builds.
- All 38 bootstrap fast tests and supported-subset reference checks passed.
  JSON request/batch counts, positive elapsed time and ordered batch percentiles
  were checked against the downloaded artifacts. No wrong replies, incomplete
  operations, protocol failures or correctness regression were observed.

Comparison parameters: 100,000 commands per workload, 8 clients, 1024 cyclic keys,
64-byte values, 1000 warmup commands/client, 3 repetitions, loopback, no RDB/AOF.
Median throughput across the three runs, rounded to operations/second:

| Pipeline | Server | PING | SET | GET |
| --- | --- | ---: | ---: | ---: |
| 1 | Rillcache | 131,260 | 122,774 | 123,995 |
| 1 | Redis 8.10.2 | 63,329 | 59,811 | 61,518 |
| 1 | Valkey 9.1.2 | 62,076 | 59,315 | 60,336 |
| 16 | Rillcache | 1,783,653 | 1,497,398 | 1,580,693 |
| 16 | Redis 8.10.2 | 801,232 | 605,454 | 673,380 |
| 16 | Valkey 9.1.2 | 748,603 | 547,266 | 614,966 |

These are observations for a narrow workload, **not proof that Rust or this
implementation generally beats Redis/Valkey**.

### What needed fixing or qualification

1. **Unnecessary reference rebuilds.** CI logs show both references recompiled
   `release.o` and relinked between pipeline profiles; their recorded binary
   digests changed. The upstream version script runs `git diff ../src ../deps`.
   Outside Git, that command can become a no-index directory comparison and
   count generated build output as dirty-source metadata. Parent-Git exclusion
   alone is insufficient. The corrected cache extracts each verified archive once
   and creates isolated, empty Git metadata (no staging or commits). The archive
   SHA-256 remains source provenance, not this synthetic Git metadata. Cross-profile
   binary digest checks now catch accidental relinks or changed executables.
2. **Unequal server parallelism.** Rillcache runs a worker per client; the
   competitors use their default I/O-thread configuration. With four available
   logical CPUs, the apparent roughly 2x lead conflates protocol/engine efficiency
   with parallel scheduling. Equal single-core server budgets, with clients on
   different CPUs, are now measured as a separate profile. The unrestricted
   profile remains useful, but answers a different question.
3. **Short samples and fixed order.** Pipelined Rillcache runs lasted only
   45–71 ms and varied by up to roughly 28% between the minimum and maximum PING
   throughput. The corrected pipeline-16 workload uses 1,000,000 commands rather
   than 100,000, with rotated server order. This reduces, but does not eliminate,
   scheduling/boost/order effects on shared runners. Smoke samples are only
   1–2 ms and must remain correctness checks, not performance acceptance tests.
4. **Different machines across jobs.** The fast-CI runner reported AMD EPYC 9V74;
   the comparison runner reported EPYC 7763. Do not compare the embedded smoke's
   absolute numbers to standalone comparison numbers or infer a regression from
   different job/host timings.
5. **Stale local outputs on failure.** The comparison directory previously kept
   an untouched server's old JSON when a rerun failed before reaching it. Known
   result/log files are now invalidated before a new comparison starts.

### Implementation conclusions

The reviewed CI data does not identify a functional defect in the implemented
subset. It also does not justify speculative sharding or unsafe/native I/O.
The global `RwLock`, argument allocation, value copies and thread-per-client
scaling remain profiling candidates. A 64-byte, cyclic-key, all-hit workload is
not production traffic or a test of the complete Redis feature set.

The next implemented story is atomic conditional SET (`NX`/`XX`), with racing-client,
binary/empty data, invalid-option and payload-budget tests. Its rejection/update
paths have reply-validated TCP and core benchmarks. No unsafe project code or
external Rust dependency is permitted to obtain better numbers.

## Reading subsequent runs

Check the source SHA, compiler, CPU model, affinity, client/server order and
reference binary digests before comparing values. Compare matching workloads,
pipeline lengths and CPU-budget profiles; request counts differ intentionally
between pipeline 1 and 16. Read all repetitions and batch-latency spread, not
just the fastest rate. A controlled dedicated-host baseline, independent client
cross-check, CPU/RSS measurement and realistic mixed/miss workloads remain future
work before a general performance claim.
