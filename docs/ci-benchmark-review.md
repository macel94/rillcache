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

## CPU/RSS/scheduling investigation: 10d56ec

- [Fast CI 40114110305](https://github.com/macel94/rillcache/actions/runs/40114110305):
  successful; **59 fast tests**, five smoke workloads and six core benchmarks.
  The whole verification job took about 18 seconds.
- [Comparison 40114117073](https://github.com/macel94/rillcache/actions/runs/40114117073):
  successful; **180 active records plus 12 idle samples**, eight reference
  conformance invocations, unchanged binary digests across four profiles and
  exactly one link per reference. The comparison job took about 9 minutes 17 seconds.
- All JSON was independently validated, including exact batch counts, tick/time/
  CPU-percentage consistency, PID/start-time identity, RSS and observation windows.
  Every active sample retained unchanged endpoint thread sets: nine Rillcache,
  six Redis or seven Valkey threads, with per-thread context counters matched by
  TID/start time. This does not prove absence of intermediate, unobserved churn.
- The delivered archive was checksummed and its two executables independently
  exercised, including all five profiled workloads and binary-safe NX/XX/error/
  QUIT pipelines. The **server executable is bit-for-bit identical to b654fa6**
  (SHA-256 `61bd3f2f84977a6a9a1b8f6ccd9b6deeb88867289450a7be675ea7cb8ec3a582`).
  This increment changes measurement, not the server implementation.
- [Unmodified evidence](../benchmarks/ci/10d56ec/README.md) is preserved in Git.

Two more harness issues were corrected: with zero warmup, a TCP handshake did not
prove server-side accept/worker setup had completed; every connection now first
validates an untimed PING. Also, previously all three candidates remained alive
while only one was measured. Their idle polling/cron threads could consume the
shared CPU budget. Candidates are now started, measured and reaped individually.
These changes and optional profiling make this a new measurement cohort, not a
clean before/after server optimization experiment.

### Equal-one-logical-CPU SET

Medians of three runs; eight clients, 64-byte values and other parameters as in
the corrected comparison. CPU is observed process user+system seconds divided by
measured commands, not client CPU or function-level profiling. Voluntary context
switches cover matched thread lifetimes over the separate observation window.

| Pipeline | Server | Commands/s | Batch p99, µs | CPU µs/command | Voluntary switches/command |
| --- | --- | ---: | ---: | ---: | ---: |
| 1 | Rillcache | 48,952 | 326.5 | 20.4 | 1.0078 |
| 1 | Redis 8.10.2 | 58,672 | 188.1 | 17.0 | 0.0001 |
| 1 | Valkey 9.1.2 | 57,526 | 192.2 | 17.4 | 0.0001 |
| 16 | Rillcache | 673,558 | 381.8 | 1.48 | 0.0632 |
| 16 | Redis 8.10.2 | 597,827 | 305.6 | 1.67 | <0.0001 |
| 16 | Valkey 9.1.2 | 538,714 | 341.4 | 1.85 | <0.0001 |

This reproduces the important tradeoff: weaker unpipelined throughput/tails;
higher pipelined SET throughput, but worse batch p99. Different shared-VM runs
must not turn these figures into a claimed runtime improvement or regression.

### What the new counters establish

- **Scheduling/I/O deserves priority over speculative sharding.** At pipeline 1,
  even map-free PING consumes about **19.8 µs of Rillcache CPU/command**, versus
  16.0 for Redis and 16.4 for Valkey. About **94.5% of Rillcache's observed PING
  CPU time is system time**; it makes about **1.0075 voluntary switches/command**.
  SET/GET show the same approximately one-switch-per-command pattern. The map
  lock cannot be the sole explanation; the blocking I/O/scheduling path is a
  strong candidate. These counters do not identify a precise kernel hot function
  or establish that every extra switch causes the latency difference.
- **Batching amortizes that pattern.** Pipeline-16 SET has about 0.0632 voluntary
  switches/command, approximately one per 16-command batch, and 1.48 CPU µs/command.
  Its better throughput does not erase the worse tail latency in the table.
- **Unrestricted throughput uses more CPU.** Default-threading pipeline-1 SET
  reaches about 120,355 commands/s using about **188% of one logical CPU**, versus
  about 58,174 at 100% for Redis. This is not an equal-per-core victory.
- **Idle wakeups are measurable.** Across four profiles Rillcache records roughly
  **937–938 voluntary switches/second with no clients**, versus about 10 for each
  reference. Its one-second samples record one CPU tick (about 1% of one CPU);
  references record zero ticks. Tick quantization means neither zero-cost
  references nor a precise idle-CPU ratio can be inferred from these short windows.
- **RSS is now visible, not equivalent functionality.** Single-core SET endpoint
  RSS is about 2.8–2.9 MiB for Rillcache, versus about 12.8–14.8 MiB for Redis and
  12.7–12.8 MiB for Valkey. These are approximate process endpoint readings with
  different allocators/features, not per-workload peaks, allocator accounting or
  a general memory-efficiency claim. VmHWM includes startup and previous work.

Local checks also covered five repeated socket/CLI suites, release tests,
packaging, invalid configuration, stale-result invalidation and success/failure
process cleanup. A local `pidstat -u -r -w -t` smoke cross-check confirmed the
counter scopes and approximate units. A longer local probe exceeded its time
bound on a heavily loaded six-CPU host and was **not** used as performance evidence.

The next runtime experiment should be a separately specified, bounded safe-std
I/O/scheduling change, tested against this cohort's PING/SET/GET controls. Include
no-client and many-idle-client CPU, fairness under large pipelines, backpressure,
all resource limits, joined shutdown, binary safety and multi-key atomicity. Do
not trade these properties for a busy-spin throughput score or bypass the
no-unsafe/no-dependencies policy.

## Reading subsequent runs

Check the source SHA, compiler, CPU model, affinity, client/server order and
reference binary digests before comparing values. Compare matching workloads,
pipeline lengths, profiling modes and CPU-budget profiles; request counts differ
intentionally between pipeline 1 and 16. Read all repetitions, batch-latency spread,
CPU windows and thread coverage, not just the fastest rate. A controlled dedicated-
host baseline, independent client cross-check, finer-grained CPU/memory attribution
and realistic mixed/miss workloads remain future work before a general claim.
