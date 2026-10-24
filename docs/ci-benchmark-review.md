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

The follow-up implemented atomic conditional SET (`NX`/`XX`), with racing-client,
binary/empty data, invalid-option and payload-budget tests. Its rejection/update
paths have reply-validated TCP and core benchmarks. No unsafe project code or
external Rust dependency is permitted to obtain better numbers.

## Corrected comparison: b654fa6

- [Fast CI 40103835947](https://github.com/macel94/rillcache/actions/runs/40103835947):
  successful; 47 fast tests, five TCP smoke workloads and six core benchmarks.
  `make ci` still took about 3 seconds; the whole job about 21 seconds.
- [Comparison 40103838600](https://github.com/macel94/rillcache/actions/runs/40103838600):
  successful; four profiles, five workloads, three runs per server: **180 validated
  records**. All eight current-reference conformance invocations passed.
- Reference build logs contain exactly one server link per competitor, exclusively
  in the dedicated build step. Both binary digests stayed identical across all
  four profiles. Native builds took about 3 minutes 21 seconds; the expanded
  comparison job about 9 minutes, deliberately outside the fast feedback path.
- The delivered CI archive's checksum was verified and its extracted executable
  independently exercised with binary keys, NX/XX, rejected writes, invalid
  combinations and a pipeline. All replies matched.
- [Unmodified corrected evidence](../benchmarks/ci/b654fa6/README.md) and
  [original evidence](../benchmarks/ci/ed15e2f/README.md) are preserved in Git rather
  than relying only on expiring Actions artifacts.

The comparison host again reported AMD EPYC 7763 and four available logical CPUs.
The single-core profiles restricted every server to logical CPU 0 and clients to
CPUs 1–3; actual affinity and client CPU-count fields were checked. These are
logical-CPU budgets, not isolated physical cores; SMT/shared-host interference
still applies. Pipeline 1 used 100,000 commands; pipeline 16 used 1,000,000.
Other workload parameters match the bootstrap comparison.

### Equal-budget results

Median throughput, operations/second; three repetitions:

| Pipeline | Server | PING | SET | GET |
| --- | --- | ---: | ---: | ---: |
| 1 | Rillcache | 50,185 | 48,827 | 48,645 |
| 1 | Redis 8.10.2 | 60,555 | 57,031 | 58,587 |
| 1 | Valkey 9.1.2 | 59,290 | 55,889 | 57,563 |
| 16 | Rillcache | 756,589 | 671,622 | 681,666 |
| 16 | Redis 8.10.2 | 777,299 | 573,319 | 650,088 |
| 16 | Valkey 9.1.2 | 728,221 | 529,268 | 596,916 |

Median of the three runs' SET **batch p99** values, microseconds:

| Pipeline | Rillcache | Redis | Valkey |
| --- | ---: | ---: | ---: |
| 1 | 333.7 | 204.9 | 208.9 |
| 16 | 385.8 | 334.1 | 362.4 |

This reveals a genuine performance limitation, not a protocol correctness failure:

- At pipeline 1, Rillcache's SET throughput is about **14% below Redis** and **13%
  below Valkey** under the equal one-logical-CPU budget, with substantially worse
  tail latency. Its original default-threading lead is not a per-core victory.
- At pipeline 16, SET throughput is about **17% above Redis** and **27% above
  Valkey** for this narrow workload, but batch p99 is still worse. GET's lead over
  Redis is only about 5%, and PING slightly trails Redis. No general win is established.
- The unrestricted profile still shows high throughput (about 1.65M SET/s), but
  answers an out-of-box parallel-throughput question, not isolated per-core latency.
- PING also slows under the one-CPU budget even though it does not access the key
  map. Therefore the shared `RwLock` cannot be the sole explanation. The blocking
  thread-per-connection model, scheduling/context switches and I/O fairness are
  strong profiling candidates; these measurements do **not** prove a precise hot
  function or justify speculative sharding.
- Longer pipelined runs reduced variability: unrestricted PING's observed
  min-to-max throughput spread was roughly 3%, versus roughly 28% in the short
  bootstrap sample. That is better evidence, not a dedicated-hardware guarantee.
- The fast-CI microbenchmark host changed from EPYC 9V74 to EPYC 7763, and the
  store benchmark harness now validates replies using a shared helper. Do not
  interpret the old/new raw nanoseconds/op as a clean engine regression measurement.

The next performance story should measure server CPU, RSS and scheduling/context
switches alongside PING/SET/GET before selecting a bounded, safe-standard-library
I/O change. It must preserve binary safety, connection/frame/payload limits,
backpressure, shutdown and multi-key atomicity, and measure idle CPU plus tail
latency rather than maximizing a single throughput number. Unsafe code, FFI or
external Rust dependencies are not an escape hatch.

## Reading subsequent runs

Check the source SHA, compiler, CPU model, affinity, client/server order and
reference binary digests before comparing values. Compare matching workloads,
pipeline lengths and CPU-budget profiles; request counts differ intentionally
between pipeline 1 and 16. Read all repetitions and batch-latency spread, not
just the fastest rate. A controlled dedicated-host baseline, independent client
cross-check, CPU/RSS measurement and realistic mixed/miss workloads remain future
work before a general performance claim.
