# Profile-guided hot paths (S16–S18)

This increment targets measured costs, not an all-workload performance victory.
The baseline is `3dca0c7fb4beb9923bd05f0d891602a5cd9b3790`. The published
[comparative dashboard](../benchmarks/README.md) identifies its own tested source;
container/correctness smoke and local experiments are not a newer Redis comparison.

## Current-source diagnostic follow-up

The [2026-10-27/28 source-pinned bottleneck study](../benchmarks/studies/current-bottlenecks/README.md)
measures collection/cardinality and administrative-query scaling after the later
compatibility work. It retains 84 checked component samples, controls, CPU profiles,
losses and limits. It is not a replacement native comparison or an optimization
claim; the historical profiles below remain labeled with their original baseline.

## Investigation

The preceding published comparison was weakest with one logical server CPU and
pipeline 1, including PING. PING does not touch the key map: the global map lock
cannot explain the entire gap. Separate user-space `perf` profiles covered PING,
SET, GET, rejected-existing NX and admitted-existing XX; `strace` investigated
blocking/fixed-worker/Valkey scheduling. Frozen normal-release server/client
binaries were kept separately from symbolized profiling builds. Instrumented
throughput is deliberately excluded from elapsed-time claims.

Examples of baseline **sampled user cycles**, not total CPU or wall time:

- SET: `expire_key` 16.64%; two hashing symbols together 22.47%; malloc/free
  together 10.15%. The implementation repeatedly looked up an existing key and
  allocated a fresh value on every equal-size overwrite.
- Existing NX: `expire_key` 26.33%, despite the benchmark containing no expiring
  keys. Existing XX: `expire_key` 17.59% and the two hashing symbols 23.54%.
- GET: allocation/free and generic decimal formatting appeared alongside parsing
  and engine work. Source inspection confirmed a redundant successful lookup.
- PING and syscall traces exposed argument-vector allocation and blocking/sleep
  behavior even without map access. Fixed workers could park for 1 ms immediately
  after sending replies, before their clients' next requests arrived.

These percentages are one profile, use symbolized binaries, exclude kernel work,
and are not additive end-to-end speedups. Kernel I/O/scheduling remains important.

## What Valkey suggested—and what was not copied

Valkey **9.1.2**, alongside Redis **8.10.2**, was rechecked against upstream stable
releases during this investigation. The pins are not an evergreen latest claim.
Relevant Valkey sources:

