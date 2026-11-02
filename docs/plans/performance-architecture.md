# Performance architecture: measured gaps and candidate designs

**Status: RESEARCH. This document changes no runtime behavior.** It records where
Rillcache lost to Redis and Valkey, what those projects actually do on the same
paths, which designs the repository adopted or rejected under its safety and
dependency policy, and what evidence was required. The bounded Mio readiness
reactor merged as PR #139 (`4a52fc0`) after passing both protected gates on its
merge candidate (section 2.3); this document merged as PR #141 (`f572bd8`).
Later increments added opt-in Tokio transport, multi-owner execution, cached
mode/authorization words and batched owner replies, and the required four-CPU job
now also compares Dragonfly (sections 2.5–2.6). Two further runtime experiments —
the short-read drain elision (#155) and shared-store socket workers (#157) — were
measured and rejected (sections 2.6–2.7); the next multi-reactor design remains a
candidate (section 4.8), not a default change. A gate pass on one head never
substitutes for a fresh head, and every dependency still needs explicit review.

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

### 2.7 Shared-store socket workers (PR #157, rejected)

Candidate: default plain Unix TCP runs `min(4, CPUs)` independent Mio reactors,
each owning disjoint connections over the existing locked Store; no sharding,
migration, io_uring or dependency change. The original 20-case gate rejected it
**12/20** (run 41380968312):

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
data rows collapse because shared `StoreHandle`/bridge/lock execution is not the
owner-local data path; the one-CPU budget selects one reactor, keeps the local
executor, and passes every row. Diagnosis: multiple reactors are viable only for
connection-local commands; data commands must stay on the owner-local executor.
The PR was closed with its branch and evidence retained, not rerun or relabeled.

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

**Verdict:** the kernel-readiness idea was copied and adapted to a reviewed safe
wrapper (Mio, section 4.1). Raw `epoll` with `unsafe` and busy polling by default
were rejected.

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
counting and unsafe/ownership machinery). A read-only/batched-write experiment is
a later, separate candidate (4.5) and must not worsen P1 tails.

### 3.3 Parsing

- **Valkey** parses incrementally with per-client state (`reqtype`, `multibulklen`,
  `bulklen`, `qb_pos`) and keeps arguments borrowed from the query buffer for
  small arguments, copying only large/long-lived objects; the command queue lets
  it defer execution without reparsing
  ([`networking.c#L4237`](https://github.com/valkey-io/valkey/blob/9.1.2/src/networking.c#L4237),
  [`networking.c#L4299`](https://github.com/valkey-io/valkey/blob/9.1.2/src/networking.c#L4299)).
- **Redis** follows the same incremental multibulk/inline state machine.
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
  expires structure and is checked on lookup.
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
  ([`server.c` `call()`](https://github.com/valkey-io/valkey/blob/9.1.2/src/server.c#L3830)).
- **Rillcache** has an inlined PING/GET/SET/ECHO path that avoids family routing
  and the transaction gate, but it first evaluates a chain of about a dozen
  mode/observer atomics and four case-insensitive command comparisons
  ([`session/mod.rs`](../../src/session/mod.rs)). Every command then takes the
  transaction-gate read lock and the data read/write lock even when no journal,
  transaction or observer is active.

**Verdict:** adapt by collapsing the observer/mode checks into one atomically
published mode word (4.3) and by making the transaction gate a fast no-op when no
exclusive operation is registered (4.4). Do not copy the full command table
abstraction; the current split is easier to keep out of the hot path.

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

## 4. Candidate architectures

Each candidate names its mechanism, expected effect, cost/risk and acceptance
evidence. Section 4.1 and the mode-word work (4.3) are implemented; 4.2 was
deprioritized after analysis, 4.5 is partially implemented, and 4.7 is a rejected
experiment.

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

### 4.8 Multi-reactor with one-time owner migration (specified, not implemented)

One owner reactor keeps the local executor, maintenance and every data command.
N−1 worker reactors accept connections and execute connection-local commands
(PING/ECHO/QUIT). A connection migrates once to the owner when its first data
command appears: prior replies flush, the unparsed input/session/buffers move,
the fd is deregistered and registered on the owner, and reply order is preserved.
PING scales across workers; SET/GET/P16 keep the proven owner-local data path.
This is intentionally different from the rejected shared-store experiment and
from per-command shard hops.

Risks: migration ordering and pending-output/backpressure handling, accept
distribution, max-client admission across reactors, idle-timeout maintenance, and
joined shutdown with in-flight migrations. Acceptance: both protected gates, a
migration/pipeline/session-state test matrix, no lost or reordered replies, no
new dependency, no sharding/io_uring/unsafe code. Not started; it is a separate
large default-path increment, not a documentation claim.

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
same native evidence and protected gates as any other runtime change.

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
| RDB/replication work as a performance fix | Out of scope and unrelated to the measured P1 gap. |
| Eliding the edge-triggered empty-receive drain probe (PR #155) | Cut receive syscalls ~16% locally but lost two Valkey P1 medians; the probe is also a user-space readiness batch. See 2.6/4.7. |
| Shared-store multi-reactor data execution (PR #157) | PING scaled +53% but data collapsed -57.6% at P1 and about -94% at P16; the owner-local executor is required. See 2.7/4.8. |
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
- **AC14 (owner migration, when 4.8 is implemented).** A connection shall change
  reactor ownership at most once before its first data command, with prior replies
  flushed and order preserved; PING/ECHO shall remain scalable on workers; data
  commands shall execute on the owner-local executor; admission, idle timeout and
  shutdown shall join every reactor and in-flight migration.

INVEST story for the next runtime increment (Dragonfly pipeline-1 margin):
independently valuable and testable, with one recorded mechanism, prediction,
matched baseline and rejection condition. Definition of done: AC1–AC10 and AC13
as applicable, `make ci`, both performance gates, all 14 contexts, an up-to-date
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
- Rejected shared-store socket workers: PR #157, original-gate run 41380968312
  (12/20), branch `perf/socket-workers` retained; the bundle also contains the
  design-doc update from the closed #156.
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
  outside it.
- The remaining measured P1 gap is against Dragonfly on fast hosts; it is
  host-sensitive and still fails the strict median on some cohorts. Readiness was
  necessary but not sufficient, and the rejected drain probe shows that syscall
  counts alone are not a throughput proxy.
- Cross-client write coalescing (4.5) remains unproven; do not add it
  speculatively. The resumable parser (4.2) is deprioritized because incomplete
  frames do not rescan payload bytes; revisit only with large-argument evidence.
- The reviewed dependency exceptions are rustls, Tokio, Mio and
  `core_affinity`, merged through their own PRs. A future head still needs fresh
  gates, and any further dependency needs its own review. The portable fallback
  remains the documented boundary for TLS, fixed workers and non-Unix builds.
- The short-read elision and shared-store socket workers are rejected
  experiments, not unfinished tasks: their branches and gate artifacts are
  retained, and neither may be revived without a different mechanism and fresh
  evidence. The next credible design is the one-time owner migration in 4.8.
- Redis 8.10.2 and Valkey 9.1.2 are pins, not an evergreen "latest" claim; source
  links are immutable tags.
