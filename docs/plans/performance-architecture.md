# Performance architecture: measured gaps and candidate designs

**Status: RESEARCH. This document changes no runtime behavior.** It records where
Rillcache lost to Redis and Valkey, what those projects actually do on the same
paths, which designs the repository adopted or rejected under its safety and
dependency policy, and what evidence was required. The bounded Mio readiness
reactor merged as PR #139 (`4a52fc0`) after passing both protected gates on its
merge candidate (section 2.3); this document merged as PR #141 (`f572bd8`).
Later increments added opt-in Tokio transport, multi-owner execution, cached
mode/authorization words and batched owner replies; the required four-CPU job now
also compares Dragonfly (sections 2.5–2.6). A shared-store socket-worker
experiment was measured and rejected (section 2.7); owner-migration socket workers
merged as PR #159 (`31a6006`) after both four-CPU gates passed (section 4.8).
Round-robin accept distribution then failed the Dragonfly gate at the old
`min(4, CPUs)` count (PR #161), and the follow-up `2 × CPUs` count, capped at
eight, failed both four-CPU gates on pipeline-1 ping (PR #164, section 2.8). The
eight-reactor count is rejected; this document now records that evidence without
a runtime change. A later refresh onto the isolated command counters passed both
four-CPU gates twice (section 2.10). A gate pass on one head never substitutes for
a fresh head, and every dependency still needs explicit review.

## 1. Scope, constraints and evidence discipline

The question is narrow: which measured per-request costs on the default
plain-TCP path still trail pinned Redis 8.10.2 or Valkey 9.1.2, and which
architecture removes them without weakening correctness. This is not a general
feature roadmap, a production SLO or a claim that one design wins every workload.

Non-negotiable constraints ([requirements](../requirements.md)):

- Application code is safe Rust; every target keeps `#![forbid(unsafe_code)]`.
  No FFI, raw pointers, custom allocator, native extension or build script in
  project code.
- External Rust dependencies are explicit, reviewed exceptions, vendored for
  offline builds. The authorized set is rustls and Mio plus their required
  transitive crates; it was extended by PR #139 through review, not incrementally.
- Expiry, accounting, multi-key atomicity, bounded I/O/backpressure, client
  deadlines, TLS behavior and joined shutdown are invariants, not tuning knobs.
- Performance claims come from comparable, repeated end-to-end runs against
  current pinned servers. Local screens are diagnostics; the protected gates are
  the acceptance authority.

Evidence rules used below:

1. Protected-gate numbers are authoritative for a specific head. Every losing
   median, spread, tail and CPU record is retained; incomplete or failed cohorts
   are never relabeled.
2. Local diagnostics are labeled with host, load and distortion. They motivate a
   candidate; they do not establish a gate result.
3. Numbers from different runners or dates are not blended into a single
   comparison. A pass on one host never substitutes for a failing head.
4. No threshold relaxation, omitted workload, retry-until-green or historical
   pass reuse is permitted by [the quality gate](../pr-quality-gate.md).

## 2. Measured position

### 2.1 Original 20-case gate (one inherited or one-CPU budget)

At `d42d20c` (PR #139 before the readiness reactor) the original gate passed
**20/20** ([run 40938475281](https://github.com/macel94/rillcache/actions/runs/40938475281)).
Rillcache led Valkey by roughly +10% to +46% in every row. At one logical CPU,
pipeline 1, batch p99 was about 94–103 µs versus 114–125 µs, and sampled CPU was
about 9.9–10.5 µs/command versus 11.1–11.8 µs. In that harness Rillcache was
already cheaper per command; the remaining losses were in the different
four-CPU/small-request regime below.

### 2.2 Official four-CPU gate at `d42d20c`: 7/10

[Run 40938475263](https://github.com/macel94/rillcache/actions/runs/40938475263)
used the unmodified official `valkey-benchmark`, exactly four logical server CPUs,
eight clients, two generator threads, 1024 random keys, 64-byte values, seed 42,
8000 warmups and three alternating 10-second repetitions:

| Workload | P | Rillcache req/s [min–max] | Valkey req/s [min–max] | Delta | p99 ms R/V | Gate |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| ping | 1 | 108663.80 [104272.90–113424.00] | 110481.20 [109933.20–110632.10] | **-1.64%** | 0.119/0.143 | FAIL |
| get | 1 | 108447.40 [108089.60–110755.30] | 112324.00 [111535.10–112758.60] | **-3.45%** | 0.119/0.119 | FAIL |
| set-nx-existing | 1 | 109423.30 [108630.40–110099.90] | 111362.20 [109292.00–112680.20] | **-1.74%** | 0.119/0.127 | FAIL |
| set | 1 | 108989.20 [107922.10–110401.80] | 108930.80 [105250.30–109590.00] | +0.05% | 0.119/0.151 | PASS |
| set-xx-existing | 1 | 109779.50 [109552.50–110781.50] | 108648.30 [107521.80–108701.70] | +1.04% | 0.119/0.135 | PASS |
| ping | 16 | 1546361.62 [1499240.00–1549355.25] | 1067780.75 [991060.81–1114864.00] | +44.8% | 0.119/0.207 | PASS |
| set | 16 | 1107211.25 [1085358.38–1118392.00] | 596992.00 [586886.38–620472.00] | +85.5% | 0.191/0.311 | PASS |
| get | 16 | 1388321.62 [1366964.75–1416968.00] | 700310.38 [669054.38–722257.62] | +98.2% | 0.159/0.271 | PASS |
| set-nx-existing | 16 | 1356686.38 [1324678.38–1359307.25] | 691304.00 [659496.00–704574.38] | +96.2% | 0.159/0.279 | PASS |
| set-xx-existing | 16 | 1095526.38 [1085572.75–1100985.62] | 583428.81 [580857.62–588904.00] | +87.7% | 0.191/0.319 | PASS |

What this shape says:

- The loss is confined to **pipeline 1 at four CPUs**, where per-request
  scheduling and readiness dominate. Every pipelined case wins by 44.8–98.2%.
- Tails are not the problem: Rillcache p99 is equal or lower in all five P1 rows
  and much lower at P16. The loss is sustained throughput, not a latency cliff.
- The three losses are 1.6–3.5% with overlapping spreads, so noise can block the
  gate; but a passing head must still show three more strict wins, not a
  statistical argument about overlap.
- Absolute rates differ strongly between GitHub runners: an identical tree
  (`8acd843`, the head merged as `0e04603`) passed 10/10 at 166k req/s P1 on a
  faster host ([run 40805057097](https://github.com/macel94/rillcache/actions/runs/40805057097)).
  That is a same-tree, different-host observation, **not** a controlled proof that
  the PR's experiments caused the loss.

The same-host counterexample is useful: it shows the gate is winnable on the
existing architecture and that host throughput shifts the regime. It also means
the next design must be judged on a fresh matched cohort, not by subtracting
numbers from different runs.

### 2.3 Readiness candidate (PR #139, Mio reactor): both gates pass

The workflow's merge candidate `3eee1f2` for PR #139 (base `0e04603`) passed
both protected gates on the same candidate:

- Original 20-case gate: **PASS 20/20**, run
  [40978287533](https://github.com/macel94/rillcache/actions/runs/40978287533).
  P1 leads were +10.0% to +38.4% and P16 leads +33.6% to +56.8%; P1 p99 was
  93–107 µs versus 123–163 µs, and sampled CPU 8.7–9.6 µs/command versus
  9.9–12.7 µs.
- Official four-CPU gate: **PASS 10/10**, run
  [40978287560](https://github.com/macel94/rillcache/actions/runs/40978287560):

| Workload | P | Rillcache req/s [min–max] | Valkey req/s [min–max] | Delta | p99 ms R/V |
| --- | ---: | ---: | ---: | ---: | ---: |
| ping | 1 | 113262.10 [108956.20–113400.90] | 106593.70 [98075.10–112747.00] | +6.3% | 0.103/0.143 |
| get | 1 | 116728.40 [116527.10–117486.90] | 110331.20 [109170.10–112363.70] | +5.8% | 0.103/0.135 |
| set | 1 | 112831.40 [108705.70–114098.80] | 104227.10 [95541.60–106538.60] | +8.3% | 0.111/0.151 |
| set-nx-existing | 1 | 116537.20 [116141.30–117262.20] | 107347.50 [107265.10–111597.10] | +8.6% | 0.103/0.143 |
| set-xx-existing | 1 | 108888.40 [105855.00–111907.00] | 98503.20 [94527.80–104323.30] | +10.5% | 0.119/0.159 |
| ping | 16 | 1552553.62 [1546131.25–1564340.75] | 1039027.19 [985857.62–1065284.75] | +49.4% | 0.111/0.207 |
| get | 16 | 1402915.25 [1389920.00–1419420.75] | 723689.62 [714308.81–736043.19] | +93.9% | 0.207/0.271 |
| set | 16 | 1178524.75 [1149126.38–1186552.00] | 579691.19 [569094.38–604552.00] | +103.3% | 0.183/0.311 |
| set-nx-existing | 16 | 1431443.25 [1410710.38–1435998.38] | 687755.19 [685700.81–712848.00] | +108.1% | 0.223/0.279 |
| set-xx-existing | 16 | 1160624.00 [1148563.25–1188038.38] | 586873.62 [579529.62–601660.81] | +97.8% | 0.191/0.311 |

With the readiness design this head wins every four-CPU case, including the
three P1 rows that failed at `d42d20c`. P1 p99 is lower than Valkey's in all five
rows; the P1 spreads are narrow and overlap only for ping, so the strict median
remains the acceptance rule rather than a claim about noise. The
head differs from `d42d20c` by readiness plus the earlier accept, accounting and
scheduling changes; the evidence supports the combined head, not readiness alone.

### 2.4 Local syscall diagnostic (not a gate result)

On this shared development VM (load average 10–15, heavy steal, roughly 3.5k
req/s instead of 100k+), paired `perf stat` tracepoint counts over the same
10,000-request P1 run showed, per benchmark command:

| Path | Receive-class syscalls | Send-class syscalls | epoll_wait | futex |
| --- | ---: | ---: | ---: | ---: |
| Rillcache pre-readiness (polling handoff) | ~6.2 | 1.0 | 0 | ~0.18 |
| Valkey 9.1.2 | ~1.0 | 1.0 | ~0.4 | 0 |
| Rillcache pre-readiness, P16 | ~0.38 | ~0.06 | 0 | ~0.012 |
| Valkey 9.1.2, P16 | ~0.065 | ~0.06 | ~0.02 | 0 |

`recv`/`read` counts include idle polling passes, which grow with elapsed time on
a throttled host; the ratios are therefore diagnostic of mechanism, not a
prediction at gate rates. The earlier `strace` study independently counted
16,375 receives (5,560 `EAGAIN`) for 10,808 sends in a four-worker run
([I/O scheduling](../io-scheduling.md)). Both point at the same mechanism: a
user-space poller without kernel readiness re-asks every borrowed socket, while
the C servers ask the kernel once. That is the largest single architectural
difference on the P1 path.

### 2.5 Same-setup Dragonfly gate and host sensitivity

PR #153 added a checksum-pinned Dragonfly v2.0.2 comparison to the same required
four-CPU job: identical five commands, P1/P16, eight clients, 1024 random keys,
64-byte values, 8,000 warmups, official ten-second samples and three alternating
repetitions. Rillcache must strictly beat Dragonfly in all ten medians as well as
Valkey's ten. The analysis and phased roadmap are in
[Dragonfly-class performance](dragonfly-class-performance.md); Dragonfly is an
external benchmark target, not a dependency.

Measured cohorts show how much the P1 row depends on the host:

| Cohort | Host scale (Rill P1 ping) | Valkey gate | Dragonfly gate | P1 ping detail |
| --- | ---: | --- | --- | --- |
| `d7ec5b3` | ~220k req/s | PASS 10/10 | **FAIL 9/10** | 220,157 vs 245,317 req/s (−10.3%), p99 0.087 vs 0.047 ms |
| `6ba01b6` | ~115k req/s | PASS 10/10 | PASS 10/10 | P1 rows passed on the slower host |
| PR #155 candidate | ~107k req/s | **FAIL 8/10** | PASS 10/10 | ping 106,001 vs 110,183 (−3.8%), get 110,054 vs 110,399 (−0.3%) |

The only repeated Dragonfly loss is pipeline-1 `ping` on fast hosts, where
per-request scheduling and syscall batching dominate; every pipelined row and all
SET/GET/NX/XX P1 rows have large margins. The strict median rule still fails a
cohort on that row. No threshold, workload or host substitution is permitted, so
the row remains an explicit optimization and measurement target, not noise to be
argued away.

### 2.6 Short-read drain elision (PR #155, rejected)

The kernel-readiness reactor re-queued a productive connection and then issued an
immediate empty receive to confirm that an edge-triggered read had drained. A
candidate skipped that probe after a short Linux read (full chunks, `EINTR`,
buffered input/output, closing and asynchronous states still retained the
continuation). A paired local debug capture measured receive-class syscalls
falling from about 1.7 to 1.0 per command and total network syscalls by roughly
16%, but local throughput was unusable on the contended host.

The unchanged four-CPU gate rejected the candidate: Dragonfly passed 10/10, but
Valkey failed 8/10 with ping P1 −3.8% and get P1 −0.3% (run 41368946995). The
diagnosis is that the empty receive also acts as a user-space readiness batch for
already-active clients: removing it shifts work to another `epoll_wait` wake and
can add P1 latency. The syscall count alone was not a throughput proxy. The
candidate was closed, its branch and artifacts were retained, and it was not
rerun. This is a rejected option in section 7.

### 2.7 Shared-store socket workers (rejected)

The first multi-reactor attempt ran `min(4, CPUs)` independent Mio reactors over
the existing locked Store, with no sharding, migration or dependency change. The
original 20-case gate rejected it **12/20** (run 41380968312):

| Budget | P | Workload | Rillcache | Valkey | Delta |
| --- | ---: | --- | ---: | ---: | ---: |
| inherited | 1 | ping | 160,813 | 105,100 | +53.0% |
| inherited | 1 | set | 42,462 | 100,038 | -57.6% |
| inherited | 1 | get | 42,669 | 100,584 | -57.6% |
| inherited | 16 | set | 61,926 | 964,787 | -93.6% |
| inherited | 16 | get | 62,069 | 1,086,520 | -94.3% |
| one-cpu | 1 | set | 108,883 | 97,242 | +12.0% |
| one-cpu | 16 | set | 1,398,149 | 953,753 | +46.6% |

The PING result confirms that connection-local work scales across reactors. The
data collapse shows that shared `StoreHandle`/bridge/lock execution is not the
owner-local data path; the one-CPU budget selects one reactor, keeps the local
executor, and passes every row. The PR was closed with its branch and artifacts
retained, not rerun or relabeled.

### 2.8 Eight connection-local reactors (rejected)

Owner-migration then kept the owner-local data path and moved only PING/ECHO/QUIT
to worker reactors. PR #159 (`31a6006`) passed Valkey 10/10 and Dragonfly 10/10,
but the P1 PING margin against Valkey was only +2.9% on that host. A follow-up
made the owner the only acceptor and distributed new connections round-robin
(PR #161, `1401a79`). At the old `min(4, CPUs)` count it passed Valkey 10/10 but
failed Dragonfly 8/10 (run
[41420512129](https://github.com/macel94/rillcache/actions/runs/41420512129)):
ping P1 was −7.0% and ping P16 −2.1%, while the four data P1 and four data P16
rows still won.

The next candidate kept round-robin distribution and changed the default to
`2 × allowed CPUs` reactors capped at eight (one on a single CPU), so the
four-CPU gate would have eight lanes, one per P1 client. Data connections still
migrated once to the owner-local executor. The prediction was that P1 PING would
rise because the eight connection-local clients no longer shared four loops, and
that data rows would be unchanged.

The prediction was wrong. On head `e790299` both four-CPU comparisons failed only
pipeline-1 ping (run
[41441782769](https://github.com/macel94/rillcache/actions/runs/41441782769)):

| Comparison | Ping P1 Rillcache | Competitor | Delta | Ping P16 Rillcache | Competitor | Delta |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Valkey 9.1.2 | 85,675.7 | 110,402.4 | **−22.4%** | 1,236,521.6 | 974,910.4 | +26.8% |
| Dragonfly 2.0.2 | 85,839.7 | 94,620.0 | **−9.3%** | 1,236,715.3 | 1,084,894.4 | +14.0% |

Every other row passed: P1 SET/GET/NX/XX stayed around 112–115k req/s with
p99 0.111–0.119 ms, and the data P16 rows won by 45–107% against Valkey and
75–185% against Dragonfly. The original 20-case gate passed 20/20 on the same
head (run 41441783049). The loss is therefore specific to the extra reactors, not
to data execution or parsing.

Diagnosis: eight reactor threads on four CPUs oversubscribe the host. Each
productive reactor also takes the 8 µs `FAST_WAIT` spin, so the extra threads
consume runnable time without adding parallel work; P1 ping fell from the
117–138k range of the four-reactor cohorts to about 86k. Adding reactors beyond
the CPU budget is not a scaling mechanism for P1 latency. The runtime candidate
met its rejection condition; its implementation is reverted and the evidence is
recorded here. The branch, commits and both artifact summaries are retained. This
document is the only surviving change from that branch.

### 2.9 Pipeline-1 ping variance and the four-CPU client/server overlap

The merged owner-migration runtime has now produced four four-CPU cohorts with
the same binary and gate. They differ only in run placement and host noise:

| Cohort | P1 ping Rillcache | Valkey | Delta | p99 R/V |
| --- | ---: | ---: | ---: | ---: |
| PR #159 merge run | 117,430.7 | 114,097.1 | +2.92% | 0.095/0.127 |
| PR #160 docs-only | 110,968.0 | 114,032.6 | −2.69% | — |
| PR #164 docs-only, first | 112,894.7 | 115,545.1 | −2.29% | 0.103/0.119 |
| PR #164 docs-only, second | 107,787.9 | 112,658.8 | −4.32% | 0.135/0.135 |

One pass and three losses span +2.92% to −4.32% for an unchanged runtime. The
failing #164 heads still had p99 at or below Valkey's and passed Dragonfly 10/10
and every other Valkey row, so this is not a tail or data-path regression.

The failing artifact's manifest explains the mechanism:
`client_cpus: [0, 1, 2, 3]`, `server_cpus: [0, 1, 2, 3]`. The runner has no
disjoint client CPUs, so two generator threads share all four CPUs with the
server. The merged runtime then has six runnable threads (four reactors plus two
generators) where Valkey has three (one server plus two generators). The extra
reactors compete with the load generator; the eight-reactor candidate made the
same contention much worse (−22.4%). The gate text already intends disjoint
client CPUs "when available"; this host cannot provide them.

**Verdict:** the P1 ping row is a gate-topology and shared-runner variance
limitation for this configuration, not a proven server deficit. Do not tune the
reactor count again on this evidence: more reactors lost badly and fewer reactors
are a different experiment without a measured mechanism. A defensible next step
is either a gate runner with disjoint client CPUs (CI/policy change, owner
review) or a different scheduling mechanism that does not add runnable server
threads. The docs-only record must not be re-pushed repeatedly to fish for a
noise pass.

### 2.10 Post-refresh cohorts: command-counter isolation and two both-gates passes

Main then advanced to `9692e07`, which pads each of the 64 sharded command
counters to a 128-byte boundary instead of packing eight adjacent atomics into a
cache line. The docs-only performance record was refreshed onto that runtime
(head `c1564aa`, base `9692e07`) and passed every required context:

| Gate | P1 ping Rillcache | Competitor | Delta | P1 p99 R/C | P16 ping Rillcache | Competitor | Delta |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Valkey 9.1.2 | 112,884.5 | 105,144.5 | +7.36% | 0.111/0.151 | 1,550,105.6 | 1,024,718.4 | +51.3% |
| Dragonfly 2.0.2 | 111,035.5 | 95,328.0 | +16.48% | 0.111/0.103 | 1,558,257.6 | 1,097,716.8 | +42.0% |

The next docs head (`4f0ed0c`, PR #169, base `f28fe60`) repeated the result on
the same runtime: Valkey 10/10 with P1 ping 114,615.3 vs 111,403.2 (+2.88%),
p99 0.103/0.127; Dragonfly 10/10 with 106,109.3 vs 92,961.5 (+14.14%),
p99 0.119/0.111; the original gate passed again. Two consecutive heads now pass
both four-CPU comparisons, versus one pass and three losses in the four cohorts
before the counter change (section 2.9).

Both comparisons passed 10/10, the original gate passed, and the four-CPU runner
still had `client_cpus == server_cpus == [0,1,2,3]`. The margin is therefore not
explained by disjoint client placement.

The plausible mechanism is false sharing: connection-local PING/ECHO run on
several reactors, and every command increments `total_commands_processed`
through a shard chosen by client id. Eight adjacent 8-byte atomics shared one
64-byte line, so different reactors could invalidate each other even though the
logical counters were disjoint. A 128-byte stride removes that cross-reactor
line ping-pong; it is a safe-Rust layout change with no allocator, dependency or
wire change. It also explains why the effect is largest on the multi-reactor P1
ping rows and small on data rows, which execute on one owner.

Limit: two passing cohorts do not isolate the counter change from host noise,
and the prior band means a future head can still fail the same row. The claim
supported here is that the refreshed runtime passed both four-CPU comparisons
twice with larger P1 ping margins, consistent with the false-sharing mechanism;
it is not a universal or permanent win. Section 4.10 records the matched A/B
needed to establish causality.

## 3. Per-component comparison with Redis and Valkey

Pins: Valkey `9.1.2` and Redis `8.10.2` source tags, as in
[hot-paths](../hot-paths.md). Line links are to those tags. This section
separates what is worth copying, what must be adapted, and what must not be
copied into this repository.

### 3.1 Readiness and per-request scheduling

- **Valkey** registers every connection with `ae` and blocks in
  `epoll_wait`/`kqueue`/`poll`; `readQueryFromClient` reads up to a 16 KiB
  `PROTO_IOBUF_LEN` chunk and processes the input buffer
  ([`networking.c#L4385`](https://github.com/valkey-io/valkey/blob/9.1.2/src/networking.c#L4385),
  [`server.h#L212`](https://github.com/valkey-io/valkey/blob/9.1.2/src/server.h#L212),
  [`ae_epoll.c#L114`](https://github.com/valkey-io/valkey/blob/9.1.2/src/ae_epoll.c#L114)).
  One thread serves all eight P1 clients; no per-request thread wakeup or shared
  data lock is involved.
- **Redis** uses the same event-loop model with an optional I/O-thread pool
  (`io-threads`, default 1) that offloads reads/writes and some poll work
  ([`networking.c#L1663`](https://github.com/redis/redis/blob/8.10.2/src/networking.c#L1663),
  [`config.c#L3507`](https://github.com/redis/redis/blob/8.10.2/src/config.c#L3507)).
  Default behavior is still single-threaded readiness.
- **Rillcache before `4a52fc0`** had no readiness multiplexer in safe `std`.
  Default mode was a blocking thread per client; on small CPU budgets sockets
  were lent to a coordinator that polled them nonblocking in round-robin
  ([`pool.rs`](../../src/server/pool.rs), [`handoff.rs`](../../src/server/handoff.rs)).
  Steady state avoided per-request owner wakeups, but every pass called `recv` on
  every borrowed socket, including empty ones. That was the measured extra
  receive-class work above. The merged reactor now supplies kernel readiness on
  default Unix plain TCP; the fallback remains for TLS, fixed workers and
  non-Unix builds.
- **Triggering mode is a real difference, not a tuning detail.** Redis and Valkey
  register `EPOLLIN`/`EPOLLOUT` without `EPOLLET`
  ([`valkey ae_epoll.c#L83`](https://github.com/valkey-io/valkey/blob/9.1.2/src/ae_epoll.c#L83),
  [`redis ae_epoll.c#L64`](https://github.com/redis/redis/blob/8.10.2/src/ae_epoll.c#L64)),
  so the kernel keeps reporting a socket until its queue is drained. The vendored
  Mio selector always adds `EPOLLET`
  ([`selector/epoll.rs#L130`](../../vendor/mio/src/sys/unix/selector/epoll.rs#L130)),
  which is why Rillcache re-queues a productive client and probes once more for
  `WouldBlock`: under edge-triggered readiness that probe is the correctness
  mechanism, not redundant work. Replacing it with level-triggered semantics
  would require raw `epoll_ctl`/`epoll_wait` (unsafe FFI, rejected) or a selector
  dependency whose level-triggered behavior is part of the reviewed API. This is
  the structural explanation for the PR #155 result in section 2.6.

**Verdict:** the kernel-readiness idea was copied and adapted to a reviewed safe
wrapper (Mio, section 4.1). Raw `epoll` with `unsafe` and busy polling by default
were rejected. Level-triggered copying remains unavailable without a policy
change, and the measured drain-elision attempt confirms the probe is load-bearing.

### 3.2 Reply construction and write coalescing

- **Valkey** writes small replies into a 16 KiB per-client static buffer and
  spills larger replies into an sds list; `_writeToClient` uses `writev` when the
  list is non-empty, and `beforeSleep` flushes pending writes after the event
  loop has processed ready clients
  ([`networking.c#L737`](https://github.com/valkey-io/valkey/blob/9.1.2/src/networking.c#L737),
  [`networking.c#L2866`](https://github.com/valkey-io/valkey/blob/9.1.2/src/networking.c#L2866),
  [`networking.c#L3348`](https://github.com/valkey-io/valkey/blob/9.1.2/src/networking.c#L3348),
  [`server.c#L1812`](https://github.com/valkey-io/valkey/blob/9.1.2/src/server.c#L1812)).
  Small numeric/status replies use shared objects. Valkey 9.1 adds reply
  reference/encoded-buffer machinery for copy avoidance on the I/O-thread path.
  Both servers also cap a single event at `NET_MAX_WRITES_PER_EVENT` (64 KiB) so a
  fast client with a huge reply cannot monopolize the loop
  ([`valkey networking.c#L2838`](https://github.com/valkey-io/valkey/blob/9.1.2/src/networking.c#L2838),
  [`redis networking.c#L2887`](https://github.com/redis/redis/blob/8.10.2/src/networking.c#L2887));
  Valkey enables copy avoidance from seven I/O threads by default through the
  hidden `min-io-threads-avoid-copy-reply` setting
  ([`valkey config.c#L3386`](https://github.com/valkey-io/valkey/blob/9.1.2/src/config.c#L3386)).
- **Redis** has the same 16 KiB buffer + reply list + `writev` shape and defers
  writes to `handleClientsWithPendingWrites`
  ([`networking.c#L514`](https://github.com/redis/redis/blob/8.10.2/src/networking.c#L514),
  [`networking.c#L2847`](https://github.com/redis/redis/blob/8.10.2/src/networking.c#L2847),
  [`networking.c#L2958`](https://github.com/redis/redis/blob/8.10.2/src/networking.c#L2958)).
- **Rillcache** writes RESP bytes directly into a reused `Vec<u8>` with
  specialized decimal/bulk headers and flushes after the command batch
  ([`protocol.rs`](../../src/protocol.rs), [`session/mod.rs`](../../src/session/mod.rs)).
  There is no per-reply allocation, but also no cross-client write coalescing:
  each ready client's output is written in its own pass.

**Verdict:** the formatting and allocation posture is already competitive; do
not add shared-object tables or encoded-buffer references (they need reference
counting and unsafe/ownership machinery). Rillcache's bounded output chunks and
soft backpressure already prevent one client from monopolizing a pass, so the
64 KiB per-event cap is a design confirmation rather than a missing mechanism.
Cross-client deferred flushing (`beforeSleep` shape) is a later, separate
candidate (4.5) and must not worsen P1 tails.

### 3.3 Parsing

- **Valkey** parses incrementally with per-client state (`reqtype`, `multibulklen`,
  `bulklen`, `qb_pos`) and keeps arguments borrowed from the query buffer for
  small arguments, copying only large/long-lived objects; the command queue lets
  it defer execution without reparsing
  ([`networking.c#L4237`](https://github.com/valkey-io/valkey/blob/9.1.2/src/networking.c#L4237),
  [`networking.c#L4299`](https://github.com/valkey-io/valkey/blob/9.1.2/src/networking.c#L4299)).
- **Redis** follows the same incremental multibulk/inline state machine. Its
  8.10 parser builds a `pendingCommand` and separates read handling from
  `processInputBuffer`, while Valkey 9.1 keeps a `cmd_queue` so pipelined
  commands are consumed without reparsing
  ([`redis networking.c#L3690`](https://github.com/redis/redis/blob/8.10.2/src/networking.c#L3690),
  [`valkey networking.c#L4237`](https://github.com/valkey-io/valkey/blob/9.1.2/src/networking.c#L4237)).
- **Rillcache** parses complete frames from a contiguous input vector, borrowing
  slices; up to eight arguments live on the stack and larger commands use bounded
  fallback storage ([`protocol.rs#L76`](../../src/protocol.rs#L76)). It has no
  resumable parser state: while a single large frame is incomplete, each new read
  re-scans the bytes already buffered. For ordinary 14–128-byte commands this is
  irrelevant; for multi-megabyte bulk arguments split over many reads it is
  quadratic in the number of reads.

**Verdict:** adapt the resumable-state idea (4.2) only if a large-frame workload
is in scope; do not copy the C object/refcount parser.

### 3.4 Storage, hashing and expiry

- **Valkey/Redis** use an incremental-rehash hash table (`dict`) with a per-process
  SipHash seed, so growth is amortized in small steps rather than one stop-the-world
  rehash ([`dict.c#L123`](https://github.com/valkey-io/valkey/blob/9.1.2/src/dict.c#L123),
  [`dict.c#L327`](https://github.com/valkey-io/valkey/blob/9.1.2/src/dict.c#L327),
  [`redis dict.c#L299`](https://github.com/redis/redis/blob/8.10.2/src/dict.c#L299)).
  String values use embstr/int/raw encodings; expiry lives in a dedicated
  expires structure and is checked on lookup. Valkey 9.1 moved the main keyspace
  to a `kvstore` and rehashes it in bounded time slices during cron
  ([`valkey server.c#L1325`](https://github.com/valkey-io/valkey/blob/9.1.2/src/server.c#L1325));
  Redis 8.10 keeps the dict step-on-access model
  ([`redis dict.c#L471`](https://github.com/redis/redis/blob/8.10.2/src/dict.c#L471)).
- **Rillcache** uses `std::collections::HashMap<Vec<u8>, Entry>` with the
  standard randomized SipHash hasher and one `RwLock<Data>`; expiry is an ordered
  `BTreeSet<(i64, Vec<u8>)>` and every mutation records a WATCH version
  ([`data.rs#L304`](../../src/engine/data.rs#L304)). Equal-size SET reuses the
  value allocation and GET reuses its lookup, as delivered earlier.
- The map hasher is not exposed by `std`; replacing it would need a dependency or
  a home-grown keyed hash. Both are policy-sensitive (HashDoS resistance) and are
  not justified by the current 1024-key gate.

**Verdict:** avoid custom allocators, non-keyed hashing and sharding. The
incremental-rehash benefit is real but secondary to readiness at this keyspace
size; revisit only with a large-keyspace measurement.

### 3.5 Per-command dispatch and observability

- **Valkey/Redis** look up the command once in a command table, then run `call()`
  which updates counters and optional slowlog/latency/commandstats hooks. Most
  hooks are cheap flags or no-ops on the benchmark path
  ([`valkey server.c#L3830`](https://github.com/valkey-io/valkey/blob/9.1.2/src/server.c#L3830),
  [`valkey lookupCommand`](https://github.com/valkey-io/valkey/blob/9.1.2/src/server.c#L3480),
  [`redis call`](https://github.com/redis/redis/blob/8.10.2/src/server.c#L3992)).
  The lookup itself is a dict probe of the command name (case-insensitive sds
  hash), not a linear scan.
- **Rillcache** has an inlined PING/GET/SET/ECHO path that avoids family routing
  and the transaction gate, but it first evaluates a chain of about a dozen
  mode/observer atomics and four case-insensitive command comparisons
  ([`session/mod.rs`](../../src/session/mod.rs)). Every command then takes the
  transaction-gate read lock and the data read/write lock even when no journal,
  transaction or observer is active.
- **Command counters are a multi-reactor cost the C servers do not have.** Valkey
  and Redis update `stat_numcommands` on one execution thread, so a plain integer
  is enough. Rillcache's connection-local PING/ECHO run on several reactors, and
  every command increments a 64-way sharded atomic chosen by client id. Before
  PR #165 the 8-byte atomics were packed eight per cache line, so independent
  reactors could invalidate each other's line even though the counters were
  logically disjoint. The 128-byte stride removes that false sharing; it is the
  only measured mechanism in this review that targets P1 ping without adding
  runnable threads (section 2.10).

**Verdict:** adapt by collapsing the observer/mode checks into one atomically
published mode word (4.3) and by making the transaction gate a fast no-op when no
exclusive operation is registered (4.4). Do not copy the full command table
abstraction; the current split is easier to keep out of the hot path. Keep
cross-reactor mutable counters cache-line separated when they are updated per
command.

### 3.6 Threading model

- Redis's and Valkey's optional I/O threads parallelize socket reads/writes but
  keep command execution ordered; enabling them is a configuration change, not
  the default, and adds synchronization.
- Rillcache's fixed-worker mode (`--threads`) and its multicore client-handoff
  leases exist for parallel execution. The lease policy won P16 by large margins
  but its polling model is the suspected P1 cost; a readiness reactor is
  single-threaded by design and therefore gives up the multi-CPU execution used
  at P16 unless a later sharded/worker design is added.

**Verdict:** readiness is the default I/O owner (4.1); fixed workers and the
portable fallback remain; an io-thread pool was not imported. Any parallel command
execution must preserve multi-key atomicity and is explicitly deferred.

### 3.7 Clock, expiry and command-time snapshots

- **Valkey/Redis** cache wall-clock milliseconds in the event loop and refresh the
  snapshot when an execution unit starts. Every command uses
  `commandTimeSnapshot()` instead of calling `gettimeofday`/`clock_gettime` again,
  which also keeps a multi-key command from seeing two different "now" values
  ([`valkey server.c#L332`](https://github.com/valkey-io/valkey/blob/9.1.2/src/server.c#L332),
  [`valkey enterExecutionUnit`](https://github.com/valkey-io/valkey/blob/9.1.2/src/server.c#L1367),
  [`redis commandTimeSnapshot`](https://github.com/redis/redis/blob/8.10.2/src/server.c#L262)).
  Expiry uses this cached time; `serverCron` refreshes it for housekeeping.
- **Rillcache** already avoids the clock when the database has no expirations:
  `now()` returns `0` while `expirations` is empty
  ([`commands.rs#L95`](../../src/engine/commands.rs#L95)), so the gate's non-TTL
  SET/GET path does not pay a clock syscall. Once any key has a TTL, each command
  that consults expiry calls `SystemClock::now_ms()`, a `SystemTime::now()` per
  call behind the injectable `Clock` trait
  ([`engine.rs#L118`](../../src/engine.rs#L118)).

**Verdict:** do not add a speculative cached clock to the gate path; it would not
run there. A TTL-heavy workload is the only case where the C servers' snapshot
shape is cheaper, and it is not in the protected gate. If measured TTL workloads
show a clock cost, adapt the snapshot idea behind the existing `Clock` trait:
publish one millisecond value per reactor pass and let commands read it, keeping
the injectable test clock and single-command time consistency. That is safe Rust
with no new dependency. It is candidate 4.9, not part of this increment.

### 3.8 Decision matrix: best of each project

| Component | Redis 8.10.2 | Valkey 9.1.2 | Rillcache today | Decision |
| --- | --- | --- | --- | --- |
| Readiness | level-triggered epoll, single loop | level-triggered epoll, single loop | Mio edge-triggered reactor with drain probe | **Keep/adapt:** edge-triggered through reviewed Mio; level-triggered needs unsafe FFI or a new selector policy |
| Per-request scheduling | event loop, optional I/O threads (default 1, immutable) | event loop, optional I/O threads (default 1, modifiable) | owner reactor plus at most `min(4, CPUs)` connection-local workers | **Adapt, bounded:** more reactors than CPUs oversubscribed P1 ping (−22.4% Valkey); reject counts above `min(4, CPUs)` without a different mechanism |
| Read path | 16 KiB `PROTO_IOBUF_LEN`, `pendingCommand` | 16 KiB buffer, `cmd_queue` | 16 KiB chunk, complete-frame parse | **Keep:** equivalent for the measured small-command regime |
| Reply path | 16 KiB buffer + reply list + `writev`, deferred flush, 64 KiB cap | same plus encoded-buffer copy avoidance from 7 I/O threads | reused `Vec<u8>`, inline flush, bounded chunks | **Keep:** formatting is allocation-free; deferral is 4.5 and unproven |
| Parsing | incremental multibulk/inline state | incremental state plus command queue | complete-frame fast path, no resumable cursor | **Keep:** incomplete frames do not rescan payload; revisit only with large-argument evidence |
| Storage | dict, SipHash, step rehash | kvstore, SipHash, time-budgeted rehash | `std` HashMap + `RwLock<Data>`, BTreeSet expiry | **Avoid:** no safe incremental-rehash std API; custom hash/allocator is policy-sensitive |
| Dispatch | dict command table + `call()` | dict command table + `call()` | inlined PING/SET/GET/ECHO, checked dispatcher otherwise | **Keep:** inlining is cheaper for gate commands |
| Time | cached `mstime` per execution unit | cached `mstime` per execution unit | `SystemTime::now()` only when expirations exist | **Adapt only if TTL evidence:** candidate 4.9 |
| Expiry | lazy + active cycle | lazy + active cycle | lazy on access + 100 ms purge tick | **Keep:** gate has no TTL rows; correctness covered by tests |

## 4. Candidate architectures

Each candidate names its mechanism, expected effect, cost/risk and acceptance
evidence. Section 4.1 and the mode-word work (4.3) are implemented; 4.2 was
deprioritized after analysis, 4.5 is partially implemented, 4.6 is deferred,
4.7 is a rejected experiment, 4.8 is merged with its reactor count rejected,
4.9 is a TTL-only candidate that is not implemented, and 4.10 is a diagnostic
A/B for the already-merged counter layout.

### 4.1 Bounded Mio readiness reactor (merged in PR #139)

Default Unix plain-TCP mode registers the listener and connections with Mio,
which uses epoll on Linux and kqueue where available. Edge-triggered readiness is
retained in a runnable queue until `WouldBlock`, so a bounded pass never depends
on a second edge. Bounded accept (64) and run (256) quanta, slot recycling with
non-recycled tokens, a 1 ms asynchronous tick for blocked/pubsub/monitoring
clients, a 100 ms maintenance tick (journal sync, expiry, idle timeouts) and a
10 ms stop tick bound fairness, latency and shutdown. TLS and non-Unix builds keep
the existing managed/blocking fallback; `--threads` keeps fixed lanes.

Expected effect: replace per-socket empty `recv` probes with one kernel wait, so
P1 four-CPU throughput stops losing to Valkey while P16 remains single-thread
competitive. Measured at the merge candidate: 10/10 four-CPU wins and 20/20
original wins (section 2.3). Later increments kept the reactor as the default and
added opt-in Tokio transport (#143), multi-owner execution (#145), configuration/
observer/authorization mode words (#146–#149), next-request absorption (#150) and
batched owner replies (#152). Risks: edge-triggered lost wakeups, stale token reuse, fairness
under backpressure, async delivery latency, single-thread CPU ceiling, fd
deregistration with cloned administration handles. Mitigations are the runnable
queue, `EINTR` retry flag, token identity checks, readiness integration tests,
and the unchanged gate.

Acceptance: both protected gates 20/20 and 10/10 on a fresh head; all 14 contexts;
no lost/duplicated replies under fragmented, pipelined, half-closed and
backpressured tests; joined shutdown; TLS/fixed/non-Unix behavior unchanged;
dependency policy test and owner review for the reviewed dependency exceptions.

### 4.2 Resumable RESP parse state (deprioritized)

The original concern was repeated scanning of a large incomplete frame. Source
inspection showed that the parser does not scan payload bytes while incomplete:
it re-reads only the small length headers and checks buffer length, so a single
large value is not quadratic in bytes. The remaining cost is proportional to the
number of argument headers per read, bounded by `max_arguments`. A resumable
cursor would therefore add state and branches to the hot path without a measured
large-argument workload showing a problem. Revisit only with such a measurement;
the existing complete-frame fast path stays.

### 4.3 Single observer/mode word (implemented)

The observer, security and authorization decisions now load one word rather than
a chain of separate atomics (#148, #149), and configuration-dependent effects are
cached behind one configuration epoch (#146, #147). The hot path still falls into
the fully checked dispatcher whenever any collector, pause, monitor, subscriber
or security bit is set.

Acceptance met: the existing observer matrix, RESP2/RESP3, error accounting,
slowlog/commandstats/latency and all-mode tests pass; no wire or dependency
change. Further merging of per-session flags into the word is optional, not a
current performance claim.

### 4.4 Gate elision when no exclusive operation can run (not implemented, higher risk)

Replace the unconditional transaction-gate `RwLock` read on every command with a
fast path that skips it while no journal, MULTI/EXEC or exclusive maintenance is
registered, re-checking before execution. This is only acceptable with a
linearization argument and stress tests for concurrently starting transactions,
journal append and multi-key commands.

Acceptance: a concurrency model note reviewed by the owner; deterministic tests
for gate acquisition races; no atomicity/expiry/accounting regression; measured
gain only if it survives both gates. Reject if the proof is not small.

### 4.5 Cross-client write coalescing (partially implemented)

PR #152 batches owner replies without blocking the reactor, which covers the
multi-owner reply hop. Cross-client write coalescing on the default reactor
(Valkey's `beforeSleep` shape) remains deferred: writing inside each service call
is still the lower-latency P1 choice.
Bounded by output bytes and client count. Risk: added latency and buffer growth;
must not worsen P1 p99 or backpressure.

Acceptance: p99 not worse in any gate row; bounded memory; slow-reader and
shutdown tests; only pursued if 4.1's evidence shows writes are the remaining
cost.

### 4.6 Sharded keyspace or parallel command execution (deferred)

A deterministic routing seam (#140) and opt-in multi-owner execution (#145) now
exist, but the default remains one owner because it is still the fastest verified
mode. Sharding the live keyspace would complicate multi-key atomicity, expiry,
WATCH and journal ordering; the roadmap in
[Dragonfly-class performance](dragonfly-class-performance.md) treats multi-owner
scaling as a separate phased program with its own acceptance, not as a default
switch.

### 4.7 Short-read drain elision (rejected)

Mechanism, local syscall evidence, failed gate rows and diagnosis are in section
2.6. The empty receive is retained: it is also a user-space readiness batch, and
removing it shifted cost to `epoll_wait` and lost two Valkey P1 medians. Do not
revive it without a different readiness mechanism and fresh gate evidence.

### 4.8 Owner-migration socket workers and accept distribution (merged; count rejected)

One reactor is the only acceptor, keeps the owner-local executor, maintenance and
every data command, and assigns new connections round-robin to the other reactors
so a shared-listener wakeup cannot skew connection-local work onto one worker. The
merged default remains `min(4, allowed CPUs)`. A `2 × allowed CPUs` count, capped
at eight, was measured and rejected on pipeline-1 ping (section 2.8). The other
reactors own their assigned connections and execute only bare
connection-local commands (PING/ECHO/QUIT). Before the first data command a worker
flushes prior replies, deregisters the socket, transfers the session and unparsed
input, and hands the connection to the owner, which registers it and continues in
reply order. `--socket-workers 1` keeps the single owner-local reactor; fixed
workers, TLS, Tokio and multi-owner paths are unchanged. No key is sharded and
there is no per-command owner hop.

The mechanism directly follows the rejected shared-store result: PING needs
connection-local scaling, while data needs the owner-local executor. Risks are
migration ordering with pending output, shared-listener accept distribution,
admission across reactors, idle maintenance, and joined shutdown with in-flight
transfers. Tests cover worker counts, PING/SET/GET and mixed pipelines, session
state across migration, transactions, pub/sub, blocking commands,
large/backpressured output, idle timeout, max clients, session isolation and
shutdown. PR #159 merged this mechanism with both four-CPU gates passing; PR #161
added round-robin distribution but failed Dragonfly at `min(4, CPUs)`. The
`2 × CPUs`/eight count then failed both gates on P1 ping and is rejected
(section 2.8); do not revive it without a different scheduling mechanism and
fresh evidence. Both protected gates plus the unchanged functional contexts
decide every count change; no throughput claim is made before they pass.

### 4.9 Cached command-time snapshot (not implemented)

Mechanism: publish one wall-clock millisecond value per reactor pass and use it
for expiry/eviction checks instead of `SystemTime::now()` per command, matching
the `commandTimeSnapshot()` shape without changing single-command time
consistency ([section 3.7](#37-clock-expiry-and-command-time-snapshots)).

Prediction: a TTL-heavy P1 workload (every key has a deadline) drops one
`clock_gettime` per command and removes the `SystemTime` conversion from the
command path; non-TTL rows are unchanged because `now()` already returns `0`.

Rejection condition: no measured syscall/time reduction, any expiry boundary
error (a key alive at command start must stay alive for that command), any
original-gate or four-CPU gate loss, or any p99 regression. The default gate has no TTL
rows, so this candidate must first add a matched local TTL workload; it is not
justified by the current gate alone. No dependency is needed: the value can be
stored behind the existing `Clock` trait and refreshed on the reactor maintenance
tick.

Acceptance if implemented: deterministic tests cover a key expiring between two
commands but not within one, a pass that refreshes the snapshot, injectable test
clocks, and the same expiry/accounting behavior; the matched local A/B shows
fewer clock calls; both protected gates still pass.

### 4.10 Command-counter padding A/B (diagnostic, not implemented)

Mechanism: PR #165 changed the 64 sharded `total_commands_processed` atomics from
packed 8-byte entries to 128-byte-spaced entries. The two post-refresh cohorts
passed both four-CPU gates with larger P1 ping margins (section 2.10), but a
gate pass cannot separate the layout effect from shared-runner variance.

Prediction: on the same host, with an otherwise identical binary, the 128-byte
layout should have lower P1 ping cost than the packed layout only when several
connection-local reactors execute PING/ECHO. With `--socket-workers 1` the
counters are written by one reactor, so the difference should disappear. The
effect should also be absent on data-only P1 rows, which execute on the owner.
A hardware-counter check (cache-to-cache transfers or store-buffer contention on
the counter lines) is supporting evidence, not a throughput proxy.

Baseline and method: freeze both binaries (`scripts/compare-builds.py`), alternate
them on one host and one CPU placement, use the same client and at least three
rotated repetitions, and retain raw JSON plus spread. Run both reactor counts
(1 and the default) to test the interaction. Do not compare numbers from
different hosts or from the protected runner.

Rejection condition: no difference outside the measured spread at multiple
reactors, or a difference that persists with one reactor (which would point at
something other than cross-reactor false sharing). A rejected A/B leaves the
layout change in place only if the gates remain green; it does not justify a new
layout knob.

Acceptance if completed: the matched A/B establishes direction or rejects the
mechanism, all raw samples and CPU placement are retained, and both protected
gates still pass on any resulting head. This is a diagnosis of an already-merged
change, not a new runtime candidate.

## 5. Required libraries, dependencies and policy impact

| Package | Version | Why | Vendored | License | Targets |
| --- | --- | --- | --- | --- | --- |
| `mio` | `=1.2.4` | readiness backend for the default reactor and Tokio's I/O driver | yes | MIT | Unix; Windows code paths compile but are not used by the reactor |
| `tokio` | `=1.54.1` | opt-in async plaintext transport, co-located with the owner executor | yes | MIT | Unix |
| `core_affinity` | `=0.8.3` | safe best-effort worker pinning for the opt-in owner/transport modes | yes | MIT | Unix/Windows |
| `libc` | existing | already required by ring/rustls and readiness crates | yes | MIT/Apache-2.0 | Unix |
| `windows-sys` 0.61.2, `windows-link` | transitive | Mio/Tokio/core-affinity Windows backends only | yes | MIT/Apache-2.0 | Windows |
| `bytes`, `socket2`, `pin-project-lite`, `num_cpus` | transitive | Tokio/net/affinity support; already allowlisted | yes | MIT/Apache-2.0 | build/target dependent |

Policy impact (explicit, not incidental):

- [`docs/requirements.md`](../requirements.md) authorizes rustls, Tokio (with Mio
  as its readiness backend) and `core_affinity`, plus required transitive crates.
  This was a governance change: the dependency-policy test, README/description,
  Cargo manifest and this design must agree.
- `tests/policy.rs` allowlists the reviewed packages and keeps every other
  target/feature graph rejection. Vendoring preserves offline builds and
  checksum verification; the application crate still forbids unsafe code.
- Dependency internals use `unsafe` FFI, exactly as the standard library does.
  The project's safety claim is about project code and the reviewed, vendored API
  boundary. This is disclosed, not implied away.
- CI/policy files are code-owner owned; each extension required the configured
  review and cannot be self-approved or merged by bypass. Any further addition
  takes the same path.
- Rejected libraries: direct `libc` epoll/io_uring (would put `unsafe` in project
  code), alternative async runtimes (`async-std`, `smol`, `polling`, `nix`) with
  more surface or no measured need, `mimalloc`/`jemalloc` (allocator change),
  `dashmap`/`ahash` (storage/hashing policy and multi-key semantics), any
  benchmark-only dependency.

Operationally, the default remains the Mio one-owner reactor; Tokio transport,
multi-owner execution and worker pinning are opt-in. TLS, fixed workers and
non-Unix builds keep their existing fallbacks. A future default change needs the
same native evidence and protected gates as any other runtime change. The
rejected eight-reactor experiment and candidate 4.9 add no package: they stay
inside the reviewed Mio/std surface. Level-triggered readiness would need either raw `epoll`
in project code or a new selector dependency, so it is a policy decision, not a
free tuning change.

## 6. Risks

- **Edge-triggered correctness:** every readiness event must be drained to
  `WouldBlock`; the runnable queue and token identity checks are the safety net.
  Lost wakeups would appear as stalls, so tests cover fragmented frames, half-close
  behind queued bytes, backpressured replies and writable-interest removal.
- **Fairness and starvation:** a client with a large pipeline can occupy bounded
  quanta; other ready clients are served in the same iteration, and the run
  quantum bounds one iteration.
- **Async latency:** blocked/pubsub/monitoring clients are retried on a 1 ms tick,
  not readiness; this is a delivery-latency bound, not zero-latency wakeup.
- **Idle timeout granularity:** connection expiry is checked on the 100 ms
  maintenance tick, so closure can be up to 100 ms late.
- **Single-thread ceiling:** the reactor executes commands on one thread; P16
  wins come from cheaper per-command work, not parallelism. A future CPU-bound
  workload could need workers, with the atomicity caveats in 4.6.
- **Reactor oversubscription:** `2 × CPUs` connection-local reactors on a
  four-CPU host means more runnable threads than CPUs. The measured result was a
  22.4% P1 ping loss to Valkey and a 9.3% loss to Dragonfly while every data row
  passed (section 2.8). Keep the default at or below the CPU budget; adding
  reactors is not a P1 latency mechanism.
- **Accept-lane skew:** one owner accepts and assigns round-robin. A worker that
  stalls can delay its lane's new connections while other lanes continue; the
  owner keeps accepting and never blocks on a lane send, and admission remains a
  single shared bound.
- **Platform scope:** non-Unix and TLS stay on the existing fallback; no
  readiness claim is made there.
- **Dependency risk:** a new vendored crate adds supply-chain and review surface;
  the pin, checksum and policy test reduce but do not remove it.
- **Measurement risk:** shared runners can fail a PR through noise; the response
  is a genuine source change and fresh evidence, never a retry loop or threshold
  change.
- **Policy risk:** the Mio authorization widened a stated constraint. It is
  reviewed and merged, but each further crate needs the same explicit review;
  "already vendored" is never authorization by itself.

## 7. Rejected options

| Option | Why rejected |
| --- | --- |
| Raw `epoll`/`kqueue`/`io_uring` via `unsafe` FFI | Violates safe-Rust/no-FFI project policy; error-prone lifetime and cancellation handling. |
| Busy polling as the default | Spends a core, worsens idle CPU/energy and tails; the earlier bounded spin policy is already evidence-limited. |
| Unconditional post-reply `yield_now` | Previously regressed batched throughput; only a narrowed policy survived. |
| Two independent handoff groups at four CPUs without a verified SMT-disabled hint | Failed 5/10 on a two-core/four-thread runner; retained as failed evidence. |
| Copying Redis/Valkey I/O-thread pools | Adds synchronization and configuration complexity; command execution ordering and atomicity would need a separate design. |
| Sharded keyspace / per-shard locks | Weakens or complicates multi-key atomicity, WATCH, expiry and journal ordering for a P16 regime that already wins. |
| Custom allocator or non-keyed/weak hash | Dependency/policy and HashDoS risk; no measured need at gate keyspace sizes. |
| Zero-copy reply references / encoded buffers | Needs reference counting and unsafe-adjacent ownership; formatting is already allocation-free per reply. |
| Copying level-triggered `epoll` semantics | Requires raw `epoll` FFI in project code (safe-Rust violation) or a new selector dependency; Mio's edge-triggered probe is load-bearing, as PR #155 showed. |
| Copying Valkey's I/O-thread reply copy avoidance | Depends on encoded-buffer reference tracking and an I/O-thread pool; no gate evidence that the current copy path is the remaining cost. |
| Starting more connection-local reactors than CPUs (`2 × CPUs`, max eight) | Failed both four-CPU gates on P1 ping (Valkey −22.4%, Dragonfly −9.3%) while data rows passed; the extra threads oversubscribe the four-CPU budget (section 2.8). |
| RDB/replication work as a performance fix | Out of scope and unrelated to the measured P1 gap. |
| Eliding the edge-triggered empty-receive drain probe (PR #155) | Cut receive syscalls ~16% locally but lost two Valkey P1 medians; the probe is also a user-space readiness batch. See 2.6/4.7. |
| Retrying the four-CPU gate until it passes, or lowering/omitting cases | Forbidden by the quality gate; destroys the meaning of the evidence. |

## 8. Acceptance criteria (EARS)

Definitions: "the four-CPU gate" is the unchanged official
`valkey-benchmark` workflow; "the original gate" is the unchanged 20-case
workflow; "a matched local A/B" means the same host, toolchain, release profile,
frozen binaries and alternating order, with raw samples retained.

- **AC1 (readiness default).** When Rillcache is built for Unix and started in
  default plain-TCP mode, the server shall use kernel readiness and shall report
  that policy through `scheduling_policy()` and embedded benchmark JSON.
- **AC2 (no lost work).** When a client sends fragmented, pipelined, half-closed
  or backpressured input, the server shall deliver every reply exactly once and
  in order, and shall not stall other clients.
- **AC3 (four-CPU acceptance).** When the final head is measured by the four-CPU
  gate, Rillcache shall have a strictly higher median than Valkey in all 10
  cases; otherwise the change shall remain unmerged with the losing evidence
  retained.
- **AC4 (original acceptance).** When the same head is measured by the original
  gate, Rillcache shall win all 20 cases; no historical pass shall substitute.
- **AC5 (tails and CPU).** When either gate reports p99 and sampled CPU, P1
  batch p99 shall be no more than 5% above the pre-change head in any row, or
  lower than Valkey's p99 in that row, and sampled CPU shall not exceed the
  pre-change head by more than 5% in any P1 row. Every regression shall be
  reported even when throughput passes.
- **AC6 (bounded I/O).** When one client pipelines large frames or stops reading,
  per-client input/output shall remain bounded, other clients shall progress, and
  the accept path shall reject over-limit clients as before.
- **AC7 (lifecycle).** When shutdown is requested in any mode, all client
  threads/reactors shall join, sockets shall close, and pending journal sync shall
  complete within the documented bound.
- **AC8 (fallbacks).** When TLS, fixed workers, non-Unix or an unsupported
  readiness setup is used, behavior, limits and tests shall remain those of the
  existing implementation.
- **AC9 (policy).** When the dependency graph is inspected for any
  target/feature, only the reviewed rustls/Tokio/Mio/core-affinity packages and
  their required transitives shall appear; the application crate shall keep
  `#![forbid(unsafe_code)]`; vendored offline builds and checksums shall verify;
  CI/policy changes shall have owner review.
- **AC10 (evidence).** When a performance claim is published, it shall include
  source identities, runner/host, CPU placement, order, raw medians/spreads,
  tails, failures and limitations; no claim shall exceed the measured scope.
- **AC11 (parser, when 4.2 is implemented).** When a single frame arrives in K
  reads, the parser shall rescan at most the current incomplete argument, so total
  scanned bytes are O(frame) rather than O(K·frame); malformed/limit behavior
  shall be unchanged.
- **AC12 (observers).** When any observer, auth, ACL, pause, MULTI or subscriber
  state is active, the checked path shall run and accounting shall match the
  existing matrix; the fast path shall be used only when the mode word says it is
  safe.
- **AC13 (Dragonfly).** When the unchanged four-CPU job compares Dragonfly, the
  final head shall have a strictly higher median in all ten workload/pipeline
  cases, with a losing row retained as failed evidence rather than explained
  away; the Valkey comparison and original gate remain independently required.
- **AC14 (owner migration).** When plain TCP starts with multiple reactors, a
  connection shall change reactor ownership at most once before its first data
  command, with prior replies flushed and order preserved; PING/ECHO shall remain
  scalable on workers; data commands shall execute on the owner-local executor;
  admission, idle timeout and shutdown shall join every reactor and in-flight
  migration. A throughput pass is valid only if both protected gates pass.
- **AC15 (reactor count).** When a candidate changes the default plain Unix TCP
  reactor count above the CPU hint, it shall start at most
  `2 × allowed CPUs` (one on a single CPU), report the count through
  `scheduling_policy()`, assign new connections round-robin, and keep
  `--socket-workers 1..16` as an override. It is accepted only if the original
  gate wins 20/20 and both four-CPU comparisons win 10/10 on the final head.
  The `2 × CPUs`/eight candidate failed both four-CPU gates on P1 ping and is
  rejected; the merged default stays at `min(4, CPUs)`.
- **AC16 (cached clock, only if 4.9 is implemented).** When any key has an
  expiration, a command shall observe one stable millisecond snapshot for its
  whole execution, refreshed at most once per reactor pass; a key alive at
  command start shall not expire inside that command; injectable test clocks and
  existing expiry/accounting tests shall pass; a matched TTL-heavy A/B shall show
  fewer clock calls, and both protected gates shall still pass.

INVEST story for the next runtime increment (Dragonfly pipeline-1 margin):
independently valuable and testable, with one recorded mechanism, prediction,
matched baseline and rejection condition. Definition of done: AC1–AC15 as
applicable, `make ci`, both performance gates, all 14 contexts, an up-to-date
head, owner review for any dependency/policy change, and a concise evidence
report that keeps every failed cohort. The phased multi-owner/socket-worker
program in [Dragonfly-class performance](dragonfly-class-performance.md) is the
current roadmap; individual phases are separate increments.

## 9. Reproduction and evidence map

- Current gates: `python3 scripts/pr_gate_policy.py --live`; the two performance
  workflows are the acceptance authority. Required contexts are listed in
  [pr-quality-gate](../pr-quality-gate.md) and
  [`.github/pr-workflow-gates.json`](../../.github/pr-workflow-gates.json).
- Gate artifacts retain raw medians, spreads, p99, CPU and hashes; read the
  manifest's `candidate_source`, `complete` and `gate_passed` before citing.
- Dragonfly comparison: `python3 scripts/test-dragonfly-performance.py` and
  `python3 scripts/pr-dragonfly-performance.py --check <artifact>`; the design
  and phased roadmap are in [Dragonfly-class performance](dragonfly-class-performance.md).
- Rejected short-read experiment: PR #155, gate run 41368946995 (Dragonfly 10/10,
  Valkey 8/10), branch `perf/reactor-short-read-drain` retained.
- Round-robin at `min(4, CPUs)`: PR #161, four-CPU run 41420512129 (Valkey 10/10,
  Dragonfly 8/10: ping P1 −7.0%, ping P16 −2.1%), branch
  `perf/accept-distribution` retained.
- `2 × CPUs`/eight-reactor count: PR #164, original gate PASS 20/20
  (run 41441783049); four-CPU run 41441782769 failed Valkey 9/10 (ping P1 −22.4%)
  and Dragonfly 9/10 (ping P1 −9.3%). Artifacts retained under
  `/root/perf-evidence/pr164/` in the development environment; branch commits
  retained, runtime change reverted, design record kept here.
- Post-refresh pass (section 2.10): PR #164 head `c1564aa` on base `9692e07`,
  original gate run 41473631832 PASS, four-CPU run 41473631873 PASS Valkey 10/10
  (ping P1 +7.36%) and Dragonfly 10/10 (ping P1 +16.48%); artifacts under
  `/root/perf-evidence/pr164/four-cpu-pass/`; merged as `f28fe60`.
- Repeated post-refresh pass: PR #169 head `4f0ed0c`, four-CPU run 41482049139
  PASS Valkey 10/10 (ping P1 +2.88%) and Dragonfly 10/10 (ping P1 +14.14%);
  artifacts under `/root/perf-evidence/pr169/four-cpu/`; merged as `ffa2181`.
- Counter-layout A/B (section 4.10): not yet run; requires frozen packed and
  padded binaries on one host, both reactor counts, and raw samples.
- Development loop: `make dev-check` for editing feedback, `make test` after a
  complete logical change and `make ci` before review, per
  [development-cycle](../development-cycle.md). Local screens never replace the
  protected gates.
- Local paired diagnostic used in 2.4 (diagnostic only):
  `perf stat -p <server-pid> -e 'syscalls:sys_enter_recvfrom,...' -- sleep <n>`
  around a `valkey-benchmark` run with both servers pinned to the same CPUs.
- Store-path and instruction A/B: `scripts/compare-builds.py` and the documented
  hot-path study, with frozen binaries and retained raw JSON.
- Dependency verification: `make ci` runs the policy test, which executes
  `cargo tree` for all targets/features and rejects non-allowlisted packages.

## 10. Limits and open questions

- The protected gate uses small strings, cyclic/random keys, eight clients and
  short three-sample medians. It is a merge policy, not a statistical proof or a
  production SLO; mixed TTLs, large values, saturation and tail SLOs remain
  outside it. It compares Valkey and Dragonfly; Redis 8.10.2 is pinned in the
  source analysis and the older one-CPU study, not in the required four-CPU
  comparison, so a Redis four-CPU loss would not be visible to the gate.
- The P1 ping row is host-sensitive. The refreshed runtime passed both four-CPU
  comparisons twice (section 2.10), but earlier identical binaries also lost that
  row by up to 4.32% on the same shared runner (section 2.9). Readiness was
  necessary but not sufficient, and the rejected drain probe shows that syscall
  counts
  alone are not a throughput proxy. Round-robin at four reactors improved
  Valkey but still lost Dragonfly PING; the eight-reactor response then lost P1
  ping to both and is rejected. The remaining P1 question needs a different
  scheduling mechanism, not a larger thread count.
- The four-CPU runner used for the recent cohorts has `client_cpus` equal to
  `server_cpus` (`[0,1,2,3]`), so the load generator shares the server CPUs.
  That biases a multi-reactor server against a single-thread competitor and puts
  the P1 ping row inside a +2.9% to −4.3% run-to-run band for an unchanged
  runtime
  (section 2.9). A disjoint-client-CPU runner would measure the server's lower
  p99 without generator contention; changing the runner is a CI/policy decision,
  not a server tuning knob.
- Cross-client write coalescing (4.5) remains unproven; do not add it
  speculatively. The resumable parser (4.2) is deprioritized because incomplete
  frames do not rescan payload bytes; revisit only with large-argument evidence.
- The reviewed dependency exceptions are rustls, Tokio, Mio and
  `core_affinity`, merged through their own PRs. A future head still needs fresh
  gates, and any further dependency needs its own review. The portable fallback
  remains the documented boundary for TLS, fixed workers and non-Unix builds.
- The short-read elision is a rejected experiment, not an unfinished task: its
  branch and gate artifacts are retained, and it must not be revived without a
  different readiness mechanism and fresh evidence.
- Redis 8.10.2 and Valkey 9.1.2 are pins, not an evergreen "latest" claim; source
  links are immutable tags.