- [`networking.c`](https://github.com/valkey-io/valkey/blob/9.1.2/src/networking.c):
  shared/retained query buffers, borrowed arguments, batching and specialized
  decimal reply headers instead of general formatting on every response.
- [`sds.c`](https://github.com/valkey-io/valkey/blob/9.1.2/src/sds.c): capacity-aware
  mutable strings. This motivates reuse, not a claim that Valkey's SET algorithm
  is identical to ours.
- [`t_string.c`](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_string.c):
  command-specific string/object paths and integer encodings.
- [`io_threads.c`](https://github.com/valkey-io/valkey/blob/9.1.2/src/io_threads.c)
  and [`ae_epoll.c`](https://github.com/valkey-io/valkey/blob/9.1.2/src/ae_epoll.c):
  coordinated I/O and real kernel readiness notification. Rust std has no portable
  socket-readiness multiplexer. We did **not** add FFI, unsafe epoll, a dependency,
  a new allocator, weaker hashing, or a pretend epoll implementation.

The Rust implementations are newly generated safe code, not transplanted C.

## Delivered changes and invariants

1. **Reuse validated state:** avoid expiry lookups when the exact expiry index is
   empty; reuse GET's lookup and SET's replacement-size information under the same
   lock. Randomized hashing, atomic admission and expiry reclamation remain intact.
2. **Reuse overwrite storage:** replace equal/similar-sized values in existing
   capacity. Empty or disproportionately smaller values release the old buffer
   instead of retaining an arbitrarily large allocation. Payload is still not RSS.
3. **Borrow small argument lists:** both server modes dispatch at most eight
   argument references on the stack. Larger commands grow bounded fallback
   storage. The owned `Request` API keeps its vector, without a second vector
   copy. Both APIs share validation and never execute incomplete/invalid frames.
4. **Specialize array/bulk headers:** a bounded stack buffer produces decimal RESP
   lengths. This removes generic formatting overhead; the old `write!` path did
   not necessarily allocate a temporary string. Wire bytes remain identical.
5. **Bound fixed-worker active polling:** after real I/O/command progress, each
   fixed lane polls for up to 50 µs before returning to the existing 1 ms idle
   park. Empty/spurious passes never rearm the window. This spends active CPU to
   avoid premature sleeps; it is not readiness notification or free CPU. The
   interval is checked between bounded work passes, not a hard real-time deadline.
   Default per-client threading is unchanged. Fairness, limits, backpressure and
   joined shutdown remain correctness gates. Embedded JSON records `active_poll_us`;
   reporting rejects missing/mismatched new policy evidence while reading legacy
   no-active-poll cohorts unchanged.

6. **Skip unused mutation history and hot-command dispatch.** WATCH versions are
   always maintained, but per-write journal-event key copies are collected only
   while append-only persistence or keyspace notifications need them; disabling
   both releases the bounded history immediately. `PING` bypasses family routing
   entirely, while `GET`, `SET` and `ECHO` call the small inlined strings hot
   entry directly from the engine before the family router (see
   [Dispatch module layout](#dispatch-module-layout)).
   Two local core runs measured store-path medians of roughly 40 → 22–27 ns for
   PING, 106 → 93–96 ns for GET and 335 → 278–312 ns for SET against the
   pre-change tree. These are isolated shared-VM store results with visible run
   variance, not network throughput or production claims.

Tests cover capacity shrink/retention, changed bytes, payload and expiry indexes,
OOM preservation/reaping, parser stack/fallback boundaries, fragmentation,
execute-once behavior, decimal digit boundaries/usize::MAX, window expiry without
rearming, concurrent writes, slow readers and shutdown in all three modes.

## Dispatch module layout

`src/engine.rs` keeps `Store::execute` / `execute_locked` as the entry point and
the direct PING fast path. Every other store command routes through a family
module in `src/engine/dispatch/`:

- `strings`: the GET/SET/ECHO micro-path plus the remaining string, bitfield,
  HyperLogLog and multi-key string commands;
- `keyspace`: key lifecycle, expiry and keyspace-wide commands;
- `collections`: list/hash/set/zset commands, scans, GEO and SORT;
- `streams`: `X*` commands;
- `pubsub`: `PUBLISH` and `SPUBLISH`;
- `admin`: configuration, server administration, discovery and unsupported
  command replies.

Connection state remains in the session module: `MULTI`/`EXEC`/`WATCH`,
subscriptions and blocking waits are not store-dispatch commands. `EXEC`
replays its queued commands through `execute_locked`, so those commands use the
normal family path.

PING stays in `engine.rs` ahead of family routing, and it calls the strings hot
entry (`GET`/`SET`/`ECHO`) directly and inlined before `route`. Add a new
command's arm and handler to its owning family module, not to a central match.
Mark a family entry point `#[inline]` only when the hot path needs it; the large
string remainder deliberately stays out of line so it does not bloat
GET/SET/ECHO.

## Evidence discipline and rejected experiments

Initial ad-hoc microbenchmark copies accidentally selected a stale `core-*`
executable from a glob. **All timing comparisons from those copies were discarded.**
Corrected captures use Cargo `--message-format=json` and the unique matching
`compiler-artifact.executable`. Server captures used explicit frozen paths and
were unaffected by that mistake. A parser wrapper that copied its temporary
arguments into a second vector regressed and was replaced; intermediate timing
results are not presented as delivered improvements. A thin-LTO experiment did
not justify changing the normal Cargo release profile.

[Corrected repeated local microbenchmarks](../benchmarks/studies/hotpaths-local/README.md)
preserve raw samples and spreads: store medians fell roughly 23–53% in that
small cohort, and the delivered owned parser no longer showed the intermediate
wrapper's regression. Matched store-path instruction diagnostics also fell;
these are isolated hot-path findings, not network throughput promises. The shared
six-CPU development VM produced severe timing variance (including roughly 67k–321k
SET commands/s for one intermediate TCP candidate). Those TCP samples cannot
establish an end-to-end win. The owned parser remains explicitly measured:
removing allocation from server dispatch must not hide a wrapper regression.
Fresh source-labeled runs, spreads, CPU and tails take precedence over these
exploratory numbers.

## Verified delivery results

The [archived native A/B study](../benchmarks/studies/d57cb2e/README.md) contains
360 independently checked records and a byte-identical 60-row generated table.
Default/inherited pipeline-16 SET throughput rose 40.8% and existing XX 34.1%;
one-CPU default gains were smaller. Separate user-instruction diagnostics fell
28–46% across all five workloads, not an equivalent wall-time reduction.

**Not all results improved:** default/inherited pipeline-1 XX fell 5.1% and
pipeline-16 GET fell 0.8%. Four workers sharing one CPU gained throughput but
worsened pipeline-1 batch p99 from roughly 1.2 ms to 4.4 ms. Evaluate `--threads 1`
when CPU-limited; more workers are not a latency guarantee. The fresh default-mode
Redis comparison still shows a 16.9% one-CPU pipeline-1 SET rate gap, with over
93% system-CPU share in those Rillcache store workloads. I/O/scheduling, mixed
traffic and oversubscribed-worker tails remain explicit optimization targets.

## Reproduce the matched before/after study

The manual [Hot-path before-after study](../.github/workflows/hotpaths.yml) builds
unmodified baseline sources and the selected main source with the same toolchain
and normal release profile. It freezes the **baseline client** for both servers,
alternates build order, and measures all five workloads with:

- per-client / one-worker / four-worker modes;
- inherited / one-logical-CPU server budgets, with the latter's clients on other
  allowed CPUs;
- pipelines 1/16, 20,000/1,000,000 requests, eight clients, 1024 keys, 64-byte values,
  1000 warmup commands/client and three repetitions;
- one live owned server at a time, a one-second idle sample, native process CPU
  and matched-thread accounting, whole-batch p99, executable/evidence SHA-256s;
- deterministic 60-row Markdown tables and raw JSON artifacts. Invalid/missing
  workloads, parameters, batches, rates, percentiles, process identity, CPU
  accounting, repetitions or changed binaries fail the study. Incomplete runs keep
  `complete: false`; an existing output directory is never silently reused.

This optional Linux orchestration uses Python 3's standard library and `taskset`,
not Rust dependencies or ordinary offline-CI prerequisites:

```sh
python3 scripts/compare-builds.py --self-test
python3 scripts/compare-builds.py \
  --before /absolute/frozen/baseline-server --after /absolute/frozen/candidate-server \
  --client /absolute/frozen/baseline-client \
  --before-source BASELINE_40_HEX_SHA --after-source CANDIDATE_40_HEX_SHA \
  --out target/new-study
```

Only run against disposable local servers; the tool writes benchmark keys. Never
substitute short acceptance-run timings for the full study. CPU counters are
quantized and their windows bracket request timing; inspect raw coverage and idle
samples before interpreting CPU/context switches. Shared runners are not controlled
hardware. A faster store, fewer user instructions, higher inherited-budget
throughput, or a bounded polling policy does not prove better per-core efficiency,
tails, mixed-TTL behavior, large values, every workload, or production readiness.

Current Redis/Valkey comparisons and genuine attributed Copilot assessments remain
in the separate [automatic reporting path](benchmark-reports.md). Future work
includes mixed traffic/TTLs, additional sizes/keyspaces, client/server saturation,
controlled hardware and a deliberate safe readiness/dependency boundary—not
speculative sharding that weakens multi-key semantics.

## Four-CPU pipeline-1 reductions

The official four-CPU gate stresses default thread-per-client mode at pipeline 1,
where per-request scheduling and shared-cacheline traffic dominate. Two general
reductions avoid benchmark-specific modes:

- **Sharded command counters.** `total_commands_processed` is accumulated in 64
  per-client shards and summed only when `INFO stats` is read. The previous single
  `AtomicU64` made every client thread contend on one cacheline for every command.
- **Lock-free PING.** A top-level PING with no active transaction or subscriber
  context writes `+PONG`/the echoed bulk/arity error directly from the session
  dispatcher. It no longer needs the Store transaction gate or data lock; queued
  PING and the Store fast path remain for `MULTI`/`EXEC`.

These are general server paths, not a benchmark mode. The bounded multicore client
handoff from the existing scheduler remains enabled for 2–4 CPU budgets. Burst
activity is observed across clients, not inferred from one request's pipeline
depth: a single-command reply may join an active epoch. The active-epoch check
short-circuits before the clock/window probe, the idle window bound is stored
in microseconds, and the coordinator takes one clock sample per pass instead of
one per client. Each connection reuses one borrower return channel and an inactive
lease shell. The latter retains the socket clone and placeholder session/receiver
instead of allocating/cloning/dropping them at every epoch; real buffers, session
and push receiver still move with their sole I/O owner. The existing recent-reply
window still prevents isolated replies from allocating a lease. A regression test
exercises single-command admission at CPU hints 2–4. Fixed dispatch, expiry,
accounting, 100 µs multicore idle / 50 µs one-CPU idle / 100 ms maximum epoch and shutdown semantics are unchanged.

Small-multicore policy v7 selects one coordinator when SMT is active or unknown;
four logical CPUs permit two independent round-robin groups only with an explicit
bind-time Linux SMT-disabled hint. V4's unconditional two-group cohort failed
5/10 on a two-core/four-thread runner and remains preserved, not accepted.
Empty multicore scans now use capped 1–64 processor-hint backoff, reset by real
progress, rather than immediately repeating futile socket syscalls. V7 holds a
multicore client for 100 µs of quiet instead of 50 µs so the lease spans the
pipeline-1 request interval. One-CPU v2, fixed-worker policies, the 100 ms
maximum epoch and shared transaction locks are unchanged. V6 also skips at most one rescan after a normal leased client's
empty poll, without postponing buffered work, pending output or asynchronous
session states. The hint clears before each decision; quiet borrowers still
service ready I/O before an idle return. Epoch/shutdown checks remain on every
pass. V5's 7/10 four-CPU result stays failed evidence. These changes add no
permanent thread, dependency or OS yield/sleep. Benchmark JSON records the actual
captured lane count. Tests exercise independent leadership, concurrent transaction atomicity,
reused partial-reply/push ownership and live eight-client ordering/shutdown.
The Rust Performance Book's [profiling](https://nnethercote.github.io/perf-book/profiling.html)
and [workhorse reuse](https://nnethercote.github.io/perf-book/heap-allocations.html)
guidance motivates these focused changes; shared-VM profiles are diagnostics,
not a substitute for either unchanged protected performance gate.

## INFO snapshot locking and unnecessary scans

A captured stalled `blocked_clients` test exposed a real data-RwLock deadlock:
INFO retained a read guard while `stats()`/`eviction_policy()` reacquired it,
with a blocking-pop writer waiting. [Rust documents this exact reader/writer
priority hazard](https://doc.rust-lang.org/std/sync/struct.RwLock.html).
INFO now copies data counters/policy, releases the guard before other helpers,
and reuses that snapshot. A controlled pub/sub-contention regression fails
boundedly against the old implementation rather than hanging on cleanup.
Unrequested keyspace sections no longer scan entries; requested keyspace metrics
share one pass instead of four. Existing TTL, lifecycle and payload meanings stay
unchanged; this is not an atomic cross-subsystem metrics or RSS guarantee.

## Bounded Linux idle admission waits (S-ACCEPTWAIT)

Default plain Linux admission now installs a positive 10 ms receive-timeout
before enabling blocking accept, avoiding the old accept/1 ms sleep loop when
no connection is pending. [Linux documents that SO_RCVTIMEO applies to
accept](https://man7.org/linux/man-pages/man7/socket.7.html). A safe owned duplicate
sets only the shared socket option; dropping it leaves the listener alive.
Accepted client timeouts are still reset from Config. Unsupported platforms or
setup failure retain nonblocking admission. Fixed workers, TLS, client handoffs
and their 100 ms maximum epoch are unchanged. Maintenance keeps its 100 ms target
between bounded waits; timer rounding/accept slack is not a hard deadline.

The separate [native diagnostic run 40867447709](https://github.com/macel94/rillcache/actions/runs/40867447709)
compared INFO head `956e5fe`, prototype `d9ecad9` and pinned Valkey 9.1.2 on
Ubuntu 24.04/EPYC 7763, two cores/four shared SMT threads. Three Latin-rotated
normal PING/P1 samples per variant used eight clients/two official-generator
threads, 8000 warmups and ten-second loads. Baseline median was 107,798.2
[106,294.0–107,867.5], candidate 108,673.2 [108,193.7–108,991.1] and Valkey
112,324.7 [108,364.7–114,487.9] req/s. The scoped candidate gain is about 0.8%;
it **still trails Valkey** and establishes neither an all-workload win nor tails.

Separate approximately two-second **instrumented no-load** owned-process captures
recorded accept/sleep calls of 1867/1867 versus 166/166, context switches of 1867
versus 332 and task-clock of 19.01 versus 4.89 ms; Valkey recorded 0/0 calls,
20 switches and 2.77 ms. These show substantially fewer admission wakeups, not
unprofiled CPU/throughput or production-energy guarantees. All nine normal CSVs,
six traffic profiles and three idle captures retain identities/affinity/lifecycles
and fresh distinct frozen binary hashes. The two protected performance gates
remain separate, mandatory, and unchanged.

## Release build profile

Release builds use fat LTO with one codegen unit while retaining overflow checks.
This is a general whole-program optimization for cross-module inlining of the
session/store/RESP path, not a benchmark-specific mode or a safety relaxation.
Build time increases; `make ci` still enforces the same tests and smoke benchmarks.
