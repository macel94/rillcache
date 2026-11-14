# Performance architecture: measured gaps and candidate designs

**Status: RESEARCH. Candidate 4.12 merged as PR #172 (`21ed301`) after passing
both four-CPU gates; 4.34 merged as PR #258 (`d334cde`); every other runtime item
here is historical or explicitly gated (4.15, 4.20–4.27, 4.29–4.33, 4.35–4.55).**
It records where
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
eight-reactor count is rejected. A later refresh onto the isolated command
counters passed both four-CPU gates twice (section 2.10), and a separate manual
cohort shows no measured rate loss to Redis or Valkey on five workloads, with the
remaining gap in CPU efficiency and scheduler churn (section 2.11).
Cross-candidate gate results show that changing the reactor count alone is not a
reliable P1 fix (section 2.12); the two-reactor default tried on this branch also
failed that row while every single-owner data row passed (section 2.13).
Candidate 4.12 supplied the missing mechanism: one owner-local reactor (the
batching optimum) plus a bounded non-blocking readiness poll. It merged as PR
#172 (`21ed301`) after both four-CPU gates passed (section 2.13). A clean-runner
upstream cohort then confirmed the merged default wins every measured row against
Redis and Valkey, with the remaining gap in P1 CPU and run-to-run variance rather
than rate (section 2.14); its P1 p99 is not consistently better and is an open
AC5 risk (section 2.15). Candidate 4.13 (adaptive bounded poll) is closed: the
miss-based and fixed-window variants are rejected analytically, and a measured
hybrid busy/sleep split did not reduce CPU and worsened p99. The P1 busy-window
cost and variance are the deliberate price of the P1 win; a local tracepoint
diagnostic attributes it to roughly eight `epoll_wait` calls per command from the
zero-timeout spin (section 2.16), and the headline 95% total is mostly the higher
rate with a small ~5–9% ping-specific per-command difference across two clean
cohorts. Further reduction needs a mechanism outside the safe-Rust/Mio boundary
(section 4.13). The Valkey four-CPU gate now uses five alternating rounds and
reports a diagnostic paired-bootstrap interval; the strict median pass rule is
unchanged. No gate pass substitutes for a fresh head, and every dependency still
needs explicit review.

This research refresh (November 2026) adds three things to that record. First,
the newest merged head `7b365dc` passed both four-CPU comparisons 10/10 with a
clear P1 ping margin (+5.05% against Valkey, +9.62% against Dragonfly) whose
paired interval still contains 1.0 (section 2.18), and a fresh five-server daily
cohort on the same head leads every measured row, with only four P1 rows inside
the advisory noise band (section 2.19). A second manual five-server cohort on
`bcd4768` (run 43164876654) repeats the throughput lead at +21.7–32.6% P1
against Redis/Valkey but reverses **all five** P1 p99 rows; the tail direction is
therefore explicitly host-dependent (sections 2.15, 2.19 and 2.22). Every
inspected gate manifest shares client/server CPUs `[0,1,2,3]` because the runner
has four logical CPUs, so placement is not a controllable variable there
(section 2.9). Second, source comparison locates a
structural reply-path difference — Redis 8.10.2 and Valkey 9.1.2 avoid the
value copy for bulk replies of at least 16 KiB — and an attempted local
large-value screen is retained as failed evidence because the shared host could
not resolve it (sections 2.20–2.21 and 4.15). Third, the counter-padding
diagnostic is closed by
analysis rather than a gate-rate A/B: after 4.12 the default has one reactor
writer, so the padded layout cannot affect the default rows (section 4.10).
Nothing here changes the default runtime; the surviving candidate needs a
measured large-value gap and its own protected evidence. A second pass extracts
per-request P1 latency and process CPU from the same retained gate samples
(section 2.22), records the failed docs-only cohort whose runtime was identical
to a passing head (section 2.17), and adds the latency-attribution diagnostic
4.19 with the conditional watch-free mutation candidate 4.20. It finds that
typical P1 latency is lower in every command while the extreme tail of mutating
commands is host-dependent, and that per-command CPU flips sign across hosts;
neither is a stable deficit. A hosted gate-rate profile (section 2.23) corrects
the local syscall picture: the bounded poll costs 0.20–0.36 `epoll_wait` per reply
and fewer context switches than Valkey, while the edge-triggered drain probe is
the remaining +12%/+37% total-syscall difference. Finally, the persistence-on
write-path source comparison (section 3.9) adds candidate 4.21: the C servers
buffer AOF bytes and write once per event loop, while we write one framed
transaction per command. A 2026-11-12 paired AOF-on cohort measures a **−66.5%
P1 / −90.2% P16** SET loss (P16 p99 15.1 vs 3.1 ms) against 8.5–33.5% for the C
servers, so 4.21 is admitted and AC23 sets the targets. The latest cohort (#217, section 2.18) resolves P1 ping
at +15.4% with an interval excluding 1.0, while the same runtime lost that row in
#216; the pair is kept as the measurement-risk evidence. The expiry and eviction
comparison (section 3.10) finds our ordered 1,280-reaps/s purge bounded and
acceptable, with a read-aware-LRU quality gap that stays unmeasured. The same
runtime also lost P1 ping again in #218 (−1.35%, 2/5), giving a three-cohort
identical-code spread of −1.35% to +15.4% (section 2.17). The key/value
representation comparison (section 3.11) locates the duplicate key buffers in the
expiry and WATCH indexes and defers an encoding rewrite to a TTL/eviction RSS
result. The blocking-wakeup comparison (section 3.12) is now measured, not just
source-based: after the LPUSH reply, Rillcache takes **1.16 ms p50 / 1.92 ms p99**
to wake a BLPOP against **0.03/0.22 ms** for Redis and Valkey on the same host
(900 wakeups each), and an idle blocked client costs **5.6% of a core (22.2% at
1000)** against <1% for the C servers — the largest measured latency and
idle-CPU gap; candidate 4.22 is admission-ready and AC25 gates the implementation.
A post-#226 re-measurement (5.0/21.4% for blocked, 4.6/22.4% subscribers,
4.2/26.6% monitors) shows the merged spin fix left the per-client scan intact.
Idle subscribers and monitors show the same tick cost — **6.0% of a core with one
idle subscriber, 22–27% with 1000**, against <1% for the C servers — and delivery
waits for the tick too: **1.05/3.60 ms** pub/sub p50/p99 and **0.37/6.15 ms** for
a MONITOR line, versus ≤0.08/1.19 ms for the C servers. Candidate 4.24 and AC27
cover the event-driven async-client shape. The rewrite comparison
(section 3.13) adds the measured key-list start pause against the C servers'
fork/COW rewrite: medians of **9–14 ms at 20k keys** and **91–107 ms at 200k**,
with full passes of 4.85 s and 43.4 s and concurrent GET p99 of 10.8/18.8 ms;
candidate 4.23 and AC26 gate it. The 2026-11-12 large-value retry (section 2.21)
has a usable 64-byte control and measures **1 MiB GET at −42% to −45% (p99
2.3–2.5×) and SET at −28% to −32%**, admitting 4.15 (reply copy) and 4.25 (input
growth). A 10k-field `HGETALL` is **43% behind Redis** (313 vs 552 req/s) while a
10k `LRANGE` leads by 39–75%, admitting 4.29 (large-hash reply path). A
10,000-member sorted set is the largest read-side ratio measured so far:
`ZRANGE 0 9` is 19.5× slower than Redis and 20.4× slower than Valkey (528 vs
10,311/10,771 req/s) and the full range is 67–78% behind, admitting 4.30
(sorted-set ordered index). Cursor iteration is the largest per-call gap found
in this record: a `SCAN COUNT 10` on 100k keys takes **42.6 ms p50 versus
0.256/0.164 ms** (167–260×) and a full traversal 4.81 s versus 0.239/0.167 s,
admitting 4.31 (incremental cursor). Collection storage is **~19× the RSS** of
the C servers for short list elements (142–143 vs 7.5 B/element) and makes
`DEL`/`UNLINK` of a 200k list **6.2–7.3 ms versus 0.13–0.32 ms**, while
`FLUSHALL ASYNC` takes 9.2 ms and saves nothing over `SYNC`; 4.32 (packed nodes)
and 4.33 (deferred drop) cover those. `APPEND`/`SETRANGE` on a 32 MB value cost
**7.2–7.35 ms versus 0.13–0.18 ms** because the whole value is rebuilt twice per
call, admitting 4.34 (in-place mutation). Collection writes are the widest gap:
one `HSET` into a 10k hash is **6.45 ms versus 0.11/0.09**, one `RPUSH` onto a
200k list **18.3 vs 0.08/0.07 ms**, and at 100k members `HSET`/`HDEL` are
**92.7–99.4 ms versus 0.08–0.32 ms** and `ZADD`/`ZREM` 61.4–63.9 versus
0.10–0.23, because every write clones and replaces the whole collection (4.35).
Random sampling is the widest cardinality-scaling loss: at 100k members
`SRANDMEMBER` is
**11.5 vs 0.2–0.3 ms**, `HRANDFIELD` **33.7 vs 0.11–0.34 ms** and
`ZRANDMEMBER` **21.3 vs 0.16–0.25 ms**, because each call clones the collection
(4.36). Bitmaps show both write-copy and scan gaps: on 16 MB, `SETBIT` is **5.3
vs 0.31–0.35 ms**, `BITCOUNT` **23.0 vs 3.4–4.6 ms**, and a one-byte `BITFIELD`
write **4.39–4.68 vs 0.087–0.12 ms** while its read-only form is at the floor
(4.37, extended). Sorted-set rank
and score reads are the largest ratio in the record at 100k: **57–72 ms** for
`ZRANK`/`ZRANGEBYSCORE`/`ZRANGE` versus 0.07–0.17 ms (378–927×), and `ZSCORE`
3.3 ms because it scans; 4.30 covers point/score/start-0 ranges and 4.38 the
rank-select paths. Lex ranges repeat it: a 10-element `ZRANGEBYLEX ... LIMIT 10`
costs **56.0–69.8 vs 0.18–0.43 ms** (163–371×) because it sorts the whole set
first, and a narrow `ZLEXCOUNT` is **15.0 vs 0.21–0.22 ms** because it scans
every member; 4.50 adds seek and span counts. Zset algebra repeats the copy/sort
shape: `ZINTERCARD` is **220.6 vs 40.4/17.1 ms** (5.5–12.9×) and
`ZINTERSTORE`/`ZDIFFSTORE` are 306.6/331.3 vs 102–111/61–62, because every source
is copied into an owned map and the result is re-sorted; the set-store variants
already lead, so 4.51 borrows the zset sources and builds one destination. The
`LIMIT` contract is ignored on both paths: `SINTERCARD`/`ZINTERCARD` with
`LIMIT 10` on 100k containers still cost **50.7/267.2 vs 0.20–0.25 ms**
(205–1,377×) because the limit is applied after the full intersection; 4.51
extends to the set path and an early exit. Observability has one keyspace-sized
walk: `INFO keyspace` (and plain `INFO`)
costs **2.79 ms at 100k keys and 10.0 ms at 500k versus 0.13–0.30 ms** (16–40×)
while `INFO stats`, `DBSIZE` and `PING` stay at the floor; 4.52 maintains the
counters. Moving one element between keys is the same cloning shape: `SMOVE` on
a 100k set is **25.5 vs 0.40/0.28 ms** and `LMOVE`/`RPOPLPUSH` on a 200k list
**39.2/41.2 vs 0.08–0.34 ms** (64–489×) because both keys are cloned and
re-encoded; 4.53 moves in place. List element edits repeat it: on a 200k list a
head `LINSERT` costs **19.1 vs 0.09/0.24 ms** (79–212×) and `LSET` at index 100k
**16.5 vs 0.14/0.28 ms** (59–118×), because the command flattens the whole list
first; 4.54 edits chunks in place. Stream consumer-group reads are
the counter-example: at a 10k-entry PEL, `XREADGROUP` history, `XPENDING` and
`XCLAIM` are at parity and the largest batch forms are only **1.3–2.1×** behind
(new read 1000 8.99 vs 5.19/6.43 ms, `XAUTOCLAIM` 1000 9.84 vs 4.67/7.50)
because only reply entries and PEL records are cloned, not the stream; section
3.39 records a measured null. Pub/sub loses a 2,000-message
burst at **1,024 delivered and
drops the subscription** (C servers: 2,000 and keep it), and a `PUBLISH` among
10,000 channels is 3–6× slower; 4.39 covers the burst contract and channel
lookup. Cardinality is equally materialized: at 100k, `SISMEMBER`/`SCARD` are
**8.0–8.7 vs 0.15–0.22 ms** because they clone and sort the whole set,
`HLEN`/`ZCARD` are 1.6–2.2 ms and `LLEN` 1.55 ms because they collect every
reference; 4.40 makes them O(1). List random access repeats the pattern:
`LINDEX`/`LPOS` on 200k cost **1.26–1.47 vs 0.07–0.14 ms** because both flatten
every element reference; 4.41 adds in-place chunk seeking. Streams repeat the write pattern:
`XADD` at 5k is **1.36 vs 0.08–0.10 ms** because it clones the whole stream and
folds the payload twice; 4.42 makes the append in place. Geospatial search
repeats the materialization: a 1 km `GEOSEARCH` over 50k is **4.34 vs 0.16–0.17
ms** and `GEOPOS` 100 is **24.9 vs 0.47–0.61 ms**, because every query scans all
members; 4.43 adds neighbor-cell search and direct lookups. Multi-member lookups
are the largest ratio yet: `ZMSCORE` with 100 members is **273 ms versus 0.19–0.63**
and `SMISMEMBER` 100 is 11.8 versus 0.21–0.39, because each requested member
rescans the materialized collection; 4.44 probes directly. Expiry is the last
systemic stall: the first `MGET`/`KEYS`/`RANDOMKEY`/`DBSIZE` after 30k keys
expire costs **36–46 vs 0.6–1.0 ms** because each calls `reap(now, usize::MAX)`,
while a lazy `GET` is at parity; 4.45 bounds the command-path expiry. `SORT ...
BY nosort LIMIT 0 10` is the other side of materialization: **7.96 vs 0.12–0.16
ms** on 100k because the whole list is copied before the slice, while the sorted
rows are competitive; 4.46 slices the source directly. Large listings repeat
the pattern at 100k: `HGETALL` is **132.3 vs 25.7–37.8 ms**, `HKEYS`/`HVALS`
98–102 vs 15.8–19.7, `KEYS` 91 vs 21–43 and `RANDOMKEY` 0.52 vs 0.21–0.29
because each sorts a fresh snapshot; 4.29 and 4.31 are extended to cover them.
The set sibling is the counter-example: `SMEMBERS` at 100k costs **84.8 vs
88.7/86.4 ms** (lead) and is at parity at 10k because the 1.5 MB reply hides the
collect+sort, so 4.29 stays scoped to hashes. One boundary is not latency at all:
Rill rejects any request above **1,024 total arguments** while Redis/Valkey
execute `MGET` 10,000 and `MSET` 600 pairs; 4.55 derives the cap from the frame
budget (section 3.47).
HyperLogLog is the remaining CPU gap: 100k pipelined adds cost **1.70 vs 0.39–0.41 s**
and 10k repeated counts **2.66 vs 0.033–0.036 s** because every command
decodes/encodes 16 KB of registers and nothing is cached; 4.47 keeps them in
place and caches the count. Bitmaps also hide a scan: a not-found `BITPOS 0` on
16 MB is **201.6 vs 5.1 ms** because `find_bit` visits every byte and bit; 4.37
is extended with a `u64` bit scan. Streams have one more clone on a control path:
`XGROUP CREATE` on a 5k stream is **8.26 vs 0.08–0.09 ms** (88–99×) because it
deep-clones every entry; 4.48 inserts the group in place. Sorted-set pops are the
largest ratio in the record: `ZPOPMIN`/`ZPOPMAX`/`ZMPOP` at 100k are **146–160 vs
0.16–0.42 ms** (373–974×) because every pop clones, sorts and replaces the whole
set; 4.49 pops from the ordered index. Lex ranges and count-only queries repeat
the sort/scan: a 10-element `ZRANGEBYLEX ... LIMIT 10` at 100k is **56.0–69.8 vs
0.18–0.74 ms** (163–371×) and a narrow `ZLEXCOUNT` is **15.0 vs 0.21–0.22 ms**
because `select_zset` sorts the whole set and the counts scan every member; 4.50
seeks the ordered index and subtracts ranks. Monitors share the pub/sub queue,
so a
2,000-command burst delivers **1,024 of 2,000 lines** per monitor (all samples)
while Redis/Valkey deliver all; 4.39 is extended to monitors. Under
continuous eviction, the 2026-11-12 cohort measures an **85.8% P16
SET loss (p99 64.5 vs 8.0 ms)** that is policy-independent and absent in the C
servers, admitting 4.26 (pooled eviction). Startup replay is **20–45× slower to
ready** (0.258 s at 50k and 1.10–1.24 s at 200k keys versus 0.011–0.061 s for the
C servers), admitting 4.27 (single-pass replay). AOF HMAC costs **−10.2% P1 /
−23.6% P16** on append and makes a 50k replay **0.21 → 0.75 s**; the append cost
is accepted as the security price and verification folds into 4.27.
`appendfsync always` stays source-only after two failed local cohorts (the
host's fsync cost is not stable enough to resolve it), and a Python connect-storm
attempt was client-bound and inconclusive. A limited TTL-heavy diagnostic
(section 3.7) finds no consistent TTL-vs-plain penalty, keeping 4.9 closed, and a
262k-key growth attempt was host-dominated and inconclusive, leaving 4.18
deferred. A
64-client/P64–P256 scaling check (section 2.25) leads Redis/Valkey by 29–122% in
every deep row, with only P1 SET near parity. Section
2.24 is a one-page open-gap
ledger that maps every remaining item to its evidence status, candidate and
admission experiment. Section 4.28 fixes the call sites, invariants and
priority for implementing the admitted candidates.

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
  offline builds. The authorized set is rustls, Tokio, Mio and `core_affinity`
  plus their required transitive crates (section 5); Mio was extended by PR #139
  through review, and each later crate took the same reviewed path, not an
  incremental "already vendored" shortcut.
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

### 1.1 Candidate admission: bound the mechanism before building it

Every rejected candidate in this document failed the same way in hindsight: the
mechanism was plausible, it was built, and only then did the measurement show the
gain was never available. The io_uring send attempt (section 4.14) is the clearest
case: it removed one `write` per request, passed the whole local suite, and still
measured 5–14% slower with byte-identical server CPU.

Two numbers decide, before any code is written:

1. **The mechanism's upper bound.** The most it can possibly gain if everything
   else stayed free. For a syscall reduction that is
   `syscalls_removed × syscall_cost / per_request_cost`; with ~0.3–0.5 µs per
   entry+exit and ~8.7 µs per request at the four-CPU gate, removing *both* socket
   syscalls is a 7–11% ceiling, and batching removes part of one. Serialising work
   onto another core is bounded by the share that can move; reusing a buffer is
   bounded by the copy.
2. **The row's measured noise.** The paired interval and the per-repetition spread
   on the row the change is meant to win (section 2.17 and the five-round cohorts:
   ±1.5–3% on `ping`/P1).

Admission rule: **build only when the upper bound exceeds the noise, with margin.**
If the bound is inside the noise band, the mechanism cannot reliably pass even if
it works perfectly, and the correct action is to record the bound and stop. Two
corollaries, both learned the hard way:

- **An ambiguous probe is a no.** The io_uring probe measured +45% in one round and
  +1% in the next on a noisy host. "Inconclusive" is not permission to invest; it
  is evidence that the effect is smaller than the measurement can resolve.
- **Do not build a mechanism whose cost lands on the same path it optimises.** The
  ring saved a syscall and added a payload copy plus a completion round trip on
  the same latency-critical pass. A candidate must reduce total work per request,
  not move it into an equally hot step.

Each candidate record must state the bound, the noise it is compared against, and
the kill criterion before implementation. The experiments that already failed —
poll consolidation, extra reactors, activity-clock coalescing, direct receive
buffers, the read/write split and io_uring batching — stay rejected; they cannot be
revived by re-describing them as new knobs.

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
productive reactor also took the then-current 8 µs blocking readiness wait, so the extra threads
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

**Placement is never disjoint on this runner.** Inspecting the manifests of every
recent cohort — #212 (pass), #217 (pass), #218 (fail), #220 (fail) and #223
(fail) — all report `client_cpus == server_cpus == allowed_cpus == [0, 1, 2, 3]`.
The runner exposes exactly four logical CPUs, so the workflow's "disjoint client
CPUs when available" clause cannot be satisfied on this runner type. Pass and
fail therefore cannot be attributed to placement, and the current one-reactor
default still shares the four CPUs with two generator threads. A true disjoint
control needs a different runner type (a CI/policy decision), not a configuration
change; until then the P1 ping row remains residual host/scheduler noise.

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

**Scope correction (current default).** Those cohorts ran the four-reactor
default of that era. After 4.12 merged, the default plain-TCP path is one
owner-local reactor, so one thread writes every sharded counter and the padded
layout has no cross-reactor writer to separate; the mechanism can only apply to
`--socket-workers` and opt-in multi-owner modes. The 128-byte layout stays (it is
already merged and costs 8 KiB), but it is not evidence for the current default's
P1 margins and must not be cited as one (section 4.10).

### 2.11 Manual cohort: throughput position and scheduling churn

A separate manual benchmark run (41477114469, tested source `9692e07`, 180
validated records, three repetitions, AMD EPYC 9V45, Redis 8.10.2 and Valkey
9.1.2 with the pinned generator) measured the same five commands at P1/P16 and
one-core budgets. It is **not** the protected gate; its generated report is now
on main (`63e1f20`, PR #168), while the deterministic tables below come from
the run artifact retained at
`/root/perf-evidence/manual-41477114469/report.md`.

| Workload | Rillcache | Redis | Valkey | Rill lead vs Redis/Valkey |
| --- | ---: | ---: | ---: | --- |
| P1 ping cmd/s | 139,341 | 111,925 | 112,235 | +24.5% / +24.1% |
| P1 set cmd/s | 117,182 | 110,191 | 106,825 | +6.3% / +9.7% |
| P1 get cmd/s | 119,947 | 110,609 | 108,514 | +8.4% / +10.5% |
| P16 ping cmd/s | 2,177,772 | 1,451,854 | 1,384,859 | +50.0% / +57.3% |
| P16 set cmd/s | 1,526,766 | 1,158,289 | 1,030,536 | +31.8% / +48.2% |

Rillcache also had the lower P1 ping batch p99 (68.9 µs versus 93.1 and 90.2).
On this cohort it does not lose a measured throughput row to Redis or Valkey.
The remaining gap is efficiency and scheduling, and it is concentrated exactly
where the multi-reactor design runs connection-local commands:

| Metric, P1 ping | Rillcache | Redis | Valkey |
| --- | ---: | ---: | ---: |
| CPU (% of one logical CPU) | 119.8 | 100.5 | 99.4 |
| CPU µs/command | 8.60 | 9.00 | 8.80 |
| Voluntary context switches/command | 0.0859 | 0.00006 | 0.00017 |
| Involuntary context switches/command | 0.0143 | 0.00003 | 0.00006 |

P1 data rows are near one CPU and about 0.0028 voluntary switches/command, so
the churn is specific to the connection-local PING path, not the owner-local
data path. P16 ping also uses 121.9% CPU because it can execute on several
reactors. The mechanism is the owner-plus-worker reactor model: a P1 ping can
involve an accept/wakeup and the then-current 8 µs readiness absorption per reactor, while
Redis and Valkey keep one thread and do not pay cross-thread wakeups. The extra
run-able thread demand is what becomes a loss when the benchmark generator
shares the same CPUs (section 2.9).

**Verdict:** do not add reactors to chase P1 ping. The measured next target is
wakeup/spin cost, not parser or storage work: the server already wins rate and
tail, but uses 20% more CPU and ~1,400x the voluntary switches on the hot
connection-local row. Candidate 4.11 specifies the diagnostic and its rejection
condition.

### 2.12 Cross-candidate synthesis: P1 ping is not a reactor-count problem

Three independent owners tested reactor counts around the then-default four
reactors. The implementations are separate; only their published four-CPU gate
evidence is compared here. Absolute rates come from different hosts and must not
be compared across rows.

| Default at four CPUs | Head | P1 ping Rillcache | Valkey | Delta | Other losses | Dragonfly |
| --- | --- | ---: | ---: | ---: | --- | --- |
| one reactor, co-located | PR #167 `d291cad` | 107,707.0 | 108,572.5 | −0.80% | P1 GET −0.11% | PASS 10/10 |
| two reactors | PR #172 `c7904c6` | 150,489.2 | 152,652.7 | −1.42% | none | PASS 10/10 |
| four reactors (historical default) | PRs #164/#169/#171/#173 cohorts | 107,787.9–117,430.7 | 112,658.8–115,545.1 | −4.32% to +2.92% | none | PASS 10/10* |
| eight reactors | PR #164 `e790299` | 85,675.7 | 110,402.4 | −22.4% | none; Dragonfly also failed P1 ping only | FAIL 9/10 |

\*The four-reactor Dragonfly comparison passed 10/10 on every cohort; the table's
Dragonfly column is about the candidate's own head.

The two-reactor head produced by far the highest absolute P1 rate (150k) and
still lost the strict Valkey median by 1.42%, with a better p99 (0.071 versus
0.095 ms). The one-reactor head cut scheduler churn but lost ping and GET
narrowly. Eight reactors is the only case with a mechanistic, large loss:
eight server reactors plus two generator threads exceed four CPUs badly. Four
reactors sits in a ±4% band and has passed both gates repeatedly.

**Verdict:** stop tuning the reactor count alone to clear the Valkey P1 ping
median. One, two, four and eight reactors have all failed that row on at least
one head; the sign is not monotonic in reactor count, and the two-reactor result
shows the row can stay negative even when absolute rate and p99 are much better.
The four-reactor default carried the most passing evidence until 4.12 merged, but
a count-only change is not a reliable fix. The merged default is now one
owner-local reactor plus the bounded readiness poll (sections 2.13 and 4.12);
another count-only change remains an unmeasured knob turn.

### 2.13 Theoretical PING ceiling, reactor batching and the bounded readiness poll

Section 2.12 shows that reactor count alone does not clear the Valkey P1 median;
this section explains why the count is still part of the mechanism, and why the
bounded poll is the part that was missing. Pipeline 1 has exactly one request in
flight per connection, so throughput is set by Little's law:
`rate = clients / round-trip latency`. With the official gate's
eight clients and two generator threads, the generator itself serializes each
thread over four connections. The floor is therefore the generator's own
synchronous `write`+`read` cycle plus the server's share of the round trip, not
the number of server CPUs. Adding server reactors cannot raise that floor; it can
only add wakeups and steal runnable time from the two generator threads.

Two mechanisms follow from the measurements.

**Batching per readiness wake.** A reactor that owns more connections can serve
more of the generator's ready requests per `epoll_wait` return. With `R = 4` on
the four-CPU gate each reactor owns two of the eight connections, so almost every
wake serves one request; with `R = 1` it owns all eight and can serve one request
per generator thread (two) before sleeping again. The gate's own data rows are
the cleanest within-run evidence: data connections always migrate to the single
owner reactor, so they are a one-reactor measurement taken beside the split PING
row. On the candidate head that lost the gate, the single-owner rows reached
155,311–159,816 req/s P1 while the two-reactor PING row reached 150,489 req/s,
even though PING does strictly less work. A single reactor is not a compromise
for P1 PING; it is the faster topology.

**Removing the sleep/wake pair.** Even with perfect batching, a reactor that has
answered every queued client and parks in `epoll_wait` pays a wakeup for the next
arrival. The request is usually already in flight within a few microseconds, so
the reactor can stay runnable for a bounded window and observe it directly. The
implementation polls with a zero timeout for at most 20 µs after a productive
pass, only while no client is queued and no accept is pending; the window is
renewed only by serviced work. The cost is bounded: an idle server spins for at
most one window after its last command and then parks for the 10 ms stop tick,
and a pipeline-16 pass never reaches the branch because its runnable queue is
never empty. Under sustained pipeline-1 load the reactor deliberately holds one
CPU busy; that is the latency/CPU trade this project chooses for P1.

Batching alone is not sufficient: section 2.12 records that the co-located
one-reactor head from PR #167 lost the same Valkey median by 0.80% while cutting
scheduler churn. That fits the decomposition above. One reactor removes the split-set
wakeups but still parks in `epoll_wait`, so it continues to pay one blocking
arrival per command; the bounded poll is the remaining mechanism, and the local
screen shows it removing most of the switches that batching left behind
(0.057–0.071 to 0.025–0.035 per command).

The same arithmetic explains the one-CPU gate, where the server is pinned to one
CPU and the generator runs on the others. One reactor is both the batching
optimum and the CPU floor: the measured P1 ping cost is about 8.5 CPU µs per
command against 8.8 for the C servers, so Rillcache is not losing on command
instructions. The remaining one-CPU bound is the kernel readiness/receive/send
round trip plus the necessary generator handoff. Batching those syscalls across
sockets would need `recvmmsg`/`sendmmsg`, `io_uring` or another FFI-level
mechanism outside the current safe-Rust/Mio boundary.

Evidence for the two mechanisms:

| Cohort | Topology | P1 ping req/s | P16 ping req/s | P1 voluntary switches/command |
| --- | --- | ---: | ---: | ---: |
| four-CPU gate, source `c7904c6` (run 41544908472) | 2 reactors, blocking wait | 150,489 (p99 0.071 ms) | 1,815,984 | not collected |
| four-CPU gate, same run, single-owner data rows | 1 reactor | 155,311–159,816 | 1,551,099–1,836,765 | not collected |
| local screen, frozen binaries | 2 reactors, blocking wait | 16,299–25,728 | 324,277–368,059 | 0.154–0.174 |
| local screen, frozen binaries | 1 reactor, blocking wait | 20,224–25,728 | — | 0.057–0.071 |
| local screen, frozen binaries | 1 reactor, 20 µs poll | 22,173–28,092 | 433,679–500,122 | 0.025–0.035 |

The middle row separates the two mechanisms: batching alone already removes most
of the per-command wakeups, and the bounded poll removes most of the rest. A
second screen of the final default binary reproduced the P1 direction
(22,193–28,447 vs 17,659–26,710 for the two-reactor default) and left P16 mixed
(326,270–427,505 vs 329,313–341,565), so no P16 gain is claimed: the point of the
one-reactor default is that it does not collapse the pipelined row.

The local host is a heavily loaded six-logical-CPU KVM, so its absolute rates are
diagnostic only; the direction and the switch counters are consistent across
paired repetitions, and the official run is the authoritative rate evidence. The
local screen used the pinned official `valkey-benchmark` with the gate's own
arguments (eight clients, two threads, 1024 random keys, 64-byte values, seed 42)
and the server and generator both pinned to CPUs `0-3`, with 6 s measured
samples (the protected gate uses 10 s).

The switch figures above are host-local and do not transfer to gate rates. The
hosted profile (section 2.23) measures 0.0002 (ping) and 0.00013 (GET) context
switches per reply for Rillcache against Valkey's 0.0019 and 0.0008, so the
bounded poll's runnable window does remove the sleep/wake pair at gate rates. The
local absolute counts stay diagnostic.

The final merged head (`c4d4698`, run 41552051338) passed both required four-CPU
comparisons:

| Gate | P1 ping Rillcache | Competitor | Delta | P1 p99 R/C |
| --- | ---: | ---: | ---: | ---: |
| Valkey 9.1.2 | 116,791.9 | 113,242.0 | +3.13% | 0.095/0.135 |
| Dragonfly 2.0.2 | 111,899.4 | 94,574.5 | +18.32% | 0.111/0.103 |

Both passed 10/10, the original gate passed, and the merged default is now one
owner-local reactor with the 20 µs bounded poll. This is the accepted exact-head
evidence for 4.12; it does not isolate the poll from host noise, but it validates
the direction the mechanism predicted and replaces the earlier two-reactor screen
that failed the same row.

Candidate 4.12 therefore implements one owner-local reactor by default plus the
bounded readiness poll. `--socket-workers 2..=16` remains the opt-in path for
PING-heavy fan-out, where a hot single reactor is the limit rather than the goal.

### 2.14 Clean-runner upstream cohort on the merged default

A separate manual upstream study (run 41563160515, source `c8b760a`) measured the
merged one-reactor + bounded-poll default against Redis 8.10.2 and Valkey 9.1.2
on one clean runner: 180/180 valid samples, six rotated repetitions, 16 clients,
disjoint server/client CPU affinity, default server mode (one owner-local
reactor), and the pinned Redis 8.10.2 `redis-benchmark` generator. It is
**advisory**, not the protected gate (which uses eight clients and the official
Valkey generator), but it is the first clean-host cohort for the merged
architecture.

| Workload | P | Rillcache | Redis | Valkey | Lead vs Redis/Valkey | Rill CPU % | Redis/Valkey CPU % |
| --- | ---: | ---: | ---: | ---: | --- | ---: | ---: |
| ping | 1 | 117,349 | 110,193 | 109,569 | +6.49% / +7.10% | 95.1 | 82.0 / 81.6 |
| set | 1 | 116,167 | 111,857 | 111,878 | +3.85% / +3.83% | 94.9 | 87.4 / 89.6 |
| get-hit | 1 | 116,167 | 111,008 | 109,890 | +4.65% / +5.71% | 94.7 | 85.9 / 85.6 |
| get-miss | 1 | 117,050 | 110,681 | 109,290 | +5.75% / +7.10% | 95.1 | 84.2 / 83.8 |
| incr | 1 | 116,845 | 111,857 | 110,011 | +4.46% / +6.21% | 94.9 | 86.1 / 85.4 |
| ping | 16 | 1,570,681 | 1,244,813 | 1,063,830 | +26.18% / +47.64% | 88.4 | 90.6 / 88.9 |
| set | 16 | 1,438,857 | 821,918 | 686,499 | +75.06% / +109.59% | 95.4 | 87.9 / 94.6 |
| get-hit | 16 | 1,470,588 | 958,466 | 843,884 | +53.43% / +74.26% | 97.7 | 86.2 / 95.9 |
| get-miss | 16 | 1,522,843 | 1,102,941 | 961,538 | +38.07% / +58.38% | 90.7 | 85.7 / 84.9 |
| incr | 16 | 1,188,123 | 933,128 | 853,487 | +27.33% / +39.21% | 96.2 | 87.6 / 95.9 |

The merged default wins every measured row. The remaining measured gaps are CPU
and run-to-run variance at pipeline 1: P1 ping uses 95.1% of one CPU versus about
82% for the C servers, and its coefficient of variation is 1.89% versus 0.16%
and 0.37%. That is the deliberate cost of the 20 µs bounded poll (section 2.13):
the reactor stays runnable to remove a sleep/wake pair. At P16 the CPU use is
comparable (88–98% versus 86–96%) and the rate lead is much larger. RSS is lower
than both references (about 5.4–6.2 MiB versus 6.6–7.3 MiB).

CPU percentage is not directly comparable when rates differ. Dividing the
reported one-CPU percentage by the measured rate gives approximately 8.1 µs per
command for Rillcache (0.951 / 117,349) versus 7.4 for Redis (0.820 / 110,193)
and 7.4 for Valkey (0.816 / 109,569) on the first clean cohort: about 9% more
per command, not a 13-point deficit. Most of the higher total CPU is the 6.5%
higher rate.

A second clean cohort (`d5da908`, run 41633321852) repeated the direction at a
faster host scale: P1 ping 171,233 vs 164,429 and 164,474, CPU 94.7% vs 86.4%
and 86.2%, p99 0.091 vs 0.087/0.087. That is about 5.5 vs 5.25 µs per command
(~5% more), and P1 SET is essentially equal (5.52 vs 5.54/5.52 µs). The P1 ping
per-command gap is therefore small and ping-specific on those two hosts (~5% and
~9%).

It is not stable in sign. The per-row #212 artifacts report process CPU seconds
and measured commands over the same window, which gives 6.97 µs/command for P1
ping and 6.06 for SET versus 7.27 and 7.60 for Valkey — Rillcache **cheaper** per
command on that host. The difference is host- and accounting-dependent, so the
honest conclusion is a small cost on some hosts and a small advantage on others,
not a stable deficit. Do not build a candidate on it.

**Verdict:** the merged architecture is confirmed as the right throughput
direction on a clean host, including against Redis, which the protected gate does
not compare. Do not trade the P1 win back for CPU without a measured alternative.
The P1 total-CPU gap is mostly the higher rate plus a small host-dependent
per-command difference whose sign flips across cohorts; the variance cost is the
bounded poll. Candidate 4.13 is closed after the miss-based, fixed-window and
hybrid variants were rejected (section 4.13). A count-only reactor change is not
a substitute. Redis remains outside the required four-CPU comparison, so this
advisory cohort does not turn into a gate claim.

### 2.15 Merged-default tail behavior

The merged one-reactor + bounded-poll default wins throughput consistently, but
its P1 ping p99 is not consistently better than the C servers. The strict
throughput gate does not fail on p99; AC5 still expects P1 batch p99 to be no more
than 5% above the pre-change head or lower than Valkey's. The observed rows:

| Cohort | Clients | P1 ping p99 ms Rillcache | Redis | Valkey | Dragonfly | Throughput vs Valkey |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| #172 final gate (run 41552051338) | 8 | 0.095 | — | 0.135 | 0.103 | +3.13% |
| #174 docs gate (run 41560156879) | 8 | 0.103 | — | 0.151 | 0.103 | +11.27% |
| #175 docs gate (run 41564296997) | 8 | 0.119 | — | 0.087 | 0.055 | +21.77% |
| #176 docs gate (run 41566914311) | 8 | 0.071 | — | 0.087 | 0.071 | +0.44% |
| #179 docs gate (run 41582234348) | 8 | 0.119 | — | 0.087 | 0.063 | +23.54% |
| Five-server daily (run 43022929390) | 8 | 0.103 | 0.135 | 0.151 | 0.103 | +13.17% |
| Five-server daily, manual (run 43164876654) | 8 | 0.111 | 0.079 | 0.087 | 0.063 | +29.49% |
| Clean-runner upstream (run 41563160515) | 16 | 0.119 | 0.111 | 0.111 | — | +7.10% |
| Clean-runner upstream, repeat (run 41633321852) | 16 | 0.091 | 0.087 | 0.087 | — | +4.10% |

The p99 direction flips with the cohort while the throughput win does not. On
the fastest #175 and #179 hosts Rillcache had 21.8–23.5% more throughput and a
37% worse p99 than Valkey; on the #172/#174 hosts it had both higher throughput
and lower p99, and #176 had a lower p99 with a near-tie rate. Both clean-runner
16-client cohorts show a small p99 disadvantage (0.119 vs 0.111 and 0.091 vs
0.087), so the effect is more consistent in that harness than in the 8-client
gate, though still small.

**Verdict:** do not claim a tail win for the merged default. The bounded poll
keeps the reactor runnable to remove a wakeup, which can help typical latency
while occasionally delaying a request behind the current pass; the effect is
host-sensitive. AC5 remains an explicit design criterion even though the
protected gate is throughput-only. A future poll/wakeup candidate must record
per-request or batch p99 and be rejected if it regresses beyond AC5, even when
throughput wins. The current default stays because its throughput and CPU
evidence is strong and no measured alternative preserves both rate and tail.

### 2.16 Local syscall count of the merged bounded poll

A local tracepoint diagnostic on the merged default (`d5da908`) counted the
server's syscalls while eight clients ran P1 ping. The host is the shared
six-logical-CPU VM at about 1.2k ops/s, not a gate host; the server was pinned to
CPU 0, clients to CPUs 1–2, and `perf stat` sampled 6.088 s while the full 20k
client run took 16.51 s. Normalizing by rate × window gives about 7,374 commands
inside the perf window:

| Tracepoint | Count | Per command (approx) |
| --- | ---: | ---: |
| `syscalls:sys_enter_epoll_wait` | 57,090 | 7.74 |
| `syscalls:sys_enter_recvfrom` | 13,607 | 1.85 |
| `syscalls:sys_enter_sendto` | 7,622 | 1.03 |
| `sched:sched_switch` | 1,821 | 0.25 |

The 20 µs bounded poll is a zero-timeout `epoll_wait` spin, so it accounts for
most of the ~8 epoll waits per command on this slow host. `recvfrom` is about one
receive plus the edge-triggered drain probe; `sendto` is about one reply write.
The local window also spends the full 20 µs per command because the next request
is far away. **A hosted gate-rate profile measures 0.20–0.36 `epoll_wait` per
reply, not ~8 (section 2.23);** the local count is a slow-host mechanism
diagnostic, not a gate prediction. It explains the CPU cost and why the hybrid
timed wait (section 4.13) reduced epoll calls but regressed p99: it traded the
spin for sleep/wake latency.

Compared with the pre-readiness diagnostic (section 2.4, ~6.2 receive-class
syscalls per command), the readiness reactor cut receive-side work to ~1.85 at
the slow local rate; the bounded poll spends those savings on epoll waits *on
that host*. At gate rates the hosted profile (section 2.23) shows 0.20–0.36
`epoll_wait` per reply and fewer context switches than Valkey, so the trade is
not an epoll excess there; the remaining excess is the drain probe. Redis and
Valkey use level-triggered single loops that amortize one wait over all ready
clients; Rill cannot copy that without raw `epoll` FFI or a different selector
policy, and a sleeping wait was measured worse on the tail.

**Verdict:** record the local syscall/tail trade as the mechanism behind the P1
CPU policy, but use the hosted profile (2.23) for gate-rate syscall counts. Do not
add another poll-window variant; the remaining safe options are outside the Mio
boundary. The local counts are diagnostic only and must not be substituted for
gate evidence.

### 2.17 Five-round cohorts: the paired interval exposes near-parity

The first five-round Valkey cohort (PR #189 docs-only head `927ccef`, base
`e74bac3`, run 41936225853, 100 samples) passed the strict median rule 10/10, but
the new paired interval shows where the cohort cannot resolve direction:

| Row | Rillcache median | Valkey median | Delta | Wins | Paired ratio [95% CI] |
| --- | ---: | ---: | ---: | ---: | --- |
| ping P1 | 117,002.6 | 114,048.4 | +2.59% | 4/5 | 1.0254 [0.9703–1.0661] |
| ping P16 | 1,606,942.4 | 986,772.8 | +62.8% | 5/5 | 1.5788 [1.5288–1.6465] |
| set P1 | 116,352.2 | 108,591.4 | +7.1% | 5/5 | 1.0747 [1.0453–1.1229] |

The P1 ping interval contains 1.0, so the cohort cannot resolve that row's
direction even though the whole-cohort median is higher; every other row has a
5/5 sweep and an interval above 1.0. Dragonfly passed 10/10 with P1 ping
115,257.5 vs 94,669.6 (+21.8%). This is evidence quality, not a threshold change:
the row still passes by the unchanged strict median rule, and the interval is
never used to pass or fail it.

A later docs-only cohort (head `90c1768`, run 41940385826) shows the opposite
side of the same host sensitivity: Valkey passed 10/10 with P1 ping 165,006.2 vs
139,149.2 (+18.6%, 5/5, ratio 1.1876 [1.0206–1.2964]), while Dragonfly failed
9/10 **only** on P1 ping (165,175.0 vs 167,637.5, −1.47%; p99 0.119/0.063). The
same runtime passed one opponent's P1 interval with a wide margin and lost the
other opponent's strict row on a fast host. That is the gate's per-opponent rule
working as intended, not a runtime regression.

A third cohort is the clearest variance example. PR #216 (head `ddcc10c`, a
docs-only change whose runtime is byte-identical to the merged `d357e2a`) failed
the Valkey P1 ping row: 138,803.5 vs 139,745.7 req/s (−0.67%), 3/5 wins, median
ratio 1.0030 [0.9371–1.0278]. Its five paired ratios were 0.986, 0.937, 1.028,
1.015 and 1.003; Rillcache's run spread was 7.4% (133.7k–143.6k) against
Valkey's 4.3% (136.8k–142.7k), while the same head passed ping P16 5/5 by
+49.7%. One low run (0.937) decided the strict median on a row whose interval
contains 1.0. The same code passed this row in #212 and #215. That is the
documented measurement limitation, not a runtime regression: do not re-push the
head to fish for a pass and do not weaken the rule. The row remains the gate's
open measurement risk (sections 2.9 and 2.15).

Two hours after that failure, the same runtime (docs-only head `bd4f25d`, base
`8b38d0b`) passed the same row at 181,489 vs 157,268 req/s (+15.4%, 5/5, ratio
1.1301 [1.1219–1.2376], run 43128571502). A later docs-only head (`506325d`,
same base, run 43134573112) failed it again at 138,976 vs 140,875 req/s
(−1.35%, 2/5, ratio 0.9819 [0.9388–1.0209]), while its P16 ping passed 5/5 at
+48.9%. All three heads carry the identical `7b365dc` runtime:

| Cohort | Docs head | P1 ping R/V req/s | Delta | Wins | Paired ratio | P16 ping |
| --- | --- | ---: | ---: | ---: | --- | ---: |
| #216 | `ddcc10c` | 138,803.5 / 139,745.7 | −0.67% | 3/5 | 1.0030 [0.9371–1.0278] | 5/5, +49.7% |
| #217 | `bd4f25d` | 181,489 / 157,268 | +15.4% | 5/5 | 1.1301 [1.1219–1.2376] | 5/5, +84.5% |
| #218 | `506325d` | 138,976.1 / 140,875.2 | −1.35% | 2/5 | 0.9819 [0.9388–1.0209] | 5/5, +48.9% |

The identical-code spread is −1.35% to +15.4%: both losses are within 1.4% and
the win is resolved with an interval above 1.0. Neither a loss nor a win proves a
code change on a single cohort; the strict median of this one shared-runner row
moves by roughly 16 points while the binary is unchanged. The row is the gate's
measurement risk, not a stable deficit, and the clean-host direction remains
positive. Every inspected manifest behind these cohorts reports the same
`client_cpus == server_cpus == [0, 1, 2, 3]` (section 2.9), so placement is not a
variable the gate can control on this runner.

The Dragonfly comparison shows the same shape. The #220 docs head (base
`cd8fa1b`, runtime identical) lost the Dragonfly P1 ping row by **0.12%**
(159,195.8 vs 159,392.2 req/s, p99 0.143/0.071) while passing every other row,
including Dragonfly P16 at +7.1%. A 0.12% strict-median loss on unchanged code is
the same measurement limitation against the second opponent, not a runtime
direction.

**Verdict:** quote the paired interval for the P1 row, not a point estimate. A
strict pass is not proof of a resolved 2.6% win, and a Dragonfly P1 loss on a
fast host does not by itself justify a runtime change. The two cohorts reinforce
the variance analysis in sections 2.9 and 2.15 and the decision to keep the
merged one-reactor default.

### 2.18 Fresh merged-head gate: 10/10 against both references

PR #212 (head `ce122e0`, merged as `7b365dc`) is the newest runtime cohort and the
first in this record produced by the parallel per-workload gate (#198): each
workload row runs in its own partial job on its own allocated four-CPU runner of
the same machine type, with both servers alternating inside that job. Rows are
therefore same-runner comparisons **within** a row, not one host for the whole
table; absolute rates across rows are not comparable.

| Workload | P | Rillcache median [min–max] req/s | Valkey median [min–max] req/s | Delta | p99 ms R/V | Dragonfly median req/s | Delta vs D | p99 ms R/D |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| ping | 1 | 142,812.70 [128,505.00–143,893.80] | 135,949.20 [128,835.60–141,391.09] | +5.05% | 0.071/0.095 | 130,215.30 | +9.62% | 0.079/0.079 |
| ping | 16 | 2,025,368.00 [1,995,001.62–2,044,620.75] | 1,383,992.00 [1,346,217.62–1,403,976.00] | +46.34% | 0.071/0.143 | 1,733,123.25 | +16.86% | 0.071/0.103 |
| set | 1 | 164,083.80 [158,745.80–166,991.09] | 130,861.00 [124,261.60–139,564.09] | +25.39% | 0.143/0.095 | 105,315.80 | +55.80% | 0.151/0.151 |
| set | 16 | 1,925,561.62 [1,895,292.75–2,000,008.00] | 1,041,502.38 [1,021,580.81–1,095,356.75] | +84.88% | 0.095/0.167 | 1,042,347.19 | +84.73% | 0.111/0.199 |
| get | 1 | 141,411.00 [137,212.20–142,645.41] | 128,805.00 [127,942.30–135,559.00] | +9.79% | 0.079/0.103 | 88,634.00 | +59.55% | 0.087/0.159 |
| get | 16 | 1,870,084.75 [1,803,880.00–1,889,492.75] | 961,918.38 [941,606.38–1,003,537.62] | +94.41% | 0.111/0.183 | 878,260.81 | +112.93% | 0.111/0.215 |
| set-nx-existing | 1 | 141,208.09 [133,202.70–142,038.91] | 127,233.70 [116,202.80–133,385.30] | +10.98% | 0.095/0.103 | 102,644.00 | +37.57% | 0.079/0.143 |
| set-nx-existing | 16 | 1,795,270.38 [1,742,236.75–1,832,148.75] | 961,220.81 [849,102.38–973,848.00] | +86.77% | 0.183/0.191 | 671,604.81 | +167.27% | 0.127/0.287 |
| set-xx-existing | 1 | 172,335.91 [169,777.50–176,829.50] | 130,284.60 [126,198.50–133,836.50] | +32.28% | 0.135/0.087 | 85,696.60 | +101.10% | 0.103/0.191 |
| set-xx-existing | 16 | 1,923,064.00 [1,856,536.00–2,016,664.00] | 1,000,467.19 [952,624.00–1,003,871.19] | +92.22% | 0.095/0.167 | 760,415.06 | +152.90% | 0.119/0.271 |

Both comparisons passed 10/10; the original 20-case gate also passed. The P1
ping margin is +5.05% against Valkey and +9.62% against Dragonfly, but the paired
ratio is still 1.0458 with a 95% interval of [0.9089–1.1085], so the interval does
not by itself resolve the row (section 2.17). P1 p99 is mixed rather than a win:
lower than Valkey for ping, get and nx, higher for set and xx; every P16 p99 is
lower. That is consistent with section 2.15: throughput and tail
are separate acceptance criteria, and this cohort does not establish a tail win
on the two P1 write rows.

The newest cohort on the same runtime is #217 (merge candidate `ce3ca13`, base
`8b38d0b`): PASS 10/10 against both references. P1 ping was 181,489 vs 157,268
req/s (+15.4%, 5/5, paired ratio 1.1301 [1.1219–1.2376] — the first cohort in
this record whose interval excludes 1.0), P1 get +4.7% (4/5, ratio 1.0692
[0.9781–1.0965]), set +7.2%, NX +8.3% and XX +29.6%; P16 ping +84.5% and P16 GET
+93.8%. p99 was 0.079/0.087 (ping), 0.087/0.103 (get), 0.111/0.151 (set) and
0.127/0.087 (XX); Dragonfly also passed 10/10 with P1 ping 117,847 vs 94,227.
This is the current acceptance evidence. The same runtime **lost** the P1 ping
median in #216 (section 2.17), so the pair is the clearest demonstration that the
row's outcome depends on the runner even when the code is identical: the
direction is not in doubt, the single-runner median is.

### 2.19 Five-server daily cohort on the same head

The advisory daily job (run 43022929390, source `7b365dc`, 250/250 validated
samples, five rotated repetitions, one fresh owned server at a time, four logical
server CPUs, official generator, 8 clients, 1024 keys, 64-byte values) measured
Rillcache, Redis 8.10.2, Valkey 9.1.2, Dragonfly 2.0.2 and Garnet 2.2.5 on the
same runner. Rillcache is the fastest server in all ten rows:

| Workload | P | Rillcache | Redis | Valkey | vs Redis | vs Valkey | p99 R/Rd/V ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | 1 | 114,400.90 | 113,261.20 | 101,086.00 | +1.01% | +13.17% | 0.103/0.135/0.151 |
| ping | 16 | 1,604,750.38 | 1,042,006.38 | 902,923.19 | +54.01% | +77.73% | 0.103/0.207/0.231 |
| set | 1 | 114,061.50 | 98,105.90 | 99,529.20 | +16.26% | +14.60% | 0.111/0.159/0.159 |
| set | 16 | 1,341,894.38 | 599,307.19 | 530,230.38 | +123.91% | +153.08% | 0.167/0.303/0.335 |
| get | 1 | 113,383.00 | 107,183.60 | 107,508.20 | +5.78% | +5.46% | 0.103/0.151/0.143 |
| get | 16 | 1,505,019.25 | 751,060.81 | 620,833.62 | +100.39% | +142.42% | 0.127/0.263/0.287 |
| set-nx-existing | 1 | 114,795.70 | 104,334.90 | 101,069.40 | +10.03% | +13.58% | 0.111/0.151/0.159 |
| set-nx-existing | 16 | 1,424,475.25 | 629,339.19 | 590,238.38 | +126.34% | +141.34% | 0.215/0.295/0.311 |
| set-xx-existing | 1 | 113,670.40 | 104,674.80 | 94,295.00 | +8.59% | +20.55% | 0.111/0.151/0.167 |
| set-xx-existing | 16 | 1,285,550.38 | 572,459.19 | 513,339.19 | +124.57% | +150.43% | 0.183/0.319/0.351 |

The advisory rule (a lead is clear only above `max(5%, twice either sample CV)`)
leaves four P1 rows unresolved even though they are numerically ahead: ping P1
and get P1 against Redis, and set P1 and get P1 against Valkey. Every P16 row and
the P1 SET/NX/XX rows are outside the band. P99 is lower than both references in
all ten rows, and RSS is lower (5.2–6.0 MiB versus 6.8–7.3 MiB). The CPU-seconds
column in the artifact is not a per-command measure: every server reports about
10 s in a 10 s window because that window includes generator orchestration, so
the clean-runner estimates in section 2.14 remain the only per-command CPU
evidence.

**Second cohort on the same runtime (manual dispatch).** A second five-server run
on merged `bcd4768` (runtime `7b365dc`, same code) completed 250/250 samples on a
faster runner (run 43164876654). Rillcache again led all ten rows, but both the
margin and the tail direction differ sharply:

| Workload | P | Rillcache | Redis | Valkey | vs Redis | vs Valkey | p99 R/Rd/V ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ping | 1 | 177,056.00 | 144,519.20 | 136,729.70 | +22.52% | +29.49% | 0.111/0.079/0.087 |
| ping | 16 | 2,500,598.50 | 1,621,520.00 | 1,480,833.62 | +54.22% | +68.86% | 0.087/0.111/0.127 |
| set | 1 | 169,595.70 | 134,291.91 | 130,702.70 | +26.29% | +29.76% | 0.127/0.087/0.087 |
| set | 16 | 1,881,206.38 | 1,078,412.75 | 948,028.81 | +74.44% | +98.43% | 0.095/0.175/0.191 |
| get | 1 | 160,354.00 | 131,724.41 | 131,327.41 | +21.73% | +22.10% | 0.143/0.087/0.095 |
| get | 16 | 2,144,225.50 | 1,197,080.00 | 1,121,528.00 | +79.12% | +91.20% | 0.231/0.151/0.159 |
| set-nx-existing | 1 | 168,511.20 | 130,726.20 | 130,270.50 | +28.90% | +29.36% | 0.119/0.087/0.095 |
| set-nx-existing | 16 | 2,111,110.50 | 1,133,843.25 | 1,110,832.00 | +86.19% | +90.05% | 0.127/0.159/0.159 |
| set-xx-existing | 1 | 171,948.30 | 129,706.40 | 135,451.00 | +32.57% | +26.94% | 0.127/0.087/0.087 |
| set-xx-existing | 16 | 1,895,451.25 | 1,054,452.75 | 947,304.00 | +79.75% | +100.10% | 0.095/0.167/0.183 |

The P1 throughput lead is now +21.7% to +32.6% against Redis and +22.1% to
+29.5% against Valkey, outside the advisory band in every row, while P1 p99 is
**worse** than both references in all five (0.111–0.143 vs 0.079–0.095 ms). At
P16 the throughput lead is 54–100%; p99 is better for ping/set/NX/XX but worse for
GET (0.231 vs 0.151/0.159). RSS is lower (5.2–6.0 MiB vs 6.7–7.2 MiB). The
CPU-seconds column again includes generator orchestration and is not a
per-command measure.

This is a second host for identical code, and it resolves two points: the 1.0%
P1 ping margin of the first cohort was a host artifact, not the true direction;
and the P1 tail is host-dependent rather than a fixed win or loss. Throughput and
tail can move in opposite directions on the same binary, which is exactly why AC5
and the server-side attribution test (4.19) exist.

**What this means.** Across the two four-CPU cohorts on identical code,
throughput leads every row, but the size of the P1 margin (1.0–22.5% vs Redis)
and the direction of the P1 tail (lower in one cohort, higher in the other) both
depend on the host. There is no stable throughput loss in these cohorts; the
residual is host-dependent latency and variance plus the per-command CPU sign
flip from section 2.14. The next section states where a real mechanism
difference remains.

### 2.20 Where a mechanism difference still exists

Thirty-nine observations; item 11 is now closed by PR #258 (`d334cde`), and the
rest are still true even though every row now leads:

1. **Syscall shape.** At hosted gate rates (section 2.23) Rillcache does 1.336
   (`recvfrom`) + 1.000 (`sendto`) + 0.200 (`epoll_wait`) syscalls per reply for
   ping and 1.710 + 1.000 + 0.362 for GET, against 1.000 + 1.000 + 0.264 and
   1.000 + 1.000 + 0.237 for Valkey. Total syscalls per reply are therefore +12%
   (ping) and +37% (GET); the excess is the edge-triggered drain probe, not the
   bounded poll, whose epoll count is comparable. Context switches are **lower**
   (0.0002 vs 0.0019 ping; 0.00013 vs 0.0008 GET). The local 7.74-epoll figure
   from section 2.16 was a slow-host artifact. This is not a first-order loss and
   the probe's removal is already rejected (section 2.6), so do not reopen it.
2. **Large-value reply copies.** Rillcache encodes a bulk value into the
   client's output `Vec` and writes that `Vec`, so a large GET copies the value
   once into output and again into the kernel. Redis 8.10.2 and Valkey 9.1.2
   both keep a refcounted reference to the stored object instead of copying when
   the value is at least 16 KiB in single-threaded mode (Redis
   [`COPY_AVOID_MIN_STRING_SIZE`](https://github.com/redis/redis/blob/8.10.2/src/server.h#L202)
   and `isCopyAvoidPreferred`
   [`networking.c#L1296`](https://github.com/redis/redis/blob/8.10.2/src/networking.c#L1296);
   Valkey `min-string-size-avoid-copy-reply` default 16384
   [`config.c#L3387`](https://github.com/valkey-io/valkey/blob/9.1.2/src/config.c#L3387)
   and `isCopyAvoidPreferred`
   [`networking.c#L287`](https://github.com/valkey-io/valkey/blob/9.1.2/src/networking.c#L287)).
   This is one reply-side place where their source does something structurally
   cheaper that ours does not, and it is invisible to the 64-byte gate (section
   2.21); the sorted-set index (item 8) is the other.
3. **Large-value parse/store copies.** Both C servers also allocate one
   `robj`/sds copy of an incoming bulk argument and store that object; our path
   copies the frame slice into a new key/value pair. That is the same one-copy
   shape, so there is no parse-side gap to copy; only the reply-side reference
   avoidance is missing.
4. **Command lookup.** Redis 8.10.2 caches the last command and reuses it when
   only the name matches (`isCommandReusable`,
   [`server.c#L110`](https://github.com/redis/redis/blob/8.10.2/src/server.c#L110),
   used at [`server.c#L4507`](https://github.com/redis/redis/blob/8.10.2/src/server.c#L4507));
   our PING/GET/SET/ECHO path compares raw bytes instead. For an 8.7 µs P1
   command the difference is a few tens of nanoseconds, below the gate's noise,
   so this is an avoid rather than a candidate (section 4.17).
5. **Hash-table growth.** Redis 8.10.2 and Valkey 9.1.2 both moved the keyspace
   to `kvstore` and both rehash incrementally, with a 1 ms cron budget
   (`INCREMENTAL_REHASHING_THRESHOLD_US` 1000,
   [`redis server.c#L1352`](https://github.com/redis/redis/blob/8.10.2/src/server.c#L1352);
   [`valkey server.c#L1325`](https://github.com/valkey-io/valkey/blob/9.1.2/src/server.c#L1325)).
   Our one-stop `HashMap` growth is invisible at the gate's 1024 keys; it is a
   large-keyspace concern, not a gate-row concern (section 4.18).
6. **Extreme-tail latency on mutating P1 commands.** On the #212 host, SET/XX p99
   is consistently ~50% above Valkey's in every paired round while p50/p95 are
   lower (section 2.22); the daily host shows the opposite. The mechanism is not
   isolated between server work, the bounded poll's parking, and generator
   contention on a shared CPU budget, so the next evidence is a server-side
   histogram (candidate 4.19), not a runtime guess.
7. **Watch-free mutation bookkeeping.** Redis 8.10.2 and Valkey 9.1.2 return from
   `touchWatchedKey` before any per-key work when no client is watching
   ([`redis multi.c#L389`](https://github.com/redis/redis/blob/8.10.2/src/multi.c#L389),
   [`valkey multi.c#L461`](https://github.com/valkey-io/valkey/blob/9.1.2/src/multi.c#L461));
   our `Data::touch` advances a sequence and updates or inserts a
   `versions: HashMap<Vec<u8>, u64>` entry on **every** mutation, allocating the
   key on first write even when nothing watches (candidate 4.20). The throughput
   bound is one hash lookup plus an occasional allocation, below gate noise; it
   is a candidate only if 4.19 attributes part of the mutation tail to it.
8. **Sorted-set ordering.** Our large sorted set is `HashMap<Vec<u8>, f64>`, so
   every `ZRANGE` collects all members and fully sorts them before slicing, and
   `ZSCORE` linearly scans the collection. Redis 8.10.2 and Valkey 9.1.2 keep a
   dict from member to skiplist node plus a skiplist (listpack below
   `zset-max-listpack-entries`), giving O(1) `ZSCORE` and O(log n + k) rank
   ranges ([`redis t_zset.c#L1539`](https://github.com/redis/redis/blob/8.10.2/src/t_zset.c#L1539),
   [`#L3413`](https://github.com/redis/redis/blob/8.10.2/src/t_zset.c#L3413);
   [`valkey t_zset.c#L1420`](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_zset.c#L1420),
   [`#L3099`](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_zset.c#L3099)).
   The 2026-11-12 cohort measures a 10k `ZRANGE 0 9` at **528 req/s versus
   10,311/10,771** and a full range at 340 versus 1,581/1,036, with the
   128-member control at parity (section 3.2, candidate 4.30).
9. **Cursor iteration.** Our `SCAN`/`HSCAN`/`SSCAN`/`ZSCAN` rebuild a sorted
   snapshot (and deep-copy collection elements) on every call, so per-call cost
   is O(keyspace) instead of O(`COUNT`). Redis and Valkey use a reverse-binary
   dict cursor with a bounded visited-bucket loop and return listpack collections
   in one call ([`redis dict.c#L1529`](https://github.com/redis/redis/blob/8.10.2/src/dict.c#L1529),
   [`valkey dict.c#L1177`](https://github.com/valkey-io/valkey/blob/9.1.2/src/dict.c#L1177)).
   The 2026-11-12 cohort measures a 100k-key `SCAN COUNT 10` at **42.6 ms p50
   versus 0.256/0.164** (167–260×) and a full traversal at 4.81 s versus
   0.239/0.167 s (section 3.16, candidate 4.31).
10. **Collection element storage and deferred free.** Lists allocate one
    `Vec<u8>` per element and `UNLINK`/`FLUSH* ASYNC` run the synchronous drop,
    so a 200k-element 8-byte list costs 6.2–7.3 ms to unlink and 143 B/element
    RSS, versus 0.13–0.32 ms and 7.5 B/element for the C servers' packed
    quicklist/listpack plus bio lazy free (section 3.17, candidates 4.32/4.33).
11. **In-place string mutation (closed).** `APPEND` and `SETRANGE` rebuilt and
    re-copied the whole value (twice), costing **7.2–7.35 ms** on a 32 MB string
    versus 0.13–0.18 ms for the C servers; PR #258 (`d334cde`) mutates in place
    and the post-fix cohort measures **0.13–0.19 ms** (section 3.18).
12. **In-place collection mutation.** Every write rebuilds the whole collection:
    a single `HSET` into a 10k hash, `ZADD` into a 10k zset, `SADD` into a 10k
    set and `RPUSH`/`LPOP` on a 200k list cost **1.2–18.3 ms** versus 0.07–0.35 ms
    for the C servers' in-place updates, with 10-element controls at the command
    floor (section 3.19, candidate 4.35).
13. **Random-member commands.** `SRANDMEMBER`, `SPOP`, `HRANDFIELD` and
    `ZRANDMEMBER` materialize the collection per call, so at 100k members they
    cost **11–34 ms** versus 0.1–0.35 ms for the C servers' dict random
    (35–298×), with the 10k numbers 1–2.6 ms and the 10-element controls at the
    floor (section 3.20, candidate 4.36).
14. **Bitmap mutation and popcount.** `SETBIT` rebuilds the value (two full
    copies) and `BITCOUNT` runs a byte-at-a-time popcount, so on a 16 MB bitmap
    they cost **5.3 ms** and **23.0 ms** versus 0.31–0.35 and 3.4–4.6 ms, while
    `GETBIT`, `STRLEN` and the 64 B controls are at the floor; the same two-copy
    write makes a one-byte `BITFIELD SET` **4.39–4.68 ms** versus 0.087–0.12,
    while read-only `BITFIELD` is already copy-free (sections 3.21 and 3.40,
    candidate 4.37 extended).
15. **Sorted-set rank, score-range and point reads.** At 100k members `ZRANK`,
    `ZREVRANK`, `ZRANGEBYSCORE LIMIT` and `ZRANGE` cost **57–72 ms** versus
    0.07–0.17 ms for the C servers' skiplist (378–927×), and `ZSCORE` costs
    3.3 ms versus 0.07–0.10 ms because it linearly scans the collection; at 10k
    the same commands are 2.4–3.1 ms (section 3.22, candidates 4.30/4.38).
16. **Pub/sub burst delivery and channel scale.** A 2,000-message pipelined burst
    to an active subscriber delivers **1,024 and silently removes the
    subscription** (the C servers deliver 2,000 and keep it), and a `PUBLISH` to
    one of 10,000 subscribed channels is 3–6× slower because the publisher scans
    every channel (section 3.23, candidate 4.39).
17. **Cardinality and membership.** At 100k members `SISMEMBER`/`SCARD` cost
    **8.0–8.7 ms** versus 0.15–0.22 ms (36–55×) because they clone and sort the
    whole set; `HLEN`/`ZCARD` cost 1.6–2.2 ms and `LLEN` 1.55 ms because they
    collect every reference, while `HEXISTS`/`STRLEN` are direct and fast
    (section 3.24, candidate 4.40).
18. **List random access.** `LINDEX` (0/middle/last) and a hit-first `LPOS` on a
    200k list cost **1.26–1.47 ms** versus 0.07–0.14 ms (9–22×) because both
    flatten every element reference; a missing `LPOS` leads because the C
    servers walk every node, and the 10-element controls are at the floor
    (section 3.25, candidate 4.41).
19. **Stream append.** Every `XADD` clones the whole `Stream` and folds its
    payload twice, so at 5,000 entries it costs **1.36 ms** versus
    0.08–0.10 ms (13–18×) and grows with stream length, while `XLEN`/`XRANGE`/
    `XREVRANGE` and the 10-entry control are at the floor (section 3.26,
    candidate 4.42).
20. **Geospatial search and point lookups.** `GEOSEARCH` scans every member, so
    a 1 km search over 50k members costs **4.34 ms** versus 0.16–0.17 ms
    (25–28×) and a 2×2 km box 4.49 versus 0.19–0.21; `GEOPOS` is 2.0 ms for one
    member and 24.9 ms for 100 (41–53×), and `GEODIST` ~19×, while a search that
    returns all 50k is reply-bound at parity (section 3.27, candidate 4.43).
21. **Multi-member point lookups.** `SMISMEMBER` clones and sorts the set and then
    scans it per member (**7.9–11.8 ms** versus 0.08–0.39), and `ZMSCORE` scans
    all members once per requested member (**8.0/33.4/273 ms** for 1/10/100
    members versus 0.10–0.63), while `HMGET` and the 10-member controls are at
    the floor (section 3.28, candidate 4.44).
22. **Mass expiry on command paths.** `MGET`/`KEYS`/`RANDOMKEY`/`DBSIZE` call
    `reap(now, usize::MAX)`, so the first command after 30k keys expire stalls
    **36–46 ms** versus 0.6–1.0 ms (37–76×) while a per-key lazy `GET` is at
    parity; the purge costs ~1.2–1.5 µs per expired key (section 3.29, candidate
    4.45).
23. **SORT `BY nosort`.** A 100k `SORT ... BY nosort LIMIT 0 10` costs
    **7.96 ms** versus 0.12–0.16 ms (50–66×) because the whole source is copied
    before the slice, while sorted/ALPHA/weighted SORT rows are 0.9–1.5× and the
    100-element controls are at the floor (section 3.30, candidate 4.46).
24. **Large-hash listing and keyspace enumeration.** At 100k, `HGETALL` is
    **132 ms** versus 25.7–37.8, `HKEYS`/`HVALS` 98–102 versus 15.8–19.7,
    `KEYS key:*` 91 versus 21–43, and `RANDOMKEY` 0.52 versus 0.21–0.29, because
    each listing sorts a fresh snapshot; `HMGET` and the 10-member controls are at
    the floor (section 3.31, extending 4.29 and 4.31).
25. **HyperLogLog add/count.** 100k pipelined `PFADD` cost **1.70 s (16.96
    µs/add)** versus 4.1/3.9 µs and 10k repeated `PFCOUNT` **2.66 s (265 µs/call)**
    versus 3.3–3.6 µs, because every command decodes/encodes the 16 KB register
    set and there is no cached cardinality; single-call p50 and the 10-element
    controls are at the floor (section 3.32, candidate 4.47).
26. **BITPOS on a large bitmap.** A not-found `BITPOS 0` on 16 MB costs **201.6
    ms** versus 5.1 ms (39–44×) because it scans byte by byte and bit by bit,
    while a first-bit hit is at the floor; `BITCOUNT` is the previously admitted
    4.37 row (section 3.33, extending 4.37).
27. **Stream group creation.** `XGROUP CREATE` deep-clones the whole stream, so
    on a 5k-entry stream it costs **8.26 ms** versus 0.08–0.09 ms (88–99×) and
    scales with length (~165 ms at 100k by extrapolation); the other XGROUP
    subcommands already mutate in place and are at parity (section 3.34,
    candidate 4.48).
28. **Sorted-set pops.** Every `ZPOPMIN`/`ZPOPMAX`/`ZMPOP` clones and sorts the
    whole set before removing one member: at 100k they cost **146–160 ms** versus
    0.16–0.42 ms (373–974×), the largest measured ratio, while the 100-member
    control is at the floor (section 3.35, candidate 4.49).
29. **MONITOR feed under bursts.** The monitor queue is the pub/sub 1,024 queue
    with a 64-line tick drain, so a 2,000-command burst delivers exactly **1,024
    lines** to every monitor in every sample while Redis/Valkey deliver 2,000;
    publisher fan-out is at parity or cheaper when it does not overflow (section
    3.37, extending 4.39).
30. **Lexicographic ranges and count-only queries.** A 10-element
    `ZRANGEBYLEX`/`ZREVRANGEBYLEX` with `LIMIT 10` on a 100k equal-score zset
    costs **56.0–69.8 ms versus 0.18–0.74** because `select_zset` sorts the whole
    set before filtering, and `ZLEXCOUNT`/`ZCOUNT` scan every member
    (**1.25–15.0 ms versus 0.13–0.39**, including an empty `ZCOUNT`); section
    3.38, candidate 4.50 on 4.30/4.38's ordered index.
31. **Stream consumer-group reads.** `XREADGROUP` history/new, `XPENDING`,
    `XCLAIM` and `XAUTOCLAIM` on a 10k-entry stream with a 10k-entry PEL stay
    within **1.3–2.1×** of Redis/Valkey (largest forms: new-read 1000 8.99 vs
    5.19/6.43 ms, autoclaim 1000 9.84 vs 4.67/7.50), while 100-entry history,
    `XPENDING` and `XCLAIM` are at parity; per-entry `StreamEntry`/`Pending` clones
    and the consumer-name copy are the residual, below the candidate threshold
    (section 3.39, measured null).
32. **BITFIELD writes on large values.** A one-byte `BITFIELD SET`/`INCRBY` on a
    16 MB value costs **4.39–4.68 ms versus 0.087–0.12** (37–52×) while
    `BITFIELD_RO`/all-`GET` reads are at the floor, because the write path clones
    the value and `put` copies it again; section 3.40, extending 4.37.
33. **Sorted-set algebra and cardinality.** At 100k members, `ZINTERCARD` costs
    **220.6 ms versus 40.4/17.1** (5.5–12.9×) and `ZINTERSTORE`/`ZDIFFSTORE`
    306.6/331.3 versus 102–111/61–62 (2.8–5.4×), because every source is copied
    into an owned map and the result is re-sorted; `ZUNIONSTORE` is 1.3–1.7×
    behind and set-store variants already lead (section 3.41, candidate 4.51).
34. **INFO keyspace over a large keyspace.** With no TTLs, `INFO keyspace` costs
    **2.79 ms at 100k keys and 10.0 ms at 500k** versus 0.13–0.30 ms for the C
    servers (16–40×), and plain `INFO` pays the same walk, while `INFO stats`,
    `DBSIZE` and `PING` stay at the floor because the keyspace section counts
    every entry instead of reading maintained sizes (section 3.42, candidate
    4.52).
35. **Two-key moves.** `SMOVE` on a 100k set costs **25.5 ms versus 0.40/0.28**
    (64–91×) and `LMOVE`/`RPOPLPUSH` on a 200k list **39.2/41.2 versus
    0.08–0.34** (79–489×), while a same-key rotate still costs 20.5 ms, because
    both keys are cloned and re-encoded for a one-element move; 10-element
    controls are at the floor (section 3.43, candidate 4.53).
36. **Cardinality `LIMIT`.** `SINTERCARD`/`ZINTERCARD` with `LIMIT 10` on
    100k-member containers still cost **50.7/267.2 ms versus 0.20–0.25** because
    the limit is applied after the full intersection (205–1,377×), while
    `LIMIT 0` costs the same as no limit and 10-member controls are at the floor;
    section 3.44, extending 4.51.
37. **List element edits.** `LINSERT` at the head of a 200k list costs **19.1 ms
    versus 0.09/0.24** (79–212×), `LSET` at index 100k 16.5 versus 0.14/0.28
    (59–118×), while the scan-bound middle insert and tail removal are 4.9–6.0×
    and 2.9–3.0×; every form flattens the whole list first; 10-element controls
    are at the floor (section 3.45, candidate 4.54).
38. **Set membership reply.** `SMEMBERS` on sets of 1k/10k/100k 8-byte members
    is at parity or **leads** (100k 84.75 ms versus 88.71/86.37; 10k 9.95 versus
    9.33/9.58), despite the same collect+sort+encode shape as the hash listings,
    because the 1.5 MB reply dominates; this bounds 4.29 and keeps it scoped to
    hashes (section 3.46, measured lead/null).
39. **Multi-key argument cap.** Rill rejects any request above **1,024 total
    arguments** (`MGET` 1,023 keys accepted, 1,024 rejected) while Redis/Valkey
    execute `MGET` 10,000, `MSET` 600 pairs and `DEL` 2,000; at 1,000 keys the
    latency is at parity or leads (`MGET` 1.74 vs 2.23/2.34 ms), so the boundary,
    not the per-command cost, is the gap (section 3.47, candidate 4.55).

### 2.21 Large-value screen: first attempt failed, retry measured

Mechanism under test: Rillcache copies a bulk value into the client output `Vec`,
while both pins keep a reference to the stored object for values of at least
16 KiB (section 2.20). The prediction was a large-GET deficit that grows with
value size and is absent at 64 bytes.

**First attempt (2026-11-11, failed).** A local paired screen with a release
build of `7b365dc` and the pinned Redis/Valkey archives was unusable: load
average was 10–19 on the shared six-CPU k3s node and the 64-byte control measured
5,414 ops/s for Rillcache and 3,437 for Redis, a ~20–33x distortion; the run was
stopped and no large-value row existed. Those files remain as failed evidence.

**Retry (2026-11-12, measured).** The same host at load ~2.7 produced a usable
cohort: server pinned to CPU 5, client to CPUs 0–3, eight clients, pipeline 1,
three Latin-rotated rounds, 64 B/1024 keys, 64 KiB/128 keys and 1 MiB/32 keys.
The 64-byte control is 39.6k/33.9k/32.2k req/s (Rill/Redis/Valkey), about 3.5x
below the gate but far from the failed 20–33x distortion, so the paired ratios
are usable as a shared-host diagnostic:

| Value | Workload | Rillcache | Redis | Valkey | Rill vs Redis/Valkey | p99 R/Rd/V ms |
| ---: | --- | ---: | ---: | ---: | ---: | --- |
| 64 B | SET | 39,633 | 33,951 | 32,151 | +16.7% / +23.3% | 1.46/0.80/0.84 |
| 64 B | GET | 40,728 | 27,437 | 32,196 | +48.4% / +26.5% | 1.39/1.22/0.91 |
| 64 KiB | SET | 14,125 | 11,537 | 12,670 | +22.4% / +11.5% | 1.82/2.58/2.25 |
| 64 KiB | GET | 10,734 | 10,970 | 14,998 | −2.2% / **−28.4%** | 3.93/1.61/1.21 |
| 1 MiB | SET | 1,337 | 1,861 | 1,959 | **−28.2% / −31.8%** | 9.79/7.37/7.82 |
| 1 MiB | GET | 776 | 1,340 | 1,409 | **−42.1% / −44.9%** | 24.93/9.87/10.91 |

The gap crosses over between 64 B and 64 KiB and grows with value size: at
1 MiB, GET is 42–45% slower with a 2.3–2.5x worse p99, and SET is 28–32% slower.
One Rillcache 1 MiB GET round stalled at 135 req/s (host), but the other two were
776 and 822 against 1,340–1,410 for the C servers, so the direction is
consistent. Absolute rates are shared-host diagnostics, not a gate result.

**What remains to be separated.** The GET loss matches the reply copy exactly
(candidate 4.15). The SET loss is the same direction but cannot be the reply
copy (a SET reply is small); the likely cause is the input buffer growing in
16 KiB reads, while Redis/Valkey reserve capacity from the declared bulk length
(`big_arg`/`sdsMakeRoomFor`, section 3.3). Candidate 4.25 covers that input
reservation. The previous copy-bandwidth bound (100–200 µs per 1 MiB at
5–10 GB/s, zero at 64 B) remains the mechanism estimate for 4.15 and is now
supported by a measured slope rather than arithmetic alone.

### 2.22 Per-request P1 latency and process CPU from the retained gate samples

The gate artifacts retain more than the summary table: each `sample.json` carries
per-request `avg/p50/p95/p99/max` latency, process CPU seconds and RSS for the
same measured window. Extracting the #212 partials (five paired rounds per row)
gives a fuller latency picture than the p99-only table in section 2.18. Values are
medians over the five runs; P1 only, because at P16 the reported latency is
pipeline-batch latency.

| Workload | Server | req/s | p50 ms | p95 ms | p99 ms | max ms | CPU µs/cmd |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| ping | Rillcache | 142,813 | 0.039 | 0.055 | 0.071 | 2.471 | 6.97 |
| ping | Valkey | 135,949 | 0.055 | 0.079 | 0.095 | 3.087 | 7.27 |
| set | Rillcache | 164,084 | 0.039 | 0.063 | 0.143 | 6.183 | 6.06 |
| set | Valkey | 130,861 | 0.055 | 0.079 | 0.095 | 3.463 | 7.60 |
| get | Rillcache | 141,411 | 0.047 | 0.063 | 0.079 | 3.607 | 7.04 |
| get | Valkey | 128,805 | 0.055 | 0.087 | 0.103 | 2.975 | 7.68 |
| set-nx-existing | Rillcache | 141,208 | 0.047 | 0.063 | 0.095 | 3.111 | 7.06 |
| set-nx-existing | Valkey | 127,234 | 0.055 | 0.087 | 0.103 | 3.303 | 7.82 |
| set-xx-existing | Rillcache | 172,336 | 0.039 | 0.055 | 0.135 | 3.295 | 5.75 |
| set-xx-existing | Valkey | 130,285 | 0.055 | 0.079 | 0.087 | 3.087 | 7.63 |

Three observations:

1. **Typical latency is better in every row.** Rillcache's p50 is 0.039–0.047 ms
   against 0.055 ms, and p95 is lower in all five commands. The client-observed
   distribution is not a tail-loss story at the median.
2. **The extreme tail on mutating P1 commands is worse on this host.** SET p99 is
   0.143 vs 0.095 ms and max 6.18 vs 3.46 ms; XX p99 is 0.135 vs 0.087 ms. This is
   consistent in **every** paired round (SET p99 0.143/0.095, 0.143/0.095,
   0.143/0.095, 0.151/0.087, 0.135/0.087; XX 0.135/0.095, 0.135/0.087,
   0.119/0.087, 0.127/0.095, 0.135/0.087), so it is not one outlier run. The
   non-mutating rows (ping, get, rejected NX) have a lower or equal p99 in the
   same samples. The correlation with mutation is exact on this host: only SET and
   XX write, and only they carry the extra p99/max population; ping, GET and
   rejected NX do not touch the store's mutation path.
3. **The direction is host-dependent.** On the first single-host daily cohort
   (section 2.19) the same SET row had rill p99 0.111 vs Valkey 0.159 and XX
   0.111 vs 0.151 — Rillcache better. The second daily cohort on identical code
   (`bcd4768`) reverses **all five** P1 rows: rill 0.111–0.143 vs Redis
   0.079–0.087 and Valkey 0.087–0.095, while throughput leads by 21.7–32.6%.
   The cohorts disagree because they run on different hosts and topologies,
   exactly the variance warning in sections 2.9 and 2.15. A client-observed tail
   cannot be attributed to the server without a server-side histogram.

The CPU column also reverses the earlier per-command conclusion (section 2.14):
using process CPU seconds divided by measured commands, Rillcache is *cheaper*
per command on this host (6.97 vs 7.27 µs ping, 6.06 vs 7.60 SET), whereas the
first clean cohort found it ~9% more expensive. Per-command CPU is within about
10% either way and its sign is host-dependent; it is not a stable deficit and
must not drive a candidate.

**Verdict:** the remaining latency risk is the extreme tail of P1 mutating
commands on some hosts, not the typical request. Isolating it needs a
server-side measurement, not another client percentile; that is candidate 4.19.
No tail win or loss is claimed beyond the specific cohorts, and AC5 remains the
acceptance rule.

### 2.23 Hosted gate-rate syscall and scheduler profile

Section 2.16's per-command syscall counts came from the shared six-CPU VM at
about 1.2k ops/s, where the bounded poll spins for a large share of each slow
command; it explicitly could not predict gate rates. A hosted instrumented
profile (run 43125144121, source `8b38d0b`, owned `perf` capture, eight clients,
two generator threads, 64-byte values, 1024 keys, P1, four shared logical CPUs)
now measures the same counters at 118–139k req/s. The values below are syscall
counts divided by reply syscalls (`sendto`/`write`), not throughput or wall
latency; the instrumented rates are quarantined and never used as a ranking.

| Workload | Server | Replies in 8 s | process CPU µs/reply | recv/read per reply | send/write per reply | epoll_wait per reply | context switches per reply |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| ping | Rillcache | 991,114 | 8.07 | 1.336 (`recvfrom`) | 1.000 (`sendto`) | 0.200 | 0.000226 |
| ping | Valkey | 1,038,700 | 7.67 | 1.000 (`read`) | 1.000 (`write`) | 0.264 | 0.001898 |
| get | Rillcache | 1,037,694 | 7.71 | 1.710 (`recvfrom`) | 1.000 (`sendto`) | 0.362 | 0.000133 |
| get | Valkey | 904,895 | 8.83 | 1.000 (`read`) | 1.000 (`write`) | 0.237 | 0.000800 |

The CPU column is process `task-clock` divided by replies (instrumented, not wall
latency); every rate in this capture is quarantined and cannot rank servers or
clear a gate. The syscall ratios are the counters' purpose and match the
per-operation method used in section 2.16.

Three corrections to the local picture follow:

1. **The bounded poll is not an epoll storm at gate rates.** It costs 0.200
   (ping) and 0.362 (GET) `epoll_wait` per reply, comparable to Valkey's 0.264
   and 0.237 — not the ~7.7/command seen on the slow local host (section 2.16).
   At gate rates the window usually returns on an event; the local number was the
   slow host spending the full window per command. The doc's earlier "several
   times the reference" framing is corrected here.
2. **The remaining syscall difference is the edge-triggered drain probe.** Rill
   does 1.336/1.710 receives per reply against 1.000, so total syscalls per reply
   are +12% (ping) and +37% (GET). That is the probe that PR #155 removed and
   that lost two Valkey P1 medians; this profile explains why it is load-bearing
   but not free. Do not reopen drain elision or poll tuning on this evidence.
3. **Scheduler churn is lower at gate rates, not higher.** Rill performs 0.0002
   (ping) and 0.00013 (GET) context switches per reply against 0.0019 and
   0.0008: the runnable window avoids the sleep/wake pair. The manual cohort's
   0.086/0.014 figures (section 2.11) were a slow-host, shared-CPU measurement.
   CPU per reply is 8.07 vs 7.67 µs (ping, +5%) and 7.71 vs 8.83 (GET, −13%),
   consistent with the host-dependent sign in section 2.14.

**Verdict:** at gate-like rates the syscall and scheduling dimensions are within
about +12% total syscalls (ping) and +37% (GET), with fewer scheduler switches and
comparable epoll waits; the difference is the drain probe, whose removal is
already rejected. This is not a first-order gate loss and it is not a reason to
change the runtime. The instrumented profile is mechanism evidence only: no
throughput, latency or production claim follows from it.

### 2.24 Open-gap ledger (state at this revision)

A map of what is still open, what was closed by bound, and which experiment would
admit a change. It repeats no new evidence; the linked sections are authoritative.

| Gap | Dimension | Evidence now | Candidate / decision | Admission evidence | Gate row |
| --- | --- | --- | --- | --- | --- |
| P1 ping strict median vs Valkey/Dragonfly | scheduling, measurement | identical-code −1.35% to +15.4%; +1.0% and +22.5% vs Redis across two daily cohorts; p99 flips with host | no runtime candidate; measurement/topology | disjoint CPUs need a different runner type; all recent cohorts share `[0,1,2,3]` | the row itself |
| P1 SET/XX far tail | latency | worse on the #212 host in every round, better on the first daily, worse on all five P1 rows in the second daily | 4.19 attribution first | paired client + server p99 histograms | none |
| Large bulk reply copy | replies | **measured**: 64 KiB GET −28.4% vs Valkey; 1 MiB GET −42% to −45%, p99 2.3–2.5× | 4.15, admitted | AC20 on the implemented change | none at 64 B |
| Large multi-bulk replies | replies | **measured**: `LRANGE` 10k leads (1,272 vs 913/726 req/s); `HGETALL` 10k is 43% behind Redis (313 vs 552) | 4.29, admitted | AC31 on the implemented change | none at 64 B |
| Sorted-set lookup and rank ranges | replies, storage | **measured**: 10k `ZRANGE 0 9` 528 vs 10,311/10,771 req/s (19.5–20.4×), full range 340 vs 1,581/1,036, `ZSCORE` 20–31% behind; 128-member control at parity | 4.30, admitted | AC32 on the implemented change | none at 64 B |
| Sorted-set rank, score and point reads | replies, storage | **measured**: at 100k, `ZRANK` 58.6–72.5 ms vs 0.07–0.16, `ZRANGEBYSCORE LIMIT 10` 62.4 vs 0.08–0.12, `ZRANGE 0 9` 57.4 vs 0.07–0.10 (378–927×), `ZSCORE` 3.27 vs 0.07–0.10; 10k 2.4–3.1 ms; controls at the floor | 4.30 (map/score/start-0) + 4.38 (rank-select), admitted | AC32/AC40 | none |
| Pub/sub bursts and channel scale | replies, latency, reliability | **measured**: 2,000-message burst delivers 1,024/2,000 and silently removes the subscription vs 2,000/2,000 and alive for both C servers; `PUBLISH` among 10,000 channels 0.38–0.89 ms vs 0.12–0.31; 1/1,000 channels at parity | 4.39, admitted (with 4.24) | AC41 | none |
| Cardinality and membership | latency | **measured**: 100k `SISMEMBER` 8.04/8.67 ms vs 0.17–0.22, `SCARD` 8.08 vs 0.15–0.20, `HLEN` 2.24 vs 0.15–0.19, `ZCARD` 1.57 vs 0.12–0.16, `LLEN` 200k 1.55 vs 0.14–0.20; `HEXISTS`/`STRLEN` and controls at the floor | 4.40, admitted | AC42 | none |
| List random access | latency | **measured**: 200k `LINDEX` 0/mid/last 1.29–1.47 ms vs 0.07–0.14, `LPOS` hit-first 1.26 vs 0.10–0.13 (9–22×); `LPOS` missing 1.81 vs 5.0–5.9 (Rill leads); controls at the floor | 4.41, admitted | AC43 | none |
| Stream append | latency, storage | **measured**: `XADD` at 5k 1.36 ms p50 (p99 11.6) vs 0.104/0.077, at 1k 0.263 vs 0.148/0.077; `XLEN`/`XRANGE`/`XREVRANGE` and 10-entry controls at the floor | 4.42, admitted | AC44 | none |
| Geospatial search and lookups | latency, storage | **measured**: 50k 1 km `GEOSEARCH` 4.34 ms vs 0.157/0.170, 2×2 km box 4.49 vs 0.211/0.186; `GEOPOS` 1/100 1.99/24.86 vs 0.14–0.19/0.47–0.61; `GEODIST` 3.12 vs 0.168; all-member search and controls at the floor | 4.43, admitted | AC45 | none |
| Multi-member point lookups | latency | **measured**: 100k `SMISMEMBER` 1/10/100 7.90/8.25/11.75 ms vs 0.07–0.39; `ZMSCORE` 8.02/33.37/273.2 vs 0.10–0.63; `HMGET` and 10-member controls at the floor | 4.44, admitted | AC46 | none |
| Mass expiry on command paths | latency | **measured**: after 30k keys expire, `MGET` 36.3 ms vs 0.69/0.98, `KEYS` 43.6 vs 0.63/0.80, `RANDOMKEY` 45.5 vs 0.65/0.60, `DBSIZE` 41.2 vs 0.58/0.83; lazy `GET` at parity | 4.45, admitted | AC47 | none |
| SORT `BY nosort` | latency | **measured**: 100k `SORT ... BY nosort LIMIT 0 10` 7.96 ms (p99 17.2) vs 0.12/0.16; sorted/ALPHA/DESC/BY-weight/GET rows 0.9–1.5×; 100-element controls at the floor | 4.46, admitted | AC48 | none |
| Large-hash listing | replies | **measured**: 100k `HGETALL` 132.3 ms vs 25.7/37.8; `HKEYS` 98.5 vs 16.5/15.8; `HVALS` 101.6 vs 19.7/17.9; 10k `HGETALL` 43% behind Redis; controls at the floor | 4.29, extended | AC31 on the implemented change | none |
| Keyspace enumeration | replies, latency | **measured**: 100k `KEYS key:*` 91.1 ms vs 42.8/21.1; `KEYS prefix` 32.7 vs 35.1/7.0; `RANDOMKEY` 0.52 vs 0.21/0.29 | 4.31, extended (index) | AC33 | none |
| MULTI/EXEC transactions | dispatch | **measured null**: 1,000-command transaction 3.02 ms vs 2.48–2.76; queue-only 1.37 vs 1.30–1.77; 1 KB transaction leads; 100-command within the host spread | keep; no candidate | n/a | none |
| MONITOR feed bursts | replies, latency, reliability | **measured**: 2,000-command burst delivers 1,024/2,000 lines per monitor (all six samples) vs 2,000/2,000; publisher fan-out at parity/cheaper while truncated | 4.39, extended | AC41 | none |
| HyperLogLog add/count | latency, CPU | **measured**: 100k pipelined `PFADD` 1.70 s (16.96 µs/add) vs 4.1/3.9 µs; 10k `PFCOUNT` 2.66 s (265 µs/call) vs 3.3–3.6 µs; single p50 and 10-element controls at the floor | 4.47, admitted | AC49 | none |
| BITPOS large bitmap | latency | **measured**: 16 MB `BITPOS 0` 201.6 ms vs 5.08/5.14 (39–44×); `BITPOS 0 0 -1` 209.4 vs 4.79/5.96; first-bit hit and 64 B control at the floor; `BITCOUNT` control 23.5 vs 4.0–4.8 | 4.37, extended | AC39 | none |
| Stream group creation | latency | **measured**: 5k `XGROUP CREATE` 8.26 ms vs 0.083/0.094 (88–99×); 10-entry control 0.338 vs 0.12/0.097; `CREATECONSUMER`/`SETID`/`DESTROY` at parity | 4.48, admitted | AC50 | none |
| Stream consumer-group reads | replies, latency | **measured null**: 10k stream/PEL, 20 repetitions; history 100 at parity but new 100 0.92 vs 0.57/0.69, new 1000 8.99 vs 5.19/6.43, `XPENDING` 1000 3.57 vs 2.93/3.72, `XAUTOCLAIM` 1000 9.84 vs 4.67/7.50, `XCLAIM` 100 at parity (1.3–2.1×, largest batches only) | no candidate; per-entry clone + consumer-name copy below threshold | re-measure after 4.42 or a multi-field/100k-PEL cohort | none |
| Sorted-set pops | latency | **measured**: 100k `ZPOPMIN` 156.7 ms vs 0.42/0.34, `ZPOPMAX` 159.9 vs 0.21/0.29, `ZPOPMIN 10` 146.6 vs 0.35/0.16, `ZMPOP` 155.8 vs 0.16/0.20 (373–974×); 100-member control at the floor | 4.49, admitted | AC51 | none |
| Lex ranges and count-only queries | replies, latency | **measured**: 100k `ZRANGEBYLEX LIMIT 10` 56.0–69.8 ms vs 0.18–0.74 (163–371×), `ZREVRANGEBYLEX` 60.1 vs 0.18–0.74, `ZLEXCOUNT` narrow 15.0 vs 0.21–0.22, `ZCOUNT` 1.25–1.75 vs 0.21–0.39 including an empty range, full 100k lex reply 144.8 vs 87.2–100.2; 10-member controls at the floor | 4.50, admitted (extends 4.30/4.38) | AC52 | none |
| Cursor iteration (SCAN/HSCAN/SSCAN/ZSCAN) | replies, storage, latency | **measured**: 100k-key `SCAN COUNT 10` 42.6 ms p50 vs 0.256/0.164 (167–260×); full traversal 4.81 s vs 0.239/0.167; 10k-field `HSCAN` 5.05 s vs 0.257/0.247 | 4.31, admitted | AC33 on the implemented change | none |
| List element packing and RSS | storage (memory), latency | **measured**: 200k 8-byte list 142–143 B/elem vs 7.5 (RSS); `DEL` 7.28 ms vs 0.166/0.130 | 4.32, admitted | AC34 on the implemented change | none |
| UNLINK and FLUSH* ASYNC | latency, storage | **measured**: `UNLINK` 200k list 6.21 ms vs 0.316/0.235; `FLUSHALL ASYNC` 20k keys 9.20 vs 0.886/0.568 and no faster than `SYNC` | 4.33, admitted | AC35 on the implemented change | none |
| In-place string mutation | latency, storage | **measured**: pre-fix `APPEND` 64 B to 32 MB 7.35 ms and `SETRANGE` 7.19; post-fix (#258) 0.131/0.185 vs 0.142/0.222 and 0.219/0.289; 64 B controls and `GETRANGE` at the floor | 4.34, implemented | AC36 met; retained as regression | none |
| In-place collection writes | latency, storage | **measured**: `HSET` 10k hash 6.45 ms vs 0.111/0.093; `ZADD` 4.21 vs 0.223/0.084; `SADD` 1.23 vs 0.083/0.351; `RPUSH`/`LPOP` 200k list 18.3/17.8 vs 0.07–0.34; at 100k `HSET`/`HDEL` 92.7–99.4 vs 0.08–0.32, `ZADD`/`ZREM` 61.4–63.9 vs 0.10–0.23, `SADD`/`SREM` 10.7–11.2 vs 0.08–0.60 (18–1160×); 10-element controls at the floor | 4.35, admitted | AC37 on the implemented change | none |
| Random-member commands | latency, storage | **measured**: at 100k, `SRANDMEMBER` 11.54 ms vs 0.199/0.326, `HRANDFIELD` 33.66 vs 0.343/0.113, `ZRANDMEMBER` 21.32 vs 0.250/0.164, `SPOP` 11.11 vs 0.304/0.126; 10-element controls at the floor | 4.36, admitted | AC38 on the implemented change | none |
| Bitmap mutation and popcount | latency, storage | **measured**: 16 MB `SETBIT` 5.32 ms vs 0.349/0.312; `BITCOUNT` 23.01 vs 3.37/4.56; 1 MB 0.33/1.85 vs 0.09–0.31; `GETBIT`/`STRLEN`/64 B controls at the floor; 16 MB `BITFIELD` write 4.39–4.68 vs 0.087–0.12 (37–52×) while `BITFIELD_RO` reads are at the floor | 4.37 (extended to `BITFIELD` writes) | AC39 on the implemented change | none |
| Set algebra (SINTER/SUNION/SDIFF/SINTERCARD) | replies, storage | **measured**: 100k non-LIMIT SINTERCARD 53.3 vs 40.1/30.0 ms and the store variants lead; `SINTERCARD ... LIMIT 10` is 50.7 vs 0.198/0.247 (205–256×) because the limit is applied after the full intersection | keep the non-LIMIT forms; extend 4.51 to the set path and early exit | AC53 for `LIMIT` | none |
| Zset algebra and cardinality | replies, latency | **measured**: 100k `ZINTERCARD` 220.6 ms vs 40.4/17.1 (5.5–12.9×), `ZINTERSTORE` 306.6 vs 110.5/61.6, `ZDIFFSTORE` 331.3 vs 102.0/61.4, `ZUNIONSTORE` 477.6 vs 366.6/280.4; store variants copy every source and re-sort the result; 10-member controls at parity | 4.51, admitted | AC53 | none |
| INFO keyspace accounting | replies, latency | **measured**: no-TTL keyspace; `INFO keyspace` 2.79 ms at 100k and 10.0 ms at 500k vs 0.13–0.30, plain `INFO` the same, while `INFO stats`/`DBSIZE`/`PING` stay at the floor because the section walks every entry | 4.52, admitted | AC54 | none |
| Two-key moves (SMOVE/LMOVE/RPOPLPUSH) | latency, storage | **measured**: `SMOVE` 100k 25.5 ms vs 0.40/0.28 (64–91×); `LMOVE`/`RPOPLPUSH` 200k 39.2/41.2 vs 0.08–0.34 (79–489×); same-key rotate 20.5 vs 0.26/0.21; 10-element controls at the floor | 4.53, admitted (extends 4.35/4.41) | AC55 | none |
| Cardinality `LIMIT` | replies, latency | **measured**: 100k `SINTERCARD ... LIMIT 10` 50.7 ms vs 0.198/0.247 (205–256×), `ZINTERCARD ... LIMIT 10` 267.2 vs 0.217/0.194 (1,231–1,377×); `LIMIT 0` equals no-limit cost; 10-member controls at the floor | 4.51, extended (set path + early exit) | AC53 | none |
| List element edits | latency, storage | **measured**: 200k `LINSERT` head 19.1 ms vs 0.09/0.24 (79–212×), `LSET` index 100k 16.5 vs 0.14/0.28 (59–118×), `LINSERT` middle 21.0 vs 3.51/4.33, `LREM` tail 21.1 vs 6.97/7.28, `LPOS RANK -1` 1.59 vs 0.16/0.27; 10-element controls at the floor | 4.54, admitted (extends 4.35/4.41) | AC56 | none |
| Set membership reply | replies | **measured lead/null**: `SMEMBERS` 100k 84.75 ms vs 88.71/86.37, 10k 9.95 vs 9.33/9.58, 1k 0.97 vs 1.11/1.24, 10-member control at the floor; the 1.5 MB reply hides the collect+sort | no candidate; keep 4.29 scoped to hashes | n/a | none |
| Multi-key argument cap | parsing, compatibility | **measured boundary**: 1,024 total args accepted, 1,025 rejected; Redis/Valkey execute `MGET` 10,000, `MSET` 600 pairs and `DEL` 2,000; 1,000-key latency is at parity or leads (`MGET` 1.74 vs 2.23/2.34) | 4.55, admitted | AC57 | none |
| Large-frame input growth | parsing | **measured**: 1 MiB SET −28% to −32%, p99 9.8 vs 7.4/7.8 ms; reply copy cannot explain a SET | 4.25, admitted | AC28 on the implemented change | none at 64 B |
| Idle WATCH version bookkeeping | storage | source; ~40–100 ns bound | 4.20, conditional | 4.19 isolates a mutation tail | none |
| One journal write per command | persistence | **measured**: AOF-on SET P1 −66.5%, P16 −90.2% (39.9k vs 406.5k), p99 15.1 vs 3.1 ms; C servers −8.5% to −33.5%; `always` source-only (local cohort failed) | 4.21, admitted | AC23 implementation + A/B | none (persistence off) |
| 1 ms blocked-client wake | latency, CPU | **measured**: after-push p50 1.16 ms / p99 1.92 ms vs 0.03/0.22 ms; idle CPU 5.6% (1 client) / 22.2% (1000) vs <1%, post-#226 5.0/21.4 | 4.22, admitted; implementation next | AC25 on the implemented change | none |
| Idle pub/sub and MONITOR clients | CPU, scheduling, latency | **measured**: 6.0/17.8/22.2% (1/100/1000 subscribers) and 5.8/20.8/27.4% (monitors) vs <1%, post-#226 4.6/13.6/22.4 and 4.2/20.8/26.6; delivery p50/p99 1.05/3.60 ms (pub/sub) and 0.37/6.15 ms (MONITOR) vs ≤0.08/1.19 ms | 4.24, admitted | AC27 on the implemented change | none |
| Rewrite start pause | latency, persistence | 9–14 ms at 20k, 91–107 ms at 200k; 4.85/43.4 s passes | 4.23 | rewrite cohort (AC26) | none (persistence off) |
| Fixed 1,280 reaps/s expiry | storage | source + arithmetic; no TTL cohort | keep; adaptive slices only if measured | 4.16 row 1 | none |
| TTL index and clock | latency, storage | **measured null** in a limited 8-client cohort (~110–125k/s; differences inside host spread) | no candidate; 4.9 stays closed | clean TTL cohort (4.16 row 1) | none |
| Mutation-time LRU, no LFU | storage | source; read-hit quality gap | keep; revisit only if measured | 4.16 row 6 | none |
| Eviction steady-state cost | storage | **measured**: P16 20,498 vs 144,365 no-eviction (−85.8%), p99 64.5 vs 8.0 ms; policy-independent; C servers ~no penalty | 4.26, admitted | AC29 on the implemented change | none (no maxmemory) |
| Startup AOF replay | persistence, availability | **measured**: 0.258 s at 50k, 1.10–1.24 s at 200k vs 0.011–0.061 s for the C servers | 4.27, admitted | AC30 on the implemented change | none (startup) |
| AOF HMAC | persistence, security | **measured**: append −10.2% P1 / −23.6% P16; 50k replay 0.21→0.75 s | accept the append cost; fold verification into 4.27 | AC30 HMAC on/off | none (persistence off) |
| Duplicate key and value encoding | storage (memory) | source; no RSS cohort | defer | TTL/eviction RSS (AC24) | none |
| Incremental keyspace rehash | storage | bound at 1024 keys; local 262k growth attempt inconclusive (host-bound max 371 ms) | deferred 4.18 | clean large-keyspace cohort | none |
| Listen backlog 128, no TCP keepalive | scheduling | local strace + source (C: 511, 300 s); Python connect storm inconclusive (2,232 vs 2,189/3,796 conn/s) | keep; needs a socket dependency/FFI | kernel-level connection-storm / idle-socket cohort | none |
| Pipeline depth and client count | scheduling | **measured**: leads every P≥16 and 64-client row by 29–122%; P1 SET +2.5–3.0% | no candidate | n/a | none |
| Drain probe; parser cursor; cached clock; last-command cache; deferred write coalescing | syscalls, parsing, latency | +12%/+37% syscalls but removal lost P1; others under noise | closed/rejected (2.6, 4.2, 4.5, 4.9, 4.17) | named workload only | none |
| Level-triggered epoll, `SO_BUSY_POLL`, `recvmmsg`/`sendmmsg` | syscalls | source; outside the Mio/`std` boundary | policy decision, not a candidate | explicit policy review | none |

### 2.25 Pipeline depth and client-count scaling (scope check)

The protected gate covers 8 clients at pipeline 1 and 16. A local paired cohort
extended the matrix to 8 clients at P1/16/64/256 and 64 clients at P16, with 100k
requests, 64-byte values, 1024 keys and three rotations. The host is shared and
host-limited (P1 median ~31k vs the gate's ~140k), so only the paired ratios are
read:

| Clients | Pipeline | Workload | Rill vs Redis | Rill vs Valkey |
| ---: | ---: | --- | ---: | ---: |
| 8 | 1 | SET | +3.0% | +2.5% |
| 8 | 1 | GET | +16.5% | +10.0% |
| 8 | 16 | SET | +49.7% | +79.7% |
| 8 | 16 | GET | +60.8% | +99.5% |
| 8 | 64 | SET | +79.6% | +114.4% |
| 8 | 64 | GET | +98.2% | +122.5% |
| 8 | 256 | SET | +54.4% | +71.2% |
| 8 | 256 | GET | +51.4% | +53.5% |
| 64 | 16 | SET | +29.5% | +36.6% |
| 64 | 16 | GET | +83.8% | +54.9% |

Rillcache leads every deep-pipeline and 64-client row, with the largest margins at
P64/P256. The only near-parity cell is P1 SET (+2.5–3.0%), consistent with the
host-sensitive P1 rows in sections 2.17 and 2.19. No scaling candidate is opened;
the P256 SET dip below P64 is shared-host noise, not a mechanism. This also
confirms that the 8-client/P16 gate result is not an artifact of a small client
set.

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
- **Large multi-bulk replies (2026-11-12).** A local paired cohort preloaded a
  10,000-element list (16-byte values) and a 10,000-field hash, then measured
  `LRANGE 0 9999` (230 KB reply) and `HGETALL` (350 KB reply), 4 clients, 250
  requests each, three rotations:

  | Command | Rillcache req/s (p50/p99 ms) | Redis 8.10.2 | Valkey 9.1.2 |
  | --- | ---: | ---: | ---: |
  | `LRANGE` 10k | **1,272** (2.09/15.5) | 913 (3.9/10.9) | 726 (4.9/18.4) |
  | `HGETALL` 10k | 313 (11.6/32.1) | **552** (7.0/16.3) | 336 (11.4/26.4) |

  Rillcache leads the list reply by 39% over Redis and 75% over Valkey, but
  `HGETALL` is **43% behind Redis** (near Valkey), and its per-byte rate is far
  below its own `LRANGE` rate (30 vs 110 MB/s per request at p50). Source:
  `HGETALL` collects every field/value reference into a `Vec`, sorts it for
  deterministic output, then encodes 20,000 bulks, while `LRANGE` iterates the
  deque directly
  ([`collections.rs`](../../src/engine/collections.rs)). Redis iterates its hash
  in internal order. This is candidate 4.29; the full sort/allocation is the
  first suspect, but the chosen order semantics and the persistence test that
  compares exact `HGETALL` bytes must be preserved or explicitly changed.
- **Perf attribution (2026-11-12, diagnostic).** A single-client 10k `HGETALL`
  loop (~268 req/s) sampled with `perf record -F 999 -g`: **11.9%** of samples in
  `core::slice::sort::unstable` (quicksort 6.9% + small-sort 5.0%), **7.8%** in
  `hash_items` collecting the reference vector, **3.3%** in `encode_bulk`, with
  libc self at ~59% (field comparisons and memcpy) and the kernel at ~15%.
  `LRANGE` on the same data has **no sort** and is ~46% kernel. The per-reply
  sort and reference collection are the measured mechanism behind the hash gap,
  not the socket write; 4.29 may remove the sort (with an explicit order
  decision), keep a sorted index, or change the large-hash representation, and
  must re-measure the chosen option.
- **Sorted-set point lookups and range replies (2026-11-12).** Same paired method
  with a 10,000-member sorted set (6-byte members, 3-digit scores), 4 clients,
  250 requests each (1,000 `ZSCORE` requests per client), three rotations:

  | Command | Rillcache req/s (p50/p99 ms) | Redis 8.10.2 | Valkey 9.1.2 |
  | --- | ---: | ---: | ---: |
  | `ZSCORE` | 6,913 (0.37/2.95) | 8,607 (0.32/2.17) | **10,019** (0.28/1.58) |
  | `ZRANGE 0 9` | 528 (4.61/13.9) | **10,311** (0.24/1.49) | 10,771 (0.28/1.37) |
  | `ZRANGE 0 9999` (120 KB) | 340 (11.3/20.3) | **1,581** (2.30/6.04) | 1,036 (3.58/7.61) |

  A ten-element range on a large set is **19.5× slower than Redis and 20.4×
  slower than Valkey** (p50 4.61 vs 0.24/0.28 ms); the full range is 78% behind
  Redis and 67% behind Valkey; `ZSCORE` is 20–31% behind (its three rotations
  ran 2,399–7,806 req/s, so the median is host-noisy, and the p50 gap is
  smaller). Controls put the loss on cardinality, not on sorted sets generally:
  a 128-member set (the listpack-size regime) is inside the host spread for all
  three, and a 1,024-member set is 17–20% behind on `ZRANGE 0 9` (p50 0.58 vs
  0.40/0.43 ms). Source: every `zset_read` calls `zset_items()`, which collects
  all member/score references into a `Vec`, then copies and fully sorts it
  before slicing, even for `0 9`; `ZSCORE` linearly scans the same `Vec`
  ([`collections.rs`](../../src/engine/collections.rs),
  [`data.rs`](../../src/engine/data.rs)). Redis and Valkey keep a dict from
  member to skiplist node for O(1) `ZSCORE` and walk the skiplist (or listpack)
  for an O(log n + k) rank range
  ([`redis t_zset.c#L1539`](https://github.com/redis/redis/blob/8.10.2/src/t_zset.c#L1539),
  [`#L3413`](https://github.com/redis/redis/blob/8.10.2/src/t_zset.c#L3413),
  [`#L708`](https://github.com/redis/redis/blob/8.10.2/src/t_zset.c#L708);
  [`valkey t_zset.c#L1420`](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_zset.c#L1420),
  [`#L3099`](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_zset.c#L3099),
  [`#L617`](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_zset.c#L617)).
  Candidate 4.30.
- **Sorted-set perf attribution (2026-11-12, diagnostic).** A single-client
  10k-member `ZRANGE 0 9` loop (~301 req/s) sampled with `perf record -F 999 -g`:
  **21.5%** in `core::slice::sort::unstable` (quicksort 11.8% + small-sort 8.4%
  + 1.2% comparison helper), a further **21.2%** in a libc copy routine the sort
  uses for its 24-byte tuple moves (`memmove`/`memcpy`), and **6.4%** in
  `zset_items`; libc self totals ~59% (including `malloc`/`free` for the
  per-request `Vec`) and the kernel is ~10%. The cost is the per-request
  collect + sort, not the 125-byte reply.

**Verdict:** the formatting and allocation posture is already competitive for the
gate's 64-byte values: no per-reply allocation, specialized headers, bounded
output. Do not add shared-object tables (they need reference counting and
unsafe-adjacent ownership machinery), and do not add cross-client deferred
flushing: at P1 only one client is ready per pass, so it has no target row, and
at P16 the current path already leads by 54–153% (section 2.19). The remaining
reply-side differences are large values and collection ordering: both Redis
8.10.2 and Valkey 9.1.2 keep a refcounted reference instead of copying a value
of at least 16 KiB, while we copy it into the output `Vec` (candidate 4.15,
admitted by the 2026-11-12 large-value cohort, section 2.21, gated on AC20),
and they keep an ordered index rather than re-sorting a large hash or sorted
set per read (candidates 4.29 and 4.30, sections 3.2). The 64 KiB per-event cap
remains a design confirmation rather than a missing mechanism.

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
- **Large-frame input growth.** The retry in section 2.21 measures a **28–32%
  1 MiB SET** loss that the reply copy cannot explain (the SET reply is small).
  Redis and Valkey reserve query-buffer capacity from the declared bulk length
  (`big_arg` plus `sdsMakeRoomFor` in
  [`redis networking.c#L3893`](https://github.com/redis/redis/blob/8.10.2/src/networking.c#L3893),
  [`valkey networking.c#L4300`](https://github.com/valkey-io/valkey/blob/9.1.2/src/networking.c#L4300)),
  while our input `Vec` grows by appending each 16 KiB read, so a 1 MiB frame
  reallocates and copies the growing buffer several times before the final
  parse. This is not the rescan bound above; it is capacity management.

**Verdict:** do not copy the C object/refcount parser and do not build a
resumable cursor. The parser re-reads only the small length headers of an
incomplete frame, so for K reads the extra work is O(K·headers), not O(K·frame):
a 64-byte header scanned 16 times is about 1 KiB of extra scanning, far below
the gate's per-command noise. Redis's last-command reuse and Valkey's
`cmd_queue` avoid a lookup/reparse that our few byte comparisons already do
cheaply (section 2.20). **Adapt the input-reservation idea** as candidate 4.25:
after a validated bulk header declares length `L`, reserve bounded input
capacity toward the frame end so a large SET does not repeatedly reallocate. The
rescan candidate 4.2 stays closed; this is a different mechanism.

### 3.4 Storage, hashing and expiry

- **Valkey/Redis** use an incremental-rehash hash table (`dict`) with a per-process
  SipHash seed, so growth is amortized in small steps rather than one stop-the-world
  rehash ([`dict.c#L123`](https://github.com/valkey-io/valkey/blob/9.1.2/src/dict.c#L123),
  [`dict.c#L327`](https://github.com/valkey-io/valkey/blob/9.1.2/src/dict.c#L327),
  [`redis dict.c#L299`](https://github.com/redis/redis/blob/8.10.2/src/dict.c#L299)).
  String values use embstr/int/raw encodings; expiry lives in a dedicated
  expires structure and is checked on lookup. Both current pins also keep the
  keyspace in a `kvstore` and rehash incrementally in a 1 ms cron budget
  ([`redis INCREMENTAL_REHASHING_THRESHOLD_US`](https://github.com/redis/redis/blob/8.10.2/src/server.h#L147),
  [`redis server.c#L1352`](https://github.com/redis/redis/blob/8.10.2/src/server.c#L1352),
  [`redis kvstore.c#L674`](https://github.com/redis/redis/blob/8.10.2/src/kvstore.c#L674),
  [`valkey server.c#L1325`](https://github.com/valkey-io/valkey/blob/9.1.2/src/server.c#L1325));
  Redis 8.10.2 still steps a rehash on access inside the dict
  ([`redis dict.c#L471`](https://github.com/redis/redis/blob/8.10.2/src/dict.c#L471)).
  The earlier claim that only Valkey 9.1 had moved to `kvstore` is corrected here.
- **Rillcache** uses `std::collections::HashMap<Vec<u8>, Entry>` with the
  standard randomized SipHash hasher and one `RwLock<Data>`; expiry is an ordered
  `BTreeSet<(i64, Vec<u8>)>`; equal-size SET reuses the value allocation and GET
  reuses its lookup. Unlike the C servers, every mutation also advances a
  sequence and inserts or updates a per-key `versions` entry even when no client
  is watching ([`touch`](../../src/engine/data.rs#L369)), which is the
  storage-path difference in section 2.20 item 7.
- Sorted sets are an unordered `HashMap<Vec<u8>, f64>` above the listpack
  threshold, so rank order and score order exist only inside each read. Redis and
  Valkey store a member→score dict plus a skiplist ordered by `(score, member)`,
  which is what makes point lookup O(1) and a rank range O(log n + k)
  ([`redis t_zset.c#L1539`](https://github.com/redis/redis/blob/8.10.2/src/t_zset.c#L1539),
  [`#L3413`](https://github.com/redis/redis/blob/8.10.2/src/t_zset.c#L3413),
  [`valkey t_zset.c#L1420`](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_zset.c#L1420),
  [`#L3099`](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_zset.c#L3099)).
  The measured 19.5–20.4× small-range loss is candidate 4.30 (section 3.2).
- The map hasher is not exposed by `std`; replacing it would need a dependency or
  a home-grown keyed hash. Both are policy-sensitive (HashDoS resistance) and are
  not justified by the current 1024-key gate.

**Verdict:** avoid custom allocators, non-keyed hashing and sharding. The
incremental-rehash benefit is real but is not a per-command cost at the gate's
1024 keys: a single growth rehash of ~1k entries is a one-off few-thousand-entry
move, and even a 1 MiB keyspace would rehash off the request path. Revisit only
with a large-keyspace measurement that shows a p99 spike during growth
(candidate 4.18). The sorted-set fix in 4.30 is contained in the collection
module and uses `std` ordered containers, so it does not reopen this storage
verdict.

### 3.5 Per-command dispatch and observability

- **Valkey/Redis** look up the command once in a command table, then run `call()`
  which updates counters and optional slowlog/latency/commandstats hooks. Most
  hooks are cheap flags or no-ops on the benchmark path
  ([`valkey server.c#L3830`](https://github.com/valkey-io/valkey/blob/9.1.2/src/server.c#L3830),
  [`valkey lookupCommand`](https://github.com/valkey-io/valkey/blob/9.1.2/src/server.c#L3480),
  [`redis call`](https://github.com/redis/redis/blob/8.10.2/src/server.c#L3992)).
  The lookup itself is a dict probe of the command name (case-insensitive sds
  hash), not a linear scan. Redis 8.10.2 additionally reuses the previous
  command when the name matches (`isCommandReusable`,
  [`server.c#L110`](https://github.com/redis/redis/blob/8.10.2/src/server.c#L110),
  [`server.c#L4507`](https://github.com/redis/redis/blob/8.10.2/src/server.c#L4507)),
  and both pins keep their fixed per-command checks (auth/ACL, arity, pause,
  propagation, latency/commandstats hooks) in `processCommand`/`call`
  ([`redis processCommand#L4467`](https://github.com/redis/redis/blob/8.10.2/src/server.c#L4467),
  [`valkey processCommand#L4278`](https://github.com/valkey-io/valkey/blob/9.1.2/src/server.c#L4278)).
  Our mode-word fast path with a handful of byte comparisons is cheaper than
  that fixed overhead, which is consistent with the per-command CPU results
  (section 2.14).
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
abstraction; the current split is easier to keep out of the hot path. Do not add
a last-command cache either: a byte comparison is already cheaper than a
`strcasecmp` and a pointer check, and the whole difference is tens of
nanoseconds per 8.7 µs command (section 4.17). Keep cross-reactor mutable
counters cache-line separated when they are updated per command; with the
one-reactor default that layout is inert (section 4.10).

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
shape is cheaper, and it is not in the protected gate. The bound for a cached
clock is one vDSO read (~20–30 ns) per command, under the gate noise, so 4.9 is
closed as a performance candidate (section 4.9). If a clean TTL cohort later
shows a clock share above the noise, adapt the snapshot idea behind the existing
`Clock` trait: publish one millisecond value per reactor pass and let commands
read it, keeping the injectable test clock and single-command time consistency.
That is safe Rust with no new dependency.

**TTL-heavy check (2026-11-12).** A limited local diagnostic used 8 Python
pipelined clients (16 deep), 1024 keys, 64-byte values, 160k commands per run and
three rotations, comparing plain keys with keys carrying a 10,000 s TTL. Medians
(req/s, plain/TTL): Rillcache SET 117k/125k, GET 112k/113k; Redis SET 111k/106k,
GET 91k/125k; Valkey SET 110k/122k, GET 120k/111k. Every within-server difference
is inside the host spread, and Rillcache is not slower than the C servers on
either form. The client and shared host bound the absolute rate (~110–125k/s), so
this is not a clean TTL cohort; it does support keeping 4.9 closed and not adding
a TTL fast path speculatively. A clean TTL cohort (4.16 row 1) is still required
for a production claim.

### 3.8 Decision matrix: best of each project

| Component | Redis 8.10.2 | Valkey 9.1.2 | Rillcache today | Decision |
| --- | --- | --- | --- | --- |
| Readiness | level-triggered epoll, single loop | level-triggered epoll, single loop | Mio edge-triggered reactor with drain probe | **Keep/adapt:** edge-triggered through reviewed Mio; level-triggered needs unsafe FFI or a new selector policy |
| Per-request scheduling | event loop, optional I/O threads (default 1, immutable) | event loop, optional I/O threads (default 1, modifiable) | one owner-local reactor plus an opt-in 2–16 connection-local worker override | **Adapt, bounded:** the measured four-CPU P1 gate favored one reactor plus a bounded poll; preserve CPU/tail costs and measure larger fan-out before choosing parallel workers |
| Blocking wakeups | same-loop `signalKeyAsReady` after the command | same | blocked clients re-enqueued and polled every 1 ms; no ready-key signal | **Adopt:** 4.22 same-pass wake + deadline timer; measured 1.16 ms p50 and 5.6–22% idle CPU vs <1%; no gate effect (3.12) |
| Async client servicing (pub/sub, MONITOR) | event-driven push per message/command; idle clients untouched | same | 1 ms tick re-enqueues every subscriber/monitor; idle CPU 6–27% | **Adopt:** 4.24 event-driven wake + deadline timer; measured gap; no gate effect (3.12) |
| Connection setup | backlog 511, TCP keepalive 300 s, NODELAY | same | std backlog 128, NODELAY, 30 s app idle timeout | **Keep:** backlog/keepalive need a socket dependency or FFI; no measured connection-storm gap (3.14) |
| Read path | 16 KiB `PROTO_IOBUF_LEN`, `pendingCommand` | 16 KiB buffer, `cmd_queue` | 16 KiB chunk, complete-frame parse | **Keep:** equivalent for the measured small-command regime |
| Small replies | 16 KiB buffer + reply list + `writev`, deferred flush, 64 KiB cap | same, plus deferred/encoded reply machinery | reused `Vec<u8>`, inline flush, bounded chunks | **Keep:** formatting is allocation-free; deferral has no P1 target and is closed (4.5) |
| Large replies (≥16 KiB) | refcounted object reference, no value copy | refcounted object reference, no value copy | one copy into the output `Vec`, then the kernel copy | **Adopt:** measured 28.4% (64 KiB GET) and 42–45% (1 MiB GET) loss; 4.15, std `Arc`/`IoSlice`, AC20 |
| Large multi-bulk replies (list/hash) | reply list/`writev`; hash iterated in internal order | same | list iterated directly; hash collects, sorts and encodes | **Adopt (list leads, hash lags):** 4.29 for 10k `HGETALL`; measured 43% behind Redis (3.2) |
| Sorted-set point lookup and rank range | member→score dict + skiplist/listpack: O(1) `ZSCORE`, O(log n + k) `ZRANGE` | same | `HashMap`; `ZSCORE` linear scan; every `ZRANGE` collects and full-sorts all members | **Adopt:** 4.30 `std` ordered index; measured 19.5–20.4× loss on 10k `ZRANGE 0 9` (528 vs 10,311/10,771 req/s), 67–78% on the full range, and 378–927× on 100k rank/range reads (3.2, 3.22, AC32/AC40) |
| Sorted-set rank-select | skiplist span counts: O(log n) `ZRANK`, deep `ZRANGE` seek | same | rank requires a full materialize+sort+linear position scan | **Adapt:** 4.38 order-statistic index; measured 100k `ZRANK` 58.6–72.5 ms vs 0.07–0.16, `ZRANGEBYSCORE LIMIT 10` 62.4 vs 0.08–0.12 (3.22, AC40) |
| Sorted-set lex ranges and counts | skiplist lex seek (`zslNthInLexRange`) and first/last rank subtraction for `ZLEXCOUNT`/`ZCOUNT` | same | `BYLEX` sorts the whole set before slicing; counts materialize and filter every member | **Adapt:** 4.50 seek + span counts on 4.30/4.38's index; measured 100k `ZRANGEBYLEX LIMIT 10` 56.0–69.8 vs 0.18–0.74 ms (163–371×), `ZLEXCOUNT` narrow 15.0 vs 0.21–0.22, `ZCOUNT` 1.25–1.75 vs 0.21–0.39 even when empty (3.38, AC52) |
| Sorted-set algebra and cardinality | one generic pass: sort sources by cardinality, iterate the smallest, `zuiFind` probe, count with early LIMIT or insert directly into the destination | same | `owned_zset` copy per source, result `HashMap` with cloned keys, full `ordered_zset` sort; `ZINTERCARD` clones into a `BTreeSet` | **Adapt:** 4.51 borrowed-source probe and single destination build; measured 100k `ZINTERCARD` 220.6 vs 40.4/17.1 ms (5.5–12.9×), `ZINTERSTORE` 306.6 vs 110.5/61.6, `ZDIFFSTORE` 331.3 vs 102.0/61.4, `ZUNIONSTORE` 477.6 vs 366.6/280.4 (3.41, AC53) |
| INFO keyspace accounting | container sizes plus maintained `avg_ttl`; O(number of databases) | same | walk every entry on each `INFO`/`INFO keyspace` to count live/expiring/subexpiry keys and sum TTLs | **Adapt:** 4.52 maintained counters and exact TTL sum; measured 500k `INFO keyspace` 10.0 vs 0.249/0.295 ms (34–40×), 100k 2.79 vs 0.171/0.134, while `INFO stats`/`DBSIZE` stay at the floor (3.42, AC54) |
| Pub/sub fan-out | channel dict lookup; append to each client output; 32 MB pub/sub buffer limit disconnects | same | linear channel scan; per-subscriber 1024-message channel; overflow silently unsubscribes; 64 frames/tick drain; monitors share the queue and silently drop lines | **Adapt:** 4.39 key lookup, byte-bounded queue with a counted drop or documented disconnect, byte-bounded drain; measured 1,024/2,000 burst delivered and subscription lost vs 2,000/2,000 alive, 10k channels 0.38–0.89 vs 0.12–0.31 ms, and 1,024/2,000 monitor lines vs 2,000 (3.23, 3.37, AC41) |
| Cardinality and membership | O(1) dict/intset/listpack probe; cached quicklist length | same | clone and sort the set (`SCARD`/`SISMEMBER`), or collect every reference (`HLEN`/`ZCARD`/`LLEN`) | **Adapt:** 4.40 `Collection::len` + direct membership; measured 100k `SISMEMBER`/`SCARD` 8.0–8.7 vs 0.15–0.22 ms, `HLEN`/`ZCARD` 1.6–2.2, `LLEN` 1.55; controls at the floor (3.24, AC42) |
| List random access | quicklist node seek with cached counts; `LPOS` scans with early stop | same | flatten every element into a reference `Vec` per `LINDEX`/`LPOS` | **Adapt:** 4.41 in-place `list_get`/`list_position`; measured 200k `LINDEX` 1.29–1.47 vs 0.07–0.14 ms, `LPOS` hit-first 1.26 vs 0.10–0.13; missing `LPOS` already leads (3.25, AC43) |
| Two-key moves | `setTypeRemove`/`setTypeAdd`; list pop/push on two borrowed containers | same | `owned_set` clones both sets; `list_values` flattens and clones both lists; both re-encode | **Adapt:** 4.53 in-place remove/insert and chunk pop/push, same-key rotate in place; measured `SMOVE` 100k 25.5 vs 0.40/0.28 ms (64–91×), `LMOVE`/`RPOPLPUSH` 200k 39.2/41.2 vs 0.08–0.34 (79–489×), rotate 20.5 vs 0.26/0.21 (3.43, AC55) |
| List element edits | quicklist node insert/replace/unlink; split/merge at listpack bounds | same | flatten and clone the whole list, edit the `VecDeque`, re-encode | **Adapt:** 4.54 chunk walk and in-place edit, reusing 4.41/4.35; measured 200k `LINSERT` head 19.1 vs 0.09/0.24 ms (79–212×), `LSET` index 100k 16.5 vs 0.14/0.28 (59–118×), middle insert/tail removal 4.9–6.0×/2.9–3.0× (3.45, AC56) |
| Set membership reply | dict/intset/listpack iteration, unspecified order | same | collect every member into a `Vec`, sort, encode | **Keep:** measured `SMEMBERS` 100k 84.75 vs 88.71/86.37 ms (lead), 10k 9.95 vs 9.33/9.58, 1k within spread; the reply hides the collect+sort, so do not extend 4.29 to sets (3.46) |
| Argument-count parsing | multi-bulk count from the declared length; no fixed 1,024 ceiling | no fixed count limit observed; `.argv` grows as declared within `proto-max-bulk-len` | `Limits.max_arguments = 1024` rejects larger arrays with `too many arguments`, no CLI override | **Adapt:** 4.55 frame-derived cap (`max(1024, min(1M, frame/6))`); measured boundary at 1,024 args, while Redis/Valkey accept `MGET` 10,000 (3.47, AC57) |
| Stream append | append one rax/listpack entry; incremental length/memory | same | clone the whole `Stream` and fold `payload_bytes()` twice per `XADD` | **Adapt:** 4.42 in-place append with delta accounting; measured 5k `XADD` 1.36 vs 0.104/0.077 ms (13–18×), 1k 0.263; reads at the floor (3.26, AC44) |
| Geospatial commands | geohash zset, nine neighbour cells, dict point lookup | same | scan every member per search; build all members and scan per `GEOPOS`/`GEODIST` member | **Adapt:** 4.43 neighbor-cell ranges over the ordered index + direct lookup; measured 50k 1 km `GEOSEARCH` 4.34 vs 0.157/0.170 ms, `GEOPOS` 100 24.86 vs 0.47–0.61; all-member search reply-bound (3.27, AC45) |
| Multi-member point lookups | one dict/intset probe per requested member | same | clone+sort the set, or scan every member once per requested member (`ZMSCORE` O(n·m)) | **Adapt:** 4.44 direct probe per member; measured 100k `ZMSCORE` 100 273.2 vs 0.19–0.63 ms, `SMISMEMBER` 100 11.75 vs 0.21–0.39; `HMGET` at the floor (3.28, AC46) |
| Expiry on command paths | lazy per-key expiry + bounded active cycle; no full purge | same | `reap(now, usize::MAX)` on `MGET`/`KEYS`/`RANDOMKEY`/`DBSIZE`/`MSET`/`SCAN` and the admission fallback | **Adapt:** 4.45 bounded lazy expiry; measured after 30k expire `MGET`/`KEYS`/`RANDOMKEY`/`DBSIZE` 36–46 vs 0.6–1.0 ms, lazy `GET` at parity (3.29, AC47) |
| SORT `BY nosort` | source-order slice without sorting or copying | same | clone every element and build an `Item` before applying `LIMIT` | **Adapt:** 4.46 direct nosort slice; measured 100k 7.96 vs 0.12/0.16 ms; sorted/ALPHA/weighted rows 0.9–1.5× (3.30, AC48) |
| Large-hash listings | dict/listpack iteration, no per-call ordering | same | collect refs, sort by field, encode | **Adapt:** 4.29 (extended to `HKEYS`/`HVALS`); measured 100k 132/98/102 vs 15.8–37.8 ms; 10-member controls at the floor (3.31, AC31) |
| Keyspace enumeration | dict scan; unordered `KEYS`; random dict sample | same | collect matched keys, sort, encode; `RANDOMKEY` walks to a random index | **Adapt:** 4.31's ordered key index serves `KEYS`/`RANDOMKEY`; measured 100k `KEYS` 91.1 vs 21–43 ms, `RANDOMKEY` 0.52 vs 0.21–0.29 (3.31, AC33) |
| HyperLogLog | registers in the string, in-place add, cached cardinality | same | decode 16 KB registers per command, re-encode and put; recompute every count | **Adapt:** 4.47 in-place registers + cached count; measured 100k adds 16.96 vs 3.9–4.1 µs and 10k counts 265 vs 3.3–3.6 µs (3.32, AC49) |
| Bitmap scan | word-wise `redisBitpos` with `trailing_zeros` | same | byte-and-bit loop in `find_bit` | **Adapt:** extend 4.37 with a `u64` scan; measured 16 MB `BITPOS 0` 201.6 vs 5.08/5.14 ms (39–44×), first-bit hit at the floor (3.33, AC39) |
| Stream `XGROUP CREATE` | insert one group in the stream dict; O(1) | same | deep-clone the whole stream, fold payload, replace via `put_stream` | **Adapt:** 4.48 in-place group insert; measured 5k 8.26 vs 0.083/0.094 ms (88–99×), other XGROUP ops at parity (3.34, AC50) |
| Stream consumer-group reads | radix PEL with a consumer pointer; reply writes stored fields directly | same | per-entry `StreamEntry` clone, two PEL inserts and a consumer-name copy per pending entry | **Keep:** measured near-parity at 10k; the 1,000-entry forms are 1.7× (new read) and 2.1× (`XAUTOCLAIM`), while 100-entry history/`XPENDING`/`XCLAIM` are at parity; re-measure after 4.42 (3.39, no candidate) |
| Sorted-set pops | skiplist head/tail pop, O(log n) | same | clone, sort and replace the whole set per pop | **Adapt:** 4.49 ordered `(score, member)` index pop; measured 100k `ZPOPMIN` 156.7 vs 0.42/0.34 ms, `ZMPOP` 155.8, `ZPOPMIN 10` 146.6 (373–974×), 100-member control at the floor (3.35, AC51) |
| Cursor iteration (SCAN/HSCAN/SSCAN/ZSCAN) | reverse-binary dict cursor, bounded visited buckets, listpack collections in one call | same | per-call full key snapshot + sort; collection deep copy + sort; unbounded expiry reap | **Adopt:** 4.31 ordered index + opaque last-element cursor; measured 42.6 ms vs 0.256/0.164 ms per `SCAN COUNT 10` (167–260×), 20–29× full traversal (3.16, AC33) |
| List/drop representation | quicklist of listpack nodes; UNLINK and ASYNC flush hand off to the bio lazy-free thread | same; DEL async by default | one heap buffer per element; UNLINK == DEL; ASYNC ignored in flush | **Adapt:** 4.32 packed nodes + 4.33 deferred drop; measured `DEL` 7.28 vs 0.166/0.130 ms, `UNLINK` 6.21 vs 0.316/0.235, 142–143 vs 7.5 B/elem, `FLUSHALL ASYNC` 9.20 vs 0.886/0.568 ms (3.17, AC34/35) |
| String mutation | sds with spare capacity: APPEND copies only the suffix, SETRANGE patches in place | same | pre-fix: rebuild full-length `Vec` and copy twice; now mutates the entry in place (#258) | **Done:** 4.34 in-place mutation; measured 32 MB post-fix `APPEND` 0.131 vs 0.142/0.219 ms and `SETRANGE` 0.185 vs 0.222/0.289 (3.18, AC36) |
| Collection writes | in-place dict/skiplist/listpack/quicklist update | same | clone the whole collection, re-encode and replace it per write | **Adapt:** 4.35 in-place mutation; measured 10k `HSET` 6.45 vs 0.111/0.093 ms, 200k `RPUSH` 18.3 vs 0.08/0.07, `LPOP` 17.8 vs 0.07–0.34; 100k `HSET`/`HDEL` 92.7–99.4 vs 0.08–0.32 and `ZADD`/`ZREM` 61.4–63.9 vs 0.10–0.23 (18–1160×); controls at the floor (3.19, AC37) |
| Random-member selection | `dictGetRandomKey`/`dictGetSomeKeys`, intset/listpack fast paths | same | clone and sort/copy the whole collection per call | **Adapt:** 4.36 dense member index after a no-copy reservoir slice; measured 100k `SRANDMEMBER` 11.54 vs 0.199/0.326 ms, `HRANDFIELD` 33.66 vs 0.343/0.113, `ZRANDMEMBER` 21.32 vs 0.250/0.164; 10k 1–2.6 ms, controls at the floor (3.20, AC38) |
| Bitmaps | in-place sds bit write; word/AVX2 popcount | same (scalar/AVX2/NEON popcount) | rebuild the value for `SETBIT` and `BITFIELD` writes (two copies); byte-at-a-time `BITCOUNT` | **Adapt:** 4.37 in-place `SETBIT`/`BITFIELD` (4.34 shape) + `u64` chunk popcount; measured 16 MB 5.32 vs 0.349/0.312 ms, `BITFIELD` write 4.39–4.68 vs 0.087–0.12, and 23.01 vs 3.37/4.56; controls at the floor (3.21, 3.40, AC39) |
| Parsing | incremental multibulk/inline state | incremental state plus command queue | complete-frame fast path, no resumable cursor | **Keep:** incomplete frames rescan only length headers, not payload; 4.2 closed by bound |
| Large-frame input | reserves query-buffer capacity from the declared bulk length (`big_arg`/`sdsMakeRoomFor`) | same | appends 16 KiB reads into a growing `Vec` | **Adopt:** 4.25 bounded reservation; measured 28–32% 1 MiB SET loss (2.21, 3.3) |
| Storage | `kvstore`, SipHash, 1 ms cron rehash + step-on-access | `kvstore`, SipHash, 1 ms cron rehash | `std` HashMap + `RwLock<Data>`, BTreeSet expiry, always-on WATCH versions | **Avoid:** no safe incremental-rehash std API; gate keyspace does not pay it (4.18). **Adapt conditionally:** copy the no-watcher fast path only if 4.19 isolates a mutation tail (4.20) |
| Journal writes | buffered `aof_buf`, one write per event loop, 1 s background fsync | same | one framed write per journaled command, 1 s fsync | **Adopt:** batch per pass (4.21); measured −66.5% P1 / −90.2% P16 AOF-on loss; keep CRC/HMAC framing and write-before-reply ordering |
| Journal rewrite | fork + COW child, parent serves; fork page-table pause | same | key-list materialisation (9–20 ms at 20k keys) then 512-key/1 MiB slices on the owner | **Keep:** no fork in safe Rust; reduce the start pause with 4.23 only if a rewrite-latency cohort shows it matters (3.13) |
| AOF replay | single AOF load; no separate per-command CRC pass | single load | checksum parse + reparse + normal execute (200k: ~1.16 s) | **Adopt:** 4.27 single-pass loader; measured 20–45× slower to ready (3.15) |
| Dispatch | dict command table + `lastcmd` reuse + `call()` | dict command table + `call()` | inlined PING/SET/GET/ECHO, checked dispatcher otherwise | **Keep:** inlining is cheaper for gate commands; last-command cache is below noise (4.17) |
| Transactions (MULTI/EXEC) | queue command refs, ACL at queue time, one reply array | same | copy args at queue, re-check ACL at EXEC, execute through the dispatcher | **Keep:** measured 1,000-command tx 3.02 vs 2.48–2.76 ms and the plain batch leads (3.36) |
| Time | cached `mstime` per execution unit; hardware-clock refresh every ≤25 calls | cached `mstime` per execution unit | `SystemTime::now()` only when expirations exist | **Keep/close 4.9:** a TTL clock call is ~20–30 ns, below noise; revisit only with a TTL-specific workload |
| Expiry | lazy + sampled active cycle, 25% CPU budget, fast 1 ms, effort levels | same | lazy + ordered `BTreeSet` reap, 128 due keys per 100 ms | **Keep:** ordered removals are exact and capped at ~0.1–0.3% CPU; adaptive bursts only with the 4.16 TTL row (3.10) |
| Eviction | 16-entry sampled pool, LRU clock on access, optional LFU | same | bounded candidate scans, LRU by mutation time, no LFU | **Adopt:** 4.26 pooled/batched eviction; measured 85.8% P16 loss under pressure, policy-independent; read-aware LRU still unmeasured (3.10) |
| Value/key encoding | embstr ≤44 B, int objects, shared key with expires, listpack/intset small collections | same | `Vec<u8>` values, key duplicated in expiry/version indexes, ordinary collection containers | **Avoid rewrite:** short strings are one allocation either way and no gate row pays it; only a TTL/eviction RSS result could justify removing the duplicate key (3.11) |

### 3.9 Persistence write path (append-only journal)

- **Redis 8.10.2 and Valkey 9.1.2** append each write command's RESP encoding to
  a shared in-memory `aof_buf` (`feedAppendOnlyFile`) and write the accumulated
  buffer once per event-loop iteration from `beforeSleep` or cron
  ([`redis aof.c#L1661`](https://github.com/redis/redis/blob/8.10.2/src/aof.c#L1661),
  [`redis flushAppendOnlyFile#L1399`](https://github.com/redis/redis/blob/8.10.2/src/aof.c#L1399),
  [`redis server.c#L2057`](https://github.com/redis/redis/blob/8.10.2/src/server.c#L2057);
  [`valkey aof.c#L1446`](https://github.com/valkey-io/valkey/blob/9.1.2/src/aof.c#L1446),
  [`valkey flushAppendOnlyFile#L1178`](https://github.com/valkey-io/valkey/blob/9.1.2/src/aof.c#L1178),
  [`valkey server.c#L1830`](https://github.com/valkey-io/valkey/blob/9.1.2/src/server.c#L1830)).
  `everysec` fsyncs when at least one second has elapsed, in a background bio
  thread ([`redis aof.c#L1233`](https://github.com/redis/redis/blob/8.10.2/src/aof.c#L1233),
  [`valkey aof.c#L908`](https://github.com/valkey-io/valkey/blob/9.1.2/src/aof.c#L908));
  `always` fsyncs the batch before the replies are considered durable; `no` never
  fsyncs. A pipeline of K writes therefore costs about one `write` per event-loop
  batch plus at most one background fsync per second, not K writes.
- **Rillcache** encodes and appends one authenticated transaction per journaled
  command inside `execute_locked`, taking the journal mutex and calling
  `write_all` per command
  ([`journal.rs#L1028`](../../src/engine/journal.rs#L1028),
  [`#L1070`](../../src/engine/journal.rs#L1070)); `everysec` syncs when one
  second has elapsed ([`#L1102`](../../src/engine/journal.rs#L1102)) and `always`
  syncs each append. Its transaction framing adds a CRC32 record and an optional
  HMAC record per transaction, which the C servers do not have.
- Consequence: at pipeline 1 the two shapes are equivalent (one command per
  batch). At pipeline K>1 with persistence on, each of our K journaled commands
  pays a separate write syscall, a mutex acquisition and a framing allocation,
  while the C servers pay one buffered write for the batch. This is invisible to
  the protected gate (persistence off).
- **Current-head measurement (2026-11-12).** Paired local SET runs, three
  Latin-rotated rounds, `appendonly yes`, `appendfsync everysec`, 8 clients,
  1024 keys, 64 B, 50k requests, fresh journal per run. Absolute rates are far
  below the gate because the host is shared; the paired AOF-on/off ratio and the
  cross-server pattern are the signal:

  | Workload | Server | AOF off | AOF on | Change | p99 off/on |
  | --- | --- | ---: | ---: | ---: | ---: |
  | P1 SET | Rillcache | 35,387 | 11,856 | **−66.5%** | 1.53/5.53 ms |
  | P1 SET | Redis 8.10.2 | 24,983 | 27,977 | +12.0% (noise) | 1.73/1.42 ms |
  | P1 SET | Valkey 9.1.2 | 25,428 | 20,923 | −17.7% | 1.72/2.47 ms |
  | P16 SET | Rillcache | 406,549 | **39,927** | **−90.2%** | 3.09/15.12 ms |
  | P16 SET | Redis 8.10.2 | 294,371 | 195,669 | −33.5% | 1.55/1.98 ms |
  | P16 SET | Valkey 9.1.2 | 168,126 | 153,763 | −8.5% | 5.08/3.50 ms |

  The rill P16 ratio is 4.0–12.5× per round (off runs varied 169k–488k; the
  AOF-on rate was stable at 39–42k), while the C servers lose at most ~1.9× in a
  round. At P1 AOF-on, Rillcache (11.9k) is slower than Valkey (20.9k) even though
  it leads with AOF off (35.4k vs 25.4k). This is a large, consistent persistence
  penalty, not host noise; HMAC was off, so the framing cost here is CRC plus the
  per-command write, not authentication.
- **HMAC cost (2026-11-12).** With `--aof-hmac-key-file` on the same server,
  AOF-on SET is 12,893 vs 14,355 req/s at P1 (−10.2%) and 31,938 vs 41,821 at P16
  (−23.6%), p99 12.0 vs 11.5 ms. A 50k-key replay takes 0.48–0.88 s (median 0.75)
  versus 0.20–0.23 s (median 0.21), a 3.6× startup increase with DBSIZE 50,000 in
  both. The append cost is one SHA-256 per transaction (in-repo safe Rust); the
  replay cost is the separate authentication pass (section 3.15). This is the
  security feature's price: do not replace the hash, reduce the per-transaction
  chain or skip verification without a security review. Candidate 4.27 can fold
  the verification into the replay pass without changing the chain.
- **`appendfsync always` (source only; local cohorts failed).** Rillcache calls
  `sync_data` inside every `append`, while the C servers fsync the buffer once
  per event-loop flush; at pipeline K this is K fsyncs versus about one. Two
  local paired attempts on 2026-11-12 were **unusable**. The first, at host load
  10+, varied from 1.6k to 107k req/s with some `always` runs faster than
  `everysec`. The second, at load ~3 with 5k requests and three rounds, still
  produced `always` above `everysec` for the C servers at P16 (Redis 148.8k vs
  137.4k, Valkey 143.8k vs 120.4k) and Rillcache P16 37.5k vs 32.7k, which is
  physically implausible if the sync is real. The host filesystem/overlay does
  not expose a stable fsync cost, so no numbers are claimed. The mechanism still
  makes `always` the strongest case for 4.21's batching: one durable write and
  one fsync per batch, with every acknowledged command still covered.

**Verdict:** the persistence-on gap is measured and large; 4.21 is admitted. Adapt
the buffering shape, keep the per-transaction CRC/HMAC framing, do not move the
write after the reply flush, and do not change the 1 s everysec policy. Do not
copy the unauthenticated AOF format. Absolute rates in the table are shared-host
diagnostics, not a gate result.

### 3.10 Expiry and eviction under memory pressure

The protected gate has no TTLs and no `maxmemory`, so both paths are exercised
only by unit/durability tests. Their shapes still differ:

- **Redis 8.10.2 and Valkey 9.1.2** expire lazily on access and run an active
  cycle from cron: 20 keys per DB per iteration
  ([`redis expire.c#L96`](https://github.com/redis/redis/blob/8.10.2/src/expire.c#L96),
  [`valkey expire.c#L122`](https://github.com/valkey-io/valkey/blob/9.1.2/src/expire.c#L122)),
  up to `CRON_DBS_PER_CALL` 16 DBs at `hz` 10
  ([`redis server.h#L123`](https://github.com/redis/redis/blob/8.10.2/src/server.h#L123)),
  a slow-cycle budget of 25% of one CPU and a 1 ms fast cycle when more than 10%
  of the sampled keys are stale
  ([`redis expire.c#L287`](https://github.com/redis/redis/blob/8.10.2/src/expire.c#L287));
  `active-expire-effort` 0–9 scales keys per loop, fast duration and the
  acceptable-stale threshold
  ([`valkey expire.c#L202`](https://github.com/valkey-io/valkey/blob/9.1.2/src/expire.c#L202)).
  Eviction samples keys into a 16-entry pool
  ([`redis evict.c#L36`](https://github.com/redis/redis/blob/8.10.2/src/evict.c#L36),
  [`valkey evict.c#L54`](https://github.com/valkey-io/valkey/blob/9.1.2/src/evict.c#L54)),
  updates an LRU clock on access and optionally keeps LFU counters.
- **Rillcache** expires lazily on access and calls `purge_expired(128)` on the
  100 ms maintenance tick
  ([`reactor.rs#L443`](../../src/server/reactor.rs#L443),
  [`pool.rs#L586`](../../src/server/pool.rs#L586)) under the data borrow; the
  ordered `BTreeSet<(deadline, key)>` reaps the earliest due keys exactly
  ([`Data::reap`](../../src/engine/data.rs#L599)), so 128 reaps are 128 real
  removals, not 128 random samples. Eviction uses bounded candidate scans and
  approximates LRU by **mutation** time because reads do not take the write lock
  ([`eviction.rs`](../../src/engine/eviction.rs)); there is no LFU.
- Arithmetic: our fixed cap is 128 × 10 = **1,280 due removals/s** at a bounded
  ~0.1–0.3% CPU and a short lock hold. Redis/Valkey's slow cycle samples 20 keys
  per active DB per 100 ms (200/s for a single active DB; 3,200/s aggregate over
  16 DBs) and can burst to 1 ms cycles when >10% of samples are stale, at up to
  25% CPU. Our ordered reap is more aggressive than the single-DB baseline and
  never removes a live key; their sampled cycle can miss due keys but bursts
  harder under a high stale ratio.
- **Eviction steady-state measurement (2026-11-12).** 100k SET requests over a
  100k-key space, 64-byte values, 8 clients, pipeline 1/16, `allkeys-lru`;
  controls are the same servers without a memory limit. Rill uses a 500 KB
  payload budget (median 7,073 evictions), Redis/Valkey 2 MB `maxmemory`
  (~4,000 evictions):

  | Workload | Server | No eviction | Eviction | Change | p99 no/evict ms |
  | --- | --- | ---: | ---: | ---: | ---: |
  | P1 SET | Rillcache | 14,895 | 8,339 | **−44.0%** | 5.95/8.20 |
  | P1 SET | Redis 8.10.2 | 12,371 | 12,150 | −1.8% | 6.20/6.40 |
  | P1 SET | Valkey 9.1.2 | 11,888 | 13,265 | +11.6% | 6.33/5.24 |
  | P16 SET | Rillcache | 144,365 | **20,498** | **−85.8%** | 8.01/64.51 |
  | P16 SET | Redis 8.10.2 | 130,085 | 171,696 | +32.0% (noisy) | 7.09/4.67 |
  | P16 SET | Valkey 9.1.2 | 117,044 | 137,789 | +17.7% (noisy) | 9.54/5.56 |

  A policy A/B on the same budget shows the LRU choice is not the cause:
  Rillcache `allkeys-lru` 27.1–31.3k and `allkeys-random` 25.3–27.6k at P16, both
  far below the no-eviction control, while Redis shows no penalty under either
  policy. The cost is the eviction path itself: per-write bounded candidate scans
  under continuous pressure versus the C servers' sampled pool that evicts in a
  pass. The budgets differ (payload vs RSS) and Rillcache evicts ~1.75× more
  keys, but that does not explain a 5–7× throughput difference. Shared-host
  diagnostic, not a gate result.
- Eviction quality: mutation-time LRU is a real policy gap, not a measured
  throughput loss. A read-heavy keyspace under `allkeys-lru` can evict keys that
  Redis/Valkey just read and would keep. Read-aware LRU would need an atomic or
  per-entry access update on GET (Redis stores the clock on lookup when an LRU
  policy is active), which adds work to the hot read path; it is not justified
  without an eviction cohort.

**Verdict: keep the ordered purge; the eviction path is now admitted as 4.26.**
The measured eviction penalty is the per-write admission scan, not the LRU
quality; a pooled/batched eviction pass is the candidate. The TTL high-churn
question remains for 4.16 row 1, and a read-aware access stamp remains
unmeasured. Do not copy adaptive expiry effort or LFU counters speculatively.

### 3.11 Key and value representation (memory, not gate rate)

The gate's 64-byte string values hide the representation differences that matter
for memory-heavy workloads:

- **Redis 8.10.2 and Valkey 9.1.2** store a short string in one embstr
  allocation (≤44 bytes,
  [`redis object.c#L337`](https://github.com/redis/redis/blob/8.10.2/src/object.c#L337)),
  encode an integer value as an `int` object with no byte buffer
  ([`tryObjectEncoding`](https://github.com/redis/redis/blob/8.10.2/src/object.c#L372),
  used by SET at
  [`t_string.c#L442`](https://github.com/redis/redis/blob/8.10.2/src/t_string.c#L442)),
  and keep small collections in a single listpack/intset block (hash 512,
  zset 128, set 512 entries by default,
  [`redis config.c#L3511`](https://github.com/redis/redis/blob/8.10.2/src/config.c#L3511),
  [`#L3593`](https://github.com/redis/redis/blob/8.10.2/src/config.c#L3593),
  [`#L3603`](https://github.com/redis/redis/blob/8.10.2/src/config.c#L3603)).
  The expiry dict shares the key pointer with the main dict; watched-key state
  exists only for clients that watch.
- **Rillcache** stores `value: Vec<u8>` for every string
  ([`Entry`](../../src/engine/data.rs#L204)), always one heap buffer even for an
  integer, and builds collections as ordinary Rust containers. TTL and WATCH
  state duplicate the full key: `expirations:
  BTreeSet<(i64, Vec<u8>)>`
  ([`data.rs#L251`](../../src/engine/data.rs#L251)) and
  `versions: HashMap<Vec<u8>, u64>`
  ([`#L252`](../../src/engine/data.rs#L252)). A TTL key that has been written can
  therefore own four buffers (main key, value, expiry key, version key) where the
  C servers own one key object, one value object and two pointer/index entries.
- Gate scope: the 64-byte non-TTL, non-WATCH SET/GET rows use exactly two
  buffers per key (main key + value), the same order as the C servers' raw
  encoding. None of the duplication above is on a protected row.

**Verdict: do not add an encoding rewrite.** Embstr is not obviously cheaper
than the current `Vec` (both are one allocation for short strings), integer
encoding would save one buffer per counter but the clean cohort already leads the
`incr` rows, and listpack for every collection is a large storage rewrite that
would touch the collection, expiry, stream and journal encodings for no gate row.
The 2026-11-12 large-collection cohort (section 3.17) changes that for lists
specifically: 142–143 B/element versus 7.5 B and 6.2–7.3 ms to drop a
200k-element list are a measured non-gate cost, so packed list nodes are admitted
as 4.32 with AC34. The actionable duplicate **key** buffers remain separate:
4.20 removes the version copy whenever nothing watches, and a shared-key/index
representation is a possible follow-up only if the TTL row's RSS shows it
matters. That design must not put a refcount or an index lookup on the GET/SET hot
path without a matched measurement. Add RSS reporting to the TTL and eviction rows
(4.16 rows 1 and 7) as the memory acceptance signal.

### 3.12 Blocking-command wakeups (latency)

- **Redis 8.10.2 and Valkey 9.1.2** park a blocked client and, when a mutation
  makes one of its keys ready, call `signalKeyAsReady` from the mutation path
  (for example `dbAdd`/`dbOverwrite` and list pushes:
  [`redis db.c#L445`](https://github.com/redis/redis/blob/8.10.2/src/db.c#L445),
  [`#L790`](https://github.com/redis/redis/blob/8.10.2/src/db.c#L790),
  [`valkey db.c#L223`](https://github.com/valkey-io/valkey/blob/9.1.2/src/db.c#L223)).
  `processCommand` then calls `handleClientsBlockedOnKeys()` immediately after
  `call()`
  ([`redis server.c#L4846`](https://github.com/redis/redis/blob/8.10.2/src/server.c#L4846),
  [`valkey server.c#L4647`](https://github.com/valkey-io/valkey/blob/9.1.2/src/server.c#L4647)),
  so the woken client is served in the same event-loop iteration without waiting
  for a timer.
- **Rillcache** adds a blocked session to the reactor's `asynchronous` set and
  re-enqueues it only every `ASYNC_TICK` = 1 ms
  ([`reactor.rs#L20`](../../src/server/reactor.rs#L20),
  [`#L467`](../../src/server/reactor.rs#L467)); the session then polls the
  blocking pop ([`session/blocking.rs#L436`](../../src/session/blocking.rs#L436)).
  Fixed-worker lanes park for 1 ms and behave the same, and there is no
  ready-key signal from the mutating command.
- Arithmetic: wake latency is quantized to 0–1000 µs (about 500 µs on average)
  plus service, against a same-loop wake in the C servers. At a blocking p99
  target below 1 ms this is the difference between meeting and missing the SLO,
  even though the protected gate has no blocking command and sees exactly zero
  effect.
- **Current-head measurement (2026-11-12).** A paired local harness ran 300
  `BLPOP`/`LPUSH` wakeups per run, three Latin-rotated runs per server (900
  wakeups each), with a randomized 0.5–2.5 ms delay between the blocking command
  and the push so the 1 ms tick phase is not aligned. `after-push` is measured
  from the LPUSH reply to the BLPOP reply; `total` includes the push round trip:

  | Server | after-push p50 | after-push p99 | total p50 | total p99 |
  | --- | ---: | ---: | ---: | ---: |
  | Rillcache | **1.16 ms** | **1.92 ms** | 1.39 ms | 2.43 ms |
  | Redis 8.10.2 | 0.031 ms | 0.221 ms | 0.505 ms | 2.21 ms |
  | Valkey 9.1.2 | 0.025 ms | 0.220 ms | 0.412 ms | 2.88 ms |

  After the push reply, Rillcache is ~37–46× slower at p50 and ~9× at p99 than
  the C servers on the same host; the total gap is ~2.7–3.4× because the LPUSH
  round trip and shared-host scheduling dominate it. The measured ~1.16 ms p50
  matches the 1 ms tick, not a parser or storage cost, and the C servers stay
  around 30 µs. This is a shared-host diagnostic, not a cross-server cohort, but
  the relative gap is large and consistent across rotations.
- **Idle CPU with blocked clients (2026-11-12).** The same 1 ms tick also keeps
  the reactor busy when nothing is pushed. Server process CPU over a 5 s window
  with N idle `BLPOP` clients, no traffic (measured before the #226 spin fix):

  | Idle blocked clients | Rillcache | Redis 8.10.2 | Valkey 9.1.2 |
  | ---: | ---: | ---: | ---: |
  | 0 | 0.6% | 0.4% | 0.4% |
  | 1 | **5.6%** | — | — |
  | 10 | 6.8% | — | — |
  | 50 | 13.0% | — | — |
  | 100 | **17.8%** | 0.2% | 0.4% |
  | 1000 | **22.2%** | 0.6% | 0.6% |

  One blocked client already costs ~5.6% of a core, and 1000 costs ~22%, while
  the C servers stay under 1%. The mechanism is visible in the reactor: a blocked
  poll that makes no progress still returns `Served::Runnable(false)`, the loop
  counts it as `serviced`, and the 20 µs `FAST_POLL` window is armed after every
  1 ms tick; on top of that, every blocked client is re-enqueued and re-polled
  each tick. Event-driven wake plus a deadline timer removes both, and the
  implementation should stop counting a no-progress blocked poll as productive
  work so the spin is not re-armed.

- **Idle pub/sub and MONITOR clients (2026-11-12).** The same `asynchronous` set
  also holds subscribers and monitoring clients, so an idle one costs CPU too
  (measured before the #226 spin fix):

  | Mode | Idle clients | Rillcache | Redis 8.10.2 | Valkey 9.1.2 |
  | --- | ---: | ---: | ---: | ---: |
  | SUBSCRIBE | 0 | 0.8% | 0.2% | 0.4% |
  | SUBSCRIBE | 1 | **6.0%** | 0.2% | 0.4% |
  | SUBSCRIBE | 100 | 17.8% | 0.6% | 0.6% |
  | SUBSCRIBE | 1000 | **22.2%** | 0.4% | 0.4% |
  | MONITOR | 0 | 0.6% | 1.6% | 0.4% |
  | MONITOR | 1 | **5.8%** | 0.4% | 0.4% |
  | MONITOR | 100 | 20.8% | 0.4% | 0.4% |
  | MONITOR | 1000 | **27.4%** | 0.4% | 0.8% |

  One idle subscriber or monitor already costs ~6% of a core; 1000 costs 22–27%,
  against under 1% for the C servers. The mechanism is identical: the 1 ms tick
  re-enqueues every `is_subscribed()`/`is_monitoring()` client, and the
  no-progress poll arms the 20 µs spin after each tick. This is a scheduling and
  CPU-efficiency gap, not a message-path cost; candidate 4.24 covers the
  event-driven servicing shape and shares the no-progress-spin fix with 4.22.
  The no-progress spin component is implemented separately in PR #226, merged as
  `b7f5694`; its own bound is about two CPU points per 1 ms tick, so it does not
  remove the per-client scan that dominates these numbers (they are pre-fix).
- **Async delivery latency (2026-11-12).** The same tick delays the message or
  monitor line. 300 publishes/commands per run, three rotated runs per server,
  randomized 0.5–2.5 ms phase; `after` is measured from the publishing or
  monitored command's reply to the subscriber/monitor receipt:

  | Path | Rillcache p50 / p99 | Redis 8.10.2 | Valkey 9.1.2 |
  | --- | ---: | ---: | ---: |
  | PUBLISH → subscriber | **1.05 / 3.60 ms** | 0.075 / 1.19 ms | 0.064 / 0.56 ms |
  | PING → MONITOR line | **0.37 / 6.15 ms** | 0.050 / 0.64 ms | 0.048 / 0.49 ms |

  Rillcache is ~14–16× slower at p50 for pub/sub and ~3–6× at p99, and ~7× at p50
  for the MONITOR line, because delivery waits for the 1 ms tick; the C servers
  deliver within the command's event-loop iteration. Total latency adds the
  command round trip and shared-host scheduling. Candidate 4.24 covers both the
  CPU and this delivery latency; the measured baselines are what AC27 targets.

- **Post-#226 re-measurement (current head, 2026-11-12).** The no-progress spin
  fix is merged, so the tables above were re-run on the current binary:
  - idle `BLPOP`: 0.6 / 5.0 / 6.6 / 12.4 / 20.4 / 21.4% at 0/1/10/50/100/1000
    clients (pre-fix 0.6 / 5.6 / 6.8 / 13.0 / 17.8 / 22.2);
  - idle `SUBSCRIBE`: 0.8 / 4.6 / 13.6 / 22.4% at 0/1/100/1000 (pre-fix
    0.8 / 6.0 / 17.8 / 22.2);
  - idle `MONITOR`: 1.0 / 4.2 / 20.8 / 26.6% at 0/1/100/1000 (pre-fix
    0.6 / 5.8 / 20.8 / 27.4).

  The values are within run-to-run noise of the pre-fix tables. The spin fix
  removed its roughly two-point component but **did not remove the per-client
  1 ms scan**, which still costs ~4–5% of a core with one idle asynchronous
  client and 20–27% at 1000 versus <1% for the C servers. 4.22/4.24 remain the
  required mechanism; the measured baselines are unchanged for AC25/AC27.

**Verdict: the gap is measured, not just arithmetic, and 4.22 is admission-ready.**
Adapt the same-pass ready-key wake; keep the 1 ms tick for timeouts and
cross-thread or fallback paths, do not busy-poll blocked clients, and add no
dependency: the signal is an in-memory ready set owned by the same owner that
holds the data lock.

### 3.13 Rewrite scheduling (fork versus cooperative slices)

The journal rewrite is the other persistence path the gate never runs, and the
C servers and Rillcache solve it structurally differently:

- **Redis 8.10.2 and Valkey 9.1.2** run `BGREWRITEAOF` through
  `rewriteAppendOnlyFileBackground`, which forks a child
  ([`redis aof.c#L3239`](https://github.com/redis/redis/blob/8.10.2/src/aof.c#L3239),
  [`valkey aof.c#L2595`](https://github.com/valkey-io/valkey/blob/9.1.2/src/aof.c#L2595))
  that writes the snapshot from a copy-on-write view while the parent serves.
  The parent pays the fork page-table pause and later COW faults under writes;
  it has no in-process per-key encoding pass.
- **Rillcache** cannot fork a copy-on-write view in safe Rust/`std`. The command
  materialises the key list, creates the temporary file and replies
  `+Background append only file rewriting started`; the maintenance loop then
  encodes at most 512 keys or 1 MiB per tick, re-encodes keys written since the
  previous round, and appends residual writes before the atomic rename
  ([persistence.md](../persistence.md), "Compaction with BGREWRITEAOF").
  Measured on the development host with 20,000 keys of 64-byte values (2.5 MB
  journal): the old synchronous pass blocked service for **130 ms**; the
  cooperative rewrite replies in **9–20 ms** (key-list materialisation), encodes
  **1.6–2.4 ms** per 512-key slice, and completes the background pass in about
  4–5 s while clients are served between slices.
- **Current-head reproduction (2026-11-12).** Re-ran the fixture on this head at
  two sizes, with one client issuing P1 GETs during a start pause and another
  run covering the whole rewrite:

  | Keys | Journal (pre / compacted) | `BGREWRITEAOF` start pause, median [range] | Full pass | Concurrent GET p99 / max |
  | ---: | ---: | ---: | ---: | ---: |
  | 20,000 | 4.3 / 2.5 MB | 9–14 [5.5–45.7] ms | 4.85 s | 10.8 / 88.5 ms (4,187 reqs) |
  | 200,000 | 42.8 / 24.8 MB | **91–107 [83.3–116.0] ms** | 43.4 s | 18.8 / **338 ms** (23,791 reqs) |

  The pause scales roughly linearly with the key count (10× keys → ~7–8×
  median), as expected from materialising the whole key list in one turn, and
  total completion scales the same way (512 keys per 100 ms tick). At production
  keyspaces the start pause is around a tenth of a second, not the small-fixture
  9–20 ms; the full-pass p99 is 10.8/18.8 ms and the maxima (88.5/338 ms) include
  shared-host scheduling beyond the rewrite itself. These are single shared-host
  diagnostics consistent with the existing method, not cross-server cohorts or
  gate results.
- Consequence: on a small dataset the fork path pays a short page-table pause
  while our start pause is 9–20 ms; on a large dataset the fork's
  page-table/COW cost can become the worse pause. This is a real latency trade,
  not a correctness difference, and it is invisible to the persistence-off gate.
  The remaining actionable piece is our **start pause**, which scales with the
  number of keys because the whole key list is materialised in one command turn.

**Verdict: keep the cooperative slices and do not copy fork/COW.** Forking is
outside the safe-Rust boundary without a new unsafe/FFI dependency, and the
current commit path is already crash-tested. Reduce the start pause with
candidate 4.23 (bounded key-list materialisation) only against a rewrite-latency
cohort; a missed key or a replay regression rejects it.

### 3.14 Connection setup and idle sockets

- **Redis 8.10.2 and Valkey 9.1.2** use a configurable, startup-immutable
  `tcp-backlog` (default **511**) and `tcp-keepalive` (default **300 s**); an
  accepted socket gets `SO_KEEPALIVE` plus `TCP_KEEPIDLE`/`TCP_KEEPINTVL`/
  `TCP_KEEPCNT` from `anetKeepAlive`
  ([`redis config.c#L3526`](https://github.com/redis/redis/blob/8.10.2/src/config.c#L3526),
  [`redis anet.c#L115`](https://github.com/redis/redis/blob/8.10.2/src/anet.c#L115);
  [`valkey config.c#L3408`](https://github.com/valkey-io/valkey/blob/9.1.2/src/config.c#L3408))
  and `TCP_NODELAY`.
- **Rillcache** uses `std::net::TcpListener::bind` and sets `TCP_NODELAY` on
  accepted sockets. A local `strace` of this build shows `SO_REUSEADDR` and
  `listen(..., 128)`; `std` exposes no safe API for the listen backlog or TCP
  keepalive. The application-level `--idle-timeout-ms` (default 30 s) closes
  idle connections instead.
- Consequence: under a reconnect storm our accept queue is 128 where the C
  servers use 511, and idle NAT/firewall state relies on the app timeout rather
  than TCP keepalive probes. The protected gate keeps eight persistent
  connections and is unaffected.
- **Local attempt (2026-11-12), inconclusive.** Eight Python threads opened
  1,000 short-lived connect/PING/close connections each (SO_LINGER 0), three
  rotations: Rillcache 2,232 conn/s (p50 1.85 ms, p99 30.9), Redis 3,796
  (1.53/7.9), Valkey 2,189 (1.82/26.8). The Python client and high variance bound
  the rate, so no direction is claimed. A kernel-level client is needed to resolve
  the accept path; the source difference (128/no keepalive vs 511/300 s) remains.

**Verdict: keep.** Raising the backlog or enabling keepalive needs a socket
setter outside `std` (a new dependency or FFI) or an OS-level sysctl, and is not
justified without a kernel-level connection-storm or idle-socket cohort; the
Python attempt above is inconclusive. The 30 s app timeout is a different
mechanism, not a keepalive substitute.

### 3.15 Startup replay (time to ready)

Recovery is part of the persistence latency and the steady-state gate never sees
it: a restart must replay the journal before the listener accepts clients.

- **Redis 8.10.2 and Valkey 9.1.2** load their append-only files once with the AOF
  loader (manifest plus base/incremental files), then serve; a separate
  per-command CRC pass is not part of the normal load.
- **Rillcache** calls `Journal::replay`, which first runs `validate_checksums` — a
  full parse and CRC pass over every transaction — and then reparses the file and
  executes every command through the normal `store.execute_command` path
  ([`journal.rs#L944`](../../src/engine/journal.rs#L944),
  [`#L592`](../../src/engine/journal.rs#L592)). A 200k-key journal is therefore
  parsed twice, and each command pays normal dispatch and execution cost before
  the process binds.

Measured (2026-11-12, 64-byte values, `everysec`, clean SIGTERM, time from process
start to first successful PING; DBSIZE verified for Rillcache at 200k and Redis at
50k):

| Keys | Rillcache | Redis 8.10.2 | Valkey 9.1.2 |
| ---: | ---: | ---: | ---: |
| 50,000 | 0.258 s | 0.011 s | 0.054 s |
| 200,000 | **1.10–1.24 s** (median ~1.16) | 0.027–0.043 s | 0.051–0.061 s |

Rillcache is ~20–45x slower to become ready. Absolute times are shared-host
diagnostics, but the ratio is large and the source explains it. With AOF HMAC on,
the 50k replay rises from 0.21 s to 0.75 s (3.6×) because the authentication pass
reads and hashes the journal again; 4.27 must fold HMAC verification into the
single replay pass while preserving the chain.

**Verdict: adapt a single-pass replay fast path (4.27).** Verify the CRC while
replaying instead of in a separate pass, and apply commands through a loader that
skips load-irrelevant side effects (WATCH version churn, notifications,
observability counters, journal re-append) while preserving expiry absolute
times, payload accounting, stream consumer/PEL state, HMAC verification and the
tail/corruption policy. Do not weaken corruption detection or serve partial
state.

### 3.16 Cursor iteration (SCAN/HSCAN/SSCAN/ZSCAN)

- **Valkey/Redis** iterate the hash table incrementally with a reverse-binary
  cursor: each call scans a bounded number of buckets, `COUNT` is a hint, and a
  `maxiterations` loop caps the empty-bucket search. No snapshot, no sort and no
  keyspace-sized allocation; MATCH/TYPE are applied per visited element, and
  listpack/intset-encoded collections return every element in one call with
  cursor 0
  ([`redis db.c#L1835`](https://github.com/redis/redis/blob/8.10.2/src/db.c#L1835),
  [`#L1972`](https://github.com/redis/redis/blob/8.10.2/src/db.c#L1972),
  [`dict.c#L1529`](https://github.com/redis/redis/blob/8.10.2/src/dict.c#L1529),
  [`db.c#L1985`](https://github.com/redis/redis/blob/8.10.2/src/db.c#L1985);
  [`valkey db.c#L1225`](https://github.com/valkey-io/valkey/blob/9.1.2/src/db.c#L1225),
  [`dict.c#L1177`](https://github.com/valkey-io/valkey/blob/9.1.2/src/dict.c#L1177)).
- **Rillcache** treats the cursor as an index into a **freshly collected and
  sorted snapshot on every call**: `scan_keys` collects every key, sorts it, and
  then slices `COUNT`; `scan_collection` deep-copies every field/value or member
  (`field.to_vec(), value.to_vec()`) and sorts it before slicing. SCAN also calls
  `data.reap(now, usize::MAX)` first, so one call can synchronously remove every
  due expiry — work bounded by the keyspace, not by `COUNT`
  ([`scan.rs`](../../src/engine/scan.rs#L112),
  [`scan_collection`](../../src/engine/scan.rs#L144)). The module comment states
  the design: "Cursors are decimal indexes into a freshly sorted snapshot per
  call."
- **Measured (2026-11-12).** A 100,000-key keyspace (`key:%06d`, 16-byte values)
  plus a 10,000-field hash, one connection, 300 fixed SCAN calls with spread
  cursors and `COUNT 10`, three rotations:

  | Path | Rillcache | Redis 8.10.2 | Valkey 9.1.2 |
  | --- | ---: | ---: | ---: |
  | `SCAN` per call, `COUNT 10` | **42.6 ms p50 / 81.3 p99** | 0.256 / 2.08 ms | 0.164 / 3.72 ms |
  | Full `SCAN`, `COUNT 1000` | **4.81 s** (101 calls) | 0.239 s (100) | 0.167 s (100) |
  | Full `HSCAN`, `COUNT 10` (10k fields) | **5.05 s** (1,000 calls) | 0.257 s (~944) | 0.247 s (~800) |

  The PING floor on the same connections is 0.21/0.16/0.10 ms. A single
  default-COUNT call is therefore **167× Redis and 260× Valkey** at p50, and a
  full traversal is 20–29× slower. Extrapolating the measured per-call p50 to
  10,000 calls, a full 100k traversal at `COUNT 10` costs Rillcache ~426 s versus
  ~2.6 s for Redis. The cost is keyspace-sized per call, not per requested
  element. Candidate 4.31.

**Verdict: adapt an incremental cursor (4.31).** Keep the cursor opaque,
resume from the last emitted element in an ordered structure, and bound each call
by `COUNT` plus index depth. The numeric index cursor and the router's
`owner×2^40 + local` packing are an explicit compatibility decision, and the
sorted-order test in `tests/scan.rs` must be updated to the chosen semantics. Do
not keep a per-call full snapshot or an unbounded expiry sweep.

### 3.17 Large-collection drop, deferred free and per-element memory

- **Valkey/Redis** store lists as a quicklist of listpack nodes, so a
  200,000-element list of 8-byte values is a few hundred packed nodes rather than
  200,000 separate allocations (`list-max-listpack-size` default −2, an 8 KiB
  node budget,
  [`redis config.c#L3511`](https://github.com/redis/redis/blob/8.10.2/src/config.c#L3511)).
  `DEL` frees synchronously unless `lazyfree-lazy-user-del` is enabled (Redis
  default 0, Valkey default 1), `UNLINK` always calls `dbAsyncDelete` and queues
  the value to the bio lazy-free thread, and `FLUSHALL/FLUSHDB ASYNC` swaps the
  keyspace structures and queues the old ones to the same thread
  ([`redis db.c#L1437`](https://github.com/redis/redis/blob/8.10.2/src/db.c#L1437),
  [`db.c#L920`](https://github.com/redis/redis/blob/8.10.2/src/db.c#L920),
  [`lazyfree.c#L328`](https://github.com/redis/redis/blob/8.10.2/src/lazyfree.c#L328),
  [`bio.c#L195`](https://github.com/redis/redis/blob/8.10.2/src/bio.c#L195),
  [`config.c#L3413`](https://github.com/redis/redis/blob/8.10.2/src/config.c#L3413);
  [`valkey db.c#L546`](https://github.com/valkey-io/valkey/blob/9.1.2/src/db.c#L546),
  [`lazyfree.c#L209`](https://github.com/valkey-io/valkey/blob/9.1.2/src/lazyfree.c#L209),
  [`config.c#L3281`](https://github.com/valkey-io/valkey/blob/9.1.2/src/config.c#L3281)).
- **Rillcache** implements `UNLINK` in the same arm as `DEL` and performs the
  drop synchronously, and accepts `FLUSHALL`/`FLUSHDB` `ASYNC` but calls
  `data.flush()` on the command thread — the mode flag is parsed and ignored
  ([`keyspace.rs`](../../src/engine/dispatch/keyspace.rs#L27),
  [`administration.rs`](../../src/engine/administration.rs#L14),
  [`data.rs`](../../src/engine/data.rs#L484)). Lists are
  `VecDeque<Vec<u8>>` or a chunked `VecDeque<VecDeque<Vec<u8>>>`, one heap
  buffer per element, so dropping a 200k-element list frees 200k allocations
  ([`data.rs`](../../src/engine/data.rs#L6)).
- **Measured (2026-11-12).** A 200,000-element list of 8-byte values (three
  rotations, large-list p50 over nine samples), a 100-element control, and
  20,000 keys for the flush rows:

  | Path | Rillcache | Redis 8.10.2 | Valkey 9.1.2 |
  | --- | ---: | ---: | ---: |
  | `DEL` 200k list | **7.28 ms** | 0.166 ms | 0.130 ms |
  | `UNLINK` 200k list | **6.21 ms** (max 9.8) | 0.316 (max 3.6) | 0.235 (max 1.9) |
  | `UNLINK` 100-element list | 0.28 ms | 0.13 ms | 0.12 ms |
  | `FLUSHALL ASYNC` 20k keys | **9.20 ms** (max 17.4) | 0.886 (max 6.8) | 0.568 (max 5.5) |
  | `FLUSHALL` 20k keys | **10.12 ms** | 3.44 ms | 1.52 ms |

  RSS for one 200k-element list (8-byte values, `/proc/<pid>/status` before and
  after preload): Rill **+27.8–28.0 MB (142–143 B/element)**, Redis +1.47 MB
  (7.5 B), Valkey +1.48–1.54 MB (7.5–7.9 B) — **~19×** the C servers.

  The drop gap is the per-element allocation, not only the async flag: Redis's
  default `DEL` is synchronous and still 44× faster, while Rill's `ASYNC` is as
  slow as `SYNC`. The 100-element control is at the command floor, so both gaps
  scale with element count. Redis/Valkey `ASYNC` saves 2.7–3.9× on flush; Rill
  saves nothing. Candidates 4.32 (packed nodes) and 4.33 (deferred drop).

**Verdict: adapt both, in this order.** 4.33 is a small, contract-visible fix:
move the dropped value/keyspace to a bounded pending-free queue drained by the
maintenance tick, count it in memory accounting, and drain on shutdown. 4.32 is
the structural fix: pack short list elements into byte-arena nodes so a drop
frees hundreds of buffers instead of hundreds of thousands and RSS falls toward
the C servers. Keep `DEL` and `FLUSH* SYNC` synchronous, matching the Redis
default; do not weaken accounting, notifications, journal ordering or WATCH.

### 3.18 In-place string mutation (APPEND/SETRANGE)

- **Redis 8.10.2 and Valkey 9.1.2** store a string in an sds with spare capacity:
  `APPEND` calls `sdsMakeRoomFor` (greedy doubling, amortized) and copies only the
  appended bytes into the existing buffer, and `SETRANGE` writes the replacement
  in place, growing the sds only when the offset is past the end
  ([`redis t_string.c#L1369`](https://github.com/redis/redis/blob/8.10.2/src/t_string.c#L1369),
  [`#L580`](https://github.com/redis/redis/blob/8.10.2/src/t_string.c#L580),
  [`redis sds.c#L341`](https://github.com/redis/redis/blob/8.10.2/src/sds.c#L341);
  [`valkey t_string.c#L791`](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_string.c#L791),
  [`#L432`](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_string.c#L432)).
  The value is unshared once, then mutated under the entry's own buffer.
- **Rillcache** rebuilds the entire value for both commands: `append` allocates a
  new `Vec` of `old length + suffix`, copies the old value, appends, and then
  `data.put` copies the new buffer into the entry again; `setrange` also allocates
  a full-length `Vec`, copies the old value, resizes, patches and copies back.
  Two full-value copies per call, so cost is O(value length) rather than
  O(appended/replaced bytes)
  ([`string_ranges.rs`](../../src/engine/string_ranges.rs),
  [`data.rs`](../../src/engine/data.rs#L774)).
- **Measured (2026-11-12).** One connection, 200 repetitions at 1 MB and 100 at
  32 MB (built by 512 KiB `APPEND`s), three rotations:

  | Command | Rillcache | Redis 8.10.2 | Valkey 9.1.2 |
  | --- | ---: | ---: | ---: |
  | `APPEND` 64 B to 1 MB value | 0.344 ms | 0.122 ms | 0.198 ms |
  | `SETRANGE` 16 B at offset 0, 1 MB value | 0.208 ms | 0.181 ms | 0.210 ms |
  | `APPEND` 64 B to 32 MB value | **7.35 ms** (p99 14.5) | 0.137 (p99 1.05) | 0.133 (p99 0.96) |
  | `SETRANGE` 16 B at offset 0, 32 MB value | **7.19 ms** (p99 9.5) | 0.184 (p99 1.84) | 0.096 (p99 0.44) |
  | `APPEND` 8 B to 64 B value | 0.080 ms | 0.120 ms | 0.168 ms |
  | `SETRANGE` 8 B, 64 B value | 0.135 ms | 0.167 ms | 0.137 ms |
  | `GETRANGE 0 15` from 32 MB value | 0.118 ms | 0.195 ms | 0.243 ms |

  The 32 MB row is **54× Redis / 55× Valkey** for `APPEND` and **39× / 75×** for
  `SETRANGE`, while the small-value controls and the read-only `GETRANGE` are at
  or below the C servers' command floor. The 7.2–7.35 ms matches two 32 MB copies
  at roughly 9 GB/s, and the 1 MB row is the same mechanism masked by the
  ~0.1–0.2 ms client floor. Candidate 4.34.
- **Post-fix validation (2026-11-13, PR #258 `d334cde`).** The landed
  `Data::mutate_string` path reserves first and then mutates the entry's own
  `Vec`; the same 1 MB/32 MB cohort now measures:

  | Command | Rillcache pre-fix | Rillcache post-fix | Redis | Valkey |
  | --- | ---: | ---: | ---: | ---: |
  | `APPEND` 64 B to 1 MB | 0.344 ms | 0.124 ms | 0.238 | 0.208 |
  | `SETRANGE` 16 B, 1 MB | 0.208 | 0.217 | 0.146 | 0.068 |
  | `APPEND` 64 B to 32 MB | **7.35** (p99 14.5) | **0.131** (p99 1.22) | 0.142 (p99 1.21) | 0.219 (p99 2.24) |
  | `SETRANGE` 16 B, 32 MB | **7.19** (p99 9.5) | **0.185** (p99 1.32) | 0.222 (p99 1.69) | 0.289 (p99 3.31) |
  | 64 B controls | 0.080/0.135 | 0.189/0.188 | 0.234/0.242 | 0.165/0.071 |

  The 32 MB mutations are **56×** and **39×** faster than the pre-fix baseline
  and now at or below the C servers' p50; the 1 MB rows, `GETRANGE` and the 64 B
  controls remain at the command floor. AC36 is met; candidate 4.34 is
  implemented.

**Verdict: adapt in-place mutation (4.34).** Add a mutable string-update path that
reserves/admits first, then grows and patches the entry's own `Vec` in place
(`extend_from_slice`, `resize`, slice copy) and updates payload accounting and
`touch` without rebuilding or re-copying the value. Keep the exact bytes, the
zero-fill growth rule, the 512 MiB limit, TTL, WATCH/version, journal and
rewrite behavior. **Implemented in PR #258 (`d334cde`); the post-fix cohort is
the regression evidence.**

### 3.19 In-place collection mutation (HSET/ZADD/SADD/LPUSH/LPOP)

- **Redis 8.10.2 and Valkey 9.1.2** mutate the existing encoding in place:
  `hashTypeSet` updates the listpack or dict entry, `zsetAdd` updates the dict
  entry plus its skiplist node, `setTypeAdd` inserts into the intset/listpack/dict,
  and `pushGenericCommand`/`popGenericCommand` push or pop one quicklist element
  ([`redis t_hash.c#L1899`](https://github.com/redis/redis/blob/8.10.2/src/t_hash.c#L1899),
  [`t_zset.c#L1601`](https://github.com/redis/redis/blob/8.10.2/src/t_zset.c#L1601),
  [`t_set.c#L126`](https://github.com/redis/redis/blob/8.10.2/src/t_set.c#L126),
  [`t_list.c#L484`](https://github.com/redis/redis/blob/8.10.2/src/t_list.c#L484),
  [`#L822`](https://github.com/redis/redis/blob/8.10.2/src/t_list.c#L822);
  [`valkey t_hash.c#L368`](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_hash.c#L368),
  [`t_zset.c#L1482`](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_zset.c#L1482),
  [`t_set.c#L117`](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_set.c#L117),
  [`t_list.c#L464`](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_list.c#L464),
  [`#L757`](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_list.c#L757)).
- **Rillcache** rebuilds the whole collection per write: `hash_set` clones every
  field and value into a new `HashMap` and replaces the entry; `zset_add` clones
  every member into a new map; `set_add` clones and sorts every member into a new
  `BTreeSet`; `push_list`/`pop_list` clone the entire list via `list_values`,
  re-encode it and replace it. Each path also recomputes `bytes`/`payload_len`
  over every element, and `put_collection` drops the old collection after the
  replacement ([`collections.rs`](../../src/engine/collections.rs#L258),
  [`#L371`](../../src/engine/collections.rs#L371),
  [`#L161`](../../src/engine/collections.rs#L161),
  [`#L540`](../../src/engine/collections.rs#L540),
  [`#L589`](../../src/engine/collections.rs#L589),
  [`data.rs`](../../src/engine/data.rs#L833)).
- **Measured (2026-11-12).** One connection, 100 repetitions per row, three
  rotations; big collections are a 10,000-field hash, a 10,000-member zset, a
  10,000-member set and a 200,000-element list; controls are 10 elements:

  | Command | Rillcache | Redis 8.10.2 | Valkey 9.1.2 |
  | --- | ---: | ---: | ---: |
  | `HSET` new field into 10k hash | **6.45 ms** | 0.111 ms | 0.093 ms |
  | `HSET` control (10 fields) | 0.179 | 0.092 | 0.088 |
  | `ZADD` new member into 10k zset | **4.21 ms** | 0.223 | 0.084 |
  | `ZADD` control | 0.232 | 0.093 | 0.369 |
  | `SADD` new member into 10k set | **1.23 ms** | 0.083 | 0.351 |
  | `SADD` control | 0.113 | 0.085 | 0.095 |
  | `RPUSH` one element onto 200k list | **18.34 ms** | 0.084 | 0.070 |
  | `RPUSH` control | 0.175 | 0.075 | 0.276 |
  | `LPOP` one element from 200k list | **17.82 ms** | 0.343 | 0.069 |
  | `LPOP` control | 0.224 | 0.078 | 0.080 |

  The gaps are **58–69×** (`HSET`), **19–50×** (`ZADD`), **3.5–15×** (`SADD`) and
  **52–262×** (`RPUSH`/`LPOP`), while every 10-element control is at the
  command floor. The 200k-list write is ~18 ms, about 90 ns per element — a
  clone-plus-free per element, or roughly 55 writes/s. Candidate 4.35.
- **100k extension (2026-11-12).** A 100,000-member hash, zset and set, 20
  repetitions, three rotations; 10-member controls:

  | Command | Rillcache | Redis 8.10.2 | Valkey 9.1.2 |
  | --- | ---: | ---: | ---: |
  | `HSET` new field | **92.7 ms** | 0.12 ms | 0.32 ms |
  | `HSET` existing field | **92.8** | 0.21 | 0.08 |
  | `HDEL` | **99.4** | 0.15 | 0.15 |
  | `ZADD` new member | **61.4** | 0.14 | 0.23 |
  | `ZINCRBY` | **57.3** | 0.21 | 0.26 |
  | `ZREM` | **63.9** | 0.10 | 0.13 |
  | `SADD` new member | **11.2** | 0.60 | 0.25 |
  | `SREM` | **10.7** | 0.20 | 0.08 |
  | 10-member controls | 0.23/0.15/0.19 | 0.11/0.20/0.19 | 0.13/0.12/0.20 |

  The write gaps grow with cardinality: **18–1160×** at 100k versus 3.5–69× at
  10k, while the controls stay at the floor. Updates and removals pay the same
  full clone/sort/replace as adds. This extends 4.35's admission evidence and
  tightens its targets to 100k.

**Verdict: adapt in-place mutation (4.35).** Reserve/admit first, then mutate the
existing collection under `entries.get_mut`: `HashMap::insert`/`remove` (hash,
zset), `BTreeSet::insert`/`remove` (set), the list's head/tail chunk (push/pop),
and update payload accounting by the delta instead of re-summing the collection.
Convert between pack and large encodings only when a threshold is crossed; empty
collections are removed as today. Keep field TTL bookkeeping, blocking wake,
WATCH/version, journal and rewrite semantics.

### 3.20 Random-member selection (SRANDMEMBER/SPOP/HRANDFIELD/ZRANDMEMBER)

- **Redis 8.10.2 and Valkey 9.1.2** answer `SRANDMEMBER`, `SPOP`, `HRANDFIELD`
  and `ZRANDMEMBER` from the hash table: `dictGetRandomKey`/`dictGetSomeKeys`
  picks a random non-empty bucket (or samples one), with intset/listpack paths
  for small collections, and `SPOP` removes one dict entry
  ([`redis t_set.c#L1065`](https://github.com/redis/redis/blob/8.10.2/src/t_set.c#L1065),
  [`#L838`](https://github.com/redis/redis/blob/8.10.2/src/t_set.c#L838),
  [`#L1130`](https://github.com/redis/redis/blob/8.10.2/src/t_set.c#L1130),
  [`t_hash.c#L5858`](https://github.com/redis/redis/blob/8.10.2/src/t_hash.c#L5858),
  [`t_zset.c#L4720`](https://github.com/redis/redis/blob/8.10.2/src/t_zset.c#L4720),
  [`dict.c#L1230`](https://github.com/redis/redis/blob/8.10.2/src/dict.c#L1230);
  [`valkey t_set.c#L953`](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_set.c#L953),
  [`#L749`](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_set.c#L749),
  [`#L1005`](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_set.c#L1005),
  [`t_hash.c#L2368`](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_hash.c#L2368),
  [`t_zset.c#L4345`](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_zset.c#L4345)).
- **Rillcache** materializes the collection first: `set_random_member` calls
  `owned_set`, which clones and sorts every member through `set_items()`;
  `hash_random_field` deep-copies every field and value; `zset_random_member`
  deep-copies every member; `set_pop` clones the set, removes via
  `iter().nth(index)`, then re-encodes and replaces it. Each command therefore
  pays O(n) time and O(n) allocation per call
  ([`collection_commands.rs`](../../src/engine/collection_commands.rs#L3398),
  [`#L3319`](../../src/engine/collection_commands.rs#L3319),
  [`#L1277`](../../src/engine/collection_commands.rs#L1277),
  [`#L4635`](../../src/engine/collection_commands.rs#L4635)).
- **Measured (2026-11-12).** One connection, 100 repetitions at 10k and 50 at
  100k, three rotations; controls are 10-element collections:

  | Command | Rillcache 10k | Redis | Valkey | Rillcache 100k | Redis | Valkey |
  | --- | ---: | ---: | ---: | ---: | ---: | ---: |
  | `SRANDMEMBER` | 1.63 ms | 0.221 | 0.223 | **11.54 ms** | 0.199 | 0.326 |
  | `SRANDMEMBER 10` | 1.39 | 0.324 | 0.304 | **11.36** | 0.295 | 0.393 |
  | `SPOP` | 1.63 | 0.333 | 0.219 | **11.11** | 0.304 | 0.126 |
  | `HRANDFIELD` | 2.56 | 0.299 | 0.176 | **33.66** | 0.343 | 0.113 |
  | `HRANDFIELD 10` | 2.09 | 0.094 | 0.272 | **32.64** | 0.331 | 0.368 |
  | `ZRANDMEMBER` | 1.32 | 0.094 | 0.244 | **21.32** | 0.250 | 0.164 |
  | `ZRANDMEMBER 10` | 1.03 | 0.124 | 0.378 | **21.57** | 0.296 | 0.160 |
  | 10-element controls | 0.13–0.31 | 0.09–0.17 | 0.19–0.34 | 0.19–0.25 | 0.17–0.29 | 0.07–0.09 |

  At 100k the gaps are **35–58×** (`SRANDMEMBER`), **85–130×** (`ZRANDMEMBER`),
  **37–88×** (`SPOP`) and **98–298×** (`HRANDFIELD`), and every 10-element
  control is at the command floor. Rill's 10k→100k ratio is 7–16× (cost grows
  with cardinality) while the C servers stay flat. Candidate 4.36.

**Verdict: adapt constant-time random selection (4.36).** A first slice removes
the clone/sort and samples by reservoir over iteration (O(n) time, O(count)
memory), which removes the allocation and both copies. The full fix adds a dense
member index (a `Vec` plus a position map with swap-remove) beside the large
set/hash/zset, making single picks and `SPOP` O(1) like the C servers' dict
random; `SMEMBERS`/`HKEYS`/`ZSCAN` order is unspecified in Redis and becomes an
explicit ordering decision. Keep small pack paths and all reply semantics.

### 3.21 Bitmap mutation and popcount (SETBIT/BITCOUNT)

- **Redis 8.10.2 and Valkey 9.1.2** follow sds semantics: `SETBIT` writes one bit
  into the existing sds (growing it only when the offset passes the end), and
  `BITCOUNT` calls a word-at-a-time popcount (`redisPopcount` uses `popcnt` on
  64-bit words; Valkey has scalar, AVX2 and NEON variants) over the byte range,
  applying masks only at the edges
  ([`redis bitops.c#L849`](https://github.com/redis/redis/blob/8.10.2/src/bitops.c#L849),
  [`#L1623`](https://github.com/redis/redis/blob/8.10.2/src/bitops.c#L1623),
  [`#L67`](https://github.com/redis/redis/blob/8.10.2/src/bitops.c#L67);
  [`valkey bitops.c#L689`](https://github.com/valkey-io/valkey/blob/9.1.2/src/bitops.c#L689),
  [`#L944`](https://github.com/valkey-io/valkey/blob/9.1.2/src/bitops.c#L944),
  [`#L58`](https://github.com/valkey-io/valkey/blob/9.1.2/src/bitops.c#L58),
  [`#L141`](https://github.com/valkey-io/valkey/blob/9.1.2/src/bitops.c#L141)).
- **Rillcache** rebuilds the whole value for `SETBIT`: a new `Vec` of the new
  length, copy of the old value, resize, one-byte patch and a second copy through
  `data.put`. `BITCOUNT` sums `count_ones()` one byte at a time, with no word or
  slice specialization
  ([`string_algorithms.rs`](../../src/engine/string_algorithms.rs#L89),
  [`#L457`](../../src/engine/string_algorithms.rs#L457)).
- **Measured (2026-11-12).** One 1 MB bitmap and one 16 MB bitmap of `0xff`
  bytes (the 16 MB one built by 512 KiB `APPEND`s); 100 repetitions at 1 MB and
  30 at 16 MB, three rotations:

  | Command | Rill 1 MB | Redis | Valkey | Rill 16 MB | Redis | Valkey |
  | --- | ---: | ---: | ---: | ---: | ---: | ---: |
  | `SETBIT` offset 0 | 0.331 ms | 0.092 | 0.172 | **5.32 ms** | 0.349 | 0.312 |
  | `GETBIT` offset 0 | 0.138 | 0.103 | 0.145 | 0.260 | 0.101 | 0.215 |
  | `BITCOUNT` whole value | 1.85 | 0.259 | 0.305 | **23.01 ms** | 3.37 | 4.56 |
  | `STRLEN` | 0.181 | 0.213 | 0.173 | 0.298 | 0.217 | 0.310 |
  | 64 B `SETBIT` | 0.163 | 0.154 | 0.099 | 0.244 | 0.334 | 0.140 |
  | 64 B `BITCOUNT` | 0.112 | 0.163 | 0.197 | 0.165 | 0.410 | 0.153 |

  At 16 MB, `SETBIT` is **15–17×** slower and `BITCOUNT` **5–7×** slower, while
  `GETBIT`, `STRLEN` and both 64 B controls are at the command floor. Rill's
  `BITCOUNT` runs at ~0.7 GB/s (23 ms for 16 MB) against the C servers' ~5 GB/s.
  Candidate 4.37.

**Verdict: adapt both, with the string fix from 4.34.** `SETBIT` should reserve
and then patch the entry's own `Vec` in place (extend and zero-fill only when the
offset passes the end), exactly the 4.34 shape; `BITCOUNT` should consume
`chunks_exact(8)` as little-endian `u64` with `count_ones()` plus masked
head/tail bytes, which stays safe Rust and needs no SIMD dependency. Keep the
BYTE/BIT range rules, offset limits, zero-fill, TTL, WATCH/journal and exact
bit numbering.

### 3.22 Sorted-set rank, score-range and point reads

- **Redis 8.10.2 and Valkey 9.1.2** answer rank and score queries from the
  skiplist and member dict: `ZSCORE` is a dict lookup, `ZRANK` uses the
  skiplist's span counts (`zslGetRank`, O(log n)), `ZRANGEBYSCORE` seeks the
  first node in range and walks `LIMIT` elements, and `ZRANGE start stop` seeks
  the start node by rank
  ([`redis t_zset.c#L645`](https://github.com/redis/redis/blob/8.10.2/src/t_zset.c#L645),
  [`#L1539`](https://github.com/redis/redis/blob/8.10.2/src/t_zset.c#L1539),
  [`#L3413`](https://github.com/redis/redis/blob/8.10.2/src/t_zset.c#L3413),
  [`#L4056`](https://github.com/redis/redis/blob/8.10.2/src/t_zset.c#L4056);
  [`valkey t_zset.c#L585`](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_zset.c#L585),
  [`#L1420`](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_zset.c#L1420),
  [`#L3099`](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_zset.c#L3099)).
- **Rillcache** materializes and sorts for every one of these reads:
  `zset_rank` calls `zset_items()` then `ordered_zset`, which copies every member
  into a `Vec<(Vec<u8>, f64)>` and sorts it before a linear `position` scan;
  `zset_range_by_score` does the same, retains the score range and then skips
  `offset` elements; the `ZRANGE` path in `zset_read` sorts all members before
  slicing; and `ZSCORE` builds the same reference vector and linearly scans it.
  At 100k that is 100,000 member copies and an O(n log n) sort per read
  ([`collection_commands.rs`](../../src/engine/collection_commands.rs#L3744),
  [`#L3896`](../../src/engine/collection_commands.rs#L3896),
  [`#L1228`](../../src/engine/collection_commands.rs#L1228),
  [`collections.rs`](../../src/engine/collections.rs#L433)).
- **Measured (2026-11-12).** A 10,000- and a 100,000-member sorted set
  (7-byte members, scores 100–899), one connection, 50/20 repetitions, three
  rotations; controls are 10 members:

  | Command | Rill 10k | Redis | Valkey | Rill 100k | Redis | Valkey |
  | --- | ---: | ---: | ---: | ---: | ---: | ---: |
  | `ZRANK` middle | 2.43 ms | 0.627 | 0.264 | **58.6 ms** | 0.155 | 0.073 |
  | `ZRANK` last | 2.44 | 0.270 | 0.267 | **72.5** | 0.126 | 0.080 |
  | `ZREVRANK` middle | 2.71 | 0.073 | 0.196 | **71.4** | 0.098 | 0.077 |
  | `ZSCORE` | 0.293 | 0.396 | 0.283 | **3.27** | 0.102 | 0.066 |
  | `ZRANGEBYSCORE -inf +inf LIMIT 0 10` | 2.93 | 0.200 | 0.224 | **62.4** | 0.080 | 0.116 |
  | `ZRANGE 50000 50009` | 2.97 | 0.155 | 0.222 | **63.7** | 0.090 | 0.167 |
  | `ZRANGE 0 9` | 3.08 | 0.159 | 0.206 | **57.4** | 0.103 | 0.073 |
  | 10-element controls | 0.24/0.14 | 0.18/0.07 | 0.13/0.14 | 0.28/0.11 | 0.10/0.07 | 0.14/0.13 |

  At 100k the rank and range reads are **378–927×** slower than Redis/Valkey and
  `ZSCORE` is 32–50×; at 10k the same commands are 2.4–3.1 ms. Rill grows
  19–24× for 10× the cardinality while the C servers stay flat, and the
  10-element controls are at the command floor. This extends 4.30 and admits
  4.38 for the rank paths that a plain ordered set cannot answer.

**Verdict: split the fix.** A `BTreeSet<(ScoreKey, member)>` plus a direct map
lookup fixes `ZSCORE`, `ZRANGE` from 0 and `ZRANGEBYSCORE` (`range()` is
O(log n + k)), which is 4.30. It does **not** give rank-select: `ZRANK`,
`ZREVRANK` and a deep `ZRANGE start` would still iterate from the first element,
so 4.38 adds an order-statistic rank index (bucketed sorted blocks with cached
counts, or an in-repo weight-balanced tree) and must keep the 4.30 score order.
Both must preserve `(score, member)` ordering, WITHSCORES formatting, LIMIT and
inclusive/exclusive bounds, and the small `ZSetPack` path.

### 3.23 Pub/sub fan-out: burst delivery and channel scale

- **Redis 8.10.2 and Valkey 9.1.2** keep subscribed channels in a dict and append
  the message to each subscriber's client output buffer. A slow subscriber is
  bounded by `client-output-buffer-limit pubsub` (32 MB soft / 8 MB hard / 60 s
  by default): when it exceeds the limit the **client is disconnected**, not
  silently unsubscribed, and the publisher's write loop drains up to 64 KiB per
  event
  ([`redis pubsub.c#L469`](https://github.com/redis/redis/blob/8.10.2/src/pubsub.c#L469),
  [`#L93`](https://github.com/redis/redis/blob/8.10.2/src/pubsub.c#L93),
  [`config.c#L3586`](https://github.com/redis/redis/blob/8.10.2/src/config.c#L3586),
  [`networking.c#L2887`](https://github.com/redis/redis/blob/8.10.2/src/networking.c#L2887);
  [`valkey pubsub.c#L508`](https://github.com/valkey-io/valkey/blob/9.1.2/src/pubsub.c#L508),
  [`config.c#L3518`](https://github.com/valkey-io/valkey/blob/9.1.2/src/config.c#L3518)).
- **Rillcache** gives every subscriber a bounded `sync_channel(1024)`, builds a
  fresh RESP frame per subscriber, and `try_send`s it; on a full queue the
  subscriber is **removed from the channel table** (`retain` returns false), so
  the publisher's subsequent messages report zero receivers and the subscription
  silently disappears. The publisher finds a channel by scanning every entry and
  then runs a full `retain` over the map on every `PUBLISH`. Subscribers drain at
  most 64 frames per 1 ms tick
  ([`pubsub.rs`](../../src/engine/pubsub.rs#L121),
  [`pool.rs`](../../src/server/pool.rs#L159)).
- **Measured (2026-11-12).** One subscriber on `burst` with a reader thread, then
  a single pipeline of 2,000 `PUBLISH` commands (32-byte payload), three
  rotations; plus one subscriber subscribed to 1 or 10,000 channels and 500/200
  timed `PUBLISH` calls:

  | Observation | Rillcache | Redis 8.10.2 | Valkey 9.1.2 |
  | --- | ---: | ---: | ---: |
  | Burst delivered (of 2,000) | **1,024** (all 3 rounds) | 2,000 | 2,000 |
  | Sum of `PUBLISH` receiver counts | **1,024** | 2,000 | 2,000 |
  | Subscription present after burst (`PUBSUB NUMSUB`) | **0** | 1 | 1 |
  | `PUBLISH` p50, 1 channel | 0.12–0.22 ms | 0.13–0.27 | 0.20–0.30 |
  | `PUBLISH` p50, 1,000 channels | 0.13 ms | 0.16–0.23 | 0.21–0.30 |
  | `PUBLISH` p50, 10,000 channels | **0.38–0.89 ms** | 0.14–0.21 | 0.12–0.31 |

  A 2,000-message burst loses **976 messages (48.8%)** and removes the
  subscription, while the C servers deliver every message and keep it. At 10,000
  channels Rill is 3–6× slower because of the linear channel scan; at 1,000
  channels the scan is still below the floor. Candidates 4.39 (contract and
  drain) and the channel lookup are the same fix area as 4.24.

**Verdict: adapt contract-correct fan-out (4.39).** Never silently unsubscribe:
bound the queue in **bytes** (Redis's pub/sub default is 32 MB), either drop with
a documented counter while keeping the subscription or disconnect the client with
an error, and drain by output bytes rather than 64 frames. Look the channel up by
key, encode one frame per (RESP version, pattern) and share it, and keep pattern
and shard semantics. This extends 4.24's event-driven wake with a burst-safety
contract.

### 3.24 Cardinality and membership commands (SCARD/SISMEMBER/HLEN/ZCARD/LLEN)

- **Redis 8.10.2 and Valkey 9.1.2** answer cardinality and membership from the
  encoding: list length is cached on the quicklist, `SCARD` is `dictSize` or the
  intset/listpack length, `SISMEMBER` is a dict/intset/listpack probe, `HLEN` is
  `hashTypeLength`, and `ZCARD` is the dict size
  ([`redis t_set.c#L821`](https://github.com/redis/redis/blob/8.10.2/src/t_set.c#L821),
  [`#L783`](https://github.com/redis/redis/blob/8.10.2/src/t_set.c#L783),
  [`t_hash.c#L5294`](https://github.com/redis/redis/blob/8.10.2/src/t_hash.c#L5294),
  [`t_zset.c#L4082`](https://github.com/redis/redis/blob/8.10.2/src/t_zset.c#L4082),
  [`t_list.c#L614`](https://github.com/redis/redis/blob/8.10.2/src/t_list.c#L614),
  [`quicklist.c`](https://github.com/redis/redis/blob/8.10.2/src/quicklist.c);
  [`valkey t_set.c#L733`](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_set.c#L733),
  [`#L703`](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_set.c#L703),
  [`t_hash.c#L1220`](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_hash.c#L1220),
  [`t_zset.c#L3753`](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_zset.c#L3753),
  [`t_list.c#L564`](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_list.c#L564)).
- **Rillcache** routes every one of these through a full materialization:
  `SCARD` and `SISMEMBER` call `set_items()`, which **clones and sorts every
  member** and then takes `.len()` or scans it; `HLEN` and `ZCARD` collect every
  reference through `hash_items()`/`zset_items()` just to take `.len()`; `LLEN`
  collects every list element reference through `list_items()` just to take
  `.len()`. `HEXISTS` and `STRLEN` use the direct paths and are fast
  ([`collections.rs`](../../src/engine/collections.rs#L240),
  [`#L355`](../../src/engine/collections.rs#L355),
  [`#L478`](../../src/engine/collections.rs#L478),
  [`#L658`](../../src/engine/collections.rs#L658)).
- **Measured (2026-11-12).** A 100,000-member set/hash/zset and a 200,000-element
  list, 100 repetitions, three rotations; controls are 10-element collections:

  | Command | Rillcache | Redis 8.10.2 | Valkey 9.1.2 |
  | --- | ---: | ---: | ---: |
  | `SISMEMBER` hit | **8.04 ms** | 0.189 ms | 0.221 ms |
  | `SISMEMBER` miss | **8.67** | 0.167 | 0.214 |
  | `SCARD` | **8.08** | 0.195 | 0.148 |
  | `HLEN` | **2.24** | 0.147 | 0.191 |
  | `ZCARD` | **1.57** | 0.117 | 0.157 |
  | `LLEN` 200k | **1.55** | 0.199 | 0.135 |
  | `HEXISTS` (control) | 0.184 | 0.290 | 0.142 |
  | `STRLEN` (control) | 0.128 | 0.158 | 0.162 |
  | 10-element controls | 0.12–0.18 | 0.07–0.18 | 0.06–0.17 |

  `SISMEMBER` and `SCARD` are **36–55×** slower, `HLEN`/`ZCARD` 10–15×, and
  `LLEN` 8–11×, while the direct `HEXISTS`/`STRLEN` controls and every small
  collection are at the command floor. Candidate 4.40.

**Verdict: adapt O(1) cardinality and membership (4.40).** Add `Collection::len`
(list = sum of node lengths, others = container length) and
`Collection::contains_member`/`contains_field` (BTreeSet/`HashMap`/intset-aware
probe) and route `SCARD`/`SISMEMBER`/`HLEN`/`ZCARD`/`LLEN` through them. Keep
`SMEMBERS`/`HKEYS`/`HGETALL` sorting and full-reply paths unchanged, and preserve
the `SetInt`/`SetPack`/`HashPack` semantics including non-integer probes.

### 3.25 List random access (LINDEX/LPOS)

- **Redis 8.10.2 and Valkey 9.1.2** seek the quicklist by node: `LINDEX` walks
  nodes using cached per-node counts and touches one listpack, and `LPOS` scans
  forward from the requested rank and stops at the first match (or at `MAXLEN`),
  without materializing the list
  ([`redis t_list.c#L621`](https://github.com/redis/redis/blob/8.10.2/src/t_list.c#L621),
  [`#L1015`](https://github.com/redis/redis/blob/8.10.2/src/t_list.c#L1015),
  [`quicklist.c`](https://github.com/redis/redis/blob/8.10.2/src/quicklist.c);
  [`valkey t_list.c#L571`](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_list.c#L571),
  [`#L937`](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_list.c#L937)).
- **Rillcache** calls `list_items()` for both: it flattens every element
  reference of the `ListPack`/`ListQuick` into a `Vec`, O(n) time and O(n)
  allocation, before indexing for `LINDEX` or scanning for `LPOS`
  ([`collection_commands.rs`](../../src/engine/collection_commands.rs#L2668),
  [`#L2321`](../../src/engine/collection_commands.rs#L2321)).
- **Measured (2026-11-12).** A 200,000-element list of 8-byte values, 100
  repetitions, three rotations; controls are a 10-element list:

  | Command | Rillcache | Redis 8.10.2 | Valkey 9.1.2 |
  | --- | ---: | ---: | ---: |
  | `LINDEX 0` | **1.47 ms** | 0.107 ms | 0.068 ms |
  | `LINDEX 100000` | **1.40** | 0.104 | 0.081 |
  | `LINDEX -1` | **1.29** | 0.102 | 0.142 |
  | `LPOS` first element | **1.26** | 0.095 | 0.127 |
  | `LPOS` missing | 1.81 | 5.02 | 5.85 |
  | `LLEN` (admitted as 4.40) | 1.36 | 0.138 | 0.117 |
  | 10-element `LINDEX`/`LPOS` | 0.10/0.10 | 0.09/0.11 | 0.12/0.12 |

  `LINDEX` is **9–22×** slower and a hit-first `LPOS` 10–13×, while a missing
  `LPOS` **leads** (1.81 vs 5.02/5.85) because Redis walks every node. The
  control is at the floor. `LLEN` is the same `list_items()` pattern and is
  covered by 4.40; `LINDEX`/`LPOS` need in-place seeking. Candidate 4.41.

**Verdict: adapt in-place list seeking (4.41).** Add `Collection::list_get`
(normalize the index, walk `ListQuick` chunks by their lengths without
allocating) and `Collection::list_position` (scan chunks in order with
`RANK`/`COUNT`/`MAXLEN` and an early stop). Keep `LRANGE`, `LLEN` (4.40) and all
reply formats unchanged.

### 3.26 Stream append (XADD)

- **Redis 8.10.2 and Valkey 9.1.2** append one entry to the stream's
  rax/listpack representation and update the length and memory accounting
  incrementally; trimming removes entries from the same structure
  ([`redis t_stream.c#L505`](https://github.com/redis/redis/blob/8.10.2/src/t_stream.c#L505),
  [`#L2621`](https://github.com/redis/redis/blob/8.10.2/src/t_stream.c#L2621);
  [`valkey t_stream.c#L425`](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_stream.c#L425),
  [`#L2041`](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_stream.c#L2041)).
- **Rillcache** clones the **entire `Stream`** on every `XADD`
  (`e.stream.clone()`, deep-cloning every entry, field and value), inserts the
  new entry, folds `payload_bytes()` over every entry, then `put_stream` folds
  `payload_bytes()` **again** before replacing the entry and dropping the old
  stream. Reads are efficient: `XLEN` is the map length and `XRANGE`/
  `XREVRANGE` use a `BTreeMap` range with `limit`
  ([`streams.rs`](../../src/engine/streams.rs#L310),
  [`#L818`](../../src/engine/streams.rs#L818),
  [`data.rs`](../../src/engine/data.rs#L182),
  [`#L877`](../../src/engine/data.rs#L877)).
- **Measured (2026-11-12).** A 1,000- and a 5,000-entry stream of one 8-byte
  field pair, 30/50 repetitions, three rotations; a 10-entry stream is the
  control:

  | Command | Rillcache | Redis 8.10.2 | Valkey 9.1.2 |
  | --- | ---: | ---: | ---: |
  | `XADD` at 1k entries | 0.263 ms | 0.148 ms | 0.077 ms |
  | `XADD` at 5k entries | **1.36 ms** (p99 11.6) | 0.104 (p99 1.87) | 0.077 (p99 1.58) |
  | `XLEN` | 0.072 | 0.156 | 0.103 |
  | `XRANGE - + COUNT 10` | 0.121 | 0.099 | 0.187 |
  | `XRANGE mid + COUNT 10` | 0.120 | 0.307 | 0.288 |
  | `XREVRANGE + - COUNT 10` | 0.118 | 0.117 | 0.136 |
  | `XADD`/`XRANGE` on 10 entries | 0.077/0.118 | 0.078/0.103 | 0.231/0.143 |

  `XADD` is **13–18×** slower at 5k and grows 5.2× for 5× the entries
  (0.263 → 1.36 ms) while the C servers stay flat; every read and the 10-entry
  control are at the command floor. Candidate 4.42.

**Verdict: adapt in-place append (4.42).** Admit the entry, then borrow
`Entry.stream` mutably and insert one `StreamEntry`, updating `payload_len` and
the stream's length by the delta instead of cloning the whole stream and folding
it twice. Keep IDs/groups/PEL, trimming, TTL/WATCH/journal/rewrite and the
read paths unchanged.

### 3.27 Geospatial search and point lookups (GEOSEARCH/GEOPOS/GEODIST)

- **Redis 8.10.2 and Valkey 9.1.2** keep the geo index as a sorted set of 52-bit
  geohash scores. A search computes the bounding geohash cells (nine neighbours
  for a radius/box), reads only those score ranges through the zset index and
  tests the candidates; `GEOPOS`/`GEODIST` look the member up in the zset dict
  ([`redis geo.c#L524`](https://github.com/redis/redis/blob/8.10.2/src/geo.c#L524),
  [`#L358`](https://github.com/redis/redis/blob/8.10.2/src/geo.c#L358),
  [`#L418`](https://github.com/redis/redis/blob/8.10.2/src/geo.c#L418),
  [`#L941`](https://github.com/redis/redis/blob/8.10.2/src/geo.c#L941),
  [`#L974`](https://github.com/redis/redis/blob/8.10.2/src/geo.c#L974);
  [`valkey geo.c#L533`](https://github.com/valkey-io/valkey/blob/9.1.2/src/geo.c#L533),
  [`#L367`](https://github.com/valkey-io/valkey/blob/9.1.2/src/geo.c#L367),
  [`#L424`](https://github.com/valkey-io/valkey/blob/9.1.2/src/geo.c#L424),
  [`#L960`](https://github.com/valkey-io/valkey/blob/9.1.2/src/geo.c#L960),
  [`#L993`](https://github.com/valkey-io/valkey/blob/9.1.2/src/geo.c#L993)).
- **Rillcache** scans the whole index for every search: `run_search` calls
  `zset_items()` and decodes/haversines every member, and `GEOPOS`/`GEODIST`
  build the same all-member vector and linearly search it once per requested
  member. The module comment states the trade: "Searches scan the bounded
  sorted-set payload linearly instead of walking nine neighbouring geohash
  cells" ([`geo.rs`](../../src/engine/geo.rs#L1),
  [`#L350`](../../src/engine/geo.rs#L350),
  [`#L203`](../../src/engine/geo.rs#L203)).
- **Measured (2026-11-12).** A 50,000-member geo grid over ~0.5°, 50
  repetitions, three rotations; a 10-member geo key is the control:

  | Command | Rillcache | Redis 8.10.2 | Valkey 9.1.2 |
  | --- | ---: | ---: | ---: |
  | `GEOSEARCH` 1 km radius | **4.34 ms** | 0.157 ms | 0.170 ms |
  | `GEOSEARCH` 5 km radius (all 50k) | 10.38 | 10.09 | 10.91 |
  | `GEOSEARCH` 2×2 km box | **4.49** | 0.211 | 0.186 |
  | `GEOPOS` 1 member | **1.99** | 0.136 | 0.190 |
  | `GEOPOS` 100 members | **24.86** | 0.469 | 0.606 |
  | `GEODIST` | **3.12** | 0.168 | 0.167 |
  | 10-member controls | 0.149/0.139 | 0.240/0.165 | 0.180/0.264 |

  A small-radius search is **25–28×** slower and a box 21–24×;
  `GEOPOS` is 10–15× for one member and **41–53×** for 100, and `GEODIST` ~19×,
  while the all-members 5 km search is reply-bound at parity and the 10-member
  controls are at the floor. Candidate 4.43.

**Verdict: adapt neighbor-cell search and direct point lookup (4.43).** Reuse
the 4.30/4.38 ordered `(score, member)` index: compute the geohash cell ranges
(including the nine neighbours and the dateline/pole edges), range-read only the
candidate scores, then apply the exact haversine/box test, sort and `COUNT`.
`GEOPOS`/`GEODIST`/`GEOHASH` and `FROMMEMBER` use the member dict/ordered index
directly. Keep the WGS-84 formulas, units and reply shapes.

### 3.28 Multi-member point lookups (SMISMEMBER/ZMSCORE)

- **Redis 8.10.2 and Valkey 9.1.2** probe the set or sorted-set index once per
  requested member: `SMISMEMBER` calls the set membership path per member and
  `ZMSCORE` calls `zsetScore` (a dict lookup) per member
  ([`redis t_set.c#L800`](https://github.com/redis/redis/blob/8.10.2/src/t_set.c#L800),
  [`t_zset.c#L4112`](https://github.com/redis/redis/blob/8.10.2/src/t_zset.c#L4112);
  [`valkey t_set.c#L714`](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_set.c#L714),
  [`t_zset.c#L3776`](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_zset.c#L3776)).
- **Rillcache** materializes once and then linearly scans per member:
  `set_mismember` calls `set_items()` (clone **and sort** every member) and scans
  that `Vec` for each requested member; the `zmscore` arm of `zset_read` collects
  every member reference and runs `members.iter().find(...)` per requested
  member, O(n·m). `HMGET` already uses `hash_get` per field and is fast, which
  isolates the two materializing paths
  ([`collection_commands.rs`](../../src/engine/collection_commands.rs#L3043),
  [`collections.rs`](../../src/engine/collections.rs#L490)).
- **Measured (2026-11-12).** A 100,000-member set/hash/zset, 50 repetitions,
  three rotations; a 10-member collection is the control:

  | Command | Rillcache | Redis 8.10.2 | Valkey 9.1.2 |
  | --- | ---: | ---: | ---: |
  | `SMISMEMBER` 1 member | **7.90 ms** | 0.094 ms | 0.075 ms |
  | `SMISMEMBER` 10 members | **8.25** | 0.227 | 0.077 |
  | `SMISMEMBER` 100 members | **11.75** | 0.394 | 0.211 |
  | `ZMSCORE` 1 member | **8.02** | 0.102 | 0.112 |
  | `ZMSCORE` 10 members | **33.37** | 0.422 | 0.328 |
  | `ZMSCORE` 100 members | **273.2** | 0.630 | 0.187 |
  | `HMGET` 1/10/100 fields | 0.23/0.36/0.32 | 0.47/0.30/0.52 | 0.11/0.07/0.17 |
  | 10-member controls | 0.33/0.26/0.26 | 0.10/0.18/0.08 | 0.53/0.10/0.09 |

  `SMISMEMBER` is **30–107×** slower, `ZMSCORE` **72–1461×** (100 members costs
  273 ms), while `HMGET` and the 10-member controls are at the command floor. The
  ratio grows with the number of requested members because the scan is repeated
  per member. Candidate 4.44.

**Verdict: adapt per-member direct probes (4.44).** Reuse 4.40's
`Collection::contains_member` for `SMISMEMBER` and a direct
`Collection::zset_score(member)` over the score map for `ZMSCORE`, once per
requested member; no `set_items()` clone/sort and no all-member `Vec`. Keep
`SISMEMBER`/`ZSCORE` (4.40/4.30), missing/invalid members, duplicates and reply
order unchanged.

### 3.29 Mass expiry and command latency

- **Redis 8.10.2 and Valkey 9.1.2** do not purge the whole keyspace on a normal
  command: reads expire only the keys they touch, the active cycle removes a
  bounded sample on its own schedule, `KEYS` filters expired entries while it
  scans, `RANDOMKEY` loops for a live key, and `DBSIZE` returns the dict size
  without forcing a purge
  ([`redis expire.c#L287`](https://github.com/redis/redis/blob/8.10.2/src/expire.c#L287),
  [`db.c#L2179`](https://github.com/redis/redis/blob/8.10.2/src/db.c#L2179),
  [`#L1605`](https://github.com/redis/redis/blob/8.10.2/src/db.c#L1605),
  [`#L1593`](https://github.com/redis/redis/blob/8.10.2/src/db.c#L1593);
  [`valkey expire.c#L459`](https://github.com/valkey-io/valkey/blob/9.1.2/src/expire.c#L459)).
- **Rillcache** calls `data.reap(now, usize::MAX)` in `KEYS`, `RANDOMKEY`,
  `DBSIZE`, `MGET`, `MSET`/`MSETEX`, `SCAN`, and in the `Data::reserve`
  admission fallback: the first command after a mass expiry synchronously removes
  **every due key** on the command thread, so its latency is proportional to the
  expired backlog, not to the command
  ([`keyspace.rs`](../../src/engine/keyspace.rs#L183),
  [`multikey.rs`](../../src/engine/multikey.rs#L160),
  [`scan.rs`](../../src/engine/scan.rs#L122),
  [`data.rs`](../../src/engine/data.rs#L646)).
- **Measured (2026-11-12).** 30,000 keys set with `PX 400`, then a 0.7 s wait so
  every key is due; one command per fresh wave, two waves per command, three
  rotations; `GET` on one expired key is the control:

  | First command after 30k expire | Rillcache | Redis 8.10.2 | Valkey 9.1.2 |
  | --- | ---: | ---: | ---: |
  | `MGET` 100 keys | **36.3 ms** | 0.69 ms | 0.98 ms |
  | `KEYS k0000*` | **43.6** | 0.63 | 0.80 |
  | `RANDOMKEY` | **45.5** | 0.65 | 0.60 |
  | `DBSIZE` | **41.2** | 0.58 | 0.83 |
  | `GET` one expired key | 0.92 | 0.75 | 0.72 |

  The purge costs **~1.2–1.5 µs per expired key** (~36–45 ms for 30k), so
  `MGET`/`KEYS`/`RANDOMKEY`/`DBSIZE` are **37–76×** slower than the C servers,
  while the per-key lazy `GET` is at parity. A larger backlog scales the stall
  linearly. Candidate 4.45.

**Verdict: adapt bounded lazy expiry (4.45).** Remove `usize::MAX` from the
command paths: expire only the keys a read touches, filter `KEYS`/`SCAN` results
by liveness, retry `RANDOMKEY` a bounded number of times, and let `DBSIZE`
return the Redis-compatible dict size (expired-but-unreaped keys may still be
counted) or count live keys without a purge. Keep the bounded periodic purge
(128/100 ms) and use a bounded reap plus eviction/admission policy on writes so
an expiry wave can never stall one command. Document the DBSIZE semantics change
and keep expired values unreadable.

### 3.30 SORT and `BY nosort`

- **Redis 8.10.2 and Valkey 9.1.2** special-case `BY nosort`: with a `LIMIT` they
  return the requested slice of the source order without sorting or copying the
  collection; sorted `SORT` still builds a vector of object references and sorts
  it, and `BY pattern` builds one weight lookup per element
  ([`redis sort.c#L178`](https://github.com/redis/redis/blob/8.10.2/src/sort.c#L178),
  [`#L226`](https://github.com/redis/redis/blob/8.10.2/src/sort.c#L226),
  [`#L331`](https://github.com/redis/redis/blob/8.10.2/src/sort.c#L331);
  [`valkey sort.c#L197`](https://github.com/valkey-io/valkey/blob/9.1.2/src/sort.c#L197),
  [`#L243`](https://github.com/valkey-io/valkey/blob/9.1.2/src/sort.c#L243)).
- **Rillcache** always materializes the source first: `source_values` clones every
  element into `Vec<Vec<u8>>`, and the SORT loop builds an `Item` (owned value,
  number, compare key) for every element even when `BY nosort` is set. With
  `BY nosort LIMIT 0 10` on a 100k list it therefore clones 100k values and then
  returns 10
  ([`sort.rs`](../../src/engine/sort.rs#L120),
  [`#L228`](../../src/engine/sort.rs#L228)).
- **Measured (2026-11-12).** A 100,000-element list of integer strings plus
  `w<i>` weight keys, 50 repetitions, three rotations; a 100-element list is the
  control:

  | Command | Rillcache | Redis 8.10.2 | Valkey 9.1.2 |
  | --- | ---: | ---: | ---: |
  | `SORT lst` (full reply) | 92.5 ms | 84.8 ms | 105.6 ms |
  | `SORT lst LIMIT 0 10` | 32.4 | 26.0 | 29.0 |
  | `SORT lst ALPHA LIMIT 0 10` | 90.2 | 108.1 | 106.8 |
  | `SORT lst DESC LIMIT 0 10` | 33.5 | 28.2 | 28.1 |
  | `SORT lst BY nosort LIMIT 0 10` | **7.96** (p99 17.2) | 0.12 (1.17) | 0.16 (0.77) |
  | `SORT lst BY w* LIMIT 0 10` | 117.5 | 92.1 | 80.4 |
  | `SORT lst LIMIT 0 10 GET #` | 30.0 | 27.3 | 26.9 |
  | 100-element controls | 0.21/0.27 | 0.15/0.26 | 0.26/0.23 |

  Sorted and `BY pattern` rows are within **0.9–1.5×** (the ALPHA row leads), but
  `BY nosort LIMIT 10` is **50–66×** slower because it still copies the whole
  list; the 100-element controls are at the floor. Candidate 4.46.

**Verdict: adapt a direct `BY nosort` slice (4.46).** When `BY` has no `*`
(`nosort`), apply `offset`/`count` to the source order without building `Item`s
or cloning the collection: index the list range, or iterate the first
`offset + count` members of a set/zset in its current deterministic order; only
`STORE` materializes the selected values. Leave the sorted/alpha/weighted paths
unchanged because they are already competitive.

### 3.31 Large-hash field listing and keyspace enumeration

- **Redis 8.10.2 and Valkey 9.1.2** iterate the hash table directly: `HGETALL`,
  `HKEYS` and `HVALS` emit fields in dict/listpack order with no per-call
  ordering work, and `KEYS` scans the keyspace and filters, returning the
  unspecified dict order without sorting. `RANDOMKEY` samples the dict
  ([`redis t_hash.c#L5336`](https://github.com/redis/redis/blob/8.10.2/src/t_hash.c#L5336),
  [`#L5453`](https://github.com/redis/redis/blob/8.10.2/src/t_hash.c#L5453),
  [`#L5457`](https://github.com/redis/redis/blob/8.10.2/src/t_hash.c#L5457),
  [`db.c#L1605`](https://github.com/redis/redis/blob/8.10.2/src/db.c#L1605),
  [`#L1593`](https://github.com/redis/redis/blob/8.10.2/src/db.c#L1593);
  [`valkey t_hash.c#L1813`](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_hash.c#L1813),
  [`#L1821`](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_hash.c#L1821),
  [`db.c`](https://github.com/valkey-io/valkey/blob/9.1.2/src/db.c)).
- **Rillcache** collects every field/value reference, **sorts it by field** and
  then encodes `HGETALL`/`HKEYS`/`HVALS`; `KEYS` collects every matched key,
  sorts it and encodes; `RANDOMKEY` picks one by iterating `HashMap` keys to a
  random index. The sort is a deliberate deterministic-order choice, not a Redis
  requirement
  ([`collection_commands.rs`](../../src/engine/collection_commands.rs#L2215),
  [`keyspace.rs`](../../src/engine/keyspace.rs#L183),
  [`#L199`](../../src/engine/keyspace.rs#L199)).
- **Measured (2026-11-12).** A 100,000-field hash (7-byte fields, 16-byte values)
  and a 100,000-key keyspace, 30 repetitions, three rotations; 10-member
  controls:

  | Command | Rillcache | Redis 8.10.2 | Valkey 9.1.2 |
  | --- | ---: | ---: | ---: |
  | `HGETALL` 100k | **132.3 ms** | 25.7 ms | 37.8 ms |
  | `HKEYS` 100k | **98.5** | 16.5 | 15.8 |
  | `HVALS` 100k | **101.6** | 19.7 | 17.9 |
  | `KEYS key:*` (100k) | **91.1** | 42.8 | 21.1 |
  | `KEYS key:0000*` (100) | 32.7 | 35.1 | 7.0 |
  | `KEYS nope*` (0) | 23.7 | 26.7 | 5.5 |
  | `RANDOMKEY` | 0.52 | 0.21 | 0.29 |
  | `HMGET` 100 fields | 0.49 | 0.47 | 0.22 |
  | 10-member controls | 0.40/0.25 | 0.49/0.09 | 0.24/0.22 |

  At 100k, `HGETALL`/`HKEYS`/`HVALS` are **3.5–6.2×** behind (the 10k gap was
  43%), `KEYS key:*` is 2.1–4.3×, and `RANDOMKEY` 1.8–2.5×, while `HMGET` and
  the small controls are at the command floor. Candidate 4.29 is extended to
  `HKEYS`/`HVALS` at 100k; the keyspace rows extend 4.31's index scope.

**Verdict: extend the existing candidates.** 4.29's ordered hash (or an
unordered reply decision) must cover `HGETALL`/`HKEYS`/`HVALS` at 100k, not just
the 10k `HGETALL` row; 4.31's ordered key index must serve `KEYS` and
`RANDOMKEY` as well as `SCAN`, so `KEYS` no longer sorts a fresh snapshot and
`RANDOMKEY` samples without an O(n) walk. Ordering for both is unspecified in
Redis, so the deterministic-order change is an explicit test decision.

### 3.32 HyperLogLog add and count (PFADD/PFCOUNT)

- **Redis 8.10.2 and Valkey 9.1.2** keep the HLL registers in the string object,
  update one register in place on `PFADD`, and store a cached cardinality in the
  HLL header (`card[8]` with the valid/dirty bit) so a repeated `PFCOUNT` is O(1)
  until the next add/merge
  ([`redis hyperloglog.c#L182`](https://github.com/redis/redis/blob/8.10.2/src/hyperloglog.c#L182),
  [`#L191`](https://github.com/redis/redis/blob/8.10.2/src/hyperloglog.c#L191),
  [`#L1642`](https://github.com/redis/redis/blob/8.10.2/src/hyperloglog.c#L1642),
  [`#L1696`](https://github.com/redis/redis/blob/8.10.2/src/hyperloglog.c#L1696);
  [`valkey hyperloglog.c#L208`](https://github.com/valkey-io/valkey/blob/9.1.2/src/hyperloglog.c#L208),
  [`#L1664`](https://github.com/valkey-io/valkey/blob/9.1.2/src/hyperloglog.c#L1664),
  [`#L1699`](https://github.com/valkey-io/valkey/blob/9.1.2/src/hyperloglog.c#L1699)).
- **Rillcache** decodes the stored HLL on every command: for the dense
  representation `inspect` copies all 16,384 register bytes, `PFADD` mutates the
  copy, `serialize()` builds a new header+registers vector, and `data.put` copies
  that into the entry again — three full copies per add. `PFCOUNT` builds a new
  empty HLL, merges the stored registers into it (another copy) and scans all
  16,384 registers every call; there is no cached cardinality
  ([`hll.rs`](../../src/engine/hll.rs#L240),
  [`#L285`](../../src/engine/hll.rs#L285)).
- **Measured (2026-11-12).** 100,000 distinct elements added in 2,000-command
  pipelines, then 10,000 repeated `PFCOUNT` calls, three rotations; a 10-element
  HLL is the control:

  | Operation | Rillcache | Redis 8.10.2 | Valkey 9.1.2 |
  | --- | ---: | ---: | ---: |
  | `PFADD` 100k (pipelined) | **1,695 ms (16.96 µs/add)** | 410 (4.10) | 392 (3.92) |
  | `PFCOUNT` 10k (pipelined) | **2,655 ms (265.5 µs/call)** | 35.7 (3.57) | 33.1 (3.31) |
  | `PFADD` single p50 | 0.187 ms | 0.151 | 0.302 |
  | `PFCOUNT` single p50 | 0.484 | 0.169 | 0.059 |
  | 10-element controls | 0.164/0.185 | 0.195/0.228 | 0.073/0.123 |

  Adds are **4.1–4.3×** and repeated counts **74–80×** slower; the single-call
  p50 hides the count cost behind the client floor, but a pipelined or repeated
  counter workload pays it every time. Candidate 4.47.

**Verdict: adapt in-place registers and a cached count (4.47).** Keep the HLL
mutable inside the entry (no decode/serialize/put copies on `PFADD`), and store a
cached cardinality in the header that `PFADD`/`PFMERGE` invalidate, so
`PFCOUNT` is O(1) when nothing changed. Preserve the exact/dense
representations, serialization/rewrite/replay, the error bound and merge
semantics.

### 3.33 BITPOS on large bitmaps

- **Redis 8.10.2 and Valkey 9.1.2** scan for a bit a word at a time:
  `redisBitpos` aligns to 64-bit words and uses `trailing_zeros` on the word or
  its complement, masking only the first and last partial words
  ([`redis bitops.c#L392`](https://github.com/redis/redis/blob/8.10.2/src/bitops.c#L392),
  [`#L1721`](https://github.com/redis/redis/blob/8.10.2/src/bitops.c#L1721);
  [`valkey bitops.c#L1041`](https://github.com/valkey-io/valkey/blob/9.1.2/src/bitops.c#L1041)).
- **Rillcache** scans byte by byte and then bit by bit: `find_bit` loops over
  every byte in the range and over all eight bit positions inside it, so a
  not-found `BITPOS key 0` visits 8 × 16M positions
  ([`string_algorithms.rs`](../../src/engine/string_algorithms.rs#L432)).
- **Measured (2026-11-12).** One 16 MB `0xff` bitmap (built by 512 KiB
  `APPEND`s), 10 repetitions, three rotations; a 64-byte bitmap is the control:

  | Command | Rillcache | Redis 8.10.2 | Valkey 9.1.2 |
  | --- | ---: | ---: | ---: |
  | `BITPOS bits 0` (not found) | **201.6 ms** | 5.08 ms | 5.14 ms |
  | `BITPOS bits 0 0 -1` | **209.4** | 4.79 | 5.96 |
  | `BITPOS bits 1` (first bit) | 0.12 | 0.60 | 0.09 |
  | `BITCOUNT bits` (control) | 23.46 | 4.04 | 4.78 |
  | 64 B `BITPOS 0` | 0.32 | 0.55 | 0.10 |

  A not-found `BITPOS 0` is **39–44×** slower, while a first-bit hit is at the
  floor; `BITCOUNT` is the previously admitted 4.37 row (5–6×). The same
  word-wise machinery fixes both. Candidate 4.37 is extended.

**Verdict: extend 4.37 with a word-wise bit scan.** Use `u64` chunks for
`find_bit`: for bit 1 take `trailing_zeros` of the word, for bit 0 of its
complement, with big-endian byte order and masked head/tail words; keep the
BYTE/BIT range rules and the not-found return values. This is the same safe
`u64` loop as the `BITCOUNT` change.

### 3.34 Stream group creation (XGROUP CREATE)

- **Redis 8.10.2 and Valkey 9.1.2** add a consumer group by inserting one entry
  in the stream's group dictionary; the stream's entries are untouched and the
  cost is O(1) independent of stream length
  ([`redis t_stream.c#L3553`](https://github.com/redis/redis/blob/8.10.2/src/t_stream.c#L3553),
  [`#L3667`](https://github.com/redis/redis/blob/8.10.2/src/t_stream.c#L3667);
  [`valkey t_stream.c#L2582`](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_stream.c#L2582)).
- **Rillcache** deep-clones the **entire `Stream`** for `XGROUP CREATE`
  (`e.stream.clone()`), inserts the group into the clone, folds
  `payload_bytes()` over every entry and replaces the key through `put_stream`.
  The other XGROUP subcommands (`SETID`, `DESTROY`, `DELCONSUMER`,
  `CREATECONSUMER`) already mutate the group maps in place
  ([`streams.rs`](../../src/engine/streams.rs#L1454),
  [`#L1492`](../../src/engine/streams.rs#L1492)).
- **Measured (2026-11-12).** A 5,000-entry stream plus a 10-entry control, 20
  unique groups per stream, three rotations:

  | Command | Rillcache | Redis 8.10.2 | Valkey 9.1.2 |
  | --- | ---: | ---: | ---: |
  | `XGROUP CREATE` 5k stream | **8.26 ms** | 0.083 ms | 0.094 ms |
  | `XGROUP CREATE` 10-entry stream | 0.338 | 0.120 | 0.097 |
  | `XGROUP CREATECONSUMER` 5k | 0.123 | 0.227 | 0.127 |
  | `XGROUP SETID` 5k | 0.099 | 0.283 | 0.099 |
  | `XGROUP DESTROY` 5k | 0.109 | 0.097 | 0.109 |
  | 10-entry `CREATECONSUMER` | 0.169 | 0.086 | 0.069 |

  `XGROUP CREATE` is **88–99×** slower and grows linearly with stream length
  (~165 ms at 100k by extrapolation), while every other XGROUP subcommand is at
  parity because it already mutates in place. Candidate 4.48.

**Verdict: adapt an in-place group insert (4.48).** Borrow `Entry.stream`
mutably, insert the group (or create an empty stream for `MKSTREAM`), call
`touch`, and leave `payload_len` unchanged because `payload_bytes` counts only
entries. Keep `BUSYGROUP`, explicit-ID/`$` semantics, PEL/consumer state,
TTL/WATCH/journal/rewrite and the other subcommands as they are.

### 3.35 Sorted-set pops (ZPOPMIN/ZPOPMAX/ZMPOP)

- **Redis 8.10.2 and Valkey 9.1.2** pop from the skiplist head or tail in
  O(log n): the min/max node is the first/last element, removed by score/member
  and the dict entry deleted; `ZPOPMIN k COUNT n` repeats n times
  ([`redis t_zset.c#L4227`](https://github.com/redis/redis/blob/8.10.2/src/t_zset.c#L4227),
  [`#L4393`](https://github.com/redis/redis/blob/8.10.2/src/t_zset.c#L4393);
  [`valkey t_zset.c#L3894`](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_zset.c#L3894),
  [`#L4037`](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_zset.c#L4037)).
- **Rillcache** materializes, sorts and replaces the whole sorted set for every
  pop: `zset_pop` calls `owned_zset` (clone every member), `ordered_zset`
  (copy every member and sort all), removes `count` members, re-encodes and
  `put_collection`s the rest. `ZPOPMIN` therefore costs O(n log n) even for one
  element, and so do `ZPOPMAX`, `ZMPOP` and the blocking variants
  ([`collection_commands.rs`](../../src/engine/collection_commands.rs#L4817)).
- **Measured (2026-11-12).** A 100,000-member sorted set (7-byte members,
  scores 100–899), 20 pops, three rotations; a 100-member set is the control:

  | Command | Rillcache | Redis 8.10.2 | Valkey 9.1.2 |
  | --- | ---: | ---: | ---: |
  | `ZPOPMIN` | **156.7 ms** | 0.42 ms | 0.34 ms |
  | `ZPOPMAX` | **159.9** | 0.21 | 0.29 |
  | `ZPOPMIN COUNT 10` | **146.6** | 0.35 | 0.16 |
  | `ZMPOP 1 z MIN` | **155.8** | 0.16 | 0.20 |
  | 100-member `ZPOPMIN` | 0.11 | 0.16 | 0.24 |

  `ZPOPMIN`/`ZPOPMAX` are **373–761×** slower, the count-10 and `ZMPOP` forms
  419–974×, while the 100-member control is at the floor. The largest measured
  ratio in the record. Candidate 4.49.

**Verdict: adapt ordered pops (4.49).** Reuse 4.30/4.38's ordered
`(score, member)` index: min/max is its first/last element, so remove that member
from the map and the index in place, update `payload_len` by the delta, and
repeat for `COUNT`. No clone, sort, re-encode or replace. `ZMPOP`/`BZPOP*` reuse
the same primitive; keep score formatting, tie order, empty-key deletion and
journal/rewrite/accounting semantics.

### 3.36 Multi-command transactions (MULTI/EXEC)

- **Redis 8.10.2 and Valkey 9.1.2** queue the parsed command objects (argument
  references, not copies), check ACL at queue time, and execute the queue in one
  guard at `EXEC`, writing one reply array
  ([`redis multi.c#L91`](https://github.com/redis/redis/blob/8.10.2/src/multi.c#L91),
  [`#L127`](https://github.com/redis/redis/blob/8.10.2/src/multi.c#L127),
  [`#L35`](https://github.com/redis/redis/blob/8.10.2/src/multi.c#L35);
  [`valkey multi.c#L162`](https://github.com/valkey-io/valkey/blob/9.1.2/src/multi.c#L162),
  [`#L195`](https://github.com/valkey-io/valkey/blob/9.1.2/src/multi.c#L195)).
- **Rillcache** copies every queued command's arguments into
  `Vec<Vec<u8>>` (capped at 10,000), re-checks ACL for each queued command at
  `EXEC`, then executes them through the normal dispatcher and writes one array.
  The default local `Store` path executes the queue directly; only the shared/
  multi-owner handle copies the queue again in the bridge
  ([`transaction.rs`](../../src/session/transaction.rs#L11),
  [`dispatch`](../../src/session/transaction.rs#L35)).
- **Measured (2026-11-12).** One pipelined `MULTI ... EXEC` per sample, 20
  repetitions, three rotations; a 1,000-command plain batch is the control:

  | Batch | Rillcache | Redis 8.10.2 | Valkey 9.1.2 |
  | --- | ---: | ---: | ---: |
  | `MULTI` + 100 `SET` + `EXEC` | 0.764 ms | 0.448 ms | 0.742 ms |
  | `MULTI` + 1,000 `SET` + `EXEC` | 3.02 | 2.76 | 2.48 |
  | `MULTI` + 100 `SET` 1 KB + `EXEC` | 0.873 | 1.042 | 1.077 |
  | `MULTI` + 1,000 `SET` + `DISCARD` | 1.37 | 1.30 | 1.77 |
  | 1,000 plain `SET` (control) | 1.42 | 2.20 | 2.59 |
  | second 1,000-command transaction | 2.85 | 2.75 | 2.59 |

  The 1,000-command transaction is within **1.1–1.2×** and the queue-only and
  plain-batch rows lead; the 100-command row is within the host spread (Rill
  0.57–1.16 ms, Redis 0.44–1.04). No candidate.

**Verdict: keep the transaction path.** The measured cost of copying arguments
and re-checking ACL is not a material gap at 1,000 commands, and the plain batch
is faster than both C servers. Re-measure only if the execution path changes
(e.g., an ordered-index or in-place collection change lands) or a 10,000-command
cohort shows a different shape.

### 3.37 MONITOR feed under a burst

- **Redis 8.10.2 and Valkey 9.1.2** append each monitored command to every
  monitor's client output buffer; a 2,000-command burst (~40 KB of MONITOR lines)
  is far below the output limit and nothing is dropped
  ([`redis replication.c#L806`](https://github.com/redis/redis/blob/8.10.2/src/replication.c#L806),
  [`server.c`](https://github.com/redis/redis/blob/8.10.2/src/server.c);
  [`valkey replication.c#L694`](https://github.com/valkey-io/valkey/blob/9.1.2/src/replication.c#L694)).
- **Rillcache** formats one MONITOR line per command, clones it per monitor and
  `try_send`s it into the same 1,024-message queue as pub/sub; a full queue
  **drops that line** and keeps the monitor attached. The reactor drains at most
  64 lines per 1 ms tick, so a burst larger than 1,024 overflows before the drain
  catches up. The source documents the drop
  ([`engine.rs`](../../src/engine.rs#L883),
  [`pool.rs`](../../src/server/pool.rs#L158)).
- **Measured (2026-11-12).** One 2,000-`PING` pipeline with 0/1/10 attached
  monitors, two bursts per configuration, three rotations:

  | Configuration | Rillcache | Redis 8.10.2 | Valkey 9.1.2 |
  | --- | ---: | ---: | ---: |
  | Lines delivered per monitor (2,000 sent) | **1,024** (all six samples) | 2,000 | 2,000 |
  | Publisher p50, 0 monitors | 5.18 ms | 1.88 | 2.37 |
  | Publisher p50, 1 monitor | 3.75 | 5.66 | 16.74 |
  | Publisher p50, 10 monitors | 5.99 | 19.78 | 27.22 |

  The line count is deterministic: Rill delivers **1,024 of 2,000** to every
  monitor in every sample, while both C servers deliver all 2,000. The publisher
  timings at 0–1 monitors are within the shared host's spread and the 10-monitor
  fan-out is cheaper in Rill because it stops at the queue bound, so the cost is
  not the finding; the silent truncation is. This extends 4.39 to MONITOR.

**Verdict: extend 4.39's contract to MONITOR.** Use the same byte-bounded queue
and drain, but never truncate silently: either drop with a counted metric and
keep the monitor, or disconnect with a documented error. Monitors are diagnostic,
so an under-counted feed is worse than a visible disconnect.

### 3.38 Lexicographic ranges and count-only sorted-set queries

- **Redis 8.10.2 and Valkey 9.1.2** keep the sorted set in a skiplist ordered by
  `(score, member)` and answer range/count queries by seeking, not scanning:
  `zrangeGenericCommand` uses `zslNthInRange` for score/rank bounds and
  `zslNthInLexRange` for lex bounds
  ([`redis t_zset.c#L469`](https://github.com/redis/redis/blob/8.10.2/src/t_zset.c#L469),
  [`#L866`](https://github.com/redis/redis/blob/8.10.2/src/t_zset.c#L866));
  `zcountCommand` and `zlexcountCommand` find the first and last in-range node and
  subtract their ranks, an **O(log n)** operation
  ([`#L3690`](https://github.com/redis/redis/blob/8.10.2/src/t_zset.c#L3690));
  [Valkey](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_zset.c#L418) is
  the same. `BYLEX` is defined over the member order of an equal-score set, which
  is exactly the skiplist's local order.
- **Rillcache** rewrites `ZRANGEBYLEX`/`ZREVRANGEBYLEX` into
  `ZRANGE ... BYLEX` ([`collection_commands.rs`](../../src/engine/collection_commands.rs#L280)),
  which materialises every member through `Collection::zset_items`
  ([`data.rs`](../../src/engine/data.rs#L111)) and then calls `select_zset`;
  `select_zset` always builds `ordered_zset` — a **full sort of the whole set** —
  before it filters by lex and applies `LIMIT`
  ([`collection_commands.rs#L798`](../../src/engine/collection_commands.rs#L798),
  [`#L4185`](../../src/engine/collection_commands.rs#L4185)). `ZCOUNT` and
  `ZLEXCOUNT` materialise all members and linearly filter them
  ([`#L3866`](../../src/engine/collection_commands.rs#L3866),
  [`#L4556`](../../src/engine/collection_commands.rs#L4556)), so even an empty
  range costs a full pass. A `LIMIT 10` therefore pays for the entire set, and a
  count pays for every member regardless of width.
- **Measured (2026-11-13).** One 100,000-member zset with score 0 for every member
  (`m000000`–`m099999`), a 10-member control, 20 repetitions per small reply and
  five per full reply, three rotations:

  | Command | Rillcache | Redis 8.10.2 | Valkey 9.1.2 |
  | --- | ---: | ---: | ---: |
  | `ZRANGEBYLEX - + LIMIT 0 10` | **55.98 ms** | 0.177 ms | 0.269 ms |
  | `ZRANGEBYLEX [m050000 [m099999 LIMIT 0 10` | **69.79** | 0.188 | 0.428 |
  | `ZREVRANGEBYLEX + - LIMIT 0 10` | **60.09** | 0.180 | 0.741 |
  | `ZLEXCOUNT - +` | **1.88** | 0.346 | 0.128 |
  | `ZLEXCOUNT [m050000 [m050100` | **15.02** | 0.224 | 0.212 |
  | `ZCOUNT -inf +inf` | **1.75** | 0.269 | 0.212 |
  | `ZCOUNT 0 0` (every member) | **1.25** | 0.387 | 0.222 |
  | `ZCOUNT 1 2` (empty) | **1.43** | 0.234 | 0.339 |
  | `ZRANGE 0 9` (known 4.30 path) | **61.62** | 0.180 | 0.150 |
  | `ZRANGEBYLEX - +` (100k reply) | **144.75** | 87.17 | 100.21 |
  | 10-member controls | 0.25–0.47 | 0.09–0.33 | 0.17–0.22 |

  A 10-element lex range is **208–371×** slower than Redis/Valkey and grows with
  set size, because it pays the same full sort as a rank range. Count-only forms
  are **5.4–14.7×** slower for the full range and **67–71×** for a narrow lex
  window; the empty `ZCOUNT 1 2` still scans all 100,000 members. The full
  100k-member reply is only 1.4–1.7× behind because the reply encoding dominates
  both sides. Candidate 4.50.

**Verdict: extend the ordered index (4.30/4.38) with seek, and admit 4.50 for
span counts.** Seek the first and last in-range position by `(score, member)`
(and by member inside an equal-score range for lex), apply `LIMIT` from the seek,
and derive counts from rank subtraction; a full-range count is just the
cardinality. Do not sort to answer a bounded range and do not scan to count.

### 3.39 Stream consumer-group reads, claims and pending queries

- **Redis 8.10.2 and Valkey 9.1.2** keep stream entries in a radix tree and the
  group/consumer PEL in radix trees keyed by ID. A `streamNACK` stores a
  **consumer pointer**, delivery time and count — not a copy of the consumer name
  ([`redis t_stream.c#L3339`](https://github.com/redis/redis/blob/8.10.2/src/t_stream.c#L3339)).
  `streamReplyWithRange` writes the stored field objects into the reply directly,
  and `xautoclaimCommand` walks the PEL from a cursor, examining at most
  `COUNT × 10` entries ([`#L4778`](https://github.com/redis/redis/blob/8.10.2/src/t_stream.c#L4778));
  [Valkey](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_stream.c#L3331)
  is the same. There is no per-entry value copy on a read.
- **Rillcache** stores `StreamEntry { fields: Vec<(Vec<u8>, Vec<u8>)> }` and, per
  pending entry, `Pending { consumer: Vec<u8>, deliveries, delivered_at }` in
  **both** the group and consumer `BTreeMap`s
  ([`data.rs`](../../src/engine/data.rs#L157)). `xreadgroup_multi` clones every
  delivered entry into the reply and inserts two PEL entries, cloning the
  consumer-name `Vec` per pending entry
  ([`streams.rs#L1668`](../../src/engine/streams.rs#L1668)); `xclaim` and
  `xautoclaim` clone each `Pending` and each `StreamEntry` into their reply
  ([`#L985`](../../src/engine/streams.rs#L985),
  [`#L1154`](../../src/engine/streams.rs#L1154)). The clone share grows with the
  batch, not with the stream.
- **Measured (2026-11-13).** One 10,000-entry stream with a single 16-byte field,
  one group whose consumer owns all 10,000 entries, 20 repetitions per row (40
  fresh groups for the `NEW` rows), three rotations:

  | Command | Rillcache | Redis 8.10.2 | Valkey 9.1.2 |
  | --- | ---: | ---: | ---: |
  | `XREADGROUP ... 0 COUNT 100` (history) | 0.91 ms | 1.14 ms | 0.62 ms |
  | `XREADGROUP ... 0 COUNT 1000` (history) | 7.00 | 7.40 | 5.80 |
  | `XREADGROUP ... > COUNT 100` (new) | 0.92 | 0.57 | 0.69 |
  | `XREADGROUP ... > COUNT 1000` (new) | **8.99** | 5.19 | 6.43 |
  | `XPENDING` summary | 0.22 | 0.16 | 0.23 |
  | `XPENDING - + 100` | 0.71 | 0.47 | 0.50 |
  | `XPENDING - + 1000` | 3.57 | 2.93 | 3.72 |
  | `XAUTOCLAIM COUNT 100` | **1.17** | 0.58 | 1.03 |
  | `XAUTOCLAIM COUNT 1000` | **9.84** | 4.67 | 7.50 |
  | `XCLAIM` 100 ids | 1.17 | 1.35 | 1.01 |
  | `XCLAIM` 100 ids `JUSTID` | 0.66 | 0.63 | 0.65 |
  | `XLEN` / `XADD` controls | 0.25 / 0.19 | 0.14 / 0.21 | 0.09 / 0.14 |

  Every command is within **1.3–2.1×** of Redis and usually inside Valkey's
  spread. The residual is concentrated in the two largest batch forms:
  `XREADGROUP > COUNT 1000` (1.7×) and `XAUTOCLAIM COUNT 1000` (2.1×), where Rill
  clones 1,000 entries or up to 10,000 `Pending` values (each with a heap
  consumer name) while the C servers link or reference them. Preloading the
  10,000 entries took **16.2 s** in Rill versus 0.1–0.2 s in both C servers — the
  already-admitted `XADD` clone-per-append path (4.42), not a new read cost.

**Verdict: measured near-parity; no candidate.** The read/claim path does not
clone the stream and stays within ~2× on the largest batches, below the
record's candidate threshold and inside the host spread. Record it as a boundary,
not an optimization target. If 4.42 changes the stream representation, or a
cohort with multi-field entries and a 100k-entry PEL shows more than ~3× with the
clone share above ~30%, the bounded fix is to iterate entries by reference and
store a consumer ID instead of a name per pending entry; that changes storage,
so it needs its own measured admission rather than being folded into 4.42.

### 3.40 BITFIELD writes on large strings

- **Redis 8.10.2 and Valkey 9.1.2** execute `BITFIELD` subcommands against the
  value's `sds` in place: `bitfieldGeneric` looks the string up with room for the
  highest write offset (`lookupStringForBitCommand`) and then each `SET`/`INCRBY`
  reads and writes the buffer with no copy
  ([`redis bitops.c#L1886`](https://github.com/redis/redis/blob/8.10.2/src/bitops.c#L1886),
  [`#L2125`](https://github.com/redis/redis/blob/8.10.2/src/bitops.c#L2125);
  [Valkey](https://github.com/valkey-io/valkey/blob/9.1.2/src/bitops.c#L1425) is
  the same). A one-byte write is O(1) regardless of value size.
- **Rillcache** already borrows the value on the read path: `BITFIELD` with only
  `GET` subcommands and `BITFIELD_RO` read `entry.value.as_slice()` with no copy
  ([`bitfield.rs#L302`](../../src/engine/bitfield.rs#L302)). The **write path**
  clones the whole value (`entry.value.clone()`, [`#L316`](../../src/engine/bitfield.rs#L316))
  before applying the subcommands, then calls `data.put(key, &bytes, …)` with the
  full slice ([`#L382`](../../src/engine/bitfield.rs#L382)) — two full-value
  copies per call for a one-byte `SET`, the same shape 4.34 removed from
  `APPEND`/`SETRANGE` and 4.37 targets for `SETBIT`.
- **Measured (2026-11-13).** One 16 MB `0x01` value, one 1 MB value and a 64 B
  control, 20 repetitions, three rotations:

  | Command | Rillcache | Redis 8.10.2 | Valkey 9.1.2 |
  | --- | ---: | ---: | ---: |
  | 16 MB `SETBIT 0 1` (known 4.37 path) | 4.37 ms | 0.260 ms | 0.150 ms |
  | 16 MB `BITFIELD SET u8 0 1` | **4.56** | 0.087 | 0.089 |
  | 16 MB `BITFIELD INCRBY u8 0 1` | **4.68** | 0.118 | 0.111 |
  | 16 MB `BITFIELD SET u63 0 1` | **4.39** | 0.113 | 0.120 |
  | 16 MB `BITFIELD` × 100 `INCRBY u8` | **4.58** | 0.305 | 0.307 |
  | 16 MB `BITFIELD_RO GET u8 0` | 0.34 | 0.25 | 0.14 |
  | 16 MB `BITFIELD GET u8 0` | 0.25 | 0.42 | 0.21 |
  | 1 MB `BITFIELD SET u8` | 0.51 | 0.34 | 0.17 |
  | 64 B `BITFIELD SET u8` | 0.19 | 0.21 | 0.09 |
  | 16 MB `GETRANGE 0 9` control | 0.12 | 0.23 | 0.12 |

  A one-byte `BITFIELD` write is **37–52×** slower at 16 MB and scales with value
  size (1 MB: 1.5–3×; 64 B: at the floor). The 100-subcommand form costs the same
  as one write because the copy, not the subcommands, dominates. The read-only
  forms are already at the floor, so only the mutation path needs the in-place
  treatment. Candidate 4.37, extended.

**Verdict: extend 4.37 to `BITFIELD` writes.** Apply the same admit-then-mutate
shape: reserve for the highest write offset, borrow the entry's `Vec`, zero-fill
only when the offset passes the end, and apply the subcommands in place; keep the
read path borrowed and unmodified.

### 3.41 Sorted-set algebra and cardinality (ZINTERSTORE/ZUNIONSTORE/ZDIFFSTORE/ZINTERCARD)

- **Redis 8.10.2 and Valkey 9.1.2** implement all four through one generic path:
  it orders the sources by cardinality, iterates the **smallest** source with a
  borrowed iterator, probes each other source with `zuiFind`, and either counts
  matches (with an early `LIMIT` exit, no destination object) or inserts each
  survivor directly into the destination zset's dict and skiplist
  ([`redis t_zset.c#L2885`](https://github.com/redis/redis/blob/8.10.2/src/t_zset.c#L2885),
  [`zinterCardCommand #L3217`](https://github.com/redis/redis/blob/8.10.2/src/t_zset.c#L3217),
  [`qsort by cardinality #L3012`](https://github.com/redis/redis/blob/8.10.2/src/t_zset.c#L3012);
  [Valkey](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_zset.c#L2914) is
  the same). No source is copied and the result is never assembled as a map and
  re-sorted.
- **Rillcache** first materialises every source into an owned
  `HashMap<Vec<u8>, f64>` through `owned_zset`
  ([`collection_commands.rs#L4330`](../../src/engine/collection_commands.rs#L4330)),
  builds the result in a second `HashMap` with cloned member keys, and then
  full-sorts it with `ordered_zset` before encoding. `zset_intercard` is worse: it
  clones the first set's members into a `BTreeSet<Vec<u8>>` and retains by probing
  the others, so a count pays a full copy plus an O(n log n) tree build and only
  applies `LIMIT` after all members are materialised
  ([`#L4492`](../../src/engine/collection_commands.rs#L4492)).
- **Measured (2026-11-13).** Two 100,000-member collections (`a`=`m000000`–
  `m099999` score 1, `b`=`m050000`–`m149999` score 2; intersection 50k, union
  150k, difference 50k), 10 repetitions, three rotations, 10-member controls:

  | Command | Rillcache | Redis 8.10.2 | Valkey 9.1.2 |
  | --- | ---: | ---: | ---: |
  | `ZINTERCARD 2` | **220.56 ms** | 40.41 ms | 17.14 ms |
  | `ZINTERSTORE` | **306.58** | 110.53 | 61.56 |
  | `ZDIFFSTORE` | **331.30** | 101.98 | 61.43 |
  | `ZUNIONSTORE` | 477.60 | 366.56 | 280.35 |
  | `SINTERCARD 2` | 41.47 | 29.58 | 13.60 |
  | `SINTERSTORE` | 44.26 | 66.90 | 36.54 |
  | `SUNIONSTORE` | 65.83 | 87.72 | 52.71 |
  | `SDIFFSTORE` | 39.58 | 61.98 | 33.42 |
  | 10-member zset controls | 0.16 | 0.45 | 0.43 |
  | 10-member set controls | 0.25 | 0.32 | 0.24 |
  | `SCARD` control (known 4.40 path) | 43.52 | 0.31 | 0.18 |

  `ZINTERCARD` is **5.5×** behind Redis and **12.9×** behind Valkey; `ZINTERSTORE`
  is 2.8–5.0× and `ZDIFFSTORE` 3.2–5.4× behind. `ZUNIONSTORE` is only 1.3–1.7×
  behind because the 150k-member result write dominates all three servers. The
  set-store variants already lead because the destination set is encoded once
  without a map-then-sort pass. Preloading the four 100k collections took 16.0 s
  in Rill versus 1.5–1.7 s, the admitted 4.35 clone-per-write path, not this
  read. Candidate 4.51.

**Verdict: adapt borrowed-source algebra (4.51).** Iterate the smallest borrowed
source, probe the others in place, count with an early `LIMIT` exit, and for the
store forms build the destination once (no intermediate map, no re-sort of a
materialised result). Keep the set-store paths as they are.

### 3.42 INFO keyspace over a large keyspace

- **Redis 8.10.2 and Valkey 9.1.2** answer `INFO keyspace` from maintained
  container sizes: `kvstoreSize(db->keys)`, `kvstoreSize(db->expires)`,
  `estoreSize(db->subexpires)` and a maintained `avg_ttl`, one iteration per
  database, so the section is **O(number of databases)** regardless of key count
  ([`redis server.c#L7073`](https://github.com/redis/redis/blob/8.10.2/src/server.c#L7073);
  [Valkey](https://github.com/valkey-io/valkey/blob/9.1.2/src/server.c) keeps the
  same counters).
- **Rillcache** builds the same fields by walking **every entry**:
  `info` filters `data.entries.values()` by `alive(now)` to count live,
  expiring and subexpiry keys and sums the remaining TTLs
  ([`discovery/mod.rs`](../../src/engine/discovery/mod.rs#L1016)). The walk runs
  whenever the requested sections include `keyspace`, `default` or `all`, which
  includes a plain `INFO`; `INFO stats` does not pay it.
- **Measured (2026-11-13).** 10,000, 100,000 and 500,000 plain string keys (no
  TTLs), preloaded with pipelined `SET`s, 20 repetitions, three rotations:

  | Keyspace | Command | Rillcache | Redis 8.10.2 | Valkey 9.1.2 |
  | ---: | --- | ---: | ---: | ---: |
  | 10k | `INFO keyspace` | 0.208 ms | 0.188 ms | 0.137 ms |
  | 100k | `INFO keyspace` | **2.791** | 0.171 | 0.134 |
  | 500k | `INFO keyspace` | **10.004** | 0.249 | 0.295 |
  | 500k | `INFO` (default) | **10.147** | 0.365 | 0.260 |
  | 500k | `INFO stats` | 0.238 | 0.178 | 0.202 |
  | 500k | `DBSIZE` | 0.185 | 0.133 | 0.181 |
  | 500k | `PING` | 0.237 | 0.103 | 0.284 |

  The cost grows linearly with the keyspace: **16–21×** behind at 100k and
  **34–40×** at 500k, while `INFO stats`, `DBSIZE` and `PING` stay at the command
  floor. That isolates the walk to the keyspace section, not to `INFO` parsing or
  the shared counters. With no TTLs, `DBSIZE` is O(1) here; a large expiring set
  additionally pays the admitted 4.45 mass-expiry path. Candidate 4.52.

**Verdict: adapt maintained keyspace accounting (4.52).** Keep live/expiring/
subexpiry key counts and a TTL sum updated on every mutation (insert, overwrite,
expire, persist, remove, reap, eviction, flush, rename/copy, replay), or read the
container sizes plus a Redis-style maintained/sampled average, and answer the
section in O(1).

### 3.43 Two-key moves (SMOVE/LMOVE/RPOPLPUSH)

- **Redis 8.10.2 and Valkey 9.1.2** move one element between two borrowed
  containers: `smoveCommand` removes from the source set and adds to the
  destination set (`setTypeRemove`/`setTypeAdd`, dict/intset/listpack) without
  copying either; `lmoveGenericCommand` pops one end of the source list and pushes
  the other end of the destination, with an in-place same-key rotate path
  ([`redis t_set.c`](https://github.com/redis/redis/blob/8.10.2/src/t_set.c),
  [`t_list.c`](https://github.com/redis/redis/blob/8.10.2/src/t_list.c);
  [Valkey](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_set.c) is the
  same). The move is O(1) for a list and O(1)/O(log n) for a set.
- **Rillcache** rebuilds both keys. `set_move` calls `owned_set` on the source and
  the destination, mutates the owned copies and re-encodes both sets
  ([`collection_commands.rs#L3073`](../../src/engine/collection_commands.rs#L3073));
  `list_move_in_data` calls `Collection::list_values` on both lists, which
  flattens and **clones every element** (`data.rs#L41`), then rebuilds both
  `VecDeque`s and re-encodes them
  ([`#L2541`](../../src/engine/collection_commands.rs#L2541)). A one-element move
  therefore costs a full copy of both sides and two encodes.
- **Measured (2026-11-13).** Two 100,000-member sets and two 200,000-element
  16-byte lists, 20 repetitions (each `SMOVE` consuming a distinct member), three
  rotations, 10-element controls:

  | Command | Rillcache | Redis 8.10.2 | Valkey 9.1.2 |
  | --- | ---: | ---: | ---: |
  | `SMOVE` on a 100k set | **25.49 ms** | 0.40 ms | 0.28 ms |
  | `LMOVE` on a 200k list | **39.15** | 0.34 | 0.08 |
  | `RPOPLPUSH` on a 200k list | **41.16** | 0.33 | 0.19 |
  | `LMOVE` same key (rotate) | **20.51** | 0.26 | 0.21 |
  | 10-member `SMOVE` control | 0.31 | 0.26 | 0.15 |
  | `LPUSH`/`SADD` controls | 0.18/0.22 | 0.24/0.36 | 0.17/0.14 |
  | `SISMEMBER` control (known 4.40 path) | 9.39 | 0.36 | 0.23 |

  The move costs **64–91×** its C counterparts for sets and **79–489×** for
  lists, because both sides are cloned and re-encoded; the same-key rotate still
  pays a full flatten of one 200k list (20.5 ms). The 10-element controls are at
  the floor, and the `SISMEMBER` control is the admitted 4.40 cardinality path,
  not a move cost. Preloading the four large keys took 13.4 s in Rill versus
  1.4–1.6 s, the 4.35 write path. Candidate 4.53.

**Verdict: adapt in-place two-key moves (4.53).** Admit the destination growth
first, then remove/pop from the source and insert/push into the destination in
place, delete an emptied source, and adjust payload by the delta; a same-key move
rotates within one container. This reuses 4.35's in-place set mutation and
4.41's chunk-level list access.

### 3.44 Cardinality `LIMIT` early exit (SINTERCARD/ZINTERCARD)

- **Redis 8.10.2 and Valkey 9.1.2** treat `LIMIT n` as a work bound: the
  cardinality path iterates the smallest source, counts matches, and breaks as
  soon as the count reaches `n`, before materialising anything
  ([`redis t_set.c#L1497`](https://github.com/redis/redis/blob/8.10.2/src/t_set.c#L1497),
  [`t_zset.c#L3027`](https://github.com/redis/redis/blob/8.10.2/src/t_zset.c#L3027);
  [Valkey](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_set.c#L1425) is
  the same). With `LIMIT 10`, the command stops after ten common members.
- **Rillcache** applies `LIMIT` after computing the whole intersection:
  `set_intercard` clones both sets, retains the full intersection and returns
  `result.len().min(limit)`
  ([`collection_commands.rs#L3200`](../../src/engine/collection_commands.rs#L3200));
  `zset_intercard` builds the full `BTreeSet` intersection and returns
  `members.len().min(limit)`
  ([`#L4492`](../../src/engine/collection_commands.rs#L4492)). A `LIMIT 10`
  therefore does exactly as much work as no limit.
- **Measured (2026-11-13).** Two 100,000-member sets and two 100,000-member zsets
  with identical membership (every member common), 20 repetitions, three
  rotations, 10-member controls:

  | Command | Rillcache | Redis 8.10.2 | Valkey 9.1.2 |
  | --- | ---: | ---: | ---: |
  | `SINTERCARD 2` (no limit) | 53.26 ms | 40.10 ms | 30.00 ms |
  | `SINTERCARD 2 ... LIMIT 10` | **50.72** | 0.198 | 0.247 |
  | `SINTERCARD 2 ... LIMIT 0` | 55.24 | 42.29 | 27.54 |
  | `ZINTERCARD 2` (no limit) | 267.35 | 43.35 | 24.45 |
  | `ZINTERCARD 2 ... LIMIT 10` | **267.22** | 0.217 | 0.194 |
  | `ZINTERCARD 2 ... LIMIT 0` | 291.02 | 39.77 | 27.79 |
  | 10-member `LIMIT 10` controls | 0.20–0.21 | 0.17–0.23 | 0.17–0.27 |
  | `EXISTS`/`PING` controls | 0.13/0.18 | 0.20/0.22 | 0.25/0.17 |

  `LIMIT 10` is **205–256×** behind for sets and **1,231–1,377×** for zsets,
  because the limit does not reduce the work at all; `LIMIT 0` (unlimited) has
  the same cost as no limit, as the contract requires. The 10-member controls are
  at the floor. Preloading the four 100k keys took 16.0 s versus 1.5–1.6 s, the
  4.35 write path. Candidate 4.51, extended.

**Verdict: extend 4.51 to set cardinality and make `LIMIT` an early exit.** Probe
the smallest borrowed source, count matches, and stop at `LIMIT`; materialising
the intersection is only needed for the store/reply forms.

### 3.45 List element edits (LINSERT/LSET/LREM)

- **Redis 8.10.2 and Valkey 9.1.2** edit a quicklist in place: `linsertCommand`
  finds the pivot node and inserts a new node, `lsetCommand` finds the indexed
  node and replaces its value, and `lremCommand` unlinks matching nodes, splitting
  or merging listpack nodes only at their bounds
  ([`redis t_list.c`](https://github.com/redis/redis/blob/8.10.2/src/t_list.c);
  [Valkey](https://github.com/valkey-io/valkey/blob/9.1.2/src/t_list.c) is the
  same). A head insert, an indexed replace and a tail removal are O(1) to O(nodes),
  not O(elements).
- **Rillcache** flattens the list first: `list_insert`, `list_set` and `list_remove`
  all call `Collection::list_values` (which clones every element into a
  `VecDeque`, [`data.rs#L41`](../../src/engine/data.rs#L41)) and then re-encode the
  whole list ([`collection_commands.rs#L2698`](../../src/engine/collection_commands.rs#L2698),
  [`#L2750`](../../src/engine/collection_commands.rs#L2750),
  [`#L2820`](../../src/engine/collection_commands.rs#L2820)). Every edit pays the
  full 200k-element copy and rebuild, even a head insert or an indexed replace.
  `LPOS ... RANK -1` repeats the flatten ([`#L2321`](../../src/engine/collection_commands.rs#L2321)).
- **Measured (2026-11-13).** One 200,000-element list of distinct 8-byte values, a
  10-element control, 20 repetitions, three rotations:

  | Command | Rillcache | Redis 8.10.2 | Valkey 9.1.2 |
  | --- | ---: | ---: | ---: |
  | `LINSERT ... BEFORE` at the head | **19.06 ms** | 0.09 ms | 0.24 ms |
  | `LSET` at index 100,000 | **16.46** | 0.14 | 0.28 |
  | `LINSERT ... BEFORE` at the middle | 21.04 | 3.51 | 4.33 |
  | `LREM` 1 near the tail | 21.09 | 6.97 | 7.28 |
  | `LPOS ... RANK -1` | 1.59 | 0.16 | 0.27 |
  | 10-element `LINSERT`/`LSET` controls | 0.30/0.32 | 0.33/0.21 | 0.24/0.16 |
  | `LPUSH` control | 0.22 | 0.08 | 0.32 |

  The O(1)-to-O(nodes) forms are **79–212×** behind (head insert) and **59–118×**
  (indexed replace); the scan-bound forms are 4.9–6.0× (middle insert) and
  2.9–3.0× (tail removal) because the C servers also walk 100k–200k entries, so
  the rebuild is only part of their cost. `LPOS` with a reverse rank is 5.9–9.9×.
  The 10-element controls are at the floor. Preloading the list took 3.9 s versus
  0.3–0.5 s, the 4.35 write path. Candidate 4.54.

**Verdict: adapt in-place list edits (4.54).** Walk the chunk deque to the pivot or
index, edit one element or node, and split/merge only at chunk bounds; reuse
4.41's chunk traversal and 4.35's delta accounting instead of flattening.

### 3.46 Set membership reply (SMEMBERS)

- **Redis 8.10.2 and Valkey 9.1.2** iterate the set's dict/intset/listpack and
  append each member to the reply; the order is unspecified. Rillcache's
  `set_read` also collects every member through `Collection::set_items`, sorts the
  `Vec`, and encodes each one
  ([`collections.rs`](../../src/engine/collections.rs#L25),
  [`data.rs#L49`](../../src/engine/data.rs#L49)), i.e. the same collect+sort+encode
  shape as the hash listings in section 3.31.
- **Measured (2026-11-13).** Sets of 1,000, 10,000 and 100,000 8-byte members, 20
  repetitions, three rotations:

  | Command | Rillcache | Redis 8.10.2 | Valkey 9.1.2 |
  | --- | ---: | ---: | ---: |
  | `SMEMBERS` 100k | 84.75 ms | 88.71 ms | 86.37 ms |
  | `SMEMBERS` 10k | 9.95 | 9.33 | 9.58 |
  | `SMEMBERS` 1k | 0.97 | 1.11 | 1.24 |
  | `SMEMBERS` 10-member control | 0.10 | 0.33 | 0.33 |
  | `EXISTS`/`PING` controls | 0.08 | 0.33/0.49 | 0.09/0.20 |

  At 100k the reply (~1.5 MB) dominates all three servers and Rill leads by 2–4%;
  at 10k the collect+sort is within 4–7% of parity and at 1k it is inside the
  host spread. The 10-member control is at the floor.
- **Why this matters for 4.29.** The hash candidate's 100k `HGETALL` costs
  132 ms for a roughly twice-larger reply, while the same collect+sort shape on
  sets costs 85 ms. That bounds what a sort-and-encode-only path can achieve on
  this host; the hash gap is not explained by sorting alone, and 4.29 must not be
  generalized to `SMEMBERS`.

**Verdict: measured lead/parity; no candidate.** Keep the deterministic set
order and do not extend 4.29 to `SMEMBERS`; the hash path stays the scoped
target.

### 3.47 Multi-key commands and the argument cap

- **Redis 8.10.2 and Valkey 9.1.2** parse a multi-bulk array from the declared
  count and accept large commands; in the same probe, `MGET` with 10,000 keys,
  `MSET` with 600 pairs and `DEL` with 2,000 keys all executed. Their parser does
  not impose a 1,024-argument ceiling
  ([`redis networking.c#L3304`](https://github.com/redis/redis/blob/8.10.2/src/networking.c#L3304);
  [Valkey](https://github.com/valkey-io/valkey/blob/9.1.2/src/networking.c) is
  the same). The frame and `proto-max-bulk-len` budgets, not an argument count,
  bound the request.
- **Rillcache** sets `max_arguments: 1024` in the default `Limits`
  ([`protocol.rs#L17`](../../src/protocol.rs#L17)) and rejects a larger array with
  `-ERR Protocol error: too many arguments`. There is no CLI override. The parser
  already bounds memory by `max_frame_bytes` (2 MiB default), so the fixed count
  is stricter than the byte budget: a 90 KB `MGET` with 10,000 keys is refused
  while a 2 MiB frame is accepted.
- **Measured (2026-11-13).** Boundary probe, one command per connection:

  | Request | Rillcache | Redis 8.10.2 | Valkey 9.1.2 |
  | --- | --- | --- | --- |
  | `MGET` 1,023 keys (1,024 args) | accepted | accepted | accepted |
  | `MGET` 1,024 keys (1,025 args) | **rejected** | accepted | accepted |
  | `MGET` 10,000 keys | **rejected** | accepted | accepted |
  | `MSET` 600 pairs (1,201 args) | **rejected** | accepted | accepted |
  | `DEL` 2,000 keys | **rejected** | accepted | accepted |

  The accepted limit is exactly `max_arguments` total arguments (command plus
  keys), so a 1,000-key `MGET` fits but a 1,000-pair `MSET` (2,001 args) does
  not. The C servers execute all of them.
- **Multi-key latency at 1,000 keys** (within the cap; 120k-key keyspace, 20
  repetitions, three rotations):

  | Command | Rillcache | Redis 8.10.2 | Valkey 9.1.2 |
  | --- | ---: | ---: | ---: |
  | `MGET` 1,000 | 1.74 ms | 2.23 ms | 2.34 ms |
  | `MSET` 500 pairs | 1.18 | 0.58 | 1.08 |
  | `EXISTS` 1,000 | 0.88 | 0.42 | 0.95 |
  | `TOUCH` 1,000 | 0.98 | 0.93 | 0.62 |
  | `DEL` 1,000 | 1.63 | 0.98 | 1.01 |
  | 10-key controls | 0.16–0.37 | 0.17–0.31 | 0.33–0.53 |

  `MGET` leads; the write forms are 1.6–2.1× behind Redis but within ~1.1× of
  Valkey and inside the host spread, so no latency candidate. The actionable
  finding is the cap, not the per-command cost. Candidate 4.55.

**Verdict: adapt a frame-derived argument cap (4.55).** Replace the fixed 1,024
with a bound derived from `max_frame_bytes` (for example `max(1024, min(1M, frame/6))`)
so legitimate large multi-key commands parse while memory stays bounded by the
frame; keep the frame and bulk limits as the request budget.

## 4. Candidate architectures

Each candidate names its mechanism, expected effect, cost/risk and acceptance
evidence. Section 4.1 and the mode-word work (4.3) are implemented; 4.2 is closed
by bound, 4.5 is closed for lack of a target row, 4.6 is deferred, 4.7 is a
rejected experiment, 4.8 is merged with its reactor count rejected, 4.9 is closed
by bound (a TTL workload may reopen it), 4.10 is closed by scope and bound, 4.11
is the historical wakeup/spin diagnostic, 4.12 is its merged
owner-local-reactor + bounded-poll implementation, 4.13 is the closed
adaptive-poll direction (its naive and hybrid variants were rejected, and the
remaining CPU is the measured P1 trade), and 4.14 is the rejected io_uring send
attempt. This refresh adds 4.15 (the only surviving new candidate: large-bulk
reply copy avoidance), 4.16 (the out-of-gate measurement program that must run
before 4.15), 4.17–4.18 (two ideas rejected by bound: command-lookup caching and
incremental rehash), and 4.19 (a latency-attribution diagnostic for the remaining
P1 write-tail question) plus 4.20 (a conditional storage candidate that only
follows if 4.19 isolates that tail). 4.21 is the admitted persistence-on
candidate for the journal write shape in section 3.9, and 4.22 is the conditional
same-pass wakeup candidate for blocking commands in section 3.12. 4.23 is the
conditional rewrite-start candidate for the fork-versus-slices trade in section
3.13, and 4.24 is the event-driven async-client candidate for pub/sub and
MONITOR in section 3.12. 4.25 is the large-frame input-reservation candidate in
section 3.3, 4.26 is the pooled/batched eviction candidate in section 3.10, and
4.27 is the startup-replay fast path in section 3.15. 4.28 maps the admitted
candidates to call sites, invariants and implementation priority, and 4.29 is
the large-hash `HGETALL` reply candidate in section 3.2. 4.30 is the sorted-set
ordered-index candidate for the measured 19.5–20.4× small-range loss in the same
section. 4.31 is the incremental cursor for the measured 167–260× per-call
`SCAN` loss in section 3.16. 4.32 and 4.33 cover the measured large-collection
RSS and deferred-drop gaps in section 3.17. 4.34 is the in-place string mutation
candidate for the measured 39–75× `APPEND`/`SETRANGE` loss on a 32 MB value in
section 3.18; it merged as PR #258 (`d334cde`) and the post-fix cohort measures
0.13–0.19 ms. 4.35 is the in-place collection mutation candidate for the measured
3.5–262× write loss in section 3.19. 4.36 is the constant-time random-member
candidate for the measured 35–298× sampling loss in section 3.20. 4.37 is the
bitmap candidate for the measured 5–17× `SETBIT`/`BITCOUNT` loss in section 3.21.
4.38 is the rank-select candidate for the measured 378–927× sorted-set rank and
range loss in section 3.22. 4.39 is the contract-correct pub/sub fan-out
candidate for the measured 2,000-message burst loss and 10k-channel scan in
section 3.23. 4.40 is the O(1) cardinality/membership candidate for the measured
36–55× `SISMEMBER`/`SCARD` loss in section 3.24. 4.41 is the in-place list-seek
candidate for the measured 9–22× `LINDEX`/`LPOS` loss in section 3.25. 4.42 is
the in-place stream-append candidate for the measured 13–18× `XADD` loss in
section 3.26. 4.43 is the geospatial neighbor-cell candidate for the measured
10–53× search and point-lookup loss in section 3.27. 4.44 is the per-member
direct-probe candidate for the measured 30–1461× multi-member lookup loss in
section 3.28. 4.45 is the bounded-lazy-expiry candidate for the measured 37–76×
mass-expiry stall in section 3.29. 4.46 is the direct `BY nosort` slice candidate
for the measured 50–66× loss in section 3.30. Section 3.31 extends 4.29 to
`HKEYS`/`HVALS` and 4.31 to `KEYS`/`RANDOMKEY` at 100k. 4.47 is the HyperLogLog
in-place-register and cached-count candidate for the measured 4× add and 74–80×
count throughput losses in section 3.32. Section 3.33 extends 4.37 with a
word-wise `BITPOS` scan. 4.48 is the in-place `XGROUP CREATE` candidate for the
measured 88–99× group-creation loss in section 3.34. 4.49 is the ordered-pop
candidate for the measured 373–974× sorted-set pop loss in section 3.35. 4.50 is
the seek-based lex-range and span-count candidate for the measured 163–371×
`ZRANGEBYLEX` and 5–71× `ZLEXCOUNT`/`ZCOUNT` losses in section 3.38; it reuses
4.30/4.38's ordered index. 4.51 is the borrowed-source zset-algebra candidate for
the measured 5.5–12.9× `ZINTERCARD` and 2.8–5.4× store losses in section 3.41;
section 3.44 extends it to the set path and the `LIMIT` early exit
(205–1,377×).
4.52 is the maintained-keyspace-counter candidate for the measured 16–40×
`INFO` walk in section 3.42. 4.53 is the in-place two-key-move candidate for the
measured 64–489× `SMOVE`/`LMOVE`/`RPOPLPUSH` loss in section 3.43; it reuses
4.35's in-place mutation and 4.41's chunk access. 4.54 is the in-place list-edit
candidate for the measured 59–212× `LINSERT`/`LSET` loss in section 3.45. 4.55
is the frame-derived argument-cap candidate for the measured 1,024-argument
rejection in section 3.47.

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

### 4.2 Resumable RESP parse state (closed by bound)

The original concern was repeated scanning of a large incomplete frame. Source
inspection showed that the parser does not scan payload bytes while incomplete:
it re-reads only the small length headers and checks buffer length, so a single
large value is not quadratic in bytes. The remaining cost is proportional to the
number of argument headers per read, bounded by `max_arguments` and by the
64-byte header width; 16 reads of a 64-byte header is about 1 KiB of extra
scanning, which is below the per-command noise even for a large value. A
resumable cursor would add state and branches to the hot path without a measured
workload showing a problem. **Closed**; do not reopen without a measured
multi-megabyte workload that shows a real scan cost. The existing complete-frame
fast path stays.

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

### 4.5 Cross-client write coalescing (closed: no target row)

PR #152 batches owner replies without blocking the reactor, which covers the
multi-owner reply hop. Cross-client write coalescing on the default reactor
(Valkey's `beforeSleep` shape) is **closed**: at pipeline 1 only one client is
ready per pass, so a cross-client batch has nothing to coalesce on the row it
would target, and at pipeline 16 the current per-client batch already leads by
54–153% (section 2.19). Writing inside each service call is the lower-latency P1
choice. Do not reopen without a measured multi-ready-client workload that shows
write syscalls as the remaining cost.

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

The implemented parallel mode keeps one owner-local executor, maintenance and
every data command; workers accept disjoint connections from the shared listener.
The round-robin accept-distribution experiment below was rejected; it is not the
current `serve_shared` implementation. The earlier `min(4, allowed CPUs)` default
was replaced by one owner-local reactor plus bounded polling (section 4.12), with
explicit `--socket-workers 2..16` retaining parallel connection-local I/O. The
`2 × allowed CPUs` count, capped at eight, lost pipeline-1 ping (section 2.8), as
did the later `ceil(CPUs/2)` default (section 2.13). Other reactors execute only
bare connection-local commands (PING/ECHO/QUIT). Before the first data command a worker
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
fresh evidence. Candidate 4.12 takes that route: it defaults to the owner reactor
alone and adds the bounded readiness poll, leaving this section as the opt-in
worker mechanism. Both protected gates plus the unchanged functional contexts
decide every count change; no throughput claim is made before they pass.

### 4.9 Cached command-time snapshot (closed by bound; TTL workload may reopen)

Mechanism: publish one wall-clock millisecond value per reactor pass and use it
for expiry/eviction checks instead of `SystemTime::now()` per command, matching
the `commandTimeSnapshot()` shape without changing single-command time
consistency ([section 3.7](#37-clock-expiry-and-command-time-snapshots)).

Bound: a `SystemTime::now()`/`clock_gettime` on this platform is a vDSO call of
roughly 20–30 ns, about 0.2–0.35% of an 8.7 µs P1 command — inside the gate's
1.5–3% noise even before considering that the default gate has no TTL keys and
`now()` already returns `0` while `expirations` is empty. The C servers' cached
time is a real shape difference, but their per-command cost is dominated by other
work, and copying the shape here cannot pass the admission rule.

**Verdict: closed as a performance candidate.** It is not worth a runtime branch
on the gate path. Reopen only if a matched TTL-heavy workload (every key has a
deadline, measured on one host with rotations) shows a clock share above the
noise; then implement it behind the existing `Clock` trait, refreshed once per
reactor pass, with tests that a key alive at command start cannot expire inside
that command and that injectable test clocks still work. No dependency is needed
either way.

### 4.10 Command-counter padding A/B (closed by scope and bound)

Mechanism: PR #165 changed the 64 sharded `total_commands_processed` atomics from
packed 8-byte entries to 128-byte-spaced entries. The two post-refresh cohorts
passed both four-CPU gates with larger P1 ping margins (section 2.10), but a
gate pass cannot separate the layout effect from shared-runner variance.

**Scope correction.** Those cohorts ran the four-reactor default of that era. The
current default is one owner-local reactor (4.12), so every increment has one
writer and there is no cross-reactor cache line to ping-pong. The layout's effect
on the protected default rows is therefore exactly zero by construction; it can
only matter with `--socket-workers 2..16` or an opt-in multi-owner mode.

**Multi-reactor bound.** With `N` reactors and eight clients packed into one
64-byte line, a counter increment can pay up to `N-1` extra coherence transfers;
at ~50–100 ns each and four reactors that is 150–300 ns of an 8.7 µs command
(1.7–3.4%), which is large enough to matter only on a gate-rate host. A shared
k3s host cannot resolve it (the 64-byte controls measured ~20–33x low, section 2.21),
and spending the protected runner on a layout the default no longer uses is not
justified.

**Verdict: keep the padded layout and close the A/B.** The layout is already
merged, costs 8 KiB and cannot hurt the default; it helps only if a future
candidate makes parallel connection-local reactors the default. Reopen the A/B
only then, with frozen packed and padded binaries, `--socket-workers 1` and `4`
controls, one host, three rotations and raw samples. Prediction for that future
run: no difference with one reactor; a padded win with four only if the
false-sharing mechanism is real. Do not cite section 2.10 as evidence for the
current default's P1 margins.

### 4.11 Reactor wait diagnostic (consolidation rejected; bounded poll in 4.12)

**Source-resolution correction:** Mio 1.2.4's Linux selector rounds a positive
sub-millisecond timeout upward: `FAST_WAIT` requests 8 us but reaches
`epoll_wait` as 1 ms, not a user-space spin. Earlier "spin" descriptions above
are hypotheses, not measured spin time. The single-poll candidate instead moves
the absorption wait to the next round's sole selector call, avoiding a second
ZERO poll before servicing readiness. Its first global trial was rejected:
PING/P1 was -0.09% and PING/P16 p99 +31%, despite GET/P1 +3.14% (run
41500430375). The combined co-location/single-reactor consolidation trial (run
41507264904) also lost 5.61% in explicit-one PING/P1 and 5.09% in explicit-four
PING/P16, despite its default-mode gains. It was rejected too. The zero-time
poll can gather more readiness and affect batching, so it is not necessarily
redundant. After diagnosis review, the co-location-only candidate restores the
original reactor algorithm entirely; only the default count changes. The
[matched hosted diagnostic](../development-cycle.md#same-job-hosted-reactor-diagnostic)
is distinct from the isolated fixed-host screening lane and from protected gates.

**Historical, rejected selector hypothesis:** section 2.11 measures about 0.086 voluntary plus 0.014 involuntary
context switches per P1 ping, against ~0.0001 for the single-thread servers,
while Rillcache still wins rate and p99. The then-current 8 µs readiness wait
after a productive pass and the owner/worker wakeups are the plausible sources.
Candidate 4.12 later implemented the measured version: one owner-local reactor
plus a bounded zero-timeout poll instead of a blocking wait. This entry stays as
the diagnostic rationale and rejection rule that 4.12 inherits. Section 2.12
records why another default reactor count alone is not an acceptable substitute
for this diagnostic.

Prediction: the voluntary-switch rate and P1 ping CPU% fall materially, with
rate and p99 unchanged or better. The effect should be strongest with several
connection-local clients per reactor and absent with one client or one reactor.
A negative result (fewer switches but lower rate or worse p99) means the spin is
buying latency and must stay.

Unrun deeper-profile proposal (not the hosted /proc evidence): freeze current
and candidate binaries, alternate on one
host and one CPU placement, use the same pinned generator and at least three
rotated repetitions, and retain per-command scheduler counters
(`sched_switch`, `epoll_wait`, receive/send-class syscalls), raw JSON and spreads.
Run default and explicit `--socket-workers 1` / `4` controls. Do not compare
numbers from different hosts or substitute a single local screen for a gate.

Rejection condition: any original-gate loss, any Valkey/Dragonfly four-CPU loss,
a P1 p99 regression beyond AC5, or no measured reduction in the switch/CPU
counters. The earlier drain-probe experiment (4.7) shows that removing a
user-space readiness batch can lose more than it saves, so this candidate must
keep the absorption path when it is doing useful work.

Acceptance if implemented: deterministic reactor scheduling tests, no lost
wakeups or ordering regressions, joined shutdown, the matched A/B above, and both
protected gates on the final head. Neither selector trial qualified; the final
co-location-only change in section 4.8 keeps this selector behavior unchanged.

### 4.12 Owner-local reactor default with a bounded readiness poll (merged in PR #172)

Mechanism: plain Unix TCP without an explicit count runs the single owner-local
reactor. `--socket-workers 2..=16` still opts into shared-store connection-local
workers, which continue to execute only PING/ECHO/QUIT and migrate data
connections to the owner. After a productive pass with an empty runnable queue
and no pending accept, the reactor polls readiness with a zero timeout for at
most 20 µs before parking for the normal stop tick; the window is renewed only by
serviced work.

Prediction: at four CPUs the P1 PING row stops paying a split-set wakeup and a
blocking `epoll_wait` arrival per command. It should join the single-owner data
rows at roughly 155–160k req/s, and the bounded poll should remove one sleep/wake
pair from the generator round trip, roughly halving voluntary switches per command
relative to the one-reactor blocking wait (and cutting them about fivefold
relative to the two-reactor default) while raising P1 PING further. P1 data rows should improve for
the same reason, P16 rows should be unchanged because their runnable queue is
never empty, and the one-CPU gate should keep one reactor while losing its
blocking wakeup. An idle server must still park within one 20 µs window and use
no measurable CPU.

Baseline and method: frozen current and candidate binaries, one host and CPU
placement, the pinned official `valkey-benchmark` with the gate's own arguments,
alternating repetitions, both explicit counts, and raw P1/P16 JSON with server
CPU and context switches. The local host is shared, so the screen shows mechanism
and direction only; only the protected gates accept the change.

Rejection condition: any original-gate loss, any Valkey or Dragonfly four-CPU
loss, a P1 batch-p99 regression beyond AC5, no reduction in per-command wakeups
or CPU, or a P16 collapse. Revert to the previous default rather than adding a
benchmark-specific mode or weakening a gate.

Acceptance if the gates pass: deterministic unit/wire tests for the selected
count and the existing worker modes, correct shutdown while a poll window is
active, current-base protection, all 14 required contexts, and fresh protected
performance evidence on the exact head. The count is reported through
`scheduling_policy()`. The bounded poll is a fixed, documented policy constant,
not a per-workload switch.

Result: PR #172 merged as `21ed301` after all 14 required contexts passed. The
final head `c4d4698` (run 41552051338) passed Valkey 10/10 with P1 ping
116,791.9 vs 113,242.0 (+3.13%) and Dragonfly 10/10 with 111,899.4 vs 94,574.5
(+18.32%); the original gate passed. The earlier two-reactor candidate and the
single-reactor blocking-wait candidate remain retained as failed evidence.

### 4.13 Adaptive bounded poll for P1 CPU/variance (closed: CPU is the P1 trade)

Mechanism: the merged 20 µs window is fixed. The clean-runner cohort (section
2.14) shows it wins rate but costs about 95% of one CPU at P1 ping versus ~82%
for Redis/Valkey, with a 1.89% coefficient of variation versus 0.16–0.37%.
Candidate 4.13 must reduce the cost of **successful** windows, because that is
where the measured CPU goes: the reactor stays runnable because the next request
usually arrives inside the window. A viable mechanism therefore has to be
client-RTT-aware or use a per-connection absorption budget; simply shrinking
after empty windows does not touch the fast path.

**Rejected before implementation: a miss-based adaptive window.** Shrinking the
window only after several empty windows leaves the measured 95% CPU unchanged,
because successful windows keep resetting it. Shrinking on hits, or using a
shorter fixed window, would cut CPU by reintroducing the sleep/wake pair for
requests that arrive late in the window, risking the very rate and p99 the window
exists to protect. Do not implement a miss counter or a fixed-window change as an
efficiency fix.

**Measured rejection: hybrid busy-prefix + timed-wait poll.** A diagnostic branch
(`f7cc8b9`) kept a 5 µs zero-timeout busy prefix and made the remaining 15 µs one
timed sleeping `epoll` wait. On the clean advisory runner (run 41612942833,
180/180 samples) it did not reduce CPU and it badly worsened the tail:

| P1 ping | Hybrid | Redis | Valkey |
| --- | ---: | ---: | ---: |
| Rate | 58,847 | 49,432 | 49,591 |
| Server CPU % | 95.5 | 83.7 | 85.0 |
| p99 ms | 0.959 | 0.231 | 0.231 |

P16 ping p99 was also worse (1.095 versus 0.407/0.463 ms). The host was slower
than the baseline cohort, so absolute rates are not comparable, but CPU did not
fall from the baseline 95.1% and the ~4x P1 p99 regression is decisive. The timed
wait reintroduced exactly the wakeup latency the busy window removes. Reject this
shape; a viable 4.13 must not rely on sleeping inside the window.

Prediction for a genuine RTT-aware candidate: P1 ping CPU falls toward the
C-server range (about 85%) and the run-to-run spread narrows, while P1 rate, p99
and P16 throughput stay within AC5. The effect should be absent in P16, where the
runnable queue never empties, and in one-CPU mode the lower CPU use may improve
rate if the generator shares the core.

Baseline and method: freeze current and candidate binaries, alternate on one
clean host, use the official generator at the gate's 8 clients and the 16-client
upstream harness as a secondary check, and retain per-sample rate, p99, server
CPU and scheduler counters. A local shared VM is a diagnostic only. Measure both
the default one-reactor mode and an explicit `--socket-workers 2` mode so the
window logic is not overfit to one topology.

Rejection condition: any original-gate loss, any Valkey/Dragonfly four-CPU loss,
a P1 p99 regression beyond AC5, no measurable CPU or variance reduction, or a P16
regression beyond the existing spread. The fixed 100 µs variant was already
rejected (section 2.13); a candidate that merely lengthens the window is not a
substitute for adaptivity.

Acceptance if implemented: deterministic tests for the window state machine and
shutdown during a window, no lost wakeups or ordering changes, the matched A/B
above, and both protected gates on the final head. Report CPU and p99 even when
throughput passes; do not claim a tail or efficiency win from medians alone.

**Verdict: close the poll-tuning direction.** The miss-based and fixed-window
variants are rejected analytically, and the hybrid busy/sleep split is rejected
on measurement. The busy-window CPU and its variance are the direct cost of
keeping the reactor runnable for successful windows; the local tracepoint count
attributes about eight `epoll_wait` calls per command to the zero-timeout spin
(section 2.16), but the hosted gate-rate profile measures only 0.20–0.36 per reply
and fewer context switches than Valkey (section 2.23), so the local eight is not
a gate-rate cost. Any mechanism that stops burning it reintroduces the sleep/wake
pair that the window exists to remove.
The headline 95% total CPU is mostly the higher measured rate, with a small
~5–9% ping-specific per-command difference on the two clean cohorts (section
2.14). Reducing the busy-window cost without losing rate or p99 would need kernel
batching or busy-poll socket options (`recvmmsg`/`sendmmsg`, `io_uring`,
`SO_BUSY_POLL`), which are outside
the safe-Rust/Mio boundary. Until that policy changes or a different topology is
measured, accept the P1 busy-window cost as the deliberate price of the P1 win
and do not open another poll-window knob. Reopen only with a genuinely new
mechanism, not a repackaged window length.

### 4.14 Completion-batched sends through an io_uring crate (rejected: slower and corrupt)

Section 4.13 closed poll tuning and named kernel batching (`recvmmsg`/`sendmmsg`,
`io_uring`, `SO_BUSY_POLL`) as the only remaining lever, outside the
safe-Rust/Mio boundary. This section records the one such mechanism that was
actually built and measured, so the boundary is not re-litigated from memory.

**Mechanism.** Replace the per-reply `write` syscall with `IORING_OP_SEND`
submissions queued by the reactor and flushed with one `io_uring_enter` per pass;
`Client::flush` defers while a ring is present, one send per socket stays
outstanding so reply order cannot change, and completions are harvested after the
service batch. The unsafe code lives in a new in-repo crate `crates/rillcache-io`
with a safe surface (`SendRing::new/submit_send/flush/next_completion/wait`), a
single issuer and no registered buffers; the application crate keeps
`#![forbid(unsafe_code)]` and a policy test asserts that no other file contains
`unsafe` and that the allowlist names only that crate. A `README.md` and
`AGENTS.md` in the crate state the ABI, ownership and failure contract. The crate
was validated against a 2,000-send partial-write stress test that compares the
peer's byte stream with the submitted payloads.

The implementation surfaced three real defects before it could be measured:
`io_sqring_offsets.ring_mask` is an **offset into the mmap**, not the mask value;
`io_uring_enter` must submit `tail - kernel_head`, because a private counter makes
a lagging kernel head re-execute an older SQE whose buffer has been reused; and a
deferred flush must not let the idle timeout close a connection whose reply is
still queued.

**Measurement** (diagnostic branch `diagnostic/io-uring-send`, run 41932571815,
108/108 revalidated samples, baseline the merged one-reactor default):

| Row | vs baseline | Server CPU (s / 10 s) |
| --- | ---: | ---: |
| PING P1 | **-5.47%** | 9.99 -> 10.00 |
| GET P1 | **-6.90%** | 10.00 -> 10.00 |
| GET P16 | **-13.58%** | 10.00 -> 10.00 |
| PING P16 | -1.30% | 9.99 -> 9.99 |

P1 p99 regressed 18-20% and GET/P16 p99 6%. Server CPU is identical in every
default and one-worker row: eliminating one `write` per request saves nothing,
because TCP processing dominates, while the payload copy into the ring and the
completion round trip cost one extra reactor pass in the latency-sensitive rows.
The stress test also still shows intermittent buffer/SQE aliasing, so the unsafe
path fails correctness as well as speed.

**Verdict: reject and do not promote.** There is no CPU to recover and no
correctness argument for shipping an unsafe transport that can place one reply's
bytes in another reply's position. The crate, wiring, policy test, docs and all
evidence remain on `diagnostic/io-uring-send` only; the application runtime is
unchanged. Cross-socket batching is also unavailable in `recvmmsg`/`sendmmsg`
(both take a single `sockfd`), and a read/write split across two threads measured
slower on an 8-connection P1 probe (2,957 and 2,286 req/s versus 3,372 and 4,907
single-threaded). With poll tuning, extra reactors, clock coalescing, direct
receive buffers, the read/write split and now io_uring send batching all measured
or proven inapplicable, `ping`/P1 remains a parity row: the five-round gate's
paired ratio is 0.9989 with a 95% interval of [0.9802-1.0128]. Closing it needs a
mechanism that changes the syscall or TCP work itself, not the reactor's
bookkeeping.

### 4.15 Large-bulk reply copy avoidance (candidate; admission measured)

**Mechanism.** Keep values below a 16 KiB threshold in the existing
capacity-reusing `Vec<u8>` (so the gate path does not change), and store values at
or above it as an `Arc<[u8]>` behind the same entry. On GET, clone the `Arc` under
the read lock and release the lock before writing; emit the bulk header with
`write_vectored` (`IoSlice` for header, value and CRLF) instead of copying the
value into the session output `Vec`. If the non-blocking socket returns
`WouldBlock`, keep the `Arc` and the written offset in the client and register
writable interest; do not service that client's later replies until the large
tail drains, which preserves reply order using the existing backpressure shape.
This is exactly the shape Redis and Valkey implement with refcounted objects
(section 2.20), but with `std::sync::Arc`, `std::io::IoSlice` and
`Write::write_vectored` — no dependency, no `unsafe`, no allocator change.

**Prediction.** One N-byte copy disappears from each large GET: about 100–200 µs
per 1 MiB at 5–10 GB/s and 6–13 µs per 64 KiB, with no change at 64 bytes because
the threshold is not crossed. Throughput and p99 on a large-value workload rise;
the protected 64-byte gate rows are unchanged and must remain so. Large
overwrites trade the current capacity reuse for a new immutable `Arc`, so the
screen includes SET as well as GET; a write-side regression rejects the candidate.

**Admission evidence (2026-11-12, section 2.21).** On a quieter pass of the same
host, 64 KiB GET is 28.4% behind Valkey (p99 3.93 vs 1.21 ms) and 1 MiB GET is
**42.1% behind Redis and 44.9% behind Valkey** (p99 24.9 vs 9.9/10.9 ms), while
the 64-byte control leads. The gap crosses between 64 B and 64 KiB and grows with
value size, matching the copy mechanism; the SET loss is handled separately by
4.25 because a SET reply is small. Admission is met; implementation still needs
the ordering/backpressure obligations and AC20.

**Admission and baseline.** Freeze the current and candidate binaries, alternate
them on one host with the same pinned generator, and run 64 B (control), 64 KiB
and 1 MiB GET/SET at P1 and P16 with at least three rotated repetitions, raw
JSON, p99 and CPU.

**Rejection condition.** No large-GET gain above the cohort spread; any original
or four-CPU gate row loss; any ordering, partial-write, slow-reader or shutdown
failure; any reprocessing of a reply; or a memory increase beyond the existing
payload accounting. Revert rather than adding a size knob or a benchmark mode.

**Acceptance (AC20).** Values below the threshold keep the current single-copy
path; large values are written without a user-space copy; **at 1 MiB GET,
throughput shall be within 5% of the best reference and p99 no worse than 1.25×
it** (measured baseline −42% to −45% and 24.9 vs 9.9/10.9 ms); every reply is
delivered exactly once and in order under fragmentation, pipelining, slow readers
and half-close; payload/expiry/backpressure accounting is unchanged; the matched
large-value A/B shows the predicted gain; and both protected gates pass on the
exact head.

### 4.16 Out-of-gate workload measurement program (next evidence)

The protected gate measures five small-string commands with persistence off. The
remaining risks are therefore *unmeasured* rather than proven losses, and the
existing single-host screens cannot resolve them on this shared k3s host (section
2.21). This program defines the evidence the next runtime candidate needs; it is a
measurement increment, not a runtime change.

Required workload rows, each against pinned Redis 8.10.2 and Valkey 9.1.2 with
the same generator, host, CPU placement and at least three rotated repetitions:

1. **TTL-heavy P1:** every key has a deadline; SET with EX and GET on hit/miss.
   Tests the clock path that the no-TTL gate never executes (candidate 4.9) and
   reports RSS as the signal for the duplicated expiry key (section 3.11).
2. **Large values:** 64 KiB and 1 MiB SET and GET at P1 and P16. Confirms 4.15
   (reply copy) and 4.25 (input growth) on a clean runner; the 2026-11-12 local
   retry in section 2.21 already measures the gap.
3. **Large multi-bulk replies:** a 1,000-element list/hash/set read, and a 1 MiB
   single bulk, at P1. The 2026-11-12 local cohort in section 3.2 already measures
   a 10k `LRANGE`/`HGETALL`; this row confirms on a clean runner and covers the
   hash path admitted as 4.29 and the sorted-set rank ranges admitted as 4.30.
4. **Mixed pipeline:** alternating PING/SET/GET/INCR in one 16-command batch.
   Tests dispatch and per-command overhead without a single-command cache.
5. **Sorted-set ranks:** a 10k-member zset with `ZRANGE 0 9`, `ZRANGE 0 9999`,
   `ZSCORE` and a write-heavy `ZADD` control at P1. Confirms 4.30 and the
   write-cost trade on a clean runner; the 2026-11-12 local cohort in section 3.2
   already measures the read gap (19.5–20.4×) and the 128/1,024 controls.
6. **Persistence-on writes:** append-only `everysec` SET at P1, to locate journal
   stalls that the gate's persistence-off rows cannot show (the AOF/streams
   increments since #194 have their own tests; this row is the performance
   counterpart).
7. **Eviction pressure:** `maxmemory` set just below the working set,
   `allkeys-lru`, and a read-heavy mixed GET/SET stream. Confirms 4.26 on a
   clean runner; the 2026-11-12 local cohort in section 3.10 already measures the
   steady-state penalty (−85.8% P16). Report RSS and read-hit retention as the
   quality signals.
8. **Cursor iteration:** a 100k-key `SCAN` with spread cursors at `COUNT 10` and
   `COUNT 1000`, a full traversal, and a 10k-field `HSCAN`, at P1. Confirms 4.31
   and its bounded-work claim on a clean runner; the 2026-11-12 local cohort in
   section 3.16 already measures 167–260× per call and 20–29× per traversal.
9. **Large-collection drop and RSS:** a 200k-element short-value list, `DEL`,
   `UNLINK`, `FLUSHALL ASYNC`/`SYNC` on 20k keys, and `/proc` RSS per element.
   Confirms 4.32 and 4.33 on a clean runner; the 2026-11-12 local cohort in
   section 3.17 already measures 142–143 vs 7.5 B/element, 6.2–7.3 vs
   0.13–0.32 ms unlink, and 9.2 ms ASYNC flush.
10. **String mutation:** `APPEND` and `SETRANGE` on a 1 MB and a 32 MB value,
   with 64 B controls and a `GETRANGE`. 4.34 landed in #258; the 2026-11-13
   post-fix cohort in section 3.18 measures 0.131/0.185 ms at 32 MB versus
   0.142–0.289 for the C servers. Keep the row as the regression check.
11. **Collection writes:** a single `HSET`/`ZADD`/`SADD` into 10k- and
   100k-member collections, `HDEL`/`ZINCRBY`/`ZREM`/`SREM`, `RPUSH`/`LPOP` on a
   200k list, and 10-element controls. Confirms 4.35 on a clean runner; the
   2026-11-12 local cohort in section 3.19 already measures 1.2–18.3 ms at 10k
   and 10.7–99.4 ms at 100k versus 0.07–0.60 ms.
12. **Random members:** `SRANDMEMBER`, `HRANDFIELD`, `ZRANDMEMBER` and `SPOP`
   on 10k and 100k collections, single and `COUNT 10`, with a distribution check
   and 10-element controls. Confirms 4.36 on a clean runner; the 2026-11-12
   local cohort in section 3.20 already measures 11–34 ms at 100k versus
   0.1–0.35 ms.
13. **Bitmaps:** `SETBIT` and `BITCOUNT` on 1 MB and 16 MB bitmaps, plus
   `GETBIT`/`STRLEN` and 64 B controls. Confirms 4.37 on a clean runner; the
   2026-11-12 local cohort in section 3.21 already measures 5.3/23.0 ms at
   16 MB versus 0.31–0.35/3.4–4.6 ms.
14. **Sorted-set rank and score reads:** `ZRANK`/`ZREVRANK` (first/middle/last),
   `ZRANGEBYSCORE ... LIMIT 0 10`, `ZRANGE 0 9`, deep `ZRANGE`, `ZSCORE`, and
   10-element controls at 10k and 100k. Confirms 4.30/4.38 on a clean runner; the
   2026-11-12 local cohort in section 3.22 already measures 57–72 ms at 100k
   versus 0.07–0.17 ms.
15. **Pub/sub bursts and channel scale:** a 2,000-message pipeline to one active
   subscriber (delivered count, `PUBSUB NUMSUB`), and `PUBLISH` latency with
   1, 1,000 and 10,000 subscribed channels. Confirms 4.39 on a clean runner; the
   2026-11-12 local cohort in section 3.23 already measures 1,024/2,000 delivered
   with the subscription gone, versus 2,000/2,000 alive.
16. **Cardinality and membership:** `SISMEMBER` (hit/miss), `SCARD`, `HLEN`,
   `ZCARD`, `LLEN`, with `HEXISTS`/`STRLEN` and 10-element controls, at 100k/200k.
   Confirms 4.40 on a clean runner; the 2026-11-12 local cohort in section 3.24
   already measures 8.0–8.7 ms for `SISMEMBER`/`SCARD` versus 0.15–0.22 ms.
17. **List random access:** `LINDEX` (0/middle/-1) and `LPOS` (hit-first and
   missing) on a 200k list, with 10-element controls. Confirms 4.41 on a clean
   runner; the 2026-11-12 local cohort in section 3.25 already measures
   1.26–1.47 ms versus 0.07–0.14 ms.
18. **Stream append and reads:** `XADD` at 1k and 5k entries, `XLEN`,
   `XRANGE`/`XREVRANGE` at the start and middle, and a 10-entry control.
   Confirms 4.42 on a clean runner; the 2026-11-12 local cohort in section 3.26
   already measures 1.36 ms at 5k versus 0.08–0.10 ms, with reads at the floor.
19. **Geospatial commands:** `GEOSEARCH` 1 km/5 km/2×2 km box, `GEOPOS` 1 and
   100 members, `GEODIST`, and 10-member controls at 50k members. Confirms 4.43
   on a clean runner; the 2026-11-12 local cohort in section 3.27 already measures
   4.34 ms versus 0.16–0.17 and 24.9 ms versus 0.47–0.61.
20. **Multi-member lookups:** `SMISMEMBER` and `ZMSCORE` with 1/10/100 members at
   100k, `HMGET` as a control, and 10-member controls. Confirms 4.44 on a clean
   runner; the 2026-11-12 local cohort in section 3.28 already measures
   7.9–11.8 ms and 8.0–273 ms versus 0.07–0.63 ms.
21. **Mass expiry:** 30k keys set with a 400 ms TTL, a 0.7 s wait, then one
   `MGET`/`KEYS`/`RANDOMKEY`/`DBSIZE`/`GET` per fresh wave. Confirms 4.45 on a
   clean runner; the 2026-11-12 local cohort in section 3.29 already measures
   36–46 ms versus 0.6–1.0 ms and lazy `GET` at parity.
22. **SORT:** a 100k integer list with weight keys; full, `LIMIT 0 10`, ALPHA,
   DESC, `BY nosort LIMIT`, `BY w*`, `GET #`, and 100-element controls. Confirms
   4.46 on a clean runner; the 2026-11-12 local cohort in section 3.30 already
   measures 7.96 ms for `BY nosort LIMIT` versus 0.12–0.16 ms.
23. **Large-hash listing and keyspace enumeration:** 100k `HGETALL`/`HKEYS`/
   `HVALS`, `KEYS key:*`/prefix/none, `RANDOMKEY`, `HMGET` control, and 10-member
   controls. Extends 4.29/4.31 on a clean runner; the 2026-11-12 local cohort in
   section 3.31 already measures 132/98/102 ms for the hash listings and 91 ms
   for `KEYS` versus 15.8–43 ms.
24. **HyperLogLog:** 100k pipelined `PFADD`, 10k repeated `PFCOUNT`, single-call
   p50s, and a 10-element control. Confirms 4.47 on a clean runner; the
   2026-11-12 local cohort in section 3.32 already measures 1.70 s versus 0.39–0.41
   for adds and 2.66 s versus 0.033–0.036 for counts.
25. **BITPOS:** `BITPOS 0` (not found), `BITPOS 0 0 -1`, `BITPOS 1` (first bit),
   `BITCOUNT`, and a 64 B control on a 16 MB bitmap. Extends 4.37 on a clean
   runner; the 2026-11-12 local cohort in section 3.33 already measures 201.6 ms
   versus 5.1 ms.
26. **Stream groups:** `XGROUP CREATE` on a 5k-entry stream and a 10-entry
   control, plus `CREATECONSUMER`/`SETID`/`DESTROY` controls. Confirms 4.48 on a
   clean runner; the 2026-11-12 local cohort in section 3.34 already measures
   8.26 ms versus 0.08–0.09 ms.
27. **Sorted-set pops:** `ZPOPMIN`/`ZPOPMAX`, `ZPOPMIN COUNT 10`, `ZMPOP 1 MIN`,
   and a 100-member control on a 100k set. Confirms 4.49 on a clean runner; the
   2026-11-12 local cohort in section 3.35 already measures 146–160 ms versus
   0.16–0.42 ms.
28. **Transactions:** `MULTI` + 100/1,000 `SET` + `EXEC`, a 100×1 KB
   transaction, queue-only `DISCARD`, and a plain 1,000-SET control. Confirms the
   measured null in section 3.36 on a clean runner.
29. **MONITOR bursts:** a 2,000-`PING` pipeline with 0/1/10 attached monitors,
   counting delivered lines per monitor and publisher time. Extends 4.39 on a
   clean runner; the 2026-11-12 local cohort in section 3.37 already measures
   1,024/2,000 lines versus 2,000/2,000.
30. **Lex ranges and count-only queries:** `ZRANGEBYLEX`/`ZREVRANGEBYLEX` with
   `LIMIT 10` (unbounded, mid-range, reverse), the full 100k reply,
   `ZLEXCOUNT` (full and narrow) and `ZCOUNT` (full, matching and empty) on a
   100k equal-score zset, with a 10-member control. Confirms 4.50 on a clean
   runner; the 2026-11-13 local cohort in section 3.38 already measures
   56.0–69.8 ms versus 0.18–0.74 and a narrow lex count of 15.0 ms versus
   0.21–0.22.
31. **Stream consumer-group reads:** `XREADGROUP` history and new (100 and 1000),
   `XPENDING` summary and extended (100/1000), `XCLAIM` (with and without
   `JUSTID`) and `XAUTOCLAIM` (100/1000) on a 10k-entry stream with a 10k-entry
   PEL, plus `XLEN`/`XADD` controls. Confirms the measured near-parity in
   section 3.39 on a clean runner; the 2026-11-13 local cohort already measures
   1.3–2.1× at the largest batches and parity at 100.
32. **BITFIELD writes:** one-byte `SET`/`INCRBY` (u8, u63) and a 100-subcommand
   `INCRBY` write on a 16 MB value, plus 1 MB and 64 B forms and the
   `BITFIELD_RO`/all-`GET` read controls. Extends 4.37 on a clean runner; the
   2026-11-13 local cohort in section 3.40 already measures 4.39–4.68 ms versus
   0.087–0.12 ms with reads at the floor.
33. **Zset algebra:** `ZINTERCARD`, `ZINTERSTORE`, `ZUNIONSTORE` and
   `ZDIFFSTORE` on two 100k zsets, plus the set-store variants and 10-member
   controls. Confirms 4.51 on a clean runner; the 2026-11-13 local cohort in
   section 3.41 already measures `ZINTERCARD` 220.6 ms versus 40.4/17.1 and the
   store forms 306.6–477.6 versus 61.4–366.6.
34. **INFO keyspace:** `INFO keyspace`, plain `INFO` and `INFO stats` at 10k,
   100k and 500k keys, with `DBSIZE` and `PING` controls, before and after
   expiry/field-TTL churn. Confirms 4.52 on a clean runner; the 2026-11-13 local
   cohort in section 3.42 already measures 2.79 ms at 100k and 10.0 ms at 500k
   versus 0.13–0.30 with the other sections at the floor.
35. **Two-key moves:** `SMOVE` between 100k sets, `LMOVE` all four
   LEFT/RIGHT combinations, `RPOPLPUSH`, same-key rotate and the blocking
   variants on 200k lists, with 10-element controls. Confirms 4.53 on a clean
   runner; the 2026-11-13 local cohort in section 3.43 already measures
   25.5/39.2/41.2 ms versus 0.08–0.40.
36. **Cardinality `LIMIT`:** `SINTERCARD`/`ZINTERCARD` with no limit, `LIMIT 10`
   and `LIMIT 0` on identical 100k-member containers, plus 10-member controls.
   Extends 4.51 on a clean runner; the 2026-11-13 local cohort in section 3.44
   already measures 50.7/267.2 ms with `LIMIT 10` versus 0.20–0.25.
37. **List element edits:** `LINSERT` (head/middle), `LSET` (middle), `LREM`
   (tail), `LPOS RANK -1` and 10-element controls on a 200k list. Confirms 4.54
   on a clean runner; the 2026-11-13 local cohort in section 3.45 already
   measures 19.1/16.5/21.0/21.1/1.59 ms versus 0.09–7.28.
38. **Set membership reply:** `SMEMBERS` on 1k/10k/100k sets plus a 10-member
   control. Confirms the measured lead/null in section 3.46 on a clean runner;
   the 2026-11-13 local cohort already measures 84.75 ms at 100k versus
   88.71/86.37 and parity at 10k.
39. **Multi-key arguments:** the `MGET` 1,023/1,024-key boundary, `MGET` 10,000,
   `MSET` 600 pairs and `DEL` 2,000, plus 1,000-key `MGET`/`MSET`
   (`500` pairs)/`EXISTS`/`TOUCH`/`DEL` latency and 10-key controls. Confirms
   4.55 and the parity in section 3.47 on a clean runner; the 2026-11-13 local
   cohort already measures the 1,024 boundary and 1.74 ms `MGET` 1,000.

Each row reports median, min/max, p99, server CPU and RSS, with raw JSON and the
exact source/binary hashes retained. A loss outside the cohort spread becomes a
candidate with its own mechanism, prediction and rejection condition, or a
documented limit if no safe mechanism exists. A win does not change the gate.

**Acceptance (AC19).** The cohort runs on a clean runner or same-type four-CPU
runners, not a shared k3s host; all thirty-nine rows are complete for all three servers;
no row is dropped, selected or relabeled; and any resulting candidate carries its
own AC5-style tail/CPU check and fresh protected gates.

### 4.17 Last-command lookup cache (rejected by bound)

Redis 8.10.2 reuses `c->lastcmd` when the name matches
(`isCommandReusable`, [`server.c#L110`](https://github.com/redis/redis/blob/8.10.2/src/server.c#L110)).
Copying it would add a stored command identity and a comparison to our hot path,
which already does a handful of `eq_ignore_ascii_case` byte comparisons on the
inlined PING/GET/SET/ECHO path. The saving is at most a few tens of nanoseconds
per 8.7 µs command (well under 1%), far below the 1.5–3% gate noise and below the
admission rule in section 1.1. **Reject;** do not add a cache or a command-table
abstraction for it. Revisit only if a profile shows dispatch above the noise on a
clean host.

### 4.18 Incremental keyspace rehash (deferred by keyspace bound)

Both pins use a `kvstore` with a 1 ms cron rehash budget (section 3.4); our
`HashMap` grows in one stop-the-world step. At the gate's 1024 keys that step is
a one-off move of about a thousand entries, not a per-command cost, and the gate
shows no growth-related tail. Implementing incremental rehash would mean a
custom table or a storage dependency, both policy-sensitive and both able to
weaken the multi-key/expiry invariants. **Defer:** reopen only with a
large-keyspace cohort (for example 1M keys) that shows a p99 spike during growth
above the noise; then the mechanism would be a bounded-slice rehash inside the
existing `RwLock<Data>` with unchanged atomicity, not a new dependency.

**Local growth attempt (2026-11-12), inconclusive.** 262,144 keys × 64 B, P16,
8 clients: a fresh-server growth run (500k SET, warmup 2k) followed by a
post-growth run (200k SET) on the same server. Medians: Rillcache 192k/124k req/s,
p99 5.0/10.1 ms, max 64/81 ms; Redis 150k/172k, 6.3/4.6, 80/28; Valkey
144k/116k, 5.8/10.2, 50/125. Post-growth runs were **not** faster than growth for
Rillcache or Valkey, and maxima reached 371 ms on the shared host. The cohort is
dominated by host scheduling and cannot resolve a rehash stall, so 4.18 stays
deferred and a clean large-keyspace cohort is still required. No numbers are
claimed as evidence of a growth tail.

### 4.19 P1 write-tail attribution (diagnostic; no runtime change unless isolated)

**Mechanism under test.** The paired #212 samples show a consistent P1 SET/XX
p99 gap on one host (SET 0.143 vs 0.095 ms, XX 0.135 vs 0.087 ms, every round)
while p50 and p95 are lower (section 2.22). The single-host daily cohort shows
the opposite direction on the same SET row (0.111 vs 0.159 ms). Client-observed
latency includes the generator stack and CPU scheduling, so it cannot say whether
the server or the client owns the tail.

**Prediction.** If the tail is server-side, a diagnostic run with per-command
server histograms enabled will show a matching SET/XX p99 population and the
non-mutating rows will not. If it is client- or scheduler-side, the server p99
will stay near p95 and the gap will shrink or disappear with disjoint client CPUs
or a larger client CPU budget.

**Method.** One host, pinned servers, the same generator, three rotations, both
servers. Run (a) with server-side `commandstats`/`LATENCY` histograms enabled and
(b) with shared versus disjoint client CPUs. Retain client per-request
percentiles, server per-command percentiles, reactor syscall/scheduler counters,
CPU and the exact placement. Enabling observers moves execution onto the checked
dispatch, so label that pass diagnostic and do not report its throughput as a
capability result.

**Rejection condition.** If the server-side p99 is not elevated, close the
direction as a measurement-topology limit and do not change the runtime. If only
mutation shows a server-side tail, isolate the expensive step (WATCH-version
bookkeeping, allocator, map growth, maintenance) with a targeted profile and open
a bounded candidate with its own prediction.

**Acceptance (AC21).** Both client- and server-observed p99 are reported for the
same run with CPU placement; a client-only tail never justifies a runtime change;
all raw histograms and the topology are retained. No dependency or runtime change
is part of this diagnostic.

### 4.20 Watch-free mutation bookkeeping (conditional on 4.19)

**Mechanism.** Maintain the per-key `versions` map only while at least one client
has an active `WATCH`, or while a scratch multi-owner executor needs its write
set, mirroring the upstream early return when nothing watches. Registering a
`WATCH` takes the data lock and flips the flag before recording versions; writes
then update or insert as today. A `WATCH` started after earlier writes snapshots
the current version (0 if untracked), and any subsequent write bumps or inserts a
different value, so the CAS check still detects the change.

**Prediction.** Removes one hash lookup and, on the first write per key, one
allocation from every mutation while nothing watches: roughly 40–100 ns of a
~6 µs command, well under 1%. That is **not** a throughput candidate under the
admission rule. It is worth building only if 4.19 shows a server-side mutation
tail or allocator pressure; in that case the prediction is a lower p99/max with
unchanged p50 and rate, and no gate-row change.

**Correctness obligations.** `WATCH` registration and the flag change happen
under the same data borrow; `touched_keys()` keeps working for scratch multi-owner
transactions (force tracking there); version-map overflow and epoch semantics stay
unchanged; a key written before the `WATCH` and again after it must still abort
`EXEC`; `FLUSHDB`/full-table paths and the bounded `MAX_TRACKED_VERSIONS` clear
keep their current behavior.

**Rejection condition.** Any `WATCH`/`EXEC`/`MULTI` regression, any change to
multi-owner scratch write sets, no tail or CPU improvement in the matched A/B, or
any gate-row loss. Then close as below-noise and keep unconditional tracking.

**Acceptance (AC22).** Default-owner mutations perform no per-key `WATCH`
bookkeeping while no client is watching; a `WATCH` registered later still aborts
on a subsequent mutation; scratch multi-owner write sets are unchanged; the
transaction/WATCH/multi-owner matrix passes; the matched A/B shows the claimed
tail or CPU effect; and both protected gates pass on the exact head.

### 4.21 Per-batch journal write coalescing (candidate; admission measured)

**Mechanism.** During one reactor service batch, encode each journaled command
into a reusable per-batch buffer (the existing CRC32/HMAC transaction frames,
concatenated) and perform one `write_all` under one journal-mutex acquisition
before the client's replies are flushed, instead of one write and one lock per
command. The 1 s `everysec` sync and the `always` sync-before-reply semantics are
unchanged; only the page-cache write is coalesced. `always` then syncs the batch
once, so every acknowledged command in it is durable, matching the C servers'
per-loop flush shape.

**Prediction.** At pipeline K, the journal path replaces K writes, K mutex
acquisitions and K transaction-buffer allocations with one of each. The measured
baseline is a **−66.5% P1 and −90.2% P16** SET loss with AOF on (section 3.9) and
P16 p99 15.1 ms versus 3.1 ms off; the C servers lose 8–34%. Coalescing should
move the P16 loss toward the C servers' single-digit-to-mid range and the P1 loss
toward Valkey's ~18%, with P16 p99 back near the AOF-off value. On the protected
gate (persistence off) the effect is exactly zero, which is why this is not a
gate candidate.

**Admission evidence (2026-11-12, section 3.9).** Three Latin-rotated local
rounds with fresh journals: AOF-on/off SET throughput is −66.5% (P1) and −90.2%
(P16) for Rillcache, against +12%/−33.5% (Redis) and −17.7%/−8.5% (Valkey); the
rill P16 ratio is 4.0–12.5× per round while the AOF-on rate is stable. The gap is
large and consistent enough to admit the candidate; the shared-host absolute
rates are not a gate claim.

**Failure contract (required design).** Commands execute in memory before the
batch is durable, so a batch-write failure must be handled explicitly: mark the
journal failed (the existing state that rejects further appends), replace every
reply in the batch with the journal error rather than acknowledging any of them,
and stop accepting writes until the operator restarts. No acknowledged command
may be missing from the journal; unacknowledged in-memory effects may be lost on
restart, which is the same contract the current per-command error path has, only
Wider. A `SIGKILL`/replay test must prove that every reply the client received is
present in the replayed journal.

**Admission and baseline.** Admission is met by the 2026-11-12 persistence-on
cohort (section 3.9). For the implementation, freeze the current and candidate
binaries, alternate them on one host with the same generator, `appendfsync
everysec` and `always`, and retain raw JSON, p99, CPU, journal-size and replay
checks.

**Rejection condition.** Any durability/replay failure, any reply-before-write
ordering, any HMAC-chain or CRC boundary error, any gate row loss, no gain above
the spread in the persistence-on row, or unbounded batch memory. Then keep the
per-command append.

**Acceptance (AC23).** With persistence on, a batch of K commands performs at
most one journal write and one journal lock acquisition; every acknowledged
command is present in the replayed journal; `always` still syncs before the reply
and performs at most one fsync per batch;
`everysec` still syncs within its 1 s bound; **P16 AOF-on SET throughput is
within 25% of AOF-off and P1 within 10%, and P16 p99 is no worse than 2× the
AOF-off p99** (measured baseline −90.2%/−66.5% and 15.1 vs 3.1 ms; C servers
−8.5% to −33.5%); kill/replay, HMAC, CRC, rewrite and multi-owner tests pass; the
matched persistence-on A/B shows the gain; and both protected gates pass on the
exact head.

### 4.22 Same-pass blocked-client wakeup (candidate; admission evidence collected)

**Mechanism.** While executing a mutation under the owner data borrow, if the
command makes a key ready for a blocked type (list push/move, sorted-set add,
stream add), record the key in a bounded ready set. After the current command
batch and before the reactor parks, enqueue the blocked clients waiting on those
keys and service them in the same pass. Keep the 1 ms `ASYNC_TICK` as the
timeout/fallback path for clients whose key is not signalled (expiry, cross-thread
or multi-owner transfer, a lost wake), so a missed signal degrades to today's
behavior rather than a lost wake.

**Prediction.** Wake p50/p99 for BLPOP/BRPOP/BLMOVE/BZPOPMIN/BZPOPMAX and
`XREAD BLOCK` drops from the 0–1000 µs tick quantization (measured 1.16 ms p50)
to the reactor's service latency (tens of µs), and idle CPU with blocked clients
falls from 5.6% (1 client) / 22% (1000) toward the C servers' <1%, because the
tick no longer re-enqueues and re-polls every blocked client and a no-progress
poll no longer arms the 20 µs `FAST_POLL` spin. The post-#226 re-measurement is
5.0% (1) / 21.4% (1000), showing the spin fix alone left the per-client scan
intact. The protected gate, which has no
blocking command, is unchanged; throughput is unchanged because the ready set is
empty when nothing blocks.

**Method.** Matched cohort with one client blocked on a key and another pushing
after a fixed delay; measure wake p50/p95/p99 for Rillcache, Redis 8.10.2 and
Valkey 9.1.2, at least three rotated repetitions, on one host or the same-type
runners. Add a many-waiter FIFO case, a timeout-versus-push race, `CLIENT
UNBLOCK`, and a pipeline where the push and the wake are in the same batch.

**Admission evidence (2026-11-12, section 3.12).** 300 wakeups per run, three
Latin-rotated runs per server, randomized 0.5–2.5 ms push delay: after the LPUSH
reply, Rillcache p50 **1.16 ms** / p99 **1.92 ms** against Redis **0.031/0.221**
and Valkey **0.025/0.220** ms on the same host. The gap matches the 1 ms tick and
is far outside host noise. The same tick costs **5.6% of a core with one idle
blocked client and 22.2% with 1000**, against <1% for the C servers. Both the
latency and CPU gaps admit the candidate. Implementation still needs the
obligations below and the AC25 acceptance.

**Risks and obligations.** Service after the batch, never re-enter the store
inside the mutating command; bound the ready set and the number of waiters
serviced per pass; preserve FIFO order among waiters; keep the wake-versus-timeout
race single-winner; keep `CLIENT UNBLOCK` and shutdown correct; keep multi-owner
transfers working (the owner must retain or transfer the waiter set). The 1 ms
fallback stays. The no-progress spin component is handled separately in PR #226
(merged as `b7f5694`); this candidate still needs to remove the 1 ms re-enqueue
and re-poll of every blocked client.

**Rejection condition.** Any lost, duplicated or out-of-order wake; any timeout or
`CLIENT UNBLOCK` regression; no measured latency improvement; no idle-CPU
improvement in the blocked-client cohort; or any protected gate row loss. Then
keep the tick and record the 1 ms bound.

**Acceptance (AC25).** A matched blocked-wake cohort shall show p99 within 200 µs
of Valkey and at least 2× better than the pre-change head (the ~1 ms tick
baseline); with 1 and 1000 idle blocked clients and no traffic, server CPU shall
stay at or below 1% and shall not scale with the client count; FIFO, ordering,
timeout, unblock, multi-owner and shutdown tests shall pass; the protected gates
shall pass on the exact head; and no dependency is added.

### 4.23 Bounded rewrite key-list materialisation (candidate, gated on rewrite latency)

**Mechanism.** Do not collect every key name in one command turn when
`BGREWRITEAOF` starts. Mark the rewrite generation and collect keys in bounded
batches across maintenance ticks (for example 2,048 names or 1 ms per tick) into
the same key list, then start encoding (or begin encoding the keys already listed
while later keys are collected, using the existing "re-encode keys written since
the previous round" reconciliation to cover keys created or deleted during
materialisation). The `+Background...` reply can be sent before the list is
complete because the snapshot is not installed until the final atomic rename.

**Prediction.** The command pause drops from the measured 14.2 ms at 20k keys
and **90.7 ms at 200k keys** to the batch bound (targets ≤2 ms and ≤20 ms
respectively) and scales with the batch rather than the keyspace; total
completion may grow by a few maintenance ticks and must stay within 25% of the
measured 4.85 s (20k) and 43.4 s (200k) baselines. Concurrent GET p99 during the
start window improves.

**Method.** 20k- and 200k-key fixtures of 64-byte values; measure the
`BGREWRITEAOF` reply latency, per-slice encode time, total completion, concurrent
client p99 and a write storm; kill/replay mid-rewrite with
`tests/durability_crash.rs`. Compare against the current head and, where
possible, Redis/Valkey fork behavior on the same fixture.

**Risks and obligations.** The key list is no longer point-in-time: a key added
during materialisation must be covered by reconciliation, a key deleted during it
must be checked at encode time, and a key created and deleted before the list
reaches it must not be resurrected. The list memory is unchanged but held longer.
Every durability/replay test and the transaction gate stay mandatory.

**Rejection condition.** Any missed, duplicated or resurrected key; total
completion more than 25% slower; any crash/replay failure; or any protected gate
row loss. Then keep the single materialisation and record the 9–20 ms bound.

**Acceptance (AC26).** At 20,000 keys the `BGREWRITEAOF` reply pause shall be
≤2 ms and at 200,000 keys ≤20 ms, i.e. bounded by the materialisation slice
rather than linear in the keyspace; concurrent writes during materialisation
shall appear exactly once in the replayed snapshot; kill/replay shall recover;
total completion shall stay within 25% of the measured 4.85 s (20k) and 43.4 s
(200k) baselines; both protected gates shall pass; and no dependency shall be
added.

### 4.24 Event-driven async-client servicing for pub/sub and MONITOR (candidate, admission measured)

**Mechanism.** Register subscribers and monitoring clients for event-driven wake
instead of the 1 ms `asynchronous` set: a `PUBLISH` (or a command while a monitor
is attached) schedules the target client or clients in a bounded queue, and the
reactor services them after the current command batch. Timeouts and pause
deadlines use a timer. A no-progress async poll must stop counting as `serviced`
so it does not arm the 20 µs `FAST_POLL` window; keep the 1 ms fallback for a
lost signal. The same shape covers blocked clients (4.22); implementing them
together avoids two separate tick paths.

**Prediction.** Idle CPU falls from the measured 6.0%/17.8%/22.2% (1/100/1000
idle subscribers) and 5.8%/20.8%/27.4% (monitors) — post-#226 4.6%/13.6%/22.4%
and 4.2%/20.8%/26.6% — to the C servers' under 1%, with no scaling beyond the
client count's bookkeeping; publish and monitor
delivery latency drops from the measured 1.05/3.60 ms (pub/sub p50/p99) and
0.37/6.15 ms (MONITOR) to same-pass service, near the C servers' 0.05–0.08 ms
p50. The protected gate has no pub/sub or MONITOR client and is unchanged.
4.39 adds the burst-safety contract to the same path: a 2,000-message burst must
not silently remove a subscriber (section 3.23).

**Method.** The idle-CPU windows above, then a matched delivery cohort: N
subscribers receiving a PUBLISH fanout and N monitors observing a P1 command
stream, p50/p99 for Rillcache, Redis 8.10.2 and Valkey 9.1.2 on one host with at
least three rotations.

**Admission evidence (2026-11-12, section 3.12).** Idle CPU is 6.0/17.8/22.2%
(1/100/1000 subscribers) and 5.8/20.8/27.4% (monitors) versus under 1% for the C
servers. Delivery after the publishing/monitored command's reply: pub/sub
1.05/3.60 ms p50/p99 against Redis 0.075/1.19 and Valkey 0.064/0.56; MONITOR
0.37/6.15 ms against 0.050/0.64 and 0.048/0.49. Both the CPU and delivery gaps
admit the candidate; implementation still needs the obligations below and AC27.

**Risks and obligations.** A missed wake would strand a subscriber or monitor;
duplicate or out-of-order delivery, fanout memory, unsubscribe while a message is
queued, monitor command ordering and shutdown with queued pushes are the failure
modes. Keep the 1 ms fallback, bound the queue, and preserve the existing
message ordering.

**Rejection condition.** Any missed, duplicated or out-of-order delivery; any
unsubscribe or shutdown regression; no idle-CPU improvement; or any protected
gate row loss.

**Acceptance (AC27).** With 1 and 1000 idle subscribers and monitors, server CPU
shall stay at or below 1% and shall not scale materially with the count (baseline
6.0/22.2% and 5.8/27.4% versus <1% for the C servers); PUBLISH and MONITOR
delivery p99 shall be within 200 µs of Valkey and at least 2× better than the
1 ms tick baseline; fanout, ordering, unsubscribe, timeout and shutdown tests
shall pass; no dependency shall be added; and both protected gates shall pass on
the exact head.

### 4.25 Large-frame input reservation (candidate; admission measured)

**Mechanism.** After parsing and validating a bulk header of length `L`, ensure
the input buffer has capacity for the remaining frame (capped by the configured
frame/bulk limits) instead of appending 16 KiB reads into a repeatedly growing
`Vec`. Reserve in bounded steps (for example 64 KiB at a time up to the declared
frame end) so a client cannot force a huge allocation by lying about the length.
The read loop, parser, limits and fragmented/truncated behavior stay unchanged.

**Prediction.** Removes the O(log n) reallocation-and-copy sequence for a large
frame. The measured baseline is a **28.2% (Redis) / 31.8% (Valkey)** 1 MiB SET
loss with p99 9.79 vs 7.37/7.82 ms (section 2.21); a reservation should move the
SET row toward the C servers. The 64-byte rows do not reserve and must not change.

**Admission evidence (2026-11-12, section 2.21).** The SET loss cannot be the
reply copy (the reply is small), and it appears only at large sizes; the C
servers reserve from the declared bulk length (`big_arg`/`sdsMakeRoomFor`,
section 3.3), while our buffer grows in 16 KiB reads. A matched A/B of the
reservation against the current parser must isolate the parse copy from the store
and socket copies before a throughput claim.

**Risks and obligations.** A declared-length reservation is a memory-DoS surface:
bound the total by `max_frame`/`max_bulk`, reserve in bounded steps, and keep
admission and per-client limits unchanged. Fragmented, truncated, malformed and
over-limit frames must behave exactly as today; no hot-path branch for small
frames beyond the existing header parse.

**Rejection condition.** No 1 MiB SET gain above the spread; any unbounded or
over-limit reservation from a lying length; any parser/limit/close regression; or
any protected gate row loss.

**Acceptance (AC28).** At 1 MiB SET, throughput shall be within 10% of the best
reference and p99 no worse than 1.25× it (measured baseline −28% to −32% and 9.79
vs 7.37/7.82 ms); a declared length that exceeds the limits shall reserve no more
than the configured bound; fragmented, truncated and malformed frames keep their
current behavior; the matched A/B shows the gain; and both protected gates pass
on the exact head.

### 4.26 Pooled/batched eviction pass (candidate; admission measured)

**Mechanism.** When a write needs memory, run one bounded eviction pass that
samples candidates into a small pool and evicts until the budget is satisfied or
a bounded key/time count is reached, instead of rescanning candidates on every
admitted write. Keep the policy semantics (`allkeys`/`volatile`,
`lru`/`random`/`ttl`), the mutation-time approximation, accounting and expiry;
if the budget cannot be met, return the existing OOM reply. Bound the pass so one
write cannot hold the data lock for an unbounded time.

**Prediction.** Under continuous eviction, P16 moves from the measured **20,498
vs 144,365 req/s (−85.8%, p99 64.5 vs 8.0 ms)** toward the no-eviction control,
and P1 from 8,339 vs 14,895; the C servers show no penalty. Target: P16 within
25% of the no-eviction control and p99 within 2× it, with no change when the
budget is not under pressure.

**Admission evidence (2026-11-12, section 3.10).** The penalty is policy-
independent on this host (Rill `allkeys-lru` 27.1–31.3k and `allkeys-random`
25.3–27.6k at P16, both far below the control; Redis shows no penalty for
either), so it is the admission scan, not the LRU quality. Rill evicts ~1.75×
more keys than the 2 MB C servers, which does not explain a 5–7× throughput
difference. Shared-host diagnostic; a matched frozen-binary A/B is still needed.

**Risks and obligations.** Evicting more than necessary, holding the data lock,
and changing policy semantics are the failure modes. Preserve exact accounting,
expiry/TTL ordering, WATCH/journal order and `noeviction`; bound keys/time per
pass; keep the existing reply when the budget cannot be met.

**Rejection condition.** No gain above the spread; over-eviction or accounting
error; any policy-semantics change; any protected gate row loss.

**Acceptance (AC29).** Under continuous eviction, P16 SET throughput shall be
within 25% of the no-eviction control and p99 within 2× it (measured baseline
−85.8%, 64.5 vs 8.0 ms); evicted-key counts and payload accounting shall be
exact; every policy (`allkeys`/`volatile` × `lru`/`random`/`ttl`) and
`noeviction` shall behave as specified; there shall be no change when the budget
is not under pressure; the matched A/B shall show the gain; and both protected
gates shall pass on the exact head.

### 4.27 Single-pass replay fast path (candidate; admission measured)

**Mechanism.** Merge checksum validation into the replay parse so each
transaction is checked once and applied once, and run the command loader in a
load mode that skips WATCH-version churn, keyspace notifications, observability
counters and journal re-append, while preserving absolute expiry times, payload
accounting, stream consumer/PEL state, HMAC chaining and the existing tail
policy. A bad frame aborts before the listener binds; no partial state is served.

**Prediction.** Time-to-ready at 200k keys falls from the measured **1.10–1.24 s
(median ~1.16)** to **≤0.3 s** and at 50k from 0.258 s to ≤0.08 s, while
DBSIZE, payload bytes and TTLs match the pre-replay state. Redis/Valkey remain at
~0.03–0.06 s; the target is a ≥4× improvement, not a claim to match C.

**Admission evidence (2026-11-12, section 3.15).** 50k keys: Rill 0.258 s vs
Redis 0.011 and Valkey 0.054; 200k: Rill 1.10–1.24 s vs 0.027–0.043 and
0.051–0.061, with DBSIZE 200,000 verified for Rill and 50,000 for Redis. The
source shows two full parses (checksum pass + replay) plus normal command
execution, which explains the ratio. With AOF HMAC on, the 50k replay is 0.75 s
versus 0.21 s, so the separate authentication pass is part of the target
(section 3.9). Shared-host diagnostic.

**Method.** 50k and 200k fixtures, three clean restarts and three SIGKILL
restarts each, raw time-to-PING, DBSIZE, `INFO` bytes/TTLs, corruption/tail
tests, HMAC on/off and stream consumer/PEL state.

**Risks and obligations.** Corruption and tail policy must be decided before
serving; HMAC verification must stay intact; expiry must remain absolute (not
re-relative); accounting, WATCH versions after load, streams state and
`noeviction` semantics must match. Do not skip a check that changes externally
visible state.

**Rejection condition.** Any missing or extra key, accounting/TTL mismatch,
corruption accepted, HMAC failure, or any protected gate row loss.

**Acceptance (AC30).** At 200,000 keys, time-to-PING shall be ≤0.3 s and at
50,000 ≤0.08 s (measured baseline 1.10–1.24 and 0.258 s); the post-replay
DBSIZE, payload bytes and TTLs shall match exactly; corrupt or truncated tails
shall be handled by the documented policy and never served; SIGKILL/replay,
HMAC on/off and stream consumer/PEL tests shall pass; both protected gates shall
pass; and no dependency shall be added.

### 4.28 Implementation map and priority for admitted candidates

The research is now measurement-limited on the shared host: the remaining
admitted items need code and their own protected gates, not more local screens.
This map fixes call sites, invariants and test surface so the next increment
starts from source. Approximate diff sizes are relative scope, not estimates.

| Candidate | Primary call sites | Invariants | Test / evidence | Scope |
| --- | --- | --- | --- | --- |
| 4.21 AOF batching | `Journal::append` (`journal.rs#L1028`), `Store::execute_command` (`engine.rs#L1228`), reactor run loop | write before reply; per-transaction CRC/HMAC; 1 s everysec; one fsync per batch; batch failure unacknowledges every reply | kill/replay, HMAC/CRC, everysec/always, multi-owner | medium |
| 4.27 replay | `Journal::replay` (`#L944`), `validate_checksums` (`#L592`), `execute_command` | corruption/tail policy; HMAC chain; absolute TTL; accounting; streams/PEL | 50k/200k time-to-PING, DBSIZE/bytes/TTL, SIGKILL | medium |
| 4.25 input reservation | `Client::receive` (`pool.rs#L109`), `parse_request` (`protocol.rs#L64`) | frame/bulk limits; fragmentation; truncated/malformed frames | 1 MiB SET, lying length, limits | small |
| 4.15 large reply | `Entry.value` (`data.rs#L205`), `encode_bulk` (`protocol.rs#L398`), `Client::flush` (`pool.rs#L73`) | reply order; partial writes; backpressure; payload accounting | 1 MiB GET, slow reader, pipelined GETs | medium |
| 4.26 eviction | `Data::reserve` (`data.rs#L646`), `eviction.rs` | policy semantics; accounting; expiry/WATCH order; `noeviction` | pressure cohort, every policy | medium |
| 4.22/4.24 async wake | reactor `asynchronous` (`reactor.rs#L91`, `#L467`), `poll_blocked_on` (`blocking.rs#L436`), pub/sub and monitor flags | FIFO; timeout gaps; `CLIENT UNBLOCK`; ordering; shutdown; multi-owner | wake latency, idle CPU, fanout | large |
| 4.23 rewrite start | `begin_rewrite` (`journal.rs#L1131`), maintenance tick (`reactor.rs#L443`) | atomic rename; replay; reconciliation | 20k/200k pause, kill/replay | medium |
| 4.30 zset index | `zset_read` (`collections.rs`), `zset_add`, `zset_items`/`ZSet` (`data.rs`), accounting | total score order with member tie-break; ±0/NaN; rank/score/lex ranges; `ZSetPack` ≤128 unchanged; memory accounting; rewrite/replay | 10k `ZRANGE 0 9` / full-range A/B, ZADD regression, zset semantics | large |
| 4.31 cursor scan | `scan_keys`/`scan_collection` (`scan.rs`), `KEYS`/`RANDOMKEY` (`keyspace.rs`), router `locked_scan`, `Data` keyspace index, `ZSet`/`Hash`/`Set` order, expiry reap | opaque cursor resume; full coverage on a stable keyspace; MATCH/COUNT/TYPE/NOVALUES; pack one-call behavior; bounded work; KEYS/RANDOMKEY order; memory accounting | 100k `SCAN COUNT 10`, full scan, 10k `HSCAN`, 100k `KEYS`/`RANDOMKEY`, SET/GET regression | large |
| 4.32 packed list nodes | `Collection::ListPack`/`ListQuick` (`data.rs`), list commands (`collections.rs`), `payload_len`, rewrite encoding | node index arithmetic; split/merge; binary safety; max element size; accounting; iteration order; blocking pop | 200k-list RSS and `DEL` A/B, RPUSH/LRANGE/LINSERT/blocking | large |
| 4.33 deferred drop | `DEL`/`UNLINK` (`dispatch/keyspace.rs`), `FLUSH*` (`administration.rs`), `Data::flush`/`remove`, maintenance tick (`reactor.rs`), accounting | queue bound; pending bytes counted; key recreation; notification/journal order; shutdown drain; SYNC unchanged | `UNLINK`/`FLUSH ASYNC` A/B, DBSIZE/EXISTS/SCAN, drain-on-shutdown | small |
| 4.34 in-place strings (implemented, #258) | `append`/`setrange` (`string_ranges.rs`), `Data::mutate_string`/`reserve`/`put` (`data.rs`), `touch`/accounting | admit before mutate; capacity growth amortized; OOM leaves the old value; zero-fill growth; 512 MiB limit; TTL/WATCH/journal | 32 MB `APPEND`/`SETRANGE` A/B, small control, bytes/copies | small, landed |
| 4.35 in-place collection writes | `hash_set`/`zset_add`/`set_add`/`push_list`/`pop_list` (`collections.rs`), `Data::entries.get_mut`, `payload_len`, field TTL, blocking wake | admit before mutate; pack↔large conversion only at thresholds; delta accounting; empty-collection removal; field TTL; WATCH/journal | 10k/100k `HSET`/`ZADD`/`SADD` plus updates/deletes, 200k `RPUSH`/`LPOP`, 10-element controls | medium |
| 4.36 random members | `set_random_member`/`set_pop`/`hash_random_field`/`zset_random_member` (`collection_commands.rs`), `owned_set`, `set_items`, `hash_items`, `zset_items`, collection representation | sample without materializing; uniform distribution; count/negative-count semantics; `SPOP` removal and empty deletion; ordering decision; small pack paths | 10k/100k `SRANDMEMBER`/`HRANDFIELD`/`ZRANDMEMBER`/`SPOP`, distribution test | medium |
| 4.37 bitmaps | `setbit`/`bitcount`/`bitpos` (`string_algorithms.rs#L89`, `#L457`, `#L524`), `bitfield` (`bitfield.rs#L258`, write clone at `#L316`, put at `#L382`), `find_bit` (`#L432`), `Data::reserve`/`put`, range normalization | in-place write after admission (`SETBIT` and `BITFIELD` write forms); read-only `BITFIELD` stays borrowed; `u64` chunk popcount and bit scan with masked head/tail; BYTE/BIT ranges; `OVERFLOW` policies; offset/512 MiB limits; zero-fill; TTL/WATCH/journal | 1 MB/16 MB `SETBIT`/`BITCOUNT`/`BITPOS`/`BITFIELD` writes, `BITFIELD_RO`, 64 B controls, range cases | small |
| 4.38 zset rank index | `zset_rank` (`collection_commands.rs#L3744`), `zset_range_by_score` (`#L3896`), `zset_read` (`collections.rs#L433`), `ZAdd`/`ZRem` paths, block index | rank-select over `(score, member)`; score moves; block split/merge; negative/reverse ranges; `ZSetPack`; accounting; replay/rewrite | 100k `ZRANK`/`ZREVRANK`/deep `ZRANGE`/`ZRANGEBYSCORE`, ZADD regression | medium |
| 4.39 pub/sub fan-out | `PubSub::publish` (`pubsub.rs#L121`), `queue_capacity`, subscriber drain (`pool.rs#L159`), reactor async tick | channel lookup by key; byte-bounded queue; no silent unsubscribe; counted drop or documented disconnect; shared frame; ordering/pattern/shard; shutdown | 2,000-message burst, 10k-channel PUBLISH, 100 subscribers, NUMSUB | medium |
| 4.40 O(1) counts | `set_read`/`hash_read`/`zset_read`/`list_len` (`collections.rs#L240`, `#L355`, `#L478`, `#L658`), `Collection` length/probe methods | SetInt/SetPack semantics; missing/empty/wrong type; no reply or ordering change; LLEN node walk | 100k `SISMEMBER`/`SCARD`/`HLEN`/`ZCARD`, 200k `LLEN`, 10-element controls | small |
| 4.41 list seek | `list_index` (`collection_commands.rs#L2668`), `list_position` (`#L2321`), `Collection::list_items`/`ListQuick` (`data.rs`) | negative index; chunk boundaries; RANK/COUNT/MAXLEN; missing/empty/wrong type; no reply change | 200k `LINDEX`/`LPOS`, 10-element controls, LRANGE/LLEN | small |
| 4.42 stream append | `xadd` (`streams.rs#L310`), `Stream`/`payload_bytes` (`data.rs#L182`), `put_stream` (`#L877`), trim/groups | append + delta accounting; IDs/counters; groups/PEL; trim; TTL/WATCH/journal/rewrite | 1k/5k `XADD`, reads, 10-entry control | small |
| 4.43 geosearch | `run_search` (`geo.rs#L350`), `geopos` (`#L203`), geohash cell helpers, ordered zset index | cell ranges/neighbours; dateline/pole; dedupe; ASC/DESC/COUNT; WGS-84; STORE | 50k 1 km/box `GEOSEARCH`, `GEOPOS` 1/100, `GEODIST`, boundary fixture | medium |
| 4.44 multi-member probes | `set_mismember` (`collection_commands.rs#L3043`), `zmscore` (`collections.rs#L490`), `contains_member`/`zset_score` | SetInt/SetPack; duplicates/order; missing/empty/wrong type; no reply change | 100k `SMISMEMBER`/`ZMSCORE` at 1/10/100, `HMGET` control | small |
| 4.45 bounded expiry | `KEYS`/`RANDOMKEY`/`DBSIZE` (`keyspace.rs#L183`), `MGET`/`MSET`/`MSETEX` (`multikey.rs#L160`), `SCAN` (`scan.rs#L122`), `Data::reap`/`reserve` (`data.rs#L599`, `#L646`) | expired values unreadable; DBSIZE semantics; admission bound; TTL/WATCH/notifications | 30k-expiry wave: `MGET`/`KEYS`/`RANDOMKEY`/`DBSIZE`, lazy `GET` | small |
| 4.46 nosort slice | `sort`/`source_values` (`sort.rs#L120`, `#L228`), `ITEMS`/limit/STORE | source order for set/zset; offset/count; STORE; missing key; no reply change | 100k `BY nosort LIMIT`, controls, other SORT rows | small |
| 4.47 HLL in place | `pfadd`/`pfcount`/`pfmerge` (`hll.rs#L240`, `#L285`, `#L313`), `Hll`/`inspect`/`serialize`, `Data::put` | cache invalidation; exact↔dense; count accuracy; serialization/rewrite/replay | 100k pipelined `PFADD`, 10k `PFCOUNT`, controls | small |
| 4.48 xgroup create | `xgroup` (`streams.rs#L1454`, `#L1492`), `Stream.groups`, `put_stream` | BUSYGROUP/MKSTREAM/ID; payload unchanged; PEL/consumers; TTL/WATCH/journal/rewrite | 5k `XGROUP CREATE`, 10-entry control, other XGROUP ops | small |
| 4.49 zset pops | `zset_pop` (`collection_commands.rs#L4817`), `owned_zset`/`ordered_zset`, ordered index | ties/scores; empty/negative count; key deletion; ZSetPack; blocking; accounting/rewrite | 100k `ZPOPMIN`/`ZPOPMAX`/`ZPOPMIN 10`/`ZMPOP`, 100-member control | medium |
| 4.50 lex seek and span counts | `select_zset`/`ordered_zset` (`collection_commands.rs#L798`), `zset_range` (`#L4185`), `zset_count` (`#L3866`), `zset_lex_count` (`#L4556`), `zset_items` (`data.rs#L111`), ordered/rank index | bound parsing; exclusive/infinite bounds; reverse + LIMIT arithmetic; mixed-score order; pack conversion; store/accounting; TTL/WATCH/journal | 100k `ZRANGEBYLEX`/`ZREVRANGEBYLEX` LIMIT forms, full reply, `ZLEXCOUNT`/`ZCOUNT` (full/matching/empty), 10-member control | medium |
| 4.51 zset algebra | `zset_algebra` (`collection_commands.rs#L4330`), `zset_intercard` (`#L4492`), `set_intercard` (`#L3200`), `set_store` (`#L3257`), `owned_zset`/`owned_set`, `ordered_zset`, `encode_zset` | source order; destination-as-source; weights/aggregate/NaN; `LIMIT` early exit and `LIMIT 0`; empty/missing/wrong type; store TTL/accounting; SetInt/SetPack/`ZSetPack`; WATCH/journal/rewrite | 100k `SINTERCARD`/`ZINTERCARD` with/without `LIMIT`, store forms, 10-member controls | medium |
| 4.52 INFO keyspace counters | `info` keyspace section (`discovery/mod.rs#L1016`), `Data` (`data.rs#L247`), insert/remove/expire/reap/evict/flush/rename/copy/replay paths, `field_ttls` | every counter delta; avg_ttl empty case; no reap on INFO; section selection; replay/rewrite/eviction/flush | 10k/100k/500k `INFO keyspace`/`INFO`/`INFO stats`/`DBSIZE`, TTL churn correctness | small |
| 4.53 two-key moves | `set_move` (`collection_commands.rs#L3073`), `list_move_in_data` (`#L2541`), `owned_set`, `Collection::list_values` (`data.rs#L41`), blocking wake | admit destination first; OOM leaves both keys unchanged; same-key rotate; empty-source deletion; TTLs per key; chunk split/merge; blocking/timeout; WATCH/journal/notification/rewrite; accounting | 100k `SMOVE`, 200k `LMOVE`/`RPOPLPUSH`/rotate, 10-element controls, blocking variants | medium |
| 4.54 list element edits | `list_insert` (`collection_commands.rs#L2698`), `list_remove` (`#L2750`), `list_set` (`#L2820`), `list_position` (`#L2321`), `Collection::list_values` (`data.rs#L41`) | pivot/index/count semantics; missing/empty; chunk split/merge; emptied-key deletion; TTL/WATCH/journal/rewrite; accounting | 200k `LINSERT` head/middle, `LSET` mid, `LREM` tail, `LPOS RANK -1`, 10-element controls | medium |
| 4.55 frame-derived argument cap | `Limits`/`parse_request` (`protocol.rs#L5`, `#L17`), `dispatch_request`, server `--max-frame-bytes`/`--max-bulk-bytes` plumbing | count bound derived from the frame; frame error unchanged; PING fast path; arity/protocol tests; bounded slice memory | `MGET` 10k, `DEL`/`EXISTS`/`TOUCH` 2k, `MSET` 600 pairs, 1,023/1,024 boundary, over-frame rejection | small |

**Priority.** 4.21 and 4.27 first: they address the largest measured
steady-state (AOF-on) and availability (replay) gaps and are contained in the
journal path. Then 4.25 and 4.15 for large values, 4.26 for eviction, 4.45 for
the measured mass-expiry stall (37–76×), and 4.33 — small and contract-visible —
for the measured synchronous `UNLINK`/`ASYNC` drop. 4.49 is the largest measured
ratio in the record (373–974×) and reuses 4.30/4.38's ordered index, so it lands
with that structure. 4.40, 4.41, 4.44, 4.46, 4.47 and 4.48 are the smallest,
highest-certainty fixes (O(1) counts/membership, in-place list seek, per-member
probes, direct nosort slice, in-place HLL with a cached count, in-place group
creation) and can land immediately. 4.42 is the stream sibling of 4.35's in-place
write and shares the delta-accounting shape. 4.35 is next: every collection write
currently clones the collection, the gaps are **3.5–1160×** (3.5–69× at 10k,
18–1160× at 100k), and it reuses 4.34's
admit-then-mutate shape. 4.37 is small and shares 4.34's in-place string path: it
removes the two copies in `SETBIT` and the `BITFIELD` write forms and the byte
loop in `BITCOUNT`/`BITPOS` (5–44×, and 37–52× for a 16 MB `BITFIELD` write).
4.36 follows, first as a no-copy reservoir slice and then the dense
index; its 100k gaps are 35–298×. 4.30, 4.38, 4.43 and 4.49 are the largest
measured read/pop ratios (378–974× and 10–53×) and should share one ordered
structure: 4.30 covers point/score/start-0 reads, 4.38 the rank-select, 4.43 the
geohash cell ranges, and 4.49 the min/max pops; 4.50 adds the lex seek and
span-count queries to the same index (163–371× for a 10-element lex range).
4.51 is independent of that structure: it removes the per-source owned copy and
the result re-sort from `ZINTERCARD`/`ZINTERSTORE`/`ZDIFFSTORE` (5.5–12.9× and
2.8–5.4×) by probing borrowed sources, extends the same probe to `SINTERCARD`,
and turns `LIMIT` into an early exit (205–1,377×). 4.52 is a small, contained observability
fix: it replaces the O(keyspace) `INFO` walk with maintained counters (16–40×)
and can land with 4.40's length work. 4.53 is the two-key-move counterpart of
4.35/4.41: it removes a full copy of both keys per element moved (64–489×). 4.54
completes the list edits: `LINSERT`/`LSET`/`LREM` stop flattening the list
(59–212× on the O(1)-to-O(nodes) forms). 4.55 is a small compatibility fix: it
derives the argument cap from the frame budget so 2,000–10,000-argument commands
parse as they do in Redis/Valkey.
4.39 is a contract/reliability
fix on the pub/sub and MONITOR path already admitted as 4.24 (section 3.37) and
should land with them. 4.34
(landed in #258) removed two full-value copies per `APPEND`/`SETRANGE` and
post-fix measures 0.13–0.19 ms at 32 MB. 4.32 follows as the structural memory
and CPU fix (142–143 → target ≤24 B/element) and may reuse 4.31's ordered-list
direction. 4.22/4.24 come last because they touch every asynchronous client path
and carry the highest ordering risk (4.39 rides with them). 4.31 is the largest
per-call ratio in the record but touches the keyspace, every collection type and
the router cursor, so it follows the journal slices and the ordered-structure
decision. 4.23 follows the existing bounded slices. 4.20 stays conditional on
4.19; 4.18 stays deferred.

### 4.29 Large-hash HGETALL reply path (candidate; admission measured)

**Mechanism.** Remove the full field/value reference `Vec` and O(n log n) sort
from `HGETALL` on large hashes. Options: (a) iterate the `HashMap` directly and
emit internal order, matching Redis's unspecified `HGETALL` order (the exact-byte
persistence test and any order-sensitive tests would need to compare unordered);
(b) keep deterministic order with a `BTreeMap`-backed large hash, so iteration is
sorted without an extra allocation and sort (measure the write cost); or (c)
maintain a sorted index on mutation. Pick the option with the best measured
tradeoff, not the first.

**Prediction.** A 10k-field `HGETALL` moves from the measured **313 req/s
(11.6/32.1 ms p50/p99)** toward Redis's 552 (7.0/16.3), while `LRANGE`, small
hashes and the protected gate are unchanged. The 43% gap and the per-byte
difference versus our own `LRANGE` (30 vs 110 MB/s per request) point at the
sort/allocation, not the socket write.

**Admission evidence (2026-11-12, section 3.2).** Preloaded 10k-field hash,
4 clients, 250 requests each, three rotations: Rill 313/312/339 req/s, Redis
552/486/582, Valkey 345/329/336; p50 11.6 vs 7.0/11.4 ms. The source path is
`collections.rs`'s `hash_items()` + sort + 20k `encode_bulk` calls. A perf
profile attributes ~11.9% of samples to `sort::unstable`, 7.8% to `hash_items`,
3.3% to `encode_bulk`, ~59% libc self and ~15% kernel; `LRANGE` has no sort and
is kernel-dominated.

**Risks and obligations.** `HGETALL` order is unspecified in Redis but this
repository's persistence test compares exact bytes; changing order is an explicit
compatibility/test decision, not an incidental optimization. A `BTreeMap` change
must preserve `HSET`/`HDEL`/`HGET` semantics and measure the write cost.

**100k extension (2026-11-12, section 3.31).** The gap grows with cardinality:
100k `HGETALL` is 132.3 ms versus 25.7/37.8, `HKEYS` 98.5 versus 16.5/15.8 and
`HVALS` 101.6 versus 19.7/17.9 (3.5–6.2×), while `HMGET` and the 10-member
controls stay at the floor. The same collect+sort+encode path covers all three
commands, so the candidate and AC31 must include `HKEYS`/`HVALS` at 100k, not
only the original 10k `HGETALL`. Do not extend it to `SMEMBERS`: the same shape
measured a lead/parity at 100k in section 3.46, so the hash answer must come from
a measured hash-specific change, not from removing the collect+sort everywhere.

**Rejection condition.** No gain above the spread; any order change without an
explicit documented decision and updated tests; any gate row loss.

**Acceptance (AC31).** A 10k-field `HGETALL` shall be within 10% of Redis or at
least 30% faster than the measured baseline; at 100,000 fields, `HGETALL`,
`HKEYS` and `HVALS` shall have p50 ≤1.5× the faster C server (≤39 ms for
`HGETALL`, ≤24 ms for `HKEYS`/`HVALS`) and be ≥3× faster than the measured
132/98/102 ms; ordering semantics shall be explicitly documented and all
order-sensitive tests updated to match; small hashes and the list path shall be
unchanged; the matched A/B shall show the gain; and both protected gates shall
pass on the exact head.

### 4.30 Sorted-set ordered index (candidate; admission measured)

**Mechanism.** Stop collecting and sorting all members per range read. Keep the
existing `HashMap<Vec<u8>, f64>` for point lookups and add an ordered index
maintained on `ZADD`/`ZINCRBY`/`ZREM`: a `BTreeSet<(ScoreKey, Vec<u8>)>` where
`ScoreKey` wraps `f64` with an `Ord` built on `f64::total_cmp` after normalizing
`-0.0` to `0.0` and rejecting NaN at parse, so iteration is `(score, member)`
order and ties match Redis. `ZSCORE` becomes the map lookup; `ZRANGE` walks the
index from 0 and `ZRANGEBYSCORE` uses `BTreeSet::range` for O(log n + k). A `std`
`BTreeSet` has no rank-select, so `ZRANK` and a deep-start `ZRANGE` are O(start +
k) and are split out to 4.38 rather than claimed here. The second option, a
lazily rebuilt sorted `Vec` snapshot invalidated on mutation, removes per-read
sorting entirely but pays O(n log n) on the first read after a write and
duplicates members; it is the fallback only if a write-heavy `ZADD` cohort
rejects the index.

**Prediction.** 10k `ZRANGE 0 9` moves from **528 req/s (4.61 ms p50)** toward
the C servers' ~10k (0.24–0.28 ms) and the 100k row from **57.4 ms to ≤0.5 ms**;
100k `ZRANGEBYSCORE ... LIMIT 0 10` from **62.4 ms to ≤0.5 ms**; 100k `ZSCORE`
from **3.27 ms to ≤0.2 ms** via the map lookup. The full range stays reply-bound;
`ZADD` pays one extra O(log n) insert/remove and the memory of a second member
copy (or a shared `Arc<[u8]>` key).

**Admission evidence (2026-11-12, sections 3.2 and 3.22).** 10,000-member zset,
4 clients, 250 requests each, three rotations: Rill `ZRANGE 0 9` 528/483/605
req/s versus Redis 10,311/8,361/13,229 and Valkey 10,771/12,789/9,239; full range
340/332/379 versus 1,424/1,581/1,824 and 1,036/759/1,091. Single-client 100k
cohort: `ZRANGE 0 9` p50 57.4 ms, `ZRANGEBYSCORE -inf +inf LIMIT 0 10` 62.4,
`ZSCORE` 3.27, versus 0.07–0.17 ms for the C servers; the 10k rows are 2.9–3.1
and the 10-element controls are at the floor. Perf at 10k: ~21.5% sort code,
~21.2% libc tuple moves in the sort, 6.4% `zset_items`.

**Risks and obligations.** Two structures must stay consistent under `ZADD`,
`ZINCRBY`, `ZREM`, `ZREMRANGEBY*`, expiry and replay; score equality and ±0.0;
member ties must be lexicographic; `ZSetPack` (≤128) must keep its current
behavior and convert atomically; `WITHSCORES` formatting and the
`ZRANGEBYSCORE`/`ZRANGEBYLEX` semantics must be unchanged (rank-select is
4.38); memory accounting and `maxmemory` eviction must count the index;
WATCH/version and transaction ordering stay as they are.

**Rejected options.** A Rust skiplist of our own adds a large hand-written
ordered structure to every mutation for the same asymptotic result as the `std`
`BTreeSet`; the `ordered-float`/`float-ord` crates add a dependency for ~20 lines
of total-order code; sorting only the requested slice is impossible because the
slice's start rank requires the global order.

**Rejection condition.** `ZADD` loses more than 10% in a matched cohort, memory
grows without an accounted bound, any zset semantic test differs, or a gate row
loses; no gain above the spread.

**Acceptance (AC32).** 10k `ZRANGE 0 9` ≥ 8,000 req/s (≥15× the 528 baseline,
within ~25% of Redis) and `ZRANGE 0 9999` ≥ 1,500 req/s (≥4.4× baseline) or
within 10% of Redis; at 100k, `ZRANGE 0 9` and
`ZRANGEBYSCORE -inf +inf LIMIT 0 10` shall have p50 ≤0.6 ms and p99 ≤2 ms
(≥90× faster than 57.4/62.4 ms, within ~8× Redis's 0.08–0.10 ms) and `ZSCORE`
p50 ≤0.2 ms (≥16× faster than 3.27 ms); the 10k→100k ratio for those rows ≤2×;
`ZADD` within 10% of its baseline in a matched cohort; every zset command,
tie-break, rewrite/replay and expiry test passes; the 128-member control and all
gate commands unchanged; `ZRANK`/`ZREVRANK`/deep-start `ZRANGE` are explicitly
out of scope here (4.38/AC40); no new dependency; both protected gates green on
the exact head.

### 4.31 Incremental cursor iteration (candidate; admission measured)

**Mechanism.** Make each `SCAN`-family call bounded by `COUNT` and index depth.
For the keyspace, maintain a per-owner ordered key index beside `entries`
(`BTreeSet<Arc<[u8]>>`, sharing the key allocation with the map, or
`BTreeSet<Vec<u8>>` if the sharing change is deferred) and resume from the last
emitted key. The cursor becomes an opaque `(owner, hex(last key))` token: the
router stops packing `owner × 2^40 + local` and carries the owner prefix plus the
token instead. A call does a range from the last key, applies MATCH/TYPE while
visiting, and caps visited elements per call so a selective `MATCH` cannot
degenerate into a keyspace walk; it returns a resume cursor. For collections,
large hashes become `BTreeMap` (option (b) of 4.29, which also removes the
`HGETALL` sort), large sets already use `BTreeSet` and can range directly, and
large sorted sets use 4.30's ordered index with `ZSCAN` defined as score order
(Redis leaves the order unspecified). Pack-encoded collections return all
elements with cursor 0, matching the C servers' listpack path. Per-call expiry
work becomes the normal bounded reap, not `reap(now, usize::MAX)`. The same
ordered key index serves `KEYS` (emit the range directly, no sort) and
`RANDOMKEY` (pick a random rank or sample the map), eliminating the per-call
sort and O(n) walk measured in section 3.31.

**Fallback.** A lazily rebuilt sorted key snapshot with the current numeric
cursor removes repeated sorts on a stable keyspace but still pays O(n log n) on
the first call after a write; it does not meet a bounded-per-call criterion and is
documented only as the fallback if the ordered index is rejected by its memory or
mutation cohort.

**Prediction.** 100k-key `SCAN COUNT 10` p50 falls from **42.6 ms** toward the C
servers' sub-millisecond range; full traversal falls from 4.81 s to ~0.25 s; the
10k `HSCAN` falls from 5.05 s to ~0.25 s; 100k `KEYS key:*` falls from 91.1 ms
toward 21–43 ms and `RANDOMKEY` from 0.52 toward ≤0.2 ms; `SET`/`GET` stay
within noise while `SET`/`HSET` pay one O(log n) index update.

**Admission evidence (2026-11-12, section 3.16).** 100,000 keys plus a
10,000-field hash, one connection, 300 spread-cursor `SCAN COUNT 10` calls and
full traversals, three rotations: per-call p50 42.64/46.76/42.58 ms versus Redis
0.26/0.32/0.24 and Valkey 0.10/0.16/0.22; full `SCAN COUNT 1000` 4.81 s over 101
calls versus 0.239 s and 0.167 s; full `HSCAN COUNT 10` 5.05 s over 1,000 calls
versus 0.257 s (~944) and 0.247 s (~800). PING floor 0.21/0.16/0.10 ms. The
measured cost is the per-call snapshot, not the reply. Section 3.31 adds the
keyspace rows: 100k `KEYS key:*` 91.1 ms versus 42.8/21.1 and `RANDOMKEY` 0.52
versus 0.21/0.29, both fixed by the same ordered index.

**Risks and obligations.** The cursor format changes from a decimal index to an
opaque token; `tests/scan.rs` asserts exact numeric cursors and sorted pages, and
the router's `owner × 2^40 + local` packing must change deliberately with tests
and a documented old-cursor behavior. Coverage on a stable keyspace must remain
exact; `MATCH`/`TYPE` selectivity must not turn one call into a whole-keyspace
walk; pack thresholds and conversions must keep their semantics; the index and
shared keys must be counted by memory accounting and rebuilt by replay/rewrite;
expiry must stay bounded; hex encoding must be binary-safe and dependency-free;
multi-owner traversal must not repeat or skip an owner.

**Rejected options.** A reverse-binary cursor over `std::collections::HashMap`
(no stable bucket API; `RawTable` is unstable and unsafe), a 40-bit hash-prefix
numeric cursor (collisions can skip keys and break coverage), the measured
per-call full snapshot, and an unbounded expiry reap.

**Rejection condition.** `SET`/`GET` or `HSET` loses more than 10% in a matched
cohort, any gate row loses, a coverage test can miss a stable key or fail to
terminate, pack semantics change, memory grows without an accounted bound, or no
gain above the spread.

**Acceptance (AC33).** On 100,000 keys, `SCAN COUNT 10` shall have p50 ≤ 1 ms and
p99 ≤ 5 ms (≥40× and ≥16× faster than the 42.6/81.3 ms baseline, within ~4×
Redis's 0.256 ms); a full `SCAN COUNT 1000` shall finish in ≤0.5 s (≥9× faster,
within ~2× Redis's 0.239 s) and a full 10k-field `HSCAN COUNT 10` in ≤0.5 s
(≥10× faster); each call's work shall be bounded by `COUNT` plus a fixed
lookahead, with no keyspace-sized allocation/sort and no unbounded expiry sweep;
a stable keyspace shall return every key exactly once across the traversal;
100k `KEYS key:*` shall be p50 ≤2× the faster C server (≤42 ms) and `RANDOMKEY`
≤0.2 ms with the ordering decision documented; the `KEYS`/`RANDOMKEY` ordering
and `DBSIZE`/expiry interactions shall be explicit tests;
MATCH/TYPE/NOVALUES, cursor errors and multi-owner traversal tests shall be updated
to the chosen semantics and pass; `SET`/`GET` shall stay within noise; no
dependency shall be added; and both protected gates shall pass on the exact head.

### 4.32 Packed short-element list nodes (candidate; admission measured)

**Mechanism.** Replace the one-`Vec<u8>`-per-element list with a quicklist-shaped
container: a `VecDeque<Node>` where `Node` is a byte arena holding short elements
inline and an out-of-line `Vec<u8>` only above a threshold, split at a byte budget
(mirroring `list-max-listpack-size -2`). A node index of cumulative element counts
keeps head/tail push/pop, index and `LSET` amortized O(1) and confines
`LINSERT`/`LREM` split/merge to one node. Iteration reads the arena sequentially,
so `LRANGE` keeps its measured lead; `payload_len` and journaling count arena plus
out-of-line bytes and the rewrite path encodes the same logical elements.

**Prediction.** A 200k-element 8-byte list falls from **142–143 B/element** toward
the C servers' 7.5 B (target ≤24) and `DEL`/`UNLINK` from **6.2–7.3 ms to ≤1 ms**
because a drop frees a few hundred nodes instead of 200,000 buffers. `RPUSH` and
`LRANGE` stay within 10% in a matched cohort; blocking pop and list index
semantics are unchanged.

**Admission evidence (2026-11-12, section 3.17).** One 200k-element list of
8-byte values, three rotations: RSS delta 27.8–28.0 MB (142–143 B/element) versus
Redis 1.47 MB (7.5 B) and Valkey 1.48–1.54 MB (7.5–7.9 B); `DEL` 7.28 ms versus
0.166/0.130 and `UNLINK` 6.21 ms versus 0.316/0.235; the 100-element control is
at the command floor. The source stores one buffer per element.

**Risks and obligations.** Node index arithmetic and split/merge are the
correctness core; binary-safe elements, a maximum element size that stays out of
line, and a bounded node budget must not change `LRANGE` order or `LINSERT`/
`LREM`/`LSET`/`LPOS`/blocking semantics; memory accounting and `maxmemory` must
count arena and out-of-line bytes; journal/rewrite must preserve exact logical
payloads; every list, persistence and blocking test must pass without unsafe code.

**Rejection condition.** `RPUSH`/`LRANGE` loses more than 10%, any list semantic
or persistence test changes, RSS is not materially lower, or a gate row loses.

**Acceptance (AC34).** A 200k-element 8-byte list shall use ≤24 B/element RSS
(≤3× the measured 7.5 and ≥6× below 143), and `DEL`/`UNLINK` of it shall be
≤1 ms p50 (≥7× faster than 7.28/6.21, within ~6× Redis's 0.166); `RPUSH`,
`LRANGE`, `LSET`, `LINSERT`, `LREM`, `LPOS` and blocking-pop semantics and tests
shall be unchanged with matched throughput within 10%; accounting/`maxmemory`,
rewrite/replay exactness and both protected gates shall pass on the exact head;
no dependency shall be added.

### 4.33 Deferred drop for UNLINK and FLUSH* ASYNC (candidate; admission measured)

**Mechanism.** Honor the `ASYNC` contract without a new thread: move the dropped
`Entry`/collection (or the whole `Data` maps on flush) into a bounded
`pending_free` queue and free a bounded number of elements/bytes per maintenance
tick; if the queue or per-command bound is exceeded, fall back to the current
synchronous drop and record the fallback. Accounting counts pending bytes until
freed, `DBSIZE`/`EXISTS`/`SCAN`/`TTL` reflect the deletion immediately, the
deletion is journaled and notified before the handoff, a recreated key is
independent, and shutdown drains the queue. `DEL` and `FLUSH* SYNC` stay
synchronous.

**Prediction.** `UNLINK` of a 200k list falls from **6.21 ms to ≤0.5 ms**
independent of element count, and `FLUSHALL ASYNC` on 20k keys from **9.20 ms to
≤1 ms**; total CPU is unchanged (the work moves to the tick), but the command
tail disappears. `DEL`/`SYNC` are unchanged.

**Admission evidence (2026-11-12, section 3.17).** `UNLINK` 200k list 6.21 ms
(max 9.8) versus Redis 0.316 and Valkey 0.235; the 100-element control is
0.28 ms; `FLUSHALL ASYNC` 20k keys 9.20 ms (max 17.4) versus 0.886/0.568, while
Rill's `SYNC` is 10.12 ms — ASYNC saves nothing. Redis/Valkey ASYNC saves
2.7–3.9× over their SYNC.

**Risks and obligations.** The queue and per-command work must be bounded with a
documented synchronous fallback; pending memory must stay in accounting and under
`maxmemory`/eviction; a flush must swap every index atomically; shutdown must
drain; notification, journal and WATCH/version order must not change; a key
recreated while its old value is pending must be correct; `DEL`/`SYNC` semantics
must not drift toward async.

**Rejected options.** An unbounded background free per command (queue/RSS growth
and no backpressure), unsafe allocator tricks or `madvise` (outside the reviewed
boundary), making `DEL` async by default (changes the Redis default contract and
notification timing without a target), and a detached free thread with no
shutdown join (racy exit and unaccounted memory).

**Rejection condition.** Any semantics or accounting test differs, pending memory
grows without a bound, shutdown does not drain, `DEL`/`SYNC` regresses more than
10%, or no gain above the spread.

**Acceptance (AC35).** `UNLINK` of a 200k-element list shall have p50 ≤0.5 ms and
p99 ≤2 ms (versus 6.21/9.8) independent of element count; `FLUSHALL ASYNC` on
20k keys shall be p50 ≤1 ms (versus 9.20) and ≤2× Redis's 0.886; `DBSIZE`,
`EXISTS`, `SCAN` and `TTL` shall reflect the deletion immediately; pending bytes
shall be accounted and bounded with a documented synchronous fallback; shutdown
shall drain; `DEL` and `FLUSHALL SYNC` shall stay within 10% of baseline;
notification/journal/WATCH ordering and persistence tests shall pass; no
dependency shall be added; and both protected gates shall pass on the exact head.

### 4.34 In-place string mutation (implemented in PR #258, `d334cde`)

**Mechanism.** Add a mutable update path: after admission/eviction through
`Data::reserve`, borrow the entry's value mutably and either `extend_from_slice`
the suffix (`APPEND`) or `resize(end, 0)` only when the offset passes the end and
copy the replacement slice (`SETRANGE`), then update `payload_len`, payload
accounting, `last_access` and `touch`. No temporary full-length `Vec`, no `put`
clone. `Vec` growth is amortized (doubling) and capacity stays on the entry, so
repeated appends stop copying the value; `try_reserve` runs first so an allocation
failure leaves the old value untouched. `SET`/`GET` and every other command keep
their existing paths.

**Prediction.** `APPEND` 64 B to 32 MB falls from **7.35 ms to ≤0.5 ms** and
`SETRANGE` 16 B from **7.19 ms to ≤0.5 ms**; 1 MB values move to the command
floor; bytes copied per call become O(delta) after the first growth step; small
values and `GETRANGE` are unchanged.

**Admission evidence (2026-11-12, section 3.18).** 1 MB and 32 MB values, one
connection, 200/100 repetitions, three rotations: 32 MB `APPEND` p50
6.81/8.86/7.35 ms versus Redis 0.14/0.18/0.12 and Valkey 0.13/0.12/0.24;
`SETRANGE` 6.98/8.86/7.19 versus 0.12/0.18/0.21 and 0.10/0.06/0.18; 64 B
controls 0.080/0.135 versus 0.12–0.17; `GETRANGE` 0.118 versus 0.195/0.243. The
7.2–7.35 ms matches two 32 MB copies at roughly 9 GB/s.

**Risks and obligations.** `reserve` can evict the key being mutated, so re-look
it up after admission and treat absence as a fresh value; the accounting delta
must be exact and an OOM must not partially mutate; `SETRANGE` growth must
zero-fill exactly and respect the 512 MiB limit; TTL, WATCH version, journal
ordering and rewrite snapshots must stay correct; `Vec` capacity growth must stay
within the existing payload limit; the `put` shrink behavior is not needed on this
path but `SET` overwrites must not regress.

**Rejected options.** Keeping the temporary-vector rebuild and only skipping the
`put` clone (still one full copy per call), a custom arena or unsafe buffer, and
changing `Data::put` to take ownership globally (touches every command for no
extra gain).

**Rejection condition.** Any bytes, semantics, TTL or journal test differs, an
OOM leaves a partial value, the small-value controls or `GETRANGE` regress, or no
gain above the spread.

**Acceptance (AC36).** `APPEND` of 64 B to a 32 MB value shall have p50 ≤0.5 ms
and p99 ≤2 ms (≥14× faster than 7.35/14.5, within ~4× Redis's 0.137/1.05);
`SETRANGE` of 16 B at offset 0 of a 32 MB value shall have p50 ≤0.5 ms (≥14×
faster than 7.19, within ~3× Redis's 0.184); a 1 MB value shall move to the
command floor; the 64 B controls and `GETRANGE` shall be unchanged; bytes
allocated/copied per operation shall be bounded by the delta plus amortized
growth; exact bytes, zero-fill growth, the 512 MiB limit,
TTL/WATCH/journal/replay/rewrite and accounting shall pass; no dependency shall
be added; and both protected gates shall pass on the exact head.

**Post-fix validation (2026-11-13, section 3.18).** With `Data::mutate_string`
merged, 32 MB `APPEND` p50 is **0.131 ms (p99 1.22)** and `SETRANGE` **0.185 ms
(p99 1.32)**, versus Redis 0.142/0.222 and Valkey 0.219/0.289; the 1 MB rows and
controls are at the command floor. AC36 is met (56×/39× faster than the
pre-fix baseline), and the landed persistence tests cover exact bytes, growth
and journal/replay. The acceptance stays as the regression contract.

### 4.35 In-place collection mutation (candidate; admission measured)

**Mechanism.** After admission/eviction through `Data::reserve`, borrow the
entry's collection mutably and update only the affected element:
`HashMap::insert` for hash and zset, `BTreeSet::insert`/`remove` for set, and
push/pop on the list's head/tail chunk, with field-TTL removal and `payload_len`
adjusted by the delta instead of re-summed. Convert between pack and large
encodings exactly when a threshold is crossed; empty collections are removed as
today; `touch`/journal/notification run after the mutation. The 10-element
controls stay on the pack path unchanged.

**Prediction.** `HSET` into a 10k hash falls from **6.45 ms to ≤0.3 ms**, `ZADD`
from 4.21 and `SADD` from 1.23; `RPUSH`/`LPOP` on a 200k list fall from
**18.3/17.8 ms to ≤0.3 ms**; at 100k members every hash/zset/set write falls
from **10.7–99.4 ms to ≤0.6 ms** and joins the C servers, because a write stops
cloning and freeing every element. Reads, small controls and the gate's string
rows are unchanged.

**Admission evidence (2026-11-12, section 3.19).** One connection, 100
repetitions, three rotations: 10k `HSET` p50 7.00/6.45/6.21 ms versus Redis
0.09/0.58/0.11 and Valkey 0.14/0.09/0.08; 200k `RPUSH` 19.86/18.34/17.36 versus
0.08/0.09/0.07 and 0.06/0.11/0.07; 200k `LPOP` 18.08/17.21/17.82 versus
0.34/0.19/0.53 and 0.07/0.21/0.07; `ZADD` and `SADD` medians 4.21/1.23 versus
0.08–0.22/0.08–0.35. Every 10-element control is at the command floor.

The 100k extension (2026-11-12, section 3.19) tightens this: 20 repetitions,
three rotations, on 100,000-member collections — `HSET` new field **92.7 ms**
versus 0.12/0.32, `HSET` existing 92.8 versus 0.21/0.08, `HDEL` 99.4 versus
0.15/0.15, `ZADD` 61.4 versus 0.14/0.23, `ZINCRBY` 57.3 versus 0.21/0.26,
`ZREM` 63.9 versus 0.10/0.13, `SADD` 11.2 versus 0.60/0.25, `SREM` 10.7 versus
0.20/0.08; the 10-member controls stay at 0.15–0.23 ms. Both adds and
updates/removals pay the same full rebuild, and the gap grows from 3.5–69× at 10k
to **18–1160×** at 100k.

**Risks and obligations.** `reserve` can evict the key being mutated, so re-look
it up and treat absence as a fresh collection; pack↔large conversion at a
threshold must preserve order and the compact representation; delta accounting
must be exact and an OOM must leave the old collection intact; field TTLs on
`HSET`/`HDEL`; empty-collection deletion; blocking-pop wake and `LPUSH` FIFO;
list chunk split/merge; WATCH version, journal and rewrite snapshots.

**Rejected options.** Keeping the rebuild but skipping the re-encode (still
O(n) copies), `Rc`/copy-on-write or a persistent-collection dependency, and
changing `Data::put_collection` to take ownership globally (touches every path
for no extra gain).

**Rejection condition.** Any collection semantic, persistence or accounting test
differs, an OOM leaves a partial collection, the small controls or a read path
regresses, or no gain above the spread.

**Acceptance (AC37).** `HSET` into a 10k hash, `ZADD` into a 10k zset and `SADD`
into a 10k set shall each have p50 ≤0.3 ms (within ~3× the faster C server and
≥4× faster than the measured 1.23–6.45 ms baselines); at 100,000 members
`HSET` (new/existing), `HDEL`, `ZADD`, `ZINCRBY`, `ZREM`, `SADD` and `SREM` shall
each have p50 ≤0.6 ms and p99 ≤2 ms (≥17× faster than the measured 10.7–99.4 ms
and within ~3× the faster C server); `RPUSH` and `LPOP` on a
200,000-element list shall have p50 ≤0.3 ms and p99 ≤2 ms (≥59× faster than
the measured 17.8–18.3 ms); the 10-element controls shall be unchanged; every
collection command, pack threshold, field TTL, blocking, persistence/rewrite/
replay and accounting test shall pass; no dependency shall be added; and both
protected gates shall pass on the exact head.

### 4.36 Constant-time random-member selection (candidate; admission measured)

**Mechanism.** Two slices. First, stop materializing: single-member commands use
reservoir sampling over the existing iteration, and count commands use a bounded
reservoir or partial Fisher–Yates over iteration, so memory is O(count) and no
sort or full copy happens. Second, add a dense member index where O(n) iteration
is still visible: for large sets a `Vec<member>` plus `HashMap<member, usize>`
with swap-remove, for hashes a `Vec<field>` beside the value map, and for zsets a
`Vec<member>` beside the score map (4.30's ordered index stays for ranges).
Single picks, count picks and `SPOP` then become O(1)/O(count) like dict random;
small pack paths keep their behavior. `SMEMBERS`/`HKEYS`/`ZSCAN` order is
unspecified in Redis, so the chosen order is an explicit test/ordering decision.

**Prediction.** At 100k, `SRANDMEMBER` falls from **11.54 ms to ≤0.6 ms**,
`HRANDFIELD` from 33.66, `ZRANDMEMBER` from 21.32 and `SPOP` from 11.11; the 10k
numbers fall to the command floor; the 10k→100k ratio becomes ≤2× (flat) instead
of 7–16×. Memory grows by ~8–16 B/member for the index, counted in accounting.

**Admission evidence (2026-11-12, section 3.20).** One connection, 100
repetitions at 10k and 50 at 100k, three rotations: 100k `SRANDMEMBER` p50
11.68/11.54/11.24 ms versus Redis 0.20/0.24/0.18 and Valkey 0.12/0.33/0.41;
`HRANDFIELD` 33.66/34.29/30.77 versus 0.34/0.14/0.40 and 0.11/0.26/0.10;
`ZRANDMEMBER` 21.32/22.76/20.61 versus 0.25/0.34/0.20 and 0.16/0.20/0.10;
`SPOP` 11.11 median versus 0.304/0.126. The 10k rows are 1.0–2.6 ms and every
10-element control is at the command floor.

**Risks and obligations.** Uniformity is the core: reservoir/swap correctness,
COUNT semantics including negative counts with repetition, and `SPOP` removal by
index plus empty deletion must be tested statistically. Pack↔large conversion
must rebuild the index atomically; index memory and tombstones must be counted
under `maxmemory`; the ordering decision for full-collection replies must be
documented and tests updated; journal, rewrite, TTL and WATCH paths must see the
same logical collection.

**Rejected options.** Sorting and picking from a clone (the measured path), a
hash-derived index without a position map (cannot remove in O(1)), and any foreign
RNG or persistent-collection dependency.

**Rejection condition.** A distribution test fails, any reply or semantic test
differs, index memory grows without an accounted bound, the 10k→100k ratio is not
flat, or a gate row loses.

**Acceptance (AC38).** At 100,000 members, `SRANDMEMBER`, `HRANDFIELD` and
`ZRANDMEMBER` (single and `COUNT 10`) and `SPOP` shall have p50 ≤0.6 ms and p99
≤2 ms (≥18× faster than the measured 11.1–33.7 ms and within ~3–5× the faster C
server); the 10k p50 shall be ≤0.3 ms and the 10k→100k p50 ratio ≤2×; a
distribution test over repeated draws shall be uniform within a stated tolerance;
10-element controls shall be unchanged; the ordering decision for
`SMEMBERS`/`HKEYS`/`ZSCAN` shall be documented with tests updated; `SPOP`
empty-deletion, TTL, blocking, persistence/rewrite/replay and accounting tests
shall pass; no dependency shall be added; and both protected gates shall pass on
the exact head.

### 4.37 Bitmap mutation and popcount (candidate; admission measured)

**Mechanism.** `SETBIT` reserves/admits the new length, then extends and patches
the entry's own `Vec` in place (zero-filling only when the offset passes the end)
instead of building a new value and copying it twice. The same in-place mutation
applies to the `BITFIELD`/`BITFIELD_RO` write forms: reserve for the highest
write offset, borrow the value, and apply `SET`/`INCRBY` subcommands directly;
the read-only `BITFIELD` path already borrows and stays unchanged. `BITCOUNT`
sums
`chunks_exact(8)` as little-endian `u64::count_ones()` and handles the head and
tail with per-byte popcount; `BITPOS` finds the first matching bit the same way
(`trailing_zeros` of the word, or of its complement for bit 0) with masked
head/tail words. BYTE and BIT ranges normalize to a byte span with edge masks.
`GETBIT` stays a direct read.

**Prediction.** 16 MB `SETBIT` falls from **5.32 ms to ≤0.5 ms**, `BITCOUNT`
from 23.0 to ≤6 ms, a not-found `BITPOS 0` from **201.6 ms to ≤8 ms** (within
~1.6× Redis), and a one-byte 16 MB `BITFIELD SET`/`INCRBY` from **4.39–4.68 ms
to ≤0.5 ms**; the 1 MB rows move to the command floor; the 64 B controls and a
first-bit hit are unchanged.

**Admission evidence (2026-11-12, sections 3.21 and 3.33; 2026-11-13, section
3.40).** One 1 MB and one
16 MB `0xff` bitmap (the 16 MB one built by 512 KiB `APPEND`s), 100/30
repetitions, three rotations: 16 MB `SETBIT` 5.32/5.58/4.85 ms versus Redis
0.09/0.71/0.35 and Valkey 0.31/0.53/0.22; `BITCOUNT` 22.48/23.24/23.01 versus
3.52/3.13/3.37 and 3.77/4.56/5.70; `BITPOS 0` 200.1/201.6/206.7 versus
5.3/5.1/4.9 and 5.7/4.6/5.1; `BITPOS 0 0 -1` 209.4 versus 4.79/5.96;
`BITPOS 1` 0.12 versus 0.60/0.09; 1 MB 0.331/1.85 versus 0.09–0.26/0.17–0.31;
64 B controls at the floor. The `BITFIELD` cohort (16 MB, 20 repetitions, three
rotations): `SET u8` 4.38/6.09/4.56 versus 0.24/0.06/0.09 and 0.42/0.09/0.07;
`INCRBY u8` 4.69/3.83/4.68 versus 0.24/0.08/0.12 and 0.55/0.11/0.07;
`SET u63` 4.38/4.71/4.39 versus 0.11/0.11/0.30 and 0.12/0.19/0.10; 100×
`INCRBY u8` 3.83/4.89/4.58 versus 0.31/0.24/0.41 and 0.31/0.54/0.25;
`BITFIELD_RO GET` 0.21/0.47/0.33 versus 0.28/0.25/0.14 and 0.20/0.09/0.14;
1 MB `SET u8` 0.14/0.59/0.51 versus 0.34/0.40/0.13 and 0.17/0.63/0.08; 64 B
controls at the floor.

**Risks and obligations.** `reserve` can evict the key being written, so re-look
it up; an OOM after a partial extend corrupts the value; zero-fill growth, the
offset cap and the 512 MiB limit must not change; BYTE/BIT range normalization
(negative indices, inverted ranges) and MSB-first bit numbering must be exact;
the not-found return values (`len*8` for bit 0 without an explicit end, `-1`
otherwise) must stay; `BITFIELD` integer widths/signs, `OVERFLOW` policies
(`WRAP`/`SAT`/`FAIL`), old-value replies and the highest-write-offset reservation
must stay exact, and the read-only path must not start copying; TTL, WATCH,
journal and rewrite must see the same value;
the popcount and bit scan must stay safe Rust (no target-feature intrinsics).

**Rejected options.** SIMD intrinsics or a CPU-feature dependency (unsafe or a
new crate for a path that a `u64` loop nearly closes), the current byte-and-bit
scan, and the rebuild-and-copy write (for `BITFIELD` as well as `SETBIT`).

**Rejection condition.** Any bit or range semantics test differs, an OOM leaves a
partial value, the controls or reads regress, or no gain above the spread.

**Acceptance (AC39).** `SETBIT` at offset 0 of a 16 MB bitmap shall have p50
≤0.5 ms and p99 ≤2 ms (≥10× faster than the measured 5.32 ms and within ~2×
Redis's 0.349 ms); a one-byte `BITFIELD SET`/`INCRBY` (u8 and u63) and a
100-subcommand write on the 16 MB value shall each have p50 ≤0.5 ms and p99
≤2 ms (≥8× faster than the measured 4.39–4.68 ms and within ~2× the faster C
server); `BITCOUNT` of the 16 MB bitmap shall have p50 ≤6 ms and p99
≤10 ms (≥3.8× faster than 23.0 ms and within ~2× Redis's 3.37 ms); a not-found
`BITPOS 0` shall have p50 ≤8 ms and p99 ≤15 ms (≥25× faster than 201.6 ms and
within ~1.6× Redis's 5.08 ms) while a first-bit hit stays at the floor; the 1 MB
rows shall be at the command floor; the 64 B controls, `GETBIT` and `STRLEN`
shall be unchanged; `BITFIELD_RO` and all-`GET` `BITFIELD` shall stay copy-free;
BYTE and BIT ranges including negative indices, the offset and 512 MiB limits,
zero-fill, `BITFIELD` widths/`OVERFLOW` policies, TTL/WATCH/journal/replay and
accounting shall pass; no dependency shall be added; and both protected gates
shall pass on the exact head.

### 4.38 Order-statistic sorted-set rank (candidate; admission measured)

**Mechanism.** 4.30's ordered set gives score order and score ranges but no
rank-select, so `ZRANK`/`ZREVRANK` and deep-start `ZRANGE` would still walk from
the first element. Add an order-statistic index over the same `(score, member)`
order: bucketed sorted blocks (e.g. 256 members each, with cached first/last
score and a count) plus cumulative counts, maintained on
`ZADD`/`ZINCRBY`/`ZREM` with split/merge. A rank is a binary search to the block,
the position within it and the sum of preceding block counts, O(log blocks +
block); reverse rank mirrors it, and `ZRANGE start stop` seeks the `start`-th
element the same way. `ZSCORE` and `ZRANGEBYSCORE` stay on 4.30's map/ordered
set. If 4.38 lands first, its block index can also serve 4.30's start-0 and
score ranges, making 4.30's separate `BTreeSet` redundant; the two candidates
should share one structure in that case.

**Prediction.** 100k `ZRANK`/`ZREVRANK` fall from **58.6–72.5 ms to ≤0.5 ms**,
`ZRANGE 50000 50009` from 63.7 to ≤0.5, the 10k rows to the command floor, and
the 10k→100k ratio from 19–24× to ≤2.5×. `ZADD` pays one block-index update in
addition to 4.30's ordered insert.

**Admission evidence (2026-11-12, section 3.22).** 100,000-member zset, one
connection, 20 repetitions, three rotations: `ZRANK` middle p50 58.56/100.91/
57.21 ms versus Redis 0.36/0.06/0.16 and Valkey 0.07/0.07/0.23; `ZRANK` last
72.5, `ZREVRANK` 71.4, `ZRANGE 50000 50009` 63.7, `ZRANGEBYSCORE ... LIMIT 0 10`
62.4, `ZRANGE 0 9` 57.4, versus 0.07–0.17 for both pins; 10k rows 2.4–3.1 ms;
10-element controls 0.28/0.11 versus 0.10–0.14.

**Risks and obligations.** A score update moves a member between blocks, so the
block index and 4.30's order must change under one admitted mutation; split and
merge boundaries, duplicate scores and member tie-break, `ZREM` count
maintenance, negative indices, reverse ranges, `LIMIT`, `ZSetPack` conversion,
memory accounting, replay/rewrite and WATCH must all stay exact. The block size
is the search-versus-update trade and needs a matched ZADD cohort.

**Rejected options.** A full in-repo AVL/weight-balanced tree with subtree sizes
(more code than bucketed counts for the same asymptotics), accepting O(n) rank
(the measured 378–927× gap), a Fenwick tree rebuilt on every mutation (O(n)
writes), and the current materialize+sort.

**Rejection condition.** Any rank/reverse/deep-range semantic test differs,
`ZADD` loses more than 10% in a matched cohort, index memory grows without a
bound, the 10k→100k ratio is not flat, or a gate row loses.

**Acceptance (AC40).** At 100k, `ZRANK`/`ZREVRANK` for the first, middle and last
member and `ZRANGE 50000 50009`/`ZREVRANGE` shall have p50 ≤0.5 ms and p99 ≤2 ms
(≥100× faster than the measured 58.6–72.5 ms and within ~8× the faster C
server); the 10k p50 shall be ≤0.2 ms and the 10k→100k ratio ≤2.5×;
`ZADD`/`ZINCRBY`/`ZREM` shall stay within 10% in a matched cohort; tie ordering,
negative indices, reverse ranges, `LIMIT`, `ZSetPack`, rewrite/replay, TTL and
accounting tests shall pass; 4.30's AC32 rows shall also pass; no dependency
shall be added; and both protected gates shall pass on the exact head.

### 4.39 Contract-correct pub/sub fan-out (candidate; admission measured)

**Mechanism.** Keep the per-subscriber boundary but make it contract-correct and
bounded in bytes. Look the channel up with `HashMap::get_mut` and skip the
full-map `retain` scan on every publish; encode one frame per (RESP version,
pattern) and share it across subscribers; queue by **bytes** with a documented
limit and one of two explicit overflow behaviors: drop the message, keep the
subscription and increment a counted metric (at-most-once is already the pub/sub
contract), or disconnect the client with a documented error, mirroring Redis's
`client-output-buffer-limit pubsub`. Drain by output bytes per wake (the existing
64 KiB output chunk) instead of a hard 64 frames, so an active reader can clear a
burst; keep the 1 ms tick as the fallback from 4.24. The same queue and contract
apply to **MONITOR**: a monitor line may be dropped with a counted metric or the
monitor disconnected, but never silently truncated.

**Prediction.** A 2,000-message burst (64 KB) to an active subscriber is fully
delivered and the subscription survives; `PUBLISH` among 10,000 channels falls
from **0.38–0.89 ms to ≤0.3 ms**; 100 subscribers receive every message with the
publisher within ~3× the C servers; a 2,000-command burst with a monitor attached
delivers all 2,000 lines; nothing is silently lost.

**Admission evidence (2026-11-12, sections 3.23 and 3.37).** One subscriber plus
a 2,000-message pipeline, three rotations: Rill delivered 1,024/2,000 and
`PUBSUB NUMSUB` returned 0 after the burst; Redis/Valkey delivered 2,000/2,000
and returned 1. `PUBLISH` p50 at 1 channel 0.12–0.22 versus 0.13–0.30, at
1,000 channels 0.13 versus 0.16–0.30, and at 10,000 channels 0.38–0.89 versus
0.12–0.31. A 2,000-command burst with 1/10 monitors delivered exactly
1,024/2,000 lines per monitor in all six samples versus 2,000/2,000 for both C
servers; publisher fan-out timings at 0–1 monitors were within the host spread
and cheaper at 10 monitors because the queue stops it.

**Risks and obligations.** Bounding by bytes must account the encoded frame and
the queue, and an overflow must never remove the subscription silently or
truncate a monitor feed without a counted/documented behavior; ordering
per publisher, at-most-once, pattern and shard channels, `UNSUBSCRIBE`/`NUMSUB`
and shutdown must stay exact; the shared frame must be immutable and the
per-subscriber RESP version/pattern variant respected; memory accounting and the
1 ms fallback stay; a metric must make drops visible.

**Rejected options.** Keeping the 1,024-message queue and silently removing the
subscriber (measured loss), an unbounded queue (memory growth), and switching to
a per-message thread or a new async runtime (outside the dependency boundary).

**Rejection condition.** Any message is lost without a counted/documented
behavior, a subscription disappears while the client stays connected, ordering or
pattern/shard semantics change, memory grows without a bound, or a gate row
loses.

**Acceptance (AC41).** A 2,000-message pipelined burst to an active subscriber
shall deliver 2,000/2,000 and keep `PUBSUB NUMSUB` at 1; if the documented byte
bound is exceeded, the behavior shall be a counted drop with the subscription
intact or a documented disconnect, never a silent removal; `PUBLISH` among
10,000 subscribed channels shall have p50 ≤0.3 ms (≥2.7× faster than 0.80 ms and
within ~2× the C server) and stay flat from 1 to 10,000 channels; 100 subscribers
shall receive every message with publisher p50 within ~3× the C servers; a
2,000-command burst with a monitor attached shall deliver all 2,000 lines (or a
counted drop/document disconnect, never a silent truncation); ordering,
pattern/shard, unsubscribe and shutdown tests shall pass; no dependency shall be
added; and both protected gates shall pass on the exact head.

### 4.40 O(1) cardinality and membership (candidate; admission measured)

**Mechanism.** Add `Collection::len()` (list = sum of the inner deque lengths,
O(nodes); set/hash/zset = the container length) and direct probes:
`BTreeSet::contains` for `Set`, `parse_integer` + `BTreeSet<i64>::contains` for
`SetInt` (a non-integer probe returns 0), a linear scan only for the small
`SetPack`/`HashPack`, and `HashMap::contains_key`/`hash_get` for `Hash`. Route
`SCARD`/`SISMEMBER`/`HLEN`/`ZCARD`/`LLEN` through them; leave
`SMEMBERS`/`HKEYS`/`HGETALL` sorting and reply construction unchanged.

**Prediction.** 100k `SISMEMBER`/`SCARD` fall from **8.0–8.7 ms to ≤0.2 ms**,
`HLEN`/`ZCARD` from 1.6–2.2 to ≤0.1, and 200k `LLEN` from 1.55 to ≤0.05; controls
and the direct `HEXISTS`/`STRLEN` paths are unchanged, with no reply or ordering
change.

**Admission evidence (2026-11-12, section 3.24).** 100k set/hash/zset and a
200k list, 100 repetitions, three rotations: `SISMEMBER` hit p50 8.45/7.70/8.04
ms versus Redis 0.19/0.07/0.40 and Valkey 0.22/0.13/0.27; miss 8.67 versus
0.17/0.21; `SCARD` 8.18/7.02/8.08 versus 0.14/0.20/0.37 and 0.18/0.15/0.08;
`HLEN` 2.24 versus 0.15/0.19; `ZCARD` 1.57 versus 0.12/0.16; `LLEN` 1.55 versus
0.20/0.14; `HEXISTS` 0.18 versus 0.29/0.14 and `STRLEN` 0.13 versus
0.16/0.16. Ten-element controls 0.12–0.18.

**Risks and obligations.** The `SetInt` probe must reproduce the current string
semantics for integer and non-integer members; `SetPack`/`HashPack` stay linear
but are small; `LLEN` becomes an O(nodes) walk (cached counts can follow if a
later cohort shows it); missing/empty keys and wrong-type errors are unchanged;
no accounting or reply changes.

**Rejected options.** A separately cached length that can drift from the
container, making `SMEMBERS` unsorted to speed `SCARD` (it does not fix
`SISMEMBER` and changes ordering), and leaving the materialization.

**Rejection condition.** Any membership or length semantics test differs, the
controls or direct paths regress, or no gain above the spread.

**Acceptance (AC42).** At 100,000 members, `SISMEMBER` (hit and miss) and
`SCARD` shall have p50 ≤0.2 ms (≥40× faster than the measured 8.0–8.7 ms and
within ~2× the faster C server); `HLEN` and `ZCARD` p50 ≤0.1 ms; `LLEN` on a
200,000-element list p50 ≤0.05 ms; the 10-element controls, `HEXISTS` and
`STRLEN` shall be unchanged; `SetInt`/`SetPack`/`HashPack` semantics (including
non-integer probes), missing keys and type errors shall pass; no dependency shall
be added; and both protected gates shall pass on the exact head.

### 4.41 In-place list seek (candidate; admission measured)

**Mechanism.** Add `Collection::list_get(index)` (normalize the negative index;
`ListPack` indexes directly, `ListQuick` walks chunk lengths without allocating,
O(nodes) ≈ 1.5k for 200k) and `Collection::list_position(value, rank, count,
maxlen)` that scans chunks in order with an early stop and rank handling. Route
`LINDEX`/`LPOS` through them; `LRANGE`, `LLEN` (4.40) and the write paths are
unchanged.

**Prediction.** 200k `LINDEX` (0/middle/last) fall from **1.29–1.47 ms to
≤0.2 ms**, a hit-first `LPOS` from 1.26 to ≤0.2, a missing `LPOS` stays ≤2 ms
(it already leads the C servers), and the 10-element controls are unchanged.

**Admission evidence (2026-11-12, section 3.25).** A 200,000-element 8-byte list,
100 repetitions, three rotations: `LINDEX 0` p50 1.473 versus Redis 0.107 and
Valkey 0.068; middle 1.404 versus 0.104/0.081; last 1.289 versus 0.102/0.142;
`LPOS` hit-first 1.256 versus 0.095/0.127; `LPOS` missing 1.806 versus
5.019/5.854; controls 0.097/0.097 versus 0.09–0.12.

**Risks and obligations.** Negative-index normalization and out-of-range nil,
chunk boundaries, `RANK` (including negative), `COUNT 0`-means-all and `MAXLEN`
semantics, missing/empty keys and wrong-type errors must not change; no reply or
accounting change.

**Rejected options.** A maintained node index just for `LINDEX` (the chunk walk
is already O(nodes)), keeping the full materialization (the measured path), and a
list representation change (4.32 covers memory independently).

**Rejection condition.** Any index or `LPOS` semantics test differs, the controls
or `LRANGE`/`LLEN` regress, or no gain above the spread.

**Acceptance (AC43).** On a 200,000-element list, `LINDEX` for 0, middle and −1
and a hit-first `LPOS` shall have p50 ≤0.2 ms and p99 ≤2 ms (≥6× faster than the
measured 1.26–1.47 ms and within ~3× the faster C server); a missing `LPOS` shall
stay ≤2 ms; the 10-element controls, `LRANGE` and `LLEN` shall be unchanged;
negative indices, `RANK`/`COUNT`/`MAXLEN`, missing keys and wrong-type errors shall
pass; no dependency shall be added; and both protected gates shall pass on the
exact head.

### 4.42 In-place stream append (candidate; admission measured)

**Mechanism.** Admit the new entry's bytes, then borrow `Entry.stream` mutably
and insert one `StreamEntry` into the existing `BTreeMap`, updating `payload_len`
and the stream counters (`entries_added`, `last_generated`, `max_deleted` on
trim) by the delta. No `stream.clone()`, no `payload_bytes()` fold and no
`put_stream` replace; groups/PEL and trimming mutate the same stream.

**Prediction.** 5k `XADD` falls from **1.36 ms to ≤0.2 ms** (p99 11.6 → ≤2) and
1k from 0.263 to ≤0.15, so the 1k→5k ratio becomes ≤2× (flat); `XLEN`,
`XRANGE`, `XREVRANGE` and the 10-entry control are unchanged. The consumer-group
read path is independent of this clone and measured near-parity in section 3.39.

**Admission evidence (2026-11-12, section 3.26).** A 1,000- and a 5,000-entry
stream, 30/50 repetitions, three rotations: 5k `XADD` p50 1.36/1.53/1.22 versus
Redis 0.30/0.10/0.07 and Valkey 0.06/0.08/0.14; 1k 0.25/0.43/0.26 versus
0.10/0.71/0.15 and 0.08/0.10/0.08; `XLEN` 0.072 versus 0.156/0.103; `XRANGE`
first/mid 0.121/0.120 versus 0.099–0.307/0.187–0.288; `XREVRANGE` 0.118 versus
0.117/0.136; 10-entry control 0.077/0.118 versus 0.078/0.103 and 0.231/0.143.

**Risks and obligations.** `reserve` can evict the key being appended, so
re-look it up; an OOM must not leave a partial entry; trimming, `entries_added`,
`last_generated`, `max_deleted`, groups/PEL, TTL/WATCH/journal/rewrite and the
reply ID must stay exact; the append must not change read or trim semantics.

**Rejected options.** Keeping the clone and only removing the second payload
fold, a separately cached length that can drift from the map, and an
append-only log outside the stream (breaks the existing range/read paths).

**Rejection condition.** Any stream, group/PEL, trim, rewrite or accounting test
differs, an OOM leaves a partial entry, the controls or reads regress, or the
1k→5k ratio is not flat.

**Acceptance (AC44).** At 5,000 entries, `XADD` shall have p50 ≤0.2 ms and p99
≤2 ms (≥6.8× faster than the measured 1.36/11.6 ms and within ~2× Redis's
0.104/1.87 ms); at 1,000 entries p50 ≤0.15 ms; the 1k→5k p50 ratio ≤2×; `XLEN`,
`XRANGE`, `XREVRANGE`, groups/PEL, trimming, rewrite/replay, TTL and accounting
shall be unchanged; the 10-entry control shall be unchanged; no dependency shall
be added; and both protected gates shall pass on the exact head.

### 4.43 Geospatial neighbor-cell search (candidate; admission measured)

**Mechanism.** Reuse 4.30/4.38's ordered `(score, member)` index. For
`GEOSEARCH`/`BYRADIUS`/`BYBOX`, compute the geohash cell size for the shape,
enumerate the nine neighbour cells (with dateline/pole handling), map each cell
to an inclusive score range, range-read only those scores, decode and apply the
exact haversine/box test, then sort (`ASC`/`DESC`) and apply `COUNT`. `FROMMEMBER`
and `GEOPOS`/`GEODIST`/`GEOHASH` use the member dict/ordered index directly.
Keep the WGS-84 formulas, units, `WITHCOORD`/`WITHDIST`/`WITHHASH` and `STORE`.

**Prediction.** 50k 1 km `GEOSEARCH` falls from **4.34 ms to ≤0.3 ms**, the
2×2 km box from 4.49 to ≤0.3, `GEOPOS` 1 from 1.99 to ≤0.2 and 100 from 24.9 to
≤0.5, and `GEODIST` from 3.12 to ≤0.2; the all-50k 5 km search stays reply-bound
and the controls are unchanged.

**Admission evidence (2026-11-12, section 3.27).** A 50,000-member geo grid over
~0.5°, 50 repetitions, three rotations: 1 km search p50 4.34/4.40/3.85 versus
Redis 0.12/0.16/0.16 and Valkey 0.14/0.17/0.23; 2×2 km box 4.49 versus
0.21/0.19; 5 km (all members) 10.38 versus 10.09/10.91; `GEOPOS` 1
3.03/1.32/1.99 versus 0.14/0.19 and 100 26.86/24.86/22.33 versus 0.47/0.61;
`GEODIST` 0.86/3.77/3.12 versus 0.168/0.167; 10-member controls 0.149/0.139
versus 0.24/0.165 and 0.18/0.264.

**Risks and obligations.** The cell computation is the correctness core:
neighbour enumeration, dateline and pole edges, inclusive score bounds, duplicate
members when cells overlap, `BYBOX` width/height versus `BYRADIUS`, `FROMMEMBER`
near an edge, and `ASC`/`DESC`/`COUNT` ordering must match today's linear result
exactly. WGS-84 formulas, units, `WITH*`, `STORE` and the zset representation stay
unchanged; the search must remain bounded by the result and payload limits.

**Rejected options.** Keeping the linear scan (measured 25–53×), caching
per-member distances (stale on zset updates and O(n) memory per query), and an
R-tree/S2 dependency (outside the policy and unnecessary for 52-bit geohash
ranges).

**Rejection condition.** Any result member, order, distance, hash or coordinate
differs from the linear reference on the boundary fixture, the controls or the
all-member search regress, or no gain above the spread.

**Acceptance (AC45).** At 50,000 members, a 1 km `GEOSEARCH` and a 2×2 km
`BYBOX` shall have p50 ≤0.3 ms and p99 ≤2 ms (≥14× faster than the measured
4.34/4.49 ms and within ~2× the faster C server); `GEOPOS` for 1 and 100 members
shall be ≤0.2 ms and ≤0.5 ms (≥45× for 100), and `GEODIST` ≤0.2 ms (≥15×); an
all-member search and the 10-member controls shall be unchanged; results
(members, order, distances, hashes, coordinates) shall match the current linear
implementation exactly on a boundary/dateline fixture; `ASC`/`DESC`/`COUNT`,
`FROMMEMBER`, `BYRADIUS`/`BYBOX`, `WITH*` and `STORE` tests shall pass; no
dependency shall be added; and both protected gates shall pass on the exact head.

### 4.44 Per-member probes for multi-member lookups (candidate; admission measured)

**Mechanism.** For `SMISMEMBER`, call `Collection::contains_member` (4.40) once
per requested member; for `ZMSCORE`, call a direct
`Collection::zset_score(member)` over the existing score map (4.30) once per
requested member. Do not call `set_items()`/`zset_items()` or build an
all-member vector. Missing members return 0/nil, duplicate members answer
independently, and the `SetInt`/`SetPack`/`HashPack` semantics stay as today.

**Prediction.** 100k `ZMSCORE` with 100 members falls from **273 ms to ≤0.5 ms**,
10 members from 33.4 to ≤0.2 and 1 from 8.0 to ≤0.15; `SMISMEMBER` with
1/10/100 members from **7.9/8.25/11.75 ms to ≤0.2/≤0.2/≤0.5**; `HMGET` and the
10-member controls are unchanged.

**Admission evidence (2026-11-12, section 3.28).** A 100,000-member set/hash/zset,
50 repetitions, three rotations: `SMISMEMBER` 1/10/100 p50
7.90/8.49/12.73, 8.66/8.25/11.75, 7.90/7.42/10.90 versus Redis
0.094/0.227/0.394 and Valkey 0.075/0.077/0.211; `ZMSCORE`
11.77/23.42/273.23, 5.35/43.97/336.85, 8.02/33.37/221.99 versus
0.102/0.422/0.630 and 0.112/0.328/0.187; `HMGET` 1/10/100 0.23/0.36/0.32
versus 0.47/0.30/0.52 and 0.11/0.07/0.17; controls 0.33/0.26/0.26.

**Risks and obligations.** `SetInt` must answer integer and non-integer probes
exactly as today; duplicates, order, missing/empty keys and wrong types must not
change; `ZSetPack` stays linear but small; no accounting, reply shape or read
path changes.

**Rejected options.** Keeping `set_items`/`zset_items` with a per-member scan
(the measured O(n·m)), and caching the materialized vector across members (still
O(n) time and memory per command).

**Rejection condition.** Any membership/score/order/type test differs, the
controls or `HMGET` regress, or no gain above the spread.

**Acceptance (AC46).** At 100,000 members, `ZMSCORE` with 100 members shall have
p50 ≤0.5 ms and p99 ≤2 ms (≥500× faster than the measured 273 ms and within ~3×
the faster C server); 10 members ≤0.2 ms and 1 member ≤0.15 ms; `SMISMEMBER` with
100 members ≤0.5 ms (≥23× faster than 11.75 ms), 10 members ≤0.2 ms and 1 member
≤0.2 ms; `HMGET` and the 10-member controls shall be unchanged; missing,
non-integer and duplicate members and the `SetInt`/`SetPack` semantics shall pass;
no dependency shall be added; and both protected gates shall pass on the exact
head.

### 4.45 Bounded lazy expiry on command paths (candidate; admission measured)

**Mechanism.** Replace every `reap(now, usize::MAX)` on a command path: reads
expire only the keys they touch (`expire_key`, already present); `KEYS` filters
`entry.alive(now)` while it already scans all keys; `SCAN` filters per visited
element (4.31); `RANDOMKEY` picks a random live key with a bounded retry and a
fallback scan; `DBSIZE` returns the Redis-compatible dict size (expired but
unreaped keys may still be counted) as an explicit semantic decision; `MSET`/
`MSETEX` expire only their destination keys; admission uses a bounded reap (e.g.
128) and then eviction/policy. A mass expiry can no longer force O(backlog) work
inside one command.

**Prediction.** After 30k keys expire, `MGET`/`KEYS`/`RANDOMKEY`/`DBSIZE` fall
from **36–46 ms to ≤2 ms**, the lazy `GET` stays ~0.9 ms, the periodic purge
drains the backlog over later ticks, and no expired value is ever returned.

**Admission evidence (2026-11-12, section 3.29).** 30,000 keys with `PX 400`, a
0.7 s wait, one command per fresh wave, two waves per command, three rotations:
`MGET` 36.29 ms (samples 29.31/51.17, 27.95/43.26, 26.94/51.35) versus Redis
0.69 and Valkey 0.98; `KEYS` 43.58 versus 0.63/0.80; `RANDOMKEY` 45.45 versus
0.65/0.60; `DBSIZE` 41.21 versus 0.58/0.83; lazy `GET` 0.92 versus 0.75/0.72.
The purge costs ~1.2–1.5 µs per expired key.

**Risks and obligations.** `DBSIZE` becomes approximate like Redis, so its tests
and documentation must say so; `KEYS`/`SCAN`/`RANDOMKEY`/`MGET` must never return
an expired value; admission after a mass expiry may temporarily refuse or evict
until the bounded purge catches up — a documented, bounded behavior instead of an
unbounded stall; TTL notifications, WATCH and accounting order stay exact.

**Rejected options.** Keeping the unbounded purge (the measured stall), raising
the periodic purge rate without bound (CPU cost), and reaping on a detached
thread (ordering/accounting complexity for no extra gain).

**Rejection condition.** Any expired key is returned, a TTL/accounting/WATCH test
differs, the admission path can stall proportionally to the backlog, or the
measured command stall remains.

**Acceptance (AC47).** After 30,000 keys expire with no intervening command,
`MGET` (100 keys), `KEYS prefix*`, `RANDOMKEY` and `DBSIZE` shall have p50 ≤2 ms
and p99 ≤5 ms (≥18× faster than the measured 36–46 ms and within ~5× the faster
C server); a lazy `GET` shall stay ≤1 ms; reads shall never return an expired
value; the `DBSIZE` semantics shall be documented with tests updated; admission
shall stay bounded and atomic; TTL expiry/notification/WATCH tests shall pass; no
dependency shall be added; and both protected gates shall pass on the exact head.

### 4.46 Direct `BY nosort` slice (candidate; admission measured)

**Mechanism.** When `BY` has no `*` (`nosort`), do not call `source_values` or
build `Item`s for the whole source: apply `offset`/`count` directly to the
sequence — index the list range without cloning, or iterate the first
`offset + count` members of a set/zset in its current deterministic order (sets
sorted by member, zsets by score/member as today). Materialize only when `STORE`
needs the selected values or no `LIMIT` is present. The sorted, ALPHA, weighted
and `GET` paths are untouched.

**Prediction.** 100k `SORT ... BY nosort LIMIT 0 10` falls from **7.96 ms to
≤0.3 ms** (p99 17.2 → ≤2), while full-nosort remains reply-bound and every other
SORT row is unchanged.

**Admission evidence (2026-11-12, section 3.30).** A 100k integer list plus
`w<i>` weight keys, 50 repetitions, three rotations: `BY nosort LIMIT 0 10`
p50 7.8/8.0/8.7 versus Redis 0.12 and Valkey 0.16 (p99 17.2/1.17/0.77); sorted
`SORT` 92.5 versus 84.8/105.6, `LIMIT` 32.4 versus 26.0/29.0, ALPHA 90.2 versus
108.1/106.8, DESC 33.5 versus 28.2/28.1, `BY w*` 117.5 versus 92.1/80.4,
`GET #` 30.0 versus 27.3/26.9; 100-element controls 0.21/0.27.

**Risks and obligations.** The set/zset source order must stay exactly as today
(sets sorted by member, zsets by score/member); negative or oversized
`offset`/`count`, `STORE`, missing keys and list chunk boundaries must not change;
no reply or accounting change.

**Rejected options.** Rewriting all SORT to sort borrowed indices (not justified:
the other rows are 0.9–1.5×), and keeping the full materialization (the measured
50–66×).

**Rejection condition.** Any source order, limit or `STORE` result differs, the
controls or another SORT row regresses, or no gain above the spread.

**Acceptance (AC48).** On a 100,000-element list, `SORT ... BY nosort LIMIT 0 10`
shall have p50 ≤0.5 ms and p99 ≤2 ms (≥16× faster than the measured 7.96/17.2 ms
and within ~4× Redis's 0.12/1.17 ms); the full, limited, ALPHA, DESC, `BY w*` and
`GET #` rows shall stay within 1.5× of baseline; the 100-element controls shall
be unchanged; offset/count edges, `STORE`, missing keys and set/zset sources
shall pass; no dependency shall be added; and both protected gates shall pass on
the exact head.

### 4.47 In-place HyperLogLog registers with a cached count (candidate; admission measured)

**Mechanism.** Keep the HLL in the entry as today (exact/dense), but mutate the
dense registers in place under `entries.get_mut` instead of decode → serialize →
`put`, updating `payload_len` by the delta. Add a cached cardinality and dirty
bit in the header (as Redis's `card[8]`): `PFADD`/`PFMERGE` invalidate,
`PFCOUNT` returns the cache when valid and otherwise computes once and stores it.
Merging mutates the destination registers in place.

**Prediction.** 100k pipelined adds fall from **1.70 s to ≤0.6 s** (≤6 µs/add)
and 10k repeated counts from **2.66 s to ≤0.15 s** (≤15 µs/call); the
single-call p50 (client floor) and the 10-element controls are unchanged.

**Admission evidence (2026-11-12, section 3.32).** 100,000 distinct elements
added in 2,000-command pipelines, then 10,000 `PFCOUNT` calls, three rotations:
`PFADD` 1,709/1,664/1,696 ms (16.96 µs/add) versus Redis 410/421/367 (4.10) and
Valkey 389/487/392 (3.92); `PFCOUNT` 2,655/2,771/2,646 ms (265.5 µs/call) versus
35.7 (3.57) and 33.1 (3.31); single `PFADD`/`PFCOUNT` p50 0.187/0.484 versus
0.151/0.169 and 0.302/0.059; 10-element controls 0.164/0.185 versus 0.195/0.228
and 0.073/0.123.

**Risks and obligations.** Every add/merge and every exact↔dense conversion must
invalidate the cache; a stale cache returns a wrong count. The serialized header
must stay compatible with rewrite/replay/restore, counts must stay within the
HLL error bound (0.81% at 16k registers) and identical before/after
serialization, and expiry/TTL/accounting must see the same payload.

**Rejected options.** Keeping a count cache outside the HLL payload
(accounting/rewrite divergence), recomputing every count (the measured 74–80×),
and a sparse-encoding rewrite (not needed for the measured gap).

**Rejection condition.** Any count, merge, serialization or conversion result
differs, the controls regress, or no gain above the spread.

**Acceptance (AC49).** 100,000 pipelined `PFADD` shall finish in ≤0.6 s
(≤6 µs/add, ≥2.8× faster than the measured 1.70 s and within ~1.6× the faster C
server); 10,000 repeated `PFCOUNT` calls shall finish in ≤0.15 s (≤15 µs/call,
≥17× faster than 2.66 s and within ~5× the faster C server); single-call p50 and
the 10-element controls shall stay within noise; counts shall be within the HLL
error bound and identical before/after serialization; the cache shall be
invalidated by add/merge and exact↔dense conversion; rewrite/replay/restore and
TTL tests shall pass; no dependency shall be added; and both protected gates
shall pass on the exact head.

### 4.48 In-place stream group creation (candidate; admission measured)

**Mechanism.** For `XGROUP CREATE`, borrow `Entry.stream` mutably, insert the
`ConsumerGroup` (or create an empty stream for `MKSTREAM`), and call `touch`;
do not clone the stream, fold `payload_bytes`, or replace through `put_stream`.
`payload_len` stays unchanged because it counts only entries, and the other
XGROUP subcommands already mutate in place.

**Prediction.** 5k `XGROUP CREATE` falls from **8.26 ms to ≤0.2 ms**, the
10-entry control is unchanged, and no read, PEL or other XGROUP path changes.

**Admission evidence (2026-11-12, section 3.34).** A 5,000-entry stream plus a
10-entry control, 20 unique groups each, three rotations: `XGROUP CREATE`
8.26 ms (7.47/13.1/8.26) versus Redis 0.083 (0.07/0.10/0.08) and Valkey 0.094
(0.09/0.13/0.09); 10-entry control 0.338 versus 0.120/0.097; `CREATECONSUMER`
0.123 versus 0.227/0.127; `SETID` 0.099 versus 0.283/0.099; `DESTROY` 0.109
versus 0.097/0.109; 10-entry `CREATECONSUMER` 0.169 versus 0.086/0.069.

**Risks and obligations.** `BUSYGROUP`, explicit-ID/`$` `last_delivered`,
`MKSTREAM` creating an empty stream, PEL/consumer state, unchanged payload
accounting, TTL/WATCH/journal/rewrite and notification order must all stay
exact; the in-place insert must not leave a partially created group on OOM (the
group maps are small and the borrow is already admitted).

**Rejected options.** Keeping the clone and only skipping the payload fold
(still O(n) copies), and a separate group registry outside the stream (breaks
range/read/rewrite consistency).

**Rejection condition.** Any group, PEL, `MKSTREAM` or `BUSYGROUP` semantics
differ, the controls regress, or no gain above the spread.

**Acceptance (AC50).** On a 5,000-entry stream, `XGROUP CREATE` shall have p50
≤0.2 ms and p99 ≤2 ms (≥40× faster than the measured 8.26 ms and within ~2.5×
the faster C server); the 10-entry control shall be unchanged;
`BUSYGROUP`/`MKSTREAM`/explicit-ID/`$`/PEL/consumer tests, journal/rewrite/replay
and TTL shall pass; no dependency shall be added; and both protected gates shall
pass on the exact head.

### 4.49 Ordered sorted-set pops (candidate; admission measured)

**Mechanism.** Reuse 4.30/4.38's ordered `(score, member)` index: `ZPOPMIN`/
`ZPOPMAX` take its first/last element (tie order `(score, member)`), remove the
member from the score map and index in place, update `payload_len` by the delta,
and repeat `COUNT` times; delete the key when empty. `ZMPOP`/`BZPOP*` reuse the
same primitive. No `owned_zset`, `ordered_zset`, re-encode or `put_collection`.

**Prediction.** 100k `ZPOPMIN`/`ZPOPMAX` fall from **156.7/159.9 ms to ≤0.3 ms**
(p99 ≤2), `ZPOPMIN COUNT 10` from 146.6 to ≤0.5 and `ZMPOP` from 155.8 to ≤0.3;
the 100-member control is unchanged and scores/tie order are preserved.

**Admission evidence (2026-11-12, section 3.35).** A 100,000-member sorted set,
20 pops, three rotations: `ZPOPMIN` 156.7 (169.3/156.7/156.4) versus Redis 0.42
(0.4/0.7/0.2) and Valkey 0.34 (0.3/0.3/0.3); `ZPOPMAX` 159.9 versus 0.21/0.29;
`ZPOPMIN COUNT 10` 146.6 versus 0.35/0.16; `ZMPOP 1 z MIN` 155.8 versus
0.16/0.20; 100-member control 0.11 versus 0.16/0.24.

**Risks and obligations.** Tie order, score formatting, empty/negative `COUNT`,
key deletion, blocking variants, journal/rewrite/accounting and the small
`ZSetPack` path must stay exact; a pop must not clone or sort the set, and a
missing key returns the empty reply.

**Rejected options.** A linear min/max scan without the sort (O(n) per pop, still
far from O(log n)), a separately cached min/max (stale on updates), and keeping
the full sort.

**Rejection condition.** Any pop order, score, empty/blocking or accounting
result differs, the controls regress, or no gain above the spread.

**Acceptance (AC51).** At 100,000 members, `ZPOPMIN` and `ZPOPMAX` shall have p50
≤0.3 ms and p99 ≤2 ms (≥500× faster than the measured 156.7/159.9 ms and within
~2× the faster C server); `ZPOPMIN COUNT 10` ≤0.5 ms (≥290×) and `ZMPOP` ≤0.3 ms
(≥500×); the 100-member control shall be unchanged; empty/negative count, tie
order, score formatting, key deletion, blocking variants, journal/rewrite/replay
and accounting tests shall pass; no dependency shall be added; and both protected
gates shall pass on the exact head.

### 4.50 Seek-based lex ranges and span counts (candidate; admission measured)

**Mechanism.** Extend the admitted ordered `(score, member)` index (4.30/4.38) so
a bounded range is answered by seeking, not by sorting:

- locate the first and last entry of the requested score or lex bounds in the
  ordered index (`BTreeMap`/block `range` for the seek) instead of building
  `ordered_zset`;
- for `BYLEX`, seek by member inside the equal-score block and preserve today's
  behaviour for mixed scores (the command is defined over equal scores);
- apply `LIMIT offset count` from the seek position, walking at most
  `offset + count` entries, with the existing negative/zero/overflow rules;
- answer `ZCOUNT`/`ZLEXCOUNT` as `rank(last) − rank(first) + 1` using the 4.38
  order-statistic ranks, and return the zset length directly for an unbounded
  range (Redis/Valkey subtract skiplist ranks);
- `ZRANGESTORE`/`ZRANGE ... STORE` copy only the selected range; pack encodings
  either answer directly (small) or convert at the existing threshold.

No new structure beyond 4.30/4.38, no dependency, no `unsafe`.

**Prediction.** At 100k equal-score members, `ZRANGEBYLEX - + LIMIT 0 10` falls
from **56.0 ms to ≤0.3 ms** and the mid/reverse forms from 69.8/60.1; a narrow
`ZLEXCOUNT` falls from **15.0 ms to ≤0.3 ms**; every `ZCOUNT` falls from
1.25–1.75 ms to ≤0.2 ms regardless of whether the range is full, full-matching or
empty; the full 100k-member reply stays within ~1.5× of the faster C server
because it is reply-bound; a 10-member control is unchanged.

**Admission evidence (2026-11-13, section 3.38).** One 100,000-member zset, score
0, members `m000000`–`m099999`, 20 repetitions per small reply and five per full
reply, three rotations: `ZRANGEBYLEX - + LIMIT 0 10` 51.0/56.0/66.0 ms versus
Redis 0.18/0.22/0.16 and Valkey 0.15/0.80/0.27; mid-range 63.2/70.5/69.8 versus
0.14–0.20 and 0.18–0.54; reverse 55.1/60.1/62.5 versus 0.13–0.69 and 0.32–1.02;
`ZLEXCOUNT` all 1.88/1.13/2.28 versus 0.17–0.37/0.07–0.29 and narrow
17.3/13.0/15.0 versus 0.21–0.42/0.07–0.34; `ZCOUNT` 1.07–2.45 versus
0.14–0.78/0.18–1.20; full 100k lex reply 137.9/159.3/144.8 versus 85.1–91.0 and
99.2–110.5; 10-member controls 0.25–0.47 versus 0.09–0.33/0.17–0.22.

**Risks and obligations.** Bound parsing (`(`, `[`, `-`, `+`, NaN scores);
exclusive and infinite bounds; reverse direction and offset arithmetic; mixed
score/member order must match today's results; pack conversion must not reorder;
empty results and destination removal for store forms; wrong type, missing key,
TTL/expiry; WATCH version, journal and rewrite snapshots; payload accounting when
a store form rewrites a key.

**Rejected options.** Sorting and then breaking early (still O(n log n));
caching the sorted vector per key (invalidated by every write and unbounded
memory); a second lex-only index (duplicate storage and more invariants to keep
consistent on every add/remove/score change).

**Rejection condition.** Any range, count, order, LIMIT, store, pack or
mixed-score result differs; the 10-member controls regress; a store form changes
accounting or expiry; or no gain above the spread.

**Acceptance (AC52).** At 100,000 equal-score members, `ZRANGEBYLEX` and
`ZREVRANGEBYLEX` with `LIMIT 0 10` (unbounded, mid-range and reverse) shall have
p50 ≤0.3 ms and p99 ≤2 ms (≥150× faster than the measured 56.0–69.8 ms and
within ~3× the faster C server); `ZLEXCOUNT` and `ZCOUNT` (full, full-matching
and empty) shall have p50 ≤0.3 ms and p99 ≤2 ms with a 10k→100k ratio ≤2×
(size-independent); the full 100k-member lex reply shall be within 1.5× the
faster C server; 10-member controls shall be unchanged; mixed-score, exclusive
and infinite bounds, LIMIT edge cases, store forms, pack thresholds, TTL/WATCH/
journal/rewrite and accounting tests shall pass; no dependency shall be added;
and both protected gates shall pass on the exact head.

### 4.51 Borrowed-source set/zset algebra and cardinality (candidate; admission measured)

**Mechanism.** Stop materialising owned copies of every source. Borrow each
source collection (the set container, `Collection::ZSet`'s map or `ZSetPack`'s
sorted vector), order the sources by length, iterate the smallest, and probe the
rest with `contains`/`get` (and weight/aggregate into `f64`, normalising NaN to
0). For `SINTERCARD`/`ZINTERCARD`, count matches and stop as soon as the `LIMIT`
is reached, with no destination object; `LIMIT 0` stays unlimited. For the store
and reply forms,
build the destination once from the surviving members — a `(score, member)` map
inserted in order, or the existing encoder on a result vector — instead of a
`HashMap` result followed by a full `ordered_zset` sort. If the destination is
also a source, read it before taking the write borrow; a missing source is empty;
a wrong type aborts before any mutation. Keep weights, `AGGREGATE SUM/MIN/MAX`,
NaN handling, `ZDIFF` (first minus rest) and destination TTL-clearing exact. No
new structure, no dependency, no `unsafe`.

**Prediction.** At 100k members, `ZINTERCARD` falls from **220.6 ms to ≤30 ms**
(≥7×, within ~2× the faster C server) and `SINTERCARD` from 53.3 to ≤45 ms with a
flat `LIMIT` exit; a `LIMIT 10` on either form falls to the command floor
(≤0.5 ms) because the scan stops after ten matches; `ZINTERSTORE` from 306.6 and
`ZDIFFSTORE` from 331.3 to ≤130 ms (≥2.3×, within ~2.1× the faster C server);
`ZUNIONSTORE` from 477.6 to ≤360 ms (≥1.3×, within ~1.3× the faster C server)
because its 150k-member result write remains dominant; the 10-member controls and
the set-store paths are unchanged.

**Admission evidence (2026-11-13, sections 3.41 and 3.44).** Two 100,000-member collections
(`a`=`m000000`–`m099999` score 1, `b`=`m050000`–`m149999` score 2), 10
repetitions, three rotations: `ZINTERCARD` 222.1/215.6/220.6 ms versus Redis
41.3/34.7/40.4 and Valkey 17.1/15.1/23.4; `ZINTERSTORE` 305.1/306.6/373.1 versus
125.5/92.5/110.5 and 65.3/57.3/61.6; `ZDIFFSTORE` 331.3/306.1/345.7 versus
99.8/102.0/103.3 and 61.4/59.6/66.0; `ZUNIONSTORE` 458.0/477.6/491.4 versus
356.0/373.3/366.6 and 280.3/286.2/268.3; `SINTERSTORE` 44.3/42.0/50.5 versus
66.9/57.2/67.3 and 36.5/29.4/39.4; 10-member controls 0.16/0.25 versus
0.32–0.45; preload 16.0 s versus 1.5–1.7 s (the 4.35 write path). The
`LIMIT` cohort (identical 100k membership): `SINTERCARD LIMIT 10`
50.72/59.09/49.50 ms versus Redis 0.26/0.15/0.20 and Valkey 0.21/0.25/0.28;
`ZINTERCARD LIMIT 10` 313.8/259.5/267.2 versus 0.22/0.18/0.33 and
0.19/0.49/0.08; `LIMIT 0` 49.6–59.1 and 275.1–299.9; 10-member controls
0.20–0.21.

**Risks and obligations.** Source ordering must not change results; duplicate
keys and the destination-as-source case; weights and `AGGREGATE SUM/MIN/MAX`
with NaN/±infinity; `LIMIT 0` meaning unlimited and a true early exit for any
positive limit; the set `SetInt`/`SetPack` and zset `ZSetPack` iteration order must
not change the count; empty/missing keys, wrong type before mutation; store
clears TTL and rewrites accounting; WATCH version, journal and rewrite snapshots;
the count path must not allocate a destination.

**Rejected options.** Keeping `owned_zset` and only sorting the result once
(still copies both sources); a borrowed `HashSet` of the smaller source (extra
allocation and hash pass); parallelising the intersection (thread coordination
for a sub-500 ms operation, outside the one-owner model).

**Rejection condition.** Any score, weight, aggregate, order, `LIMIT`, empty,
store or destination-as-source result differs; the 10-member controls or
set-store paths regress; a store form changes TTL or accounting; or no gain above
the spread.

**Acceptance (AC53).** At 100,000 members, `ZINTERCARD` shall have p50 ≤30 ms and
p99 ≤60 ms (≥7× faster than the measured 220.6 ms and within ~2× the faster C
server); `SINTERCARD` without a limit shall have p50 ≤45 ms (≥1.2× and within
~1.5× the faster C server), and `SINTERCARD`/`ZINTERCARD` with `LIMIT 10` shall
have p50 ≤0.5 ms and p99 ≤2 ms (≥100× faster than 50.7/267.2 ms and within ~2.5×
the faster C server), independent of container size; `LIMIT 0` shall match the
unlimited cost and result;
`ZINTERSTORE` and `ZDIFFSTORE` shall have p50 ≤130 ms and p99 ≤200 ms (≥2.3×
faster than 306.6/331.3 ms and within ~2.1× the faster C server); `ZUNIONSTORE`
p50 ≤360 ms (≥1.3× and within ~1.3× the faster C server); `LIMIT` shall exit
early without materialising the result; the 10-member controls shall be
unchanged; set-store variants shall not regress; weights, `AGGREGATE MIN/MAX/
SUM`, NaN/infinite scores, missing/empty keys, destination-as-source, TTL/
WATCH/journal/rewrite and accounting tests shall pass; no dependency shall be
added; and both protected gates shall pass on the exact head.

### 4.52 O(1) INFO keyspace accounting (candidate; admission measured)

**Mechanism.** Stop walking the keyspace to answer `INFO keyspace`/`INFO`/
`INFO all`. Maintain in `Data` the four values the section reports — live keys,
expiring keys, subexpiry keys and a TTL sum — updated by delta on every path that
can change them: insert/overwrite, `EXPIRE`/`PERSIST`, key removal, expiry reap
(including field-TTL deletion), eviction, `FLUSH*`, `RENAME`/`RENAMENX`/`COPY`,
and replay/rewrite load. Keep the `field_ttls` field count and add a key-level
subexpiry count (or reuse an index); `avg_ttl` is the maintained sum divided by
the expiring count. `INFO keyspace` then formats the counters and the expiration
index length in O(1); a plain `INFO` no longer touches every entry. Reaping stays
where it is today (the maintenance tick and lazy `expire_key`), so the reply does
not gain a reaping side effect. No dependency, no new container beyond integer
counters.

**Prediction.** At 500,000 keys, `INFO keyspace` and plain `INFO` fall from
**10.0/10.1 ms to ≤0.4 ms** (≥25×, within ~1.5× the faster C server) and stop
scaling with key count; at 100,000 from 2.79 ms to ≤0.25 ms; 10k, `INFO stats`,
`DBSIZE` and `PING` are unchanged.

**Admission evidence (2026-11-13, section 3.42).** 10k/100k/500k plain string
keys, 20 repetitions, three rotations: `INFO keyspace` 0.15/0.41/0.21 at 10k
versus 0.19/0.10/0.22 and 0.53/0.14/0.09; 2.94/2.79/2.70 at 100k versus
0.11/0.17/0.19 and 0.13/0.12/0.22; 10.93/10.00/9.14 at 500k versus
0.13/0.25/0.28 and 0.13/0.30/0.38; plain `INFO` 10.19/10.15/9.40 at 500k versus
0.35/0.36/0.48 and 0.22/0.37/0.26; `INFO stats` 0.11–0.30 and `DBSIZE`
0.10–0.43 at 500k, both at the floor.

**Risks and obligations.** A drifting counter is invisible and wrong: every
mutation, expiry, eviction, flush, rename/copy, field-TTL add/remove, rewrite and
replay path must update it exactly; `avg_ttl` must stay a non-negative integer
and handle an empty expiration set; the reported `keys`/`expires`/`subexpiry`
must match a full scan after mixed churn; `INFO all` and section selection must
not change; no reaping side effect may be added to `INFO`; `payload_bytes` and
other counters remain untouched.

**Rejected options.** Caching the formatted `INFO` reply (invalidated by every
mutation and still O(n) to build on the first call); sampling the keyspace and
extrapolating `keys` (reports a wrong count); iterating only the expiration set
for `keys` (misses non-expiring keys and dead-entry semantics).

**Rejection condition.** Any counter differs from a full scan after insert/
expire/persist/remove/reap/evict/flush/rename/copy/replay churn; `INFO` gains a
reap; the controls or reads regress; or no gain above the spread.

**Acceptance (AC54).** At 500,000 keys, `INFO keyspace` and plain `INFO` shall
have p50 ≤0.4 ms and p99 ≤2 ms (≥25× faster than the measured 10.0/10.1 ms and
within ~2× the faster C server), and at 100,000 keys p50 ≤0.25 ms; the reported
`keys`, `expires`, `subexpiry` and `avg_ttl` shall match a full scan after mixed
TTL/field-TTL/expiry/reap/eviction/flush/rename/copy/replay churn; `INFO stats`,
`DBSIZE`, `PING` and all other sections shall be unchanged; no dependency shall
be added; and both protected gates shall pass on the exact head.

### 4.53 In-place two-key moves (candidate; admission measured)

**Mechanism.** Move one element between two borrowed containers instead of
rebuilding both. For `SMOVE`, admit the destination growth (payload limit and
possible eviction) before mutating, then `remove` from the source set's
`BTreeSet` and `insert` into the destination's, deleting an emptied source and
updating payload by the delta. For `LMOVE`/`RPOPLPUSH`/`BLMOVE`/`BRPOPLPUSH`,
take the element from the source list's head/tail chunk (`VecDeque` of chunks),
push it onto the destination's head/tail chunk, split/merge only when a chunk
crosses its bound, delete an emptied source, and handle a same-key move as an
in-place rotation within one chunk deque. Source and destination TTLs stay on
their own keys; the destination is created without a TTL. Reuse the blocking
wake and journal/notification ordering from the current helper. No dependency, no
new container.

**Prediction.** `SMOVE` on a 100k set falls from **25.5 ms to ≤0.5 ms** (≥50×,
within ~2× the faster C server); `LMOVE`/`RPOPLPUSH` on a 200k list from
39.2/41.2 ms to **≤0.5 ms** (≥78×, within ~1.5× Redis); a same-key rotate from
20.5 ms to ≤0.4 ms (≥50×); the 10-element controls are unchanged.

**Admission evidence (2026-11-13, section 3.43).** Two 100k sets and two 200k
16-byte lists, 20 repetitions, three rotations: `SMOVE` 25.49/23.74/26.39 ms
versus Redis 0.40/0.68/0.38 and Valkey 0.28/0.16/0.36; `LMOVE`
40.64/36.79/39.15 versus 0.34/0.31/0.47 and 0.08/0.42/0.07; `RPOPLPUSH`
43.53/36.40/41.16 versus 0.23/0.33/0.85 and 0.15/0.66/0.19; same-key rotate
20.51/22.87/19.93 versus 0.24/0.26/0.44 and 0.26/0.21/0.10; 10-member `SMOVE`
0.31/0.38/0.23 versus 0.36/0.22/0.26 and 0.07/0.15/0.23; `LPUSH`/`SADD`
controls 0.18/0.22; preload 13.4 s versus 1.4–1.6 s (the 4.35 write path).

**Risks and obligations.** Destination growth admission can evict a key, so
re-look both sides after it and leave both unchanged on OOM; source and
destination can be the same key (rotate, with LEFT/LEFT and RIGHT/RIGHT identity
semantics); empty-source deletion and missing-key creation; TTLs belong to each
key and the destination starts without one; payload accounting by delta; chunk
split/merge and the 4.32/4.41 invariants; `BLMOVE`/`BRPOPLPUSH` blocking wake,
timeout and disconnection; WATCH version, journal, keyspace notifications and
rewrite snapshots must see the same two mutations; `SMOVE` on a non-member
returns 0 without touching either key.

**Rejected options.** Keeping one of the two owned copies (still O(n) on the
other side); moving through a temporary encoded value (extra encode/decode and
payload accounting); special-casing small containers only (the measured loss is
the large-container copy, and small containers are already at the floor).

**Rejection condition.** Any move direction, order, membership, TTL, empty-key,
same-key, blocking or accounting result differs; the 10-element controls regress;
or no gain above the spread.

**Acceptance (AC55).** `SMOVE` on a 100,000-member set shall have p50 ≤0.5 ms and
p99 ≤2 ms (≥50× faster than the measured 25.5 ms and within ~2× the faster C
server); `LMOVE` and `RPOPLPUSH` on a 200,000-element list shall have p50 ≤0.5 ms
and p99 ≤2 ms (≥78× faster than 39.2/41.2 ms and within ~1.5× Redis); a same-key
`LMOVE` rotate shall have p50 ≤0.4 ms; the 10-element controls shall be
unchanged; LEFT/RIGHT combinations, same-key identity, missing source,
non-member, empty-source deletion, TTLs, blocking `BLMOVE`/`BRPOPLPUSH`,
WATCH/journal/notification/rewrite and accounting tests shall pass; no dependency
shall be added; and both protected gates shall pass on the exact head.

### 4.54 In-place list element edits (candidate; admission measured)

**Mechanism.** Edit a list's chunk deque directly instead of flattening it. For
`LINSERT`, walk chunks to the pivot (front or back half, whichever is nearer),
insert the new element in that chunk, and split only when it exceeds the bound.
For `LSET`, walk to the indexed chunk, replace the element, and keep the length.
For `LREM`, walk and unlink matching elements in place, merging or dropping empty
chunks, with the positive/negative/zero count semantics. `LPOS` with a negative
rank walks from the tail and needs no flatten. Reuse 4.41's chunk traversal and
4.35's delta accounting; no new container, no dependency.

**Prediction.** At 200k elements, `LINSERT` at the head falls from **19.1 ms to
≤0.3 ms** (≥60×), `LSET` at index 100k from 16.5 to ≤0.5 ms (≥30×), `LINSERT` at
the middle from 21.0 to ≤4.5 ms (≥4.6×, within ~1.3× the faster C server),
`LREM` near the tail from 21.1 to ≤8 ms (≥2.6×, within ~1.1×), and
`LPOS ... RANK -1` from 1.59 to ≤0.6 ms (≥2.6×); the 10-element controls are
unchanged.

**Admission evidence (2026-11-13, section 3.45).** One 200,000-element list of
distinct 8-byte values, 20 repetitions, three rotations: `LINSERT` head
19.86/16.79/19.06 ms versus Redis 0.09/0.21/0.07 and Valkey 0.65/0.18/0.24;
`LSET` index 100k 21.98/16.46/15.22 versus 0.09/0.34/0.14 and 0.17/0.28/0.96;
`LINSERT` middle 21.70/16.35/21.04 versus 3.51/3.69/3.13 and 4.48/4.33/4.10;
`LREM` tail 22.88/19.90/21.09 versus 6.97/7.12/6.29 and 6.57/8.15/7.28;
`LPOS RANK -1` 1.87/1.59/1.31 versus 0.09/0.30/0.16 and 0.27/0.08/0.36;
10-element controls 0.30–0.32; preload 3.9 s versus 0.3–0.5 s (the 4.35 write
path).

**Risks and obligations.** BEFORE/AFTER pivot missing returns -1 and touched
state unchanged; LSET negative/out-of-range index errors; LREM count semantics
(positive from head, negative from tail, zero all) and deletion of an emptied
key; chunk split/merge bounds and iteration order; TTL/WATCH/journal/rewrite and
accounting by delta; `LPOS` rank/count/maxlen and missing-key behavior; blocking
commands unaffected; `ListPack` threshold conversion stays exact.

**Rejected options.** Keeping the flatten but skipping the re-encode (still an
O(n) element copy and allocation); converting the list to a different structure
for edits (touches every read path); locating the pivot by index only (does not
cover value-based LINSERT/LREM).

**Rejection condition.** Any BEFORE/AFTER, index, count, order, missing-key,
empty-key or accounting result differs; the 10-element controls regress; or no
gain above the spread.

**Acceptance (AC56).** At 200,000 elements, `LINSERT` at the head shall have p50
≤0.3 ms and p99 ≤2 ms (≥60× faster than the measured 19.1 ms); `LSET` at index
100,000 p50 ≤0.5 ms and p99 ≤2 ms (≥30× faster than 16.5 ms); `LINSERT` at the
middle p50 ≤4.5 ms and `LREM` 1 near the tail p50 ≤8 ms (≥4.6×/≥2.6× and within
~1.3×/1.1× the faster C server); `LPOS ... RANK -1` p50 ≤0.6 ms; the 10-element
controls shall be unchanged; BEFORE/AFTER/missing pivot, index edges, LREM
count semantics, emptied-key deletion, chunk split/merge, TTL/WATCH/journal/
rewrite and accounting tests shall pass; no dependency shall be added; and both
protected gates shall pass on the exact head.

### 4.55 Frame-derived argument cap (candidate; boundary measured)

**Mechanism.** Stop rejecting a request at 1,024 total arguments. Derive the
count bound from the byte budget it protects:
`max_arguments = max(1024, min(1_048_576, max_frame_bytes / MIN_ARG_BYTES))` with
`MIN_ARG_BYTES = 6` (the smallest complete bulk `$0\r\n\r\n`). The parser
already rejects a frame above `max_frame_bytes`, and the argument vector is one
`&[u8]` per argument, so total slice memory stays below
`max_frame_bytes / 6 × 16 B` (≈5.6 MiB at the 2 MiB default, transient), and the
1,048,576 ceiling keeps a very large configured frame from allocating an
unbounded vector. `--max-frame-bytes` recomputes the cap from the same helper;
there is still no separate count knob. Keep the
PING fast path, the canonical/fragment path and the error text unchanged; keep
`max_bulk_bytes` and `max_frame_bytes` as the real request budgets. No
dependency, no `unsafe`.

**Prediction.** At the default 2 MiB frame, `MGET`/`DEL`/`EXISTS`/`TOUCH` with
2,000–10,000 keys and `MSET` with 600–1,000 pairs are accepted and execute;
the `too many arguments` rejection disappears below the frame bound; a frame
above 2 MiB is still rejected before execution; command latency and the gate
rows are unchanged.

**Admission evidence (2026-11-13, section 3.47).** Boundary probe, one command
per connection: Rill accepted `MGET` 1,023 keys (1,024 args) and rejected 1,024
keys (1,025 args), 10,000 keys, `MSET` 600 pairs and `DEL` 2,000 keys with
`-ERR Protocol error: too many arguments`; Redis/Valkey accepted every case.
The 1,000-key latency cohort (120k keys, 20 repetitions, three rotations):
`MGET` 1.74 versus 2.23/2.34 ms (lead), `MSET` 500 pairs 1.18 versus 0.58/1.08,
`EXISTS` 0.88 versus 0.42/0.95, `TOUCH` 0.98 versus 0.93/0.62, `DEL` 1.63 versus
0.98/1.01, 10-key controls 0.16–0.53.

**Risks and obligations.** The bound must stay derived from the byte budget so a
large frame cannot create an unbounded argument vector; a request that exceeds
`max_frame_bytes` must still fail with the frame error, not allocate; the
`max_arguments` unit (total arguments, including the command name) must stay
documented; the parser's fast path must not change; all command arity checks and
the existing protocol tests must keep passing; no gate row may change.

**Rejected options.** Removing the count bound entirely (an argument vector not
tied to a byte budget); raising `max_frame_bytes` as the fix (it does not change
the count ceiling and increases bulk memory); a per-command argument table
(duplicates the parser budget and drifts from the frame limit).

**Rejection condition.** Any request below the frame bound is still rejected, a
request above the frame bound is executed, parser memory grows beyond the stated
bound, the PING fast path or gate rows change, or any protocol/arity test
differs.

**Acceptance (AC57).** At the default 2 MiB frame, `MGET` with 10,000 keys,
`DEL` with 2,000 keys, `EXISTS` with 2,000 keys, `TOUCH` with 2,000 keys and
`MSET` with 600 pairs shall be accepted and return the correct result; the
1,024-key boundary rejection shall be gone; a request whose complete frame
exceeds `max_frame_bytes` shall still be rejected before command execution; the
parser's transient memory shall be bounded by `max_frame_bytes`; the PING fast
path, all arity/protocol tests and both protected gates shall pass on the exact
head; and no dependency shall be added.

None needs a new dependency: 4.15 uses `Arc`/`IoSlice`, 4.30 uses a `std`
`BTreeSet` and a local total-order helper, 4.31 uses `std` `BTreeSet`/`BTreeMap`
with the in-repo hex helper, 4.32 uses byte arenas and `VecDeque`, 4.33 uses a
bounded queue plus the existing maintenance tick, 4.34 (landed in #258) uses the existing `Vec`
in place, 4.35 mutates the existing collection containers, 4.36 uses `Vec` plus
`HashMap`/`BTreeSet` indexes, 4.37 uses in-place `Vec` writes plus
`u64::count_ones`, 4.38 uses a `Vec` of sorted blocks plus counts, 4.39 uses
the existing channels and `sync_channel`, 4.40 uses the existing containers'
lengths and probes, 4.41 walks the existing list chunks, 4.42 mutates the
existing stream map, 4.43 uses geohash score ranges, 4.44 uses the existing
set/zset members, 4.45 uses the existing lazy expire path, 4.46 slices the
existing source order, 4.47 mutates the existing HLL registers, 4.48 mutates the
existing stream groups, 4.49 pops from the ordered zset index, 4.50 seeks the
same index and adds no storage, 4.51 borrows the existing zset containers and
builds the destination once, 4.52 adds integer counters to the existing
`Data`, 4.53 reuses the existing set/list containers for a two-key move, and
4.54 walks the existing list chunks for an element edit; 4.55 derives its bound
from the existing frame limit. The
rest use
the existing `std` I/O, parser and in-repo journal/crypto code. Each candidate
lands as its own PR with AC5 plus the protected gates; no combined integration
branch.

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
rejected eight-reactor experiment and the closed candidates add no package: 4.9
stays inside `std::time`/the existing `Clock` trait, 4.15 needs only
`std::sync::Arc`, `std::io::IoSlice` and `Write::write_vectored`, 4.16 is
measurement tooling, 4.17–4.18 are rejected, 4.20 needs no crate, 4.21 is
`std::fs`/`Write::write_all` plus the existing in-repo SHA-256/HMAC and CRC code,
and 4.30 is a `std::collections::BTreeSet` plus a local `f64` total-order helper
(`ordered-float`/`float-ord` are rejected as an unnecessary dependency for ~20
lines), 4.31 is `std` `BTreeSet`/`BTreeMap` plus the in-repo hex helper, 4.32 is
byte arenas plus `VecDeque`, and 4.33 is a bounded queue on the existing
maintenance tick; 4.34 mutates the existing `Vec` in place; 4.35 mutates the
existing collection containers in place; 4.36 uses `Vec` plus
`HashMap`/`BTreeSet` indexes; 4.37 uses in-place `Vec` writes plus
`u64::count_ones`; 4.38 uses a `Vec` of sorted blocks plus counts; 4.39 uses the
existing channels and `sync_channel`; 4.40 uses the existing containers' lengths
and probes; 4.41 walks the existing list chunks; 4.42 mutates the existing stream map;
4.43 uses geohash score ranges; 4.44 uses the existing set/zset members; 4.45 uses the existing lazy expire path;
4.46 slices the existing source order; 4.47 mutates the existing HLL registers; 4.48 mutates the existing stream groups; 4.49 pops from the ordered zset index; 4.50 seeks the same index and adds no storage.
None adds a crate, build script,
allocator or FFI.
No new dependency, build script, allocator or FFI is requested by this refresh. Level-triggered readiness
would need either raw `epoll` in project code or a new selector dependency, so
it remains a policy decision, not a free tuning change; `SO_BUSY_POLL` and
`recvmmsg`/`sendmmsg` are likewise outside the current Mio/`std` boundary.

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
- **Large-reply ordering (4.15):** a vectored tail must drain before later
  replies, and a partial non-blocking write must retain the `Arc` and offset.
  Getting this wrong reorders or duplicates replies; tests must cover partial
  writes, slow readers, pipelined GETs and shutdown before any gate claim.
- **Large-frame reservation risk (4.25):** a bulk length is attacker-controlled.
  Reserve only within the configured `max_frame`/`max_bulk` and in bounded steps,
  or one client can force a large allocation. Fragment, truncation and limit
  behavior must not change (AC28).
- **HGETALL order and reply-path risk (4.29):** Redis leaves `HGETALL` order
  unspecified, but this repository's deterministic sort is deliberate and the
  persistence test compares exact bytes. Changing order needs an explicit
  documented decision and updated tests; a `BTreeMap` or sorted index must not
  regress `HSET`/`HDEL` or the gate (AC31).
- **Threshold risk (4.15):** the 16 KiB split is copied from the upstream
  reference, not measured for this allocator; if a future allocation change makes
  large `Arc` creation costly, the threshold must be re-validated rather than
  assumed.
- **Measurement-host risk:** this development host is a shared k3s node with
  persistent Longhorn/Flux load. A 64-byte control measured ~20–33x below the daily
  cohort, so local screens can only diagnose mechanism, never rank servers
  (section 2.21). The daily CPU column also includes generator orchestration and
  is not a per-command measure (section 2.19).
- **Gate-topology risk:** since #198 each workload row is a partial job on its
  own runner, so cross-row absolute rates are not comparable and a per-row host
  shift can change a margin. Only within-row alternating comparisons count, and
  a missing/inconsistent partial must fail the gate rather than be filled in.
- **Attribution risk:** the counter-padding pass was measured under the old
  four-reactor default; citing it for the current one-reactor default would be
  wrong. Multi-reactor layout claims need their own A/B (section 4.10).
- **Latency-attribution risk:** a client-observed p99 includes the generator and
  scheduler. The #212 host shows a consistent SET/XX P1 tail against Valkey while
  the daily host shows the reverse (section 2.22); neither proves a server-side
  mechanism. Report both client and server percentiles before changing the
  runtime (AC21, 4.19).
- **Eviction-batching risk (4.26):** an eviction pass must not over-evict, hold
  the data lock unboundedly, or change policy/accounting/expiry semantics. Bound
  keys/time per pass, keep `noeviction` and WATCH/journal order, and verify exact
  evicted-key and payload counts (AC29).
- **Replay fast-path risk (4.27):** merging checksum validation into replay and
  skipping load-irrelevant side effects must not accept corruption, change the
  tail policy, re-relativize TTLs, break HMAC chaining, or alter stream/PEL
  state. A bad frame must abort before the listener binds (AC30).
- **HMAC cost and crypto risk:** the measured −10% P1 / −24% P16 append cost and
  the 3.6× HMAC replay are the price of per-transaction authentication. Do not
  replace the in-repo SHA-256, shorten the chain, or skip verification without a
  security review; only fold the existing verification into the replay pass
  (3.9, 3.15).
- **WATCH-version risk (4.20):** skipping per-key versions while nothing watches
  is only safe if `WATCH` registration and the tracking flag change under the
  same data borrow and if scratch multi-owner executors keep tracking. A race or
  a missed bump would silently accept a stale `EXEC`; the transaction matrix and
  an explicit later-`WATCH` test are mandatory (AC22).
- **Blocking-wake risk (4.22):** serving a woken client inside the mutating
  command would re-enter the store; lost, duplicated or out-of-order wakes,
  timeout/wake races and `CLIENT UNBLOCK` are the failure modes. Service after
  the batch, bound the ready set, preserve FIFO, and keep the 1 ms fallback so a
  missed signal degrades to today's latency rather than a hang (AC25).
- **Async-wake risk (4.24):** a missed wake strands a subscriber or MONITOR
  client; duplicate or out-of-order delivery, fanout memory, unsubscribe while a
  message is queued, monitor command ordering and shutdown with queued pushes are
  the failure modes. Keep the 1 ms fallback, bound the queue and preserve message
  ordering (AC27).
- **Rewrite-start risk (4.23):** the current 9–20 ms pause is a real latency cost
  for small datasets; spreading key materialisation makes the list
  non-point-in-time, so reconciliation must cover additions and deletions or a
  key can be missed or resurrected. Kill/replay and a write storm are mandatory
  (AC26).
- **Journal-batching durability risk (4.21):** a batch write must complete before
  any reply in the batch leaves; a failure must unacknowledge the whole batch and
  stop writes. A SIGKILL/replay test must prove every acknowledged reply is in
  the journal. Do not trade this for throughput (AC23).
- **Sorted-set index risk (4.30):** a second ordered structure must stay exactly
  consistent under `ZADD`/`ZINCRBY`/`ZREM`/`ZREMRANGEBY*`, expiry, replay and
  rewrite; a stale index would return the wrong rank order or a member that no
  longer exists. Score total order must handle ±0.0 and reject NaN at parse,
  ties must break by member like Redis, `ZSetPack` must convert atomically, and
  the index's memory must be counted against `maxmemory`; a deep-start range is
  O(start + k) and must be documented rather than hidden. `ZADD` loss above 10%
  or any semantic test difference rejects the change (AC32).
- **Cursor-scan risk (4.31):** replacing the decimal index cursor with an opaque
  token changes the wire contract and the router's `owner × 2^40 + local`
  packing; `tests/scan.rs` asserts exact cursors and sorted pages, so the change
  must update them deliberately and keep a documented old-cursor behavior. A
  buggy resume can skip or repeat keys on a stable keyspace, a selective MATCH
  can degenerate into a keyspace walk, and a new key index adds memory and an
  O(log n) mutation that must be accounted and rebuilt on replay/rewrite. Expiry
  must stay bounded, and the index must keep `SET`/`GET` within noise (AC33).
- **Packed-list risk (4.32):** a node index plus split/merge is the correctness
  core; an off-by-one in rank or a missed split changes `LRANGE`/`LSET`/`LINSERT`
  output, a large element stored inline can blow the node budget, and accounting
  or rewrite that counts logical bytes but not arena overhead reopens
  `maxmemory` and RSS questions. Every list, blocking and persistence test stays
  mandatory, and `RPUSH`/`LRANGE` must stay within 10% (AC34).
- **Deferred-drop risk (4.33):** an unbounded or unaccounted pending queue turns
  a latency fix into memory growth; pending bytes must remain in accounting and
  under eviction, shutdown must drain, and a recreated key must not observe the
  old pending value. Journal/notification ordering must be unchanged, and the
  synchronous fallback must be explicit rather than silent (AC35).
- **In-place string risk (4.34, implemented in #258):** the landed `mutate_string`
  path keeps the admission/re-lookup, OOM-before-mutation, zero-fill, 512 MiB,
  TTL/WATCH/journal/rewrite obligations; the post-fix cohort and persistence
  tests cover them. It remains a regression surface, not an open candidate.
- **In-place collection risk (4.35):** `reserve` can evict the key being mutated,
  pack↔large conversion must preserve order and compact semantics, and an OOM
  after a partial insert/pop corrupts the collection. Delta accounting, field
  TTL removal, empty-collection deletion, blocking-pop wake, chunk split/merge,
  WATCH versions, journal and rewrite must all stay exact (AC37).
- **Random-member risk (4.36):** the index must stay consistent under
  insert/remove/`SPOP`/expiry/replay; swap-remove tombstones must not bias the
  distribution or leak memory; `COUNT` (positive, negative, repeated) semantics
  and `SPOP` empty-key deletion must be exact; the full-collection ordering
  decision changes tests; index memory must be counted under `maxmemory`; a
  wrong index yields a reproducible sampling bias, so a statistical test is
  mandatory (AC38).
- **Bitmap risk (4.37):** an OOM after a partial extend, a wrong zero-fill length,
  a mis-normalized negative BYTE/BIT range or an MSB/LSB mask error silently
  returns wrong bits. The popcount must count all bits in a word regardless of
  byte order but mask partial head/tail bytes in Redis's bit order; offset and
  512 MiB limits, TTL/WATCH/journal/rewrite and accounting stay unchanged. The
  `BITFIELD` write form additionally must reserve the highest write offset,
  honor `OVERFLOW WRAP`/`SAT`/`FAIL`, return the same old/new values, and leave
  the read-only path copy-free (AC39).
- **Rank-index risk (4.38):** a score update moves a member between blocks; a
  stale cumulative count returns a wrong rank silently. Block split/merge,
  duplicate-score tie-break, `ZREM` count maintenance, negative/reverse ranges
  and `ZSetPack` conversion must be exact, and the block index must share one
  mutation with 4.30's order. `ZADD` loss above 10% or a non-flat 10k→100k ratio
  rejects it (AC40).
- **Pub/sub fan-out risk (4.39):** a byte-bound overflow must never remove the
  subscription silently; drop-with-counter and documented-disconnect are the only
  acceptable behaviors, and ordering, at-most-once, pattern/shard semantics,
  unsubscribe and shutdown must stay exact. A shared frame must respect each
  subscriber's RESP version and pattern form; the drain bound must not starve
  other clients (AC41).
- **Cardinality/membership risk (4.40):** the `SetInt` probe must match the
  current string-based semantics for integer and non-integer members, and
  `SetPack`/`HashPack` stay linear but small. Missing/empty keys, wrong-type
  errors, reply order and accounting must not change; `LLEN` is O(nodes) and may
  need cached counts only if a later cohort shows it (AC42).
- **List-seek risk (4.41):** negative-index normalization, chunk boundaries and
  the `RANK`/`COUNT 0`/`MAXLEN` rules are the semantic core; a missing key,
  empty list or wrong type must return today's reply, and `LRANGE`/`LLEN` must
  not change. The seek must not allocate per call (AC43).
- **Stream-append risk (4.42):** `reserve` can evict the key being appended; an
  OOM must leave no partial entry; trimming, `entries_added`, `last_generated`,
  `max_deleted`, groups/PEL, TTL/WATCH/journal/rewrite and the reply ID must stay
  exact, and the 1k→5k ratio must be flat (AC44).
- **Geosearch risk (4.43):** the neighbour-cell math is the correctness core —
  dateline/pole edges, inclusive score bounds, overlapping cells (dedupe), box
  versus radius, and `ASC`/`DESC`/`COUNT` ordering must reproduce the linear
  result exactly; the WGS-84 formulas, units, `WITH*` and `STORE` stay; the
  search must remain bounded by the result and payload limits (AC45).
- **Multi-member risk (4.44):** `SetInt` must answer integer and non-integer
  probes exactly as today; duplicates, order, missing/empty keys, wrong types and
  `ZSetPack`/`SetPack` remain; no reply shape, accounting or read path changes
  (AC46).
- **Expiry-stall risk (4.45):** removing the full purge must not let any read
  return an expired value, `DBSIZE` becomes approximate (documented/tests), and
  write admission must stay bounded without a false permanent OOM; the periodic
  purge drains the backlog (AC47).
- **SORT-nosort risk (4.46):** the direct slice must preserve the current set/zset
  source order, offset/count edge cases and `STORE`, and must not change any other
  SORT row (AC48).
- **Listing-order risk (4.29/4.31):** `HGETALL`/`HKEYS`/`HVALS`/`KEYS` order is
  unspecified in Redis, but this repository's deterministic sort is an existing
  test contract; removing it needs an explicit decision and updated
  persistence/listing tests. `KEYS`/`RANDOMKEY` must still exclude expired keys
  (4.45), and the keyspace index must be memory-counted and rebuilt on
  replay/rewrite (AC31/AC33).
- **HLL risk (4.47):** a stale cached cardinality returns a wrong count, so every
  add/merge and exact↔dense conversion must invalidate it; the serialized header
  must stay compatible with rewrite/replay/restore, and counts must stay within
  the error bound and identical before/after serialization (AC49).
- **BITPOS risk (4.37 extension):** the word-wise scan must keep MSB-first bit
  numbering, masked head/tail words, BYTE/BIT ranges and the not-found return
  values (`len*8` for bit 0 without an explicit end, `-1` otherwise); a
  first-bit hit must stay fast (AC39).
- **XGROUP CREATE risk (4.48):** the in-place insert must preserve `BUSYGROUP`,
  `MKSTREAM`, explicit-ID/`$`, PEL/consumer state and notification/journal order,
  with unchanged payload accounting; the 10-entry control must stay unchanged
  (AC50).
- **Zset-pop risk (4.49):** tie order `(score, member)`, score formatting,
  empty/negative `COUNT`, key deletion, blocking variants and accounting must
  stay exact; the pop must not clone or sort the set, and small `ZSetPack` sets
  keep a correct min/max path (AC51).
- **Lex/span-count risk (4.50):** exclusive, infinite and reversed bounds,
  `LIMIT` offset/count edges and mixed-score member order must produce exactly
  today's results; the span count must equal a full scan for every bound form;
  store forms must keep expiry, accounting and destination removal; `ZSetPack`
  must not be reordered (AC52).
- **Zset-algebra risk (4.51):** source order, weights and `AGGREGATE MIN/MAX/SUM`
  with NaN/±infinity must produce today's scores; a destination that is also a
  source must be read before the write borrow; `LIMIT` must not change the
  returned count; empty/missing keys, wrong type, store TTL clearing, accounting,
  `ZSetPack` conversion and WATCH/journal/rewrite must stay exact, and the count
  path must not allocate a destination (AC53).
- **INFO-counter risk (4.52):** a delta applied on only some mutation paths
  drifts silently, so every insert/overwrite/expire/persist/remove/reap/evict/
  flush/rename/copy/replay path needs a test that compares the counters with a
  full scan; `avg_ttl` must handle an empty expiration set; `INFO` must not gain
  a reaping side effect; section selection and `INFO all` must stay exact (AC54).
- **Two-key-move risk (4.53):** admission can evict the destination or source
  being moved, so both sides must be re-looked and left unchanged on OOM; a
  same-key move must rotate rather than delete; the destination inherits no TTL
  from the source; LEFT/RIGHT order, missing/non-member/empty-source results,
  chunk split/merge, blocking timeout/disconnect, WATCH/journal/notification/
  rewrite and accounting must stay exact (AC55).
- **List-edit risk (4.54):** BEFORE/AFTER with a missing pivot, negative/overflow
  indexes, `LREM` positive/negative/zero count semantics and emptied-key
  deletion must match; chunk split/merge and element order; `LPOS` rank/count/
  maxlen; TTL/WATCH/journal/rewrite and delta accounting must stay exact (AC56).
- **Argument-cap risk (4.55):** the count bound must remain derived from
  `max_frame_bytes`, so a large frame cannot create an unbounded argument
  vector; a request above the frame must still fail before execution; the
  `max_arguments` unit and CLI behaviour must stay documented; the PING fast
  path, arity checks and protocol tests must not change (AC57).
- **Persistence blind spot:** the protected gate runs with persistence off, so
  none of its rows exercises the journal write, fsync or HMAC framing. A
  persistence-on claim needs the 4.16 row-5 cohort; gate parity does not imply
  AOF parity (section 3.9).

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
| Tuning the default reactor count to clear Valkey P1 ping | One, two, four and eight reactors have each failed that row on some head; the sign is not monotonic and the two-reactor head lost 1.42% despite the highest rate and better p99 (section 2.12). |
| Miss-based adaptive poll or a shorter fixed poll window | Does not reduce the measured P1 CPU, which comes from successful windows; shrinking on hits or shortening the window risks the rate/p99 the window protects (section 4.13). |
| Hybrid 5 µs busy prefix + 15 µs timed-wait poll | Measured: CPU stayed at 95.5% and P1 ping p99 rose to 0.959 ms vs 0.231 for both references; P16 p99 also worsened. The sleep reintroduces the wakeup latency the busy window removes. Run 41612942833. |
| RDB/replication work as a performance fix | Out of scope and unrelated to the measured P1 gap. |
| Eliding the edge-triggered empty-receive drain probe (PR #155) | Cut receive syscalls ~16% locally but lost two Valkey P1 medians; the probe is also a user-space readiness batch. See 2.6/4.7. |
| A last-command lookup cache (Redis `lastcmd` shape) | Saving is tens of nanoseconds per command, below the 1.5–3% gate noise; the existing inline byte compares are already cheaper than a dict probe (4.17). |
| Incremental keyspace rehash at the gate's 1024 keys | Growth is a one-off move, not a per-command cost; a custom table or dependency would add policy and atomicity risk with no measured row (4.18). |
| Cross-client deferred flushing (`beforeSleep` shape) as a current candidate | No target row: P1 has one ready client per pass and P16 already leads by 54–153%; it would add latency and buffer risk (4.5). |
| A resumable RESP parser cursor | Incomplete frames rescan only length headers; the bound is ~1 KiB per large frame, below noise. Reopen only with a measured multi-megabyte scan cost (4.2). |
| A cached per-pass clock as a gate performance fix | A vDSO clock read is ~20–30 ns (<0.35% of a P1 command) and the gate has no TTL keys; keep it as a TTL-workload candidate only (4.9). |
| Running the counter-padding A/B on the default gate | The one-reactor default has one counter writer, so the effect is zero by construction; the old four-reactor pass must not be cited for it (4.10). |
| Zero-copy large replies without a clean large-value cohort | The 16 KiB copy-avoidance shape is real upstream, but building it on a failed shared-host screen or on theory alone violates the admission rule (2.21, 4.15). |
| `SO_BUSY_POLL`, `recvmmsg`/`sendmmsg` or level-triggered `epoll` as a free tuning change | Needs raw socket options or FFI/selector work outside the reviewed Mio/`std` boundary; a policy decision, not an experiment (section 5). |
| Ranking servers from the shared k3s development host | The 64-byte controls measured ~20–33x below the daily cohort; local numbers diagnose mechanism only and never establish a win or loss (2.21). |
| Removing `WATCH` versions unconditionally from every mutation | Would accept a stale `EXEC` whenever a key was written after `WATCH`; only a watch-aware fast path with the lock proof is safe (4.20, AC22). |
| Treating the #212 SET/XX P1 p99 gap as a server regression | The daily cohort shows the opposite direction and both are client-observed; no server histogram exists yet, so the correct action is 4.19, not a code change (2.22, AC21). |
| Writing the journal after the reply flush, or acknowledging before the write | Would acknowledge data that a crash can lose; the write must precede the reply (3.9, 4.21). |
| Busy-polling blocked clients, or shortening the 1 ms async tick for every connection | Spends CPU on all clients instead of targeting the ready-key event; the same-pass signal is the correct shape (3.12, 4.22). |
| Polling every subscriber and monitor at 1 ms as the default | Measured 22–27% of a core at 1000 idle clients versus <1% for the C servers; event-driven wake is the correct shape (3.12, 4.24). |
| Removing the 1 ms fallback and trusting the ready signal never misses | One missed signal would strand a blocked client indefinitely; the fallback is required by AC25. |
| Forking a copy-on-write rewrite child via unsafe/FFI | Outside the safe-Rust/no-FFI boundary; the cooperative slices are the reviewed adaptation (3.13). |
| Keeping the synchronous full rewrite | Measured at 130 ms of blocked service on the 20k-key fixture; already replaced by cooperative slices (docs/persistence.md). |
| Ignoring the rewrite start pause | It is a measured 14.2 ms at 20k keys and ~91 ms at 200k, a latency cost the gate cannot see; 4.23 is the bounded candidate (AC26), not a non-issue (3.13). |
| Copying Redis/Valkey's unauthenticated AOF byte stream | Drops the CRC32 transaction boundary and optional HMAC record that this project's durability/security increments added (3.9). |
| Per-command fsync under `everysec` | Multiplies fsync cost without changing the stated 1 s durability bound; the policy stays 1 s (3.9). |
| Retrying the four-CPU gate until it passes, or lowering/omitting cases | Forbidden by the quality gate; destroys the meaning of the evidence. |
| A hand-written skiplist for sorted sets | Repeats the C servers' structure in safe Rust with substantial mutation code for the same asymptotics a `std` `BTreeSet` gives (4.30). |
| Adding `ordered-float`/`float-ord` for sorted-set scores | A new dependency to express ~20 lines of `f64` total-order normalization; policy impact is not justified (4.30, section 5). |
| Sorting only the requested `ZRANGE` slice | The slice's start rank is defined by the global `(score, member)` order, so this does not reduce the work and can change results (4.30). |
| A reverse-binary cursor over `std::collections::HashMap` | `std` exposes no stable bucket cursor; `RawTable` is unstable and would put unsafe code in the project (4.31). |
| A 40-bit hash-prefix numeric cursor for `SCAN` | Collisions can skip keys, breaking the "every key exactly once" coverage guarantee; an ordered index or an opaque key token is required (4.31). |
| Keeping the per-call sorted key snapshot and only caching it | A cache still pays O(n log n) on the first call after every write and holds a duplicate key list; it does not bound per-call work (4.31). |
| Treating `UNLINK`/`FLUSH* ASYNC` as synchronous aliases | Violates the documented async contract and leaves a 6–17 ms command tail; the mode flag must either defer the free or be rejected (4.33). |
| An unbounded background free for every dropped value | A burst of large drops can grow memory and outrun the reaper; a bounded queue with a synchronous fallback is required (4.33). |
| Making `DEL` asynchronous by default | Redis's default is synchronous; changing it would alter notification/timing semantics without a measured target while the real gap is the per-element representation (4.32). |
| Keeping the temporary-vector rebuild and only removing the `put` copy | Still copies the whole value once per `APPEND`/`SETRANGE`; the measured cost is O(value), not O(delta) (4.34). |
| A custom string arena or unsafe buffer for `APPEND`/`SETRANGE` | A `Vec` with amortized growth already gives in-place mutation and bounded capacity without unsafe code (4.34, section 5). |
| Keeping the collection rebuild but skipping the re-encode | Still clones every field/element per write, so the measured O(collection) cost remains (4.35). |
| Copy-on-write or a persistent-collection crate for collection writes | Adds a dependency and indirection to every read for a write-path fix that plain `&mut` containers already solve (4.35). |
| Sorting/cloning the collection to answer a random pick | O(n log n) time and O(n) allocation per command; the measured 100k gap is 35–298× and the C servers sample the hash table directly (4.36). |
| A hash-derived random cursor without a position map | Cannot remove a sampled member in O(1) without tombstones or a scan, and collisions complicate distribution (4.36). |
| SIMD intrinsics or a CPU-feature crate for `BITCOUNT` | Needs unsafe intrinsics or a dependency; a safe `u64` chunk loop is within ~2× Redis and the measured gap is the byte loop (4.37). |
| Rebuilding a bitmap for `SETBIT` | Two full-value copies per bit write; the in-place string path already gives the correct shape (4.34, 4.37). |
| Accepting O(n) `ZRANK` because a `BTreeSet` has no rank API | The measured 100k gap is 378–927×; a bucketed rank index or order-statistic tree is required (4.38). |
| A full in-repo weight-balanced tree with subtree sizes | More hand-written balancing code than bucketed sorted blocks for the same O(log n + block) rank (4.38). |
| Sorting the sorted set on every rank/score read | The measured materialize+sort is exactly the 57–72 ms cost; it must be replaced, not cached per call (3.22, 4.30, 4.38). |
| Keeping the fixed 1,024-message subscriber queue and removing the subscriber on overflow | The measured 2,000-message burst loses 48.8% and silently drops the subscription; buffering must be byte-bounded with a counted drop or a documented disconnect (4.39). |
| An unbounded pub/sub queue per subscriber | A slow or dead subscriber would grow memory without limit; Redis bounds it and disconnects at the limit (4.39, 3.23). |
| A separately cached collection length maintained beside the container | Two sources of truth can drift; the container's own `len()` is O(1) (or O(nodes) for lists) and cannot disagree (4.40). |
| Making `SMEMBERS` unsorted so `SCARD` can reuse it | Fixes neither `SISMEMBER` nor the clone/sort cost and changes a deterministic reply order (4.40). |
| Maintaining a node/rank index just for `LINDEX` | The chunk walk is already O(nodes) without new state; materializing all references is the measured cost (4.41). |
| Keeping the stream clone and only removing the second payload fold | Still deep-copies every entry and folds the payload once per `XADD`, so the measured O(n) cost remains (4.42). |
| Keeping the linear geohash scan or caching distances per query | The scan is the measured 25–53× cost; a per-query cache is stale on updates and O(n) memory, while cell ranges reuse the ordered zset (4.43). |
| Keeping `set_items`/`zset_items` with a per-member scan | Every requested member rescans the materialized collection, so the measured 30–1461× cost grows with the member count (4.44). |
| Purging every due key on the first command after a mass expiry | The measured stall is 36–46 ms for 30k expired keys and scales with the backlog; reads should expire only what they touch (4.45). |
| Materializing the whole source for `SORT ... BY nosort LIMIT` | The measured 7.96 vs 0.12–0.16 ms is a full copy of 100k values for a 10-element slice; the other SORT rows are already 0.9–1.5× (4.46). |
| Keeping per-call sorts for `HKEYS`/`HVALS`/`KEYS` | At 100k the hash listings are 3.5–6.2× and `KEYS` 2.1–4.3× behind; 4.29's ordered hash and 4.31's key index remove the sort without changing reply contents (3.31). |
| A count cache outside the HLL payload | A separate counter diverges from rewrite/replay/restore and accounting; the cache belongs in the serialized header (4.47). |
| Keeping the `XGROUP CREATE` stream clone | It deep-copies every entry and folds the payload for a one-struct insert; the other XGROUP subcommands already mutate in place (4.48). |
| Keeping the full clone/sort for `ZPOPMIN`/`ZPOPMAX` | The measured 100k pop is 373–974× behind; the ordered index already gives the min/max element, so no sort is needed (4.49). |
| Sorting the whole zset to answer a bounded `BYLEX`/`ZCOUNT` query | A seek plus rank subtraction is O(log n); the measured 10-element lex range pays 56–70 ms for an O(n log n) sort and a narrow count scans all 100k members (4.50). Caching the sorted snapshot or adding a second lex index would duplicate storage and invalidation work for the same query. |
| Copying every source before a zset algebra or cardinality query | The measured 100k `ZINTERCARD` pays 220 ms for two owned maps plus a `BTreeSet`, while probing the smallest borrowed source and counting matches is O(smallest × keys) with no destination; the store forms already build one destination and re-sort a materialised map a second time (4.51). A borrowed `HashSet` of the smaller source only trades one allocation for another and was rejected. |
| Applying `SINTERCARD`/`ZINTERCARD` `LIMIT` after the full intersection | The measured `LIMIT 10` still pays 50.7/267.2 ms, exactly the no-limit cost; a positive limit is a work bound and must stop the probe loop after `limit` matches, while `LIMIT 0` keeps the unlimited cost (4.51). |
| Flattening a list to insert, replace or remove one element | The measured 200k head insert and indexed replace pay 19.1/16.5 ms for a full element copy and re-encode, while the chunk deque already supports a local edit; skipping only the re-encode still leaves the O(n) flatten (4.54). |
| A fixed 1,024-argument protocol cap | The measured boundary rejects a 90 KB `MGET` 10,000 while a 2 MiB frame is accepted, and Redis/Valkey execute it; the byte frame already bounds parser memory, so the count should be derived from it rather than fixed (4.55). Raising `max_frame_bytes` or removing the bound entirely was rejected because it does not tie count to memory. |
| Caching the formatted `INFO` reply instead of maintaining counters | The measured 10 ms walk happens on every `INFO`/`INFO keyspace`; a cache would still pay O(keyspace) to rebuild after the first mutation and would need a full invalidation contract. Sampling the keyspace or counting only the expiration index reports a wrong `keys` value (4.52). |
| Rebuilding both keys for a one-element two-key move | The measured 100k `SMOVE` and 200k `LMOVE`/`RPOPLPUSH` pay 20–41 ms for a full clone of both sides; a set remove/insert and a list pop/push are O(1)/O(log n) in the existing containers, and reusing the old clone for one side still leaves the other O(n) (4.53). |
| Scanning `BITPOS` byte by byte and bit by bit | The measured 16 MB not-found scan is 39–44× behind; a `u64` word scan with `trailing_zeros` is the same safe machinery as `BITCOUNT` (4.37). |

## 8. Acceptance criteria (EARS)

Definitions: "the four-CPU gate" is the official `valkey-benchmark` workflow; its
Valkey cohort now uses five alternating rounds (schema 3) and reports wins/5 plus
a deterministic paired-bootstrap 95% interval as a diagnostic. The pass rule is
unchanged: every row still requires the whole-cohort Rillcache median to be
strictly greater than Valkey's, and the interval never passes or fails a row. The
Dragonfly comparison remains three rounds. Since #198 each workload row runs in
its own partial job on its own allocated four-CPU runner of the same machine type,
so only within-row alternating comparisons are valid and a missing or
inconsistent partial fails the gate. Historical cohorts in this document used
three rounds and a single runner and are labeled as such. "The original gate" is
the unchanged 20-case workflow; "a matched local A/B" means the same host,
toolchain, release profile, frozen binaries and alternating order, with raw
samples retained.

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
- **AC15 (reactor budget).** When plain Unix TCP starts without an explicit
  count, it shall run one owner-local reactor, report that count through
  `scheduling_policy()`, keep all data commands on the owner-local executor, and
  preserve `--socket-workers 1..16` as an override. It is accepted only if the
  original gate wins 20/20 and both four-CPU comparisons win 10/10 on the final
  head. The `min(4, CPUs)`, `2 × CPUs` and `ceil(CPUs/2)` defaults remain
  historical evidence, not current defaults.
- **AC17 (bounded poll).** When a plain readiness reactor has serviced at least
  one client, has no queued client and no pending accept, it shall poll with a
  zero timeout for at most 20 µs before parking, shall renew that window only
  through another serviced pass, shall still observe stopping within that window,
  and shall not apply the window when a runnable client or accept is pending. It
  is accepted only on the same gate conditions as AC15.
- **AC16 (cached clock, only if 4.9 is reopened by a TTL cohort).** When a
  TTL-heavy cohort shows a clock share above the noise, a command shall observe
  one stable millisecond snapshot for its whole execution, refreshed at most once
  per reactor pass; a key alive at command start shall not expire inside that
  command; injectable test clocks and the existing expiry/accounting tests shall
  pass; the matched TTL A/B shall show fewer clock calls, and both protected
  gates shall still pass. Until then 4.9 stays closed (section 4.9).
- **AC18 (counter-layout scope).** When the default one-reactor path is measured,
  no performance claim shall attribute any part of its result to the 128-byte
  command-counter padding; a multi-reactor layout claim shall include a matched
  packed-versus-padded A/B with `--socket-workers 1` and `4` controls on one host,
  and shall be rejected if the one-reactor difference is outside the spread.
- **AC19 (out-of-gate evidence).** When a runtime change targets a workload the
  protected gate does not run (TTL-heavy, large values, large multi-bulk replies,
  mixed pipelines or persistence-on writes), its evidence shall come from a clean
  runner with pinned Redis/Valkey, the same generator, CPU placement, at least
  three rotated repetitions and retained raw samples; a shared k3s host, an
  incomplete cohort or a single-screen result shall not admit the change.
- **AC20 (large-reply copy avoidance, only if 4.15 is implemented).** When a bulk
  value of at least the measured threshold is returned, the server shall write it
  without copying the value into the session output buffer, while values below the
  threshold keep the current path; at 1 MiB GET, throughput shall be within 5% of
  the best reference and p99 no worse than 1.25× it (measured baseline −42% to
  −45% and 24.9 vs 9.9/10.9 ms); every reply shall be delivered exactly once
  and in order under fragmentation, pipelining, partial non-blocking writes, slow
  readers and half-close; payload/expiry/backpressure accounting shall be
  unchanged; the matched large-value A/B shall show the predicted gain; and both
  protected gates shall pass on the exact head.
- **AC21 (latency attribution).** When a latency-tail claim is made, the same run
  shall report client-observed per-request p99 and server-observed per-command
  p99 with CPU placement; a client-only p99 difference shall not justify a
  runtime change, and any accepted tail candidate shall show the server-side
  population and pass AC5 plus both protected gates.
- **AC22 (watch-free mutation parity).** When no client has an active `WATCH` and
  the default owner executes a mutation, no per-key `WATCH` state shall be
  allocated or updated; a `WATCH` registered later shall still detect every
  subsequent mutation; scratch multi-owner write sets shall be unchanged; the
  transaction/WATCH/multi-owner matrix shall pass; and the matched A/B shall show
  the claimed tail or CPU effect before both protected gates accept it.
- **AC23 (journal write coalescing, only if 4.21 is implemented).** With
  persistence on, a batch of K commands shall perform at most one journal write
  and one journal lock acquisition; every acknowledged command shall be present
  in the replayed journal; `always` shall sync before the reply and perform at
  most one fsync per batch; `everysec` shall
  sync within its 1 s bound; P16 AOF-on SET throughput shall be within 25% of
  AOF-off and P1 within 10%, and P16 p99 no worse than 2× AOF-off (baseline
  −90.2%/−66.5% and 15.1 vs 3.1 ms; C servers −8.5% to −33.5%); kill/replay,
  HMAC, CRC, rewrite and multi-owner tests shall pass; the matched
  persistence-on A/B shall show the gain; and both protected gates shall pass on
  the exact head.
- **AC24 (memory representation, only if 3.11 is implemented).** When duplicate
  key or value representation is changed, a cohort of at least one million TTL
  and counter keys shall show the RSS reduction with identical expiry, WATCH,
  collection and journal semantics; no hot-path allocation or refcount shall be
  added without a matched rate/tail result; and both protected gates shall pass on
  the exact head.
- **AC25 (blocked wake latency and idle CPU, only if 4.22 is implemented).** A
  matched blocked-wake cohort shall show p99 within 200 µs of Valkey and at
  least 2× better than the pre-change 1 ms tick baseline for BLPOP-style and
  `XREAD BLOCK` wakeups; with 1 and 1000 idle blocked clients and no traffic,
  server CPU shall stay at or below 1% and shall not scale with the client count
  (the measured baseline is 5.6% and 22.2% versus <1% for the C servers); FIFO,
  ordering, timeout, `CLIENT UNBLOCK`, multi-owner and shutdown tests shall pass;
  the 1 ms fallback shall remain; no dependency shall be added; and both
  protected gates shall pass on the exact head.
- **AC26 (rewrite start pause, only if 4.23 is implemented).** A
  `BGREWRITEAOF` shall reply within 2 ms at 20,000 keys and within 20 ms at
  200,000 keys, i.e. bounded by the materialisation slice rather than linear in
  the keyspace; concurrent writes during materialisation shall appear exactly
  once in the replayed snapshot; kill/replay shall recover; total completion
  shall stay within 25% of the measured 4.85 s (20k) and 43.4 s (200k)
  baselines; both protected gates shall pass; and no dependency shall be added.
- **AC27 (async-client idle CPU and delivery, only if 4.24 is implemented).**
  With 1 and 1000 idle subscribers and monitors, server CPU shall stay at or
  below 1% and shall not scale materially with the count (baseline 6.0/22.2% and
  5.8/27.4% versus <1% for the C servers); PUBLISH and MONITOR delivery p99 shall
  be within 200 µs of Valkey and at least 2× better than the 1 ms tick baseline;
  fanout, ordering, unsubscribe, timeout and shutdown tests shall pass; no
  dependency shall be added; and both protected gates shall pass on the exact
  head.
- **AC28 (large-frame input reservation, only if 4.25 is implemented).** At 1 MiB
  SET, throughput shall be within 10% of the best reference and p99 no worse than
  1.25× it (measured baseline −28% to −32% and 9.79 vs 7.37/7.82 ms); a declared
  length beyond the configured limits shall reserve no more than that bound;
  fragmented, truncated and malformed frames shall keep their current behavior;
  the matched A/B shall show the gain; and both protected gates shall pass on the
  exact head.
- **AC29 (pooled eviction, only if 4.26 is implemented).** Under continuous
  eviction, P16 SET throughput shall be within 25% of the no-eviction control and
  p99 within 2× it (measured baseline −85.8%, 64.5 vs 8.0 ms); evicted-key counts
  and payload accounting shall be exact; every `allkeys`/`volatile` ×
  `lru`/`random`/`ttl` policy and `noeviction` shall behave as specified; there
  shall be no change when the budget is not under pressure; the matched A/B shall
  show the gain; and both protected gates shall pass on the exact head.
- **AC30 (replay time to ready, only if 4.27 is implemented).** At 200,000 keys
  time-to-PING shall be ≤0.3 s and at 50,000 ≤0.08 s (measured baseline 1.10–1.24
  and 0.258 s); post-replay DBSIZE, payload bytes and TTLs shall match exactly;
  corrupt or truncated tails shall be handled by the documented policy and never
  served; SIGKILL/replay, HMAC on/off and stream consumer/PEL tests shall pass;
  no dependency shall be added; and both protected gates shall pass on the exact
  head.
- **AC31 (large-hash listing, only if 4.29 is implemented).** A 10,000-field
  `HGETALL` shall be within 10% of Redis or at least 30% faster than the measured
  baseline (313 req/s, 11.6/32.1 ms p50/p99; Redis 552, 7.0/16.3); at 100,000
  fields, `HGETALL`, `HKEYS` and `HVALS` shall have p50 ≤1.5× the faster C server
  (≤39 ms for `HGETALL`, ≤24 ms for `HKEYS`/`HVALS`) and be ≥3× faster than the
  measured 132/98/102 ms; ordering semantics shall be explicitly documented and
  every order-sensitive test updated to match; small hashes and `LRANGE` shall be
  unchanged; the matched A/B shall show the gain; and both protected gates shall
  pass on the exact head.
- **AC32 (sorted-set ordered index, only if 4.30 is implemented).** A
  10,000-member `ZRANGE 0 9` shall reach ≥8,000 req/s (≥15× the measured 528
  baseline and within ~25% of Redis) and `ZRANGE 0 9999` ≥1,500 req/s (≥4.4× the
  340 baseline and within 10% of Redis); `ZSCORE` shall not regress; `ZADD` shall
  stay within 10% of its matched baseline; every zset command, member tie-break,
  rewrite/replay and expiry test shall pass; the 128-member control and all gate
  commands shall be unchanged; no dependency shall be added; and both protected
  gates shall pass on the exact head.
- **AC33 (incremental cursor, only if 4.31 is implemented).** On 100,000 keys,
  `SCAN COUNT 10` shall have p50 ≤1 ms and p99 ≤5 ms (≥40× and ≥16× faster than
  the measured 42.6/81.3 ms, within ~4× Redis's 0.256 ms); a full
  `SCAN COUNT 1000` shall finish in ≤0.5 s and a full 10,000-field
  `HSCAN COUNT 10` in ≤0.5 s (≥9–10× faster than the measured 4.81/5.05 s); each
  call shall do work bounded by `COUNT` plus a fixed lookahead, with no
  keyspace-sized allocation/sort and no unbounded expiry sweep; a stable keyspace
  shall yield every key exactly once across the traversal; 100k `KEYS key:*`
  shall be p50 ≤2× the faster C server (≤42 ms) and `RANDOMKEY` ≤0.2 ms with the
  ordering decision documented; MATCH/TYPE/NOVALUES, cursor and multi-owner tests
  shall be updated to the chosen cursor semantics and pass; `SET`/`GET` shall
  stay within noise; no dependency shall be added; and both protected gates shall
  pass on the exact head.
- **AC34 (packed list nodes, only if 4.32 is implemented).** A 200,000-element
  8-byte list shall use ≤24 B/element RSS (≤3× the measured 7.5 B and ≥6× below
  the measured 142–143 B) and `DEL`/`UNLINK` of it shall be ≤1 ms p50 (≥7× faster
  than the measured 7.28/6.21 ms, within ~6× Redis's 0.166 ms); `RPUSH`, `LRANGE`,
  `LSET`, `LINSERT`, `LREM`, `LPOS` and blocking-pop semantics and tests shall be
  unchanged with matched throughput within 10%; accounting/`maxmemory`,
  rewrite/replay exactness and both protected gates shall pass on the exact head;
  no dependency shall be added.
- **AC35 (deferred drop, only if 4.33 is implemented).** `UNLINK` of a
  200,000-element list shall have p50 ≤0.5 ms and p99 ≤2 ms (versus the measured
  6.21/9.8 ms) independent of element count; `FLUSHALL ASYNC` on 20,000 keys shall
  be p50 ≤1 ms (versus 9.20 ms) and ≤2× Redis's 0.886 ms; `DBSIZE`, `EXISTS`,
  `SCAN` and `TTL` shall reflect the deletion immediately; pending bytes shall be
  accounted and bounded with a documented synchronous fallback; shutdown shall
  drain; `DEL` and `FLUSHALL SYNC` shall stay within 10% of baseline;
  notification/journal/WATCH ordering and persistence tests shall pass; no
  dependency shall be added; and both protected gates shall pass on the exact
  head.
- **AC36 (in-place string mutation, met by PR #258).** `APPEND` of 64 B to a
  32 MB value shall have p50 ≤0.5 ms and p99 ≤2 ms; `SETRANGE` of 16 B at offset
  0 of a 32 MB value shall have p50 ≤0.5 ms; a 1 MB value shall stay at the
  command floor; the 64 B controls and `GETRANGE` shall be unchanged; bytes
  copied per operation shall be bounded by the delta plus amortized growth; exact
  bytes, zero-fill growth, the 512 MiB limit, TTL/WATCH/journal/replay/rewrite
  and accounting shall pass; no dependency shall be added; and both protected
  gates shall pass on the exact head. Post-fix: `APPEND` 0.131 ms (p99 1.22) and
  `SETRANGE` 0.185 ms (p99 1.32), both 56×/39× faster than the pre-fix
  baseline and at or below the C servers' p50.
- **AC37 (in-place collection writes, only if 4.35 is implemented).** `HSET` into
  a 10,000-field hash, `ZADD` into a 10,000-member sorted set and `SADD` into a
  10,000-member set shall each have p50 ≤0.3 ms (within ~3× the faster C server
  and ≥4× faster than the measured 1.23–6.45 ms baselines); `RPUSH` and `LPOP` on
  a 200,000-element list shall have p50 ≤0.3 ms and p99 ≤2 ms (≥59× faster than
  the measured 17.8–18.3 ms); the 10-element controls shall be unchanged; every
  collection command, pack threshold, field TTL, blocking, persistence/rewrite/
  replay and accounting test shall pass; no dependency shall be added; and both
  protected gates shall pass on the exact head.
- **AC38 (constant-time random members, only if 4.36 is implemented).** At
  100,000 members, `SRANDMEMBER`, `HRANDFIELD`, `ZRANDMEMBER` (single and
  `COUNT 10`) and `SPOP` shall have p50 ≤0.6 ms and p99 ≤2 ms (≥18× faster than
  the measured 11.1–33.7 ms and within ~3–5× the faster C server); the 10k p50
  shall be ≤0.3 ms and the 10k→100k ratio ≤2×; a distribution test over repeated
  draws shall be uniform within a stated tolerance; 10-element controls shall be
  unchanged; the `SMEMBERS`/`HKEYS`/`ZSCAN` ordering decision shall be documented
  with tests updated; `SPOP` empty-deletion, TTL, blocking,
  persistence/rewrite/replay and accounting tests shall pass; no dependency shall
  be added; and both protected gates shall pass on the exact head.
- **AC39 (bitmap mutation, popcount and bit scan, only if 4.37 is implemented).**
  `SETBIT` at offset 0 of a 16 MB bitmap shall have p50 ≤0.5 ms and p99 ≤2 ms
  (≥10× faster than the measured 5.32 ms and within ~2× Redis's 0.349 ms);
  `BITCOUNT` of the 16 MB bitmap shall have p50 ≤6 ms and p99 ≤10 ms (≥3.8×
  faster than 23.0 ms and within ~2× Redis's 3.37 ms); a not-found `BITPOS 0`
  shall have p50 ≤8 ms and p99 ≤15 ms (≥25× faster than 201.6 ms and within
  ~1.6× Redis's 5.08 ms) while a first-bit hit stays at the floor; the 1 MB rows
  shall be at the command floor; the 64 B controls, `GETBIT` and `STRLEN` shall be
  unchanged; BYTE and BIT ranges including negative indices, the offset and
  512 MiB limits, zero-fill, TTL/WATCH/journal/replay and accounting shall pass;
  no dependency shall be added; and both protected gates shall pass on the exact
  head.
- **AC40 (order-statistic rank, only if 4.38 is implemented).** At 100,000
  members, `ZRANK`/`ZREVRANK` for the first, middle and last member and
  `ZRANGE 50000 50009`/`ZREVRANGE` shall have p50 ≤0.5 ms and p99 ≤2 ms (≥100×
  faster than the measured 58.6–72.5 ms and within ~8× the faster C server); the
  10k p50 shall be ≤0.2 ms and the 10k→100k ratio ≤2.5×;
  `ZADD`/`ZINCRBY`/`ZREM` shall stay within 10% in a matched cohort; tie
  ordering, negative indices, reverse ranges, `LIMIT`, `ZSetPack`,
  rewrite/replay, TTL and accounting tests shall pass; 4.30's AC32 rows shall
  also pass; no dependency shall be added; and both protected gates shall pass on
  the exact head.
- **AC41 (pub/sub burst fan-out, only if 4.39 is implemented).** A 2,000-message
  pipelined burst to an active subscriber shall deliver 2,000/2,000 and keep
  `PUBSUB NUMSUB` at 1; if the documented byte bound is exceeded, the behavior
  shall be a counted drop with the subscription intact or a documented disconnect,
  never a silent removal; `PUBLISH` among 10,000 subscribed channels shall have
  p50 ≤0.3 ms (≥2.7× faster than the measured 0.80 ms and within ~2× the C
  server) and stay flat from 1 to 10,000 channels; 100 subscribers shall receive
  every message with publisher p50 within ~3× the C servers; a 2,000-command
  burst with a monitor attached shall deliver all 2,000 lines (or a counted
  drop/document disconnect, never a silent truncation); ordering, pattern/shard,
  unsubscribe and shutdown tests shall pass; no dependency shall be added; and
  both protected gates shall pass on the exact head.
- **AC42 (O(1) cardinality and membership, only if 4.40 is implemented).** At
  100,000 members, `SISMEMBER` (hit and miss) and `SCARD` shall have p50 ≤0.2 ms
  (≥40× faster than the measured 8.0–8.7 ms and within ~2× the faster C server);
  `HLEN` and `ZCARD` p50 ≤0.1 ms; `LLEN` on a 200,000-element list p50 ≤0.05 ms;
  the 10-element controls, `HEXISTS` and `STRLEN` shall be unchanged; `SetInt`,
  `SetPack` and `HashPack` membership semantics (including non-integer probes),
  missing keys and type errors shall pass; no dependency shall be added; and both
  protected gates shall pass on the exact head.
- **AC43 (in-place list seek, only if 4.41 is implemented).** On a 200,000-element
  list, `LINDEX` for 0, middle and −1 and a hit-first `LPOS` shall have p50
  ≤0.2 ms and p99 ≤2 ms (≥6× faster than the measured 1.26–1.47 ms and within
  ~3× the faster C server); a missing `LPOS` shall stay ≤2 ms; the 10-element
  controls, `LRANGE` and `LLEN` shall be unchanged; negative indices,
  `RANK`/`COUNT`/`MAXLEN`, missing keys and wrong-type errors shall pass; no
  dependency shall be added; and both protected gates shall pass on the exact
  head.
- **AC44 (in-place stream append, only if 4.42 is implemented).** At 5,000
  entries, `XADD` shall have p50 ≤0.2 ms and p99 ≤2 ms (≥6.8× faster than the
  measured 1.36/11.6 ms and within ~2× Redis's 0.104/1.87 ms); at 1,000 entries
  p50 ≤0.15 ms; the 1k→5k p50 ratio ≤2×; `XLEN`, `XRANGE`, `XREVRANGE`,
  groups/PEL, trimming, rewrite/replay, TTL and accounting shall be unchanged;
  the 10-entry control shall be unchanged; no dependency shall be added; and both
  protected gates shall pass on the exact head.
- **AC45 (geospatial neighbor-cell search, only if 4.43 is implemented).** At
  50,000 members, a 1 km `GEOSEARCH` and a 2×2 km `BYBOX` shall have p50 ≤0.3 ms
  and p99 ≤2 ms (≥14× faster than the measured 4.34/4.49 ms and within ~2× the
  faster C server); `GEOPOS` for 1 and 100 members shall be ≤0.2 ms and ≤0.5 ms
  (≥45× for 100), and `GEODIST` ≤0.2 ms (≥15×); an all-member search and the
  10-member controls shall be unchanged; results (members, order, distances,
  hashes, coordinates) shall match the current linear implementation exactly on a
  boundary/dateline fixture; `ASC`/`DESC`/`COUNT`, `FROMMEMBER`,
  `BYRADIUS`/`BYBOX`, `WITH*` and `STORE` tests shall pass; no dependency shall be
  added; and both protected gates shall pass on the exact head.
- **AC46 (multi-member direct probes, only if 4.44 is implemented).** At 100,000
  members, `ZMSCORE` with 100 members shall have p50 ≤0.5 ms and p99 ≤2 ms (≥500×
  faster than the measured 273 ms and within ~3× the faster C server); 10 members
  ≤0.2 ms and 1 member ≤0.15 ms; `SMISMEMBER` with 100 members ≤0.5 ms (≥23×
  faster than 11.75 ms), 10 members ≤0.2 ms and 1 member ≤0.2 ms; `HMGET` and the
  10-member controls shall be unchanged; missing, non-integer and duplicate
  members and the `SetInt`/`SetPack` semantics shall pass; no dependency shall be
  added; and both protected gates shall pass on the exact head.
- **AC47 (bounded lazy expiry, only if 4.45 is implemented).** After 30,000 keys
  expire with no intervening command, `MGET` (100 keys), `KEYS prefix*`,
  `RANDOMKEY` and `DBSIZE` shall have p50 ≤2 ms and p99 ≤5 ms (≥18× faster than
  the measured 36–46 ms and within ~5× the faster C server); a lazy `GET` shall
  stay ≤1 ms; reads shall never return an expired value; the `DBSIZE` semantics
  shall be documented with tests updated; admission shall stay bounded and
  atomic; TTL expiry/notification/WATCH tests shall pass; no dependency shall be
  added; and both protected gates shall pass on the exact head.
- **AC48 (direct `BY nosort` slice, only if 4.46 is implemented).** On a
  100,000-element list, `SORT ... BY nosort LIMIT 0 10` shall have p50 ≤0.5 ms
  and p99 ≤2 ms (≥16× faster than the measured 7.96/17.2 ms and within ~4×
  Redis's 0.12/1.17 ms); the full, limited, ALPHA, DESC, `BY w*` and `GET #` rows
  shall stay within 1.5× of baseline; the 100-element controls shall be
  unchanged; offset/count edges, `STORE`, missing keys and set/zset sources shall
  pass; no dependency shall be added; and both protected gates shall pass on the
  exact head.
- **AC49 (in-place HyperLogLog with cached count, only if 4.47 is implemented).**
  100,000 pipelined `PFADD` shall finish in ≤0.6 s (≤6 µs/add, ≥2.8× faster than
  the measured 1.70 s and within ~1.6× the faster C server); 10,000 repeated
  `PFCOUNT` calls shall finish in ≤0.15 s (≤15 µs/call, ≥17× faster than 2.66 s
  and within ~5× the faster C server); single-call p50 and the 10-element controls
  shall stay within noise; counts shall be within the HLL error bound and
  identical before/after serialization; the cache shall be invalidated by
  add/merge and exact↔dense conversion; rewrite/replay/restore and TTL tests shall
  pass; no dependency shall be added; and both protected gates shall pass on the
  exact head.
- **AC50 (in-place stream group creation, only if 4.48 is implemented).** On a
  5,000-entry stream, `XGROUP CREATE` shall have p50 ≤0.2 ms and p99 ≤2 ms (≥40×
  faster than the measured 8.26 ms and within ~2.5× the faster C server); the
  10-entry control shall be unchanged; `BUSYGROUP`, `MKSTREAM`, explicit-ID/`$`,
  PEL/consumer, journal/rewrite/replay and TTL tests shall pass; no dependency
  shall be added; and both protected gates shall pass on the exact head.
- **AC51 (ordered sorted-set pops, only if 4.49 is implemented).** At 100,000
  members, `ZPOPMIN` and `ZPOPMAX` shall have p50 ≤0.3 ms and p99 ≤2 ms (≥500×
  faster than the measured 156.7/159.9 ms and within ~2× the faster C server);
  `ZPOPMIN COUNT 10` ≤0.5 ms (≥290×) and `ZMPOP` ≤0.3 ms (≥500×); the 100-member
  control shall be unchanged; empty/negative count, tie order, score formatting,
  key deletion, blocking variants, journal/rewrite/replay and accounting tests
  shall pass; no dependency shall be added; and both protected gates shall pass on
  the exact head.
- **AC52 (seek-based lex ranges and span counts, only if 4.50 is implemented).**
  At 100,000 equal-score members, `ZRANGEBYLEX` and `ZREVRANGEBYLEX` with
  `LIMIT 0 10` (unbounded, mid-range and reverse) shall have p50 ≤0.3 ms and
  p99 ≤2 ms (≥150× faster than the measured 56.0–69.8 ms and within ~3× the
  faster C server); `ZLEXCOUNT` and `ZCOUNT` (full, full-matching and empty)
  shall have p50 ≤0.3 ms and p99 ≤2 ms with a 10k→100k ratio ≤2×; the full
  100k-member lex reply shall stay within 1.5× the faster C server; 10-member
  controls shall be unchanged; mixed-score, exclusive/infinite bounds, LIMIT
  edges, store forms, pack thresholds, TTL/WATCH/journal/rewrite and accounting
  tests shall pass; no dependency shall be added; and both protected gates shall
  pass on the exact head.
- **AC53 (borrowed-source set/zset algebra and cardinality, only if 4.51 is
  implemented).** At 100,000 members, `ZINTERCARD` shall have p50 ≤30 ms and p99
  ≤60 ms (≥7× faster than the measured 220.6 ms and within ~2× the faster C
  server); `SINTERCARD` without a limit shall have p50 ≤45 ms; `SINTERCARD` and
  `ZINTERCARD` with `LIMIT 10` shall have p50 ≤0.5 ms and p99 ≤2 ms (≥100× faster
  than 50.7/267.2 ms and within ~2.5× the faster C server), independent of
  container size, with `LIMIT 0` matching the unlimited result and cost;
  `ZINTERSTORE` and `ZDIFFSTORE` shall have p50 ≤130 ms and p99 ≤200 ms (≥2.3×
  faster than 306.6/331.3 ms and within ~2.1× the faster C server);
  `ZUNIONSTORE` p50 ≤360 ms; the 10-member controls and set-store variants shall
  be unchanged;
  weights, `AGGREGATE MIN/MAX/SUM`, NaN/infinite scores, missing/empty keys,
  destination-as-source, TTL/WATCH/journal/rewrite and accounting tests shall
  pass; no dependency shall be added; and both protected gates shall pass on the
  exact head.
- **AC54 (O(1) INFO keyspace, only if 4.52 is implemented).** At 500,000 keys,
  `INFO keyspace` and plain `INFO` shall have p50 ≤0.4 ms and p99 ≤2 ms (≥25×
  faster than the measured 10.0/10.1 ms and within ~2× the faster C server), and
  at 100,000 keys p50 ≤0.25 ms; the reported `keys`, `expires`, `subexpiry` and
  `avg_ttl` shall match a full scan after insert, expire, persist, remove, reap,
  eviction, flush, rename/copy and replay churn; `INFO stats`, `DBSIZE`, `PING`
  and every other section shall be unchanged; no dependency shall be added; and
  both protected gates shall pass on the exact head.
- **AC55 (in-place two-key moves, only if 4.53 is implemented).** `SMOVE` on a
  100,000-member set shall have p50 ≤0.5 ms and p99 ≤2 ms (≥50× faster than the
  measured 25.5 ms and within ~2× the faster C server); `LMOVE` and `RPOPLPUSH`
  on a 200,000-element list shall have p50 ≤0.5 ms and p99 ≤2 ms (≥78× faster
  than 39.2/41.2 ms and within ~1.5× Redis); a same-key `LMOVE` rotate shall have
  p50 ≤0.4 ms; the 10-element controls shall be unchanged; LEFT/RIGHT
  combinations, same-key identity, missing source, non-member, empty-source
  deletion, TTLs, blocking `BLMOVE`/`BRPOPLPUSH`, WATCH/journal/notification/
  rewrite and accounting tests shall pass; no dependency shall be added; and
  both protected gates shall pass on the exact head.
- **AC56 (in-place list element edits, only if 4.54 is implemented).** At
  200,000 elements, `LINSERT` at the head shall have p50 ≤0.3 ms and p99 ≤2 ms
  (≥60× faster than the measured 19.1 ms); `LSET` at index 100,000 p50 ≤0.5 ms
  and p99 ≤2 ms (≥30× faster than 16.5 ms); `LINSERT` at the middle p50 ≤4.5 ms
  and `LREM` 1 near the tail p50 ≤8 ms (≥4.6×/≥2.6× and within ~1.3×/1.1× the
  faster C server); `LPOS ... RANK -1` p50 ≤0.6 ms; the 10-element controls
  shall be unchanged; BEFORE/AFTER/missing pivot, index edges, LREM count
  semantics, emptied-key deletion, chunk split/merge, TTL/WATCH/journal/rewrite
  and accounting tests shall pass; no dependency shall be added; and both
  protected gates shall pass on the exact head.
- **AC57 (frame-derived argument cap, only if 4.55 is implemented).** At the
  default 2 MiB frame, `MGET` with 10,000 keys, `DEL` with 2,000 keys, `EXISTS`
  with 2,000 keys, `TOUCH` with 2,000 keys and `MSET` with 600 pairs shall be
  accepted and return the correct result; the 1,024-key boundary rejection shall
  be gone; a request whose complete frame exceeds `max_frame_bytes` shall still
  be rejected before command execution; the parser's transient memory shall be
  bounded by `max_frame_bytes`; the PING fast path, all arity/protocol tests and
  both protected gates shall pass on the exact head; and no dependency shall be
  added.

INVEST story for the next increment (out-of-gate evidence, then a bounded
candidate if it loses): independently valuable and testable, with the six
workload rows in section 4.16, one recorded mechanism per resulting candidate,
a matched baseline and a rejection condition. Pipeline-1 ping is no longer the
open item: the newest gate leads it by 15.4% against Valkey with an interval
excluding 1.0 (section 2.18), while the same runtime lost it once on a shared
runner (section 2.17); the daily cohort is only 1.0% ahead of Redis inside the
advisory band. Definition of done:
AC1–AC15 as applicable, `make ci`, both performance gates, all 14 contexts, an
up-to-date head, owner review for any dependency/policy change, and a concise
evidence report that keeps every failed cohort. The phased
multi-owner/socket-worker program in
[Dragonfly-class performance](dragonfly-class-performance.md) remains the
separate roadmap.

## 9. Reproduction and evidence map

- Current gates: `python3 scripts/pr_gate_policy.py --live`; the two performance
  workflows are the acceptance authority. Required contexts are listed in
  [pr-quality-gate](../pr-quality-gate.md) and
  [`.github/pr-workflow-gates.json`](../../.github/pr-workflow-gates.json).
- Newest merged-head gate (section 2.18): run 42994675861 on head `ce122e0`,
  merged as `7b365dc`; Valkey and Dragonfly each PASS 10/10 from per-workload
  partial jobs plus a merged artifact. Retain every partial even when the merged
  verdict passes.
- Newest cohort (section 2.18): run 43128571502 for PR #217, merge candidate
  `ce3ca13` on base `8b38d0b`; Valkey and Dragonfly PASS 10/10. P1 ping
  181,489 vs 157,268 (+15.4%, 5/5, ratio 1.1301 [1.1219–1.2376]), P1 get +4.7%
  (4/5, interval contains 1.0), P16 ping +84.5%, P16 get +93.8%. The same runtime
  lost the P1 ping median in #216 (section 2.17), so both cohorts are required
  context, not a contradiction.
- Failed docs-only cohort #218 (section 2.17): run 43134573112, head `506325d`,
  base `8b38d0b`; Valkey P1 ping 138,976.1 vs 140,875.2 (−1.35%, 2/5, ratio
  0.9819 [0.9388–1.0209]), P16 ping 5/5 at +48.9%, all other rows passed. Kept
  with #216/#217 as the identical-code variance evidence; not retried until
  green.
- Placement evidence (section 2.9): the manifests of #212 (run 42994675861),
  #217 (43128571502), #218 (43134573112), #220 (43147853671) and #223
  (43171755325) all report `client_cpus == server_cpus == allowed_cpus ==
  [0,1,2,3]`; the runner has exactly four logical CPUs. Pass/fail therefore
  cannot be attributed to placement, and a disjoint control needs a different
  runner type.
- Per-request latency and process CPU (section 2.22): derived from the five
  `sample.json` files per P1 workload in the #212 partials above (run
  42994675861); no new runs. Each sample records p50/p95/p99/max and
  `server_cpu_seconds` for the measured window. Medians are over the five paired
  runs; raw files remain in the run artifacts.
- Failed docs-only cohort (section 2.17): PR #216, run 43113767982, head
  `ddcc10c` (runtime identical to merged `d357e2a`); Valkey P1 ping 138,803.5 vs
  139,745.7 (−0.67%, 3/5, ratio 1.0030 [0.9371–1.0278]), all other rows passed.
  Retained as the variance evidence; not a runtime regression and not retried
  until green.
- Hosted gate-rate profile (section 2.23): run 43125144121, source `8b38d0b`,
  artifact
  `reactor-profile-8b38d0b…-43125144121-1`, validated with
  `python3 scripts/reactor-profile.py --check <artifact>/reactor-profile`. It is
  a manual instrumented diagnosis, not a gate, ranking or latency result. Per
  reply, tell-tale counts: ping 1.336 recv + 1.000 send + 0.200 epoll; GET 1.710 +
  1.000 + 0.362; Valkey 1.000 + 1.000 + 0.264/0.237. Replies are the `sendto`/
  `write` tracepoint totals in the 8 s capture.
- Five-server daily cohort (section 2.19): run 43022929390 on source `7b365dc`,
  artifact `daily-five-server-7b365dc…-43022929390-1`, 250/250 samples. Advisory;
  never a gate substitute.
- Second five-server cohort (section 2.19): manual dispatch run 43164876654 on
  `bcd4768` (runtime `7b365dc`), 250/250 samples. Rillcache leads all ten rows;
  P1 ping 177,056 vs Redis 144,519 and Valkey 136,730 (+22.5%/+29.5%), P16
  throughput +54–100%, but all five P1 p99 are worse (0.111–0.143 vs
  0.079–0.095 ms) and P16 GET p99 is 0.231 vs 0.151/0.159. Advisory; retained as
  the host-dependent tail evidence, not a gate.
- Failed large-value screen (section 2.21): local 2026-11-11 attempt on the shared
  k3s development host with pinned Redis 8.10.2/Valkey 9.1.2; only two 64-byte
  control rows completed (5,414 and 3,437 ops/s, ~20–33x below the daily cohort),
  no large-value row, remaining runs stopped. Files retained in the development
  environment. It admits nothing and is not tracked source. **Retry 2026-11-12**
  (same host, load ~2.7) produced a usable cohort: 64 B control 39.6k/33.9k/32.2k
  (Rill/Redis/Valkey); 64 KiB GET 10,734/10,970/14,998; 1 MiB GET
  776/1,340/1,409 with p99 24.9/9.9/10.9 ms; 1 MiB SET 1,337/1,861/1,959.
  Shared-host diagnostic; raw JSON retained (untracked).
- Source comparison refresh: Redis 8.10.2 `server.h` `COPY_AVOID_MIN_STRING_SIZE`
  16384 and `isCopyAvoidPreferred`; Valkey 9.1.2
  `min-string-size-avoid-copy-reply` 16384; both `kvstoreIncrementallyRehash`
  1 ms budget; Redis `isCommandReusable`. These are code citations, not runtime
  measurements.
- Watch-free mutation comparison (sections 2.20 and 4.20): Redis
  `touchWatchedKey` early return
  ([`multi.c#L389`](https://github.com/redis/redis/blob/8.10.2/src/multi.c#L389)),
  Valkey
  ([`multi.c#L461`](https://github.com/valkey-io/valkey/blob/9.1.2/src/multi.c#L461)),
  against our `Data::touch` (`src/engine/data.rs`, around line 369). The upstream
  early return is source evidence; the per-mutation cost on this host is not
  measured, so the candidate stays conditional on 4.19.
- Persistence write comparison (section 3.9, candidate 4.21): Redis/Valkey buffer
  AOF bytes in `aof_buf` and write once per event-loop flush, with a 1 s
  background fsync for `everysec`
  ([`redis aof.c#L1399`](https://github.com/redis/redis/blob/8.10.2/src/aof.c#L1399),
  [`valkey aof.c#L1178`](https://github.com/valkey-io/valkey/blob/9.1.2/src/aof.c#L1178));
  our `Journal::append` writes one framed transaction per journaled command
  (`src/engine/journal.rs`, around line 1028).
- Persistence-on measurement (section 3.9, candidate 4.21): paired local SET
  runs, three Latin-rotated rounds, `appendonly yes`, `appendfsync everysec`,
  8 clients, 64 B, 50k requests, fresh journal per run. AOF-on/off medians:
  P1 11,856/35,387 (−66.5%) p99 5.53/1.53 ms; P16 39,927/406,549 (−90.2%) p99
  15.12/3.09 ms. Redis P1 +12.0%, P16 −33.5%; Valkey P1 −17.7%, P16 −8.5%.
  Absolute rates are shared-host diagnostics; the paired ratios and cross-server
  pattern are the evidence. HMAC was off; the framing cost is CRC plus the
  per-command write.
- AOF HMAC measurement (section 3.9, Rill only): key on/off, three rounds, AOF
  `everysec`. Append P1 14,355 vs 12,893 (−10.2%), P16 41,821 vs 31,938
  (−23.6%), p99 12.0 vs 11.5 ms; 50k replay 0.206 vs 0.749 s (3.6×), DBSIZE
  50,000 both. Shared-host diagnostic; raw JSON retained (untracked).
- `appendfsync always` attempts (section 3.9): two local 2026-11-12 cohorts
  (2 rounds/2k requests at load 10+; 3 rounds/5k requests at load ~3, P1/P16,
  Rill/Redis/Valkey). Both were unusable: the first varied 1.6k–107k req/s, the
  second still had `always` above `everysec` for the C servers at P16. Retained
  as failed evidence; no numbers claimed. The per-append-sync versus
  per-loop-fsync source difference and the 4.21 one-fsync-per-batch requirement
  remain.
- Startup replay measurement (section 3.15, candidate 4.27): 50k/200k SETs of
  64 B with `appendonly yes`, clean SIGTERM, three restarts per size, time from
  process start to first PING. Rill 0.258 s (50k) and 1.10–1.24 s (200k, DBSIZE
  200,000); Redis 0.011/0.027–0.043 (50k DBSIZE verified 50,000); Valkey
  0.054/0.051–0.061. Shared-host diagnostic; raw JSON retained (untracked).
- Pipeline/client scaling (section 2.25): 8 clients at P1/P16/P64/P256 and 64
  clients at P16, 100k requests, 64 B, 1024 keys, three rotations. Rill leads
  Redis/Valkey by 29–122% in every P≥16 and 64-client row; P1 SET +3.0/+2.5% and
  P1 GET +16.5/+10.0%. Shared-host diagnostic; raw JSON retained (untracked).
- Large multi-bulk replies (section 3.2, candidate 4.29): 10,000-element list and
  10,000-field hash, 4 clients, 250 requests each, three rotations. `LRANGE 0 9999`
  (230 KB): Rill 1,272 req/s (p50/p99 2.09/15.5), Redis 913 (3.9/10.9), Valkey 726
  (4.9/18.4). `HGETALL` (350 KB): Rill 313 (11.6/32.1), Redis 552 (7.0/16.3),
  Valkey 336 (11.4/26.4). Shared-host diagnostic; raw JSON retained (untracked).
- HGETALL perf attribution (section 3.2): single-client 10k `HGETALL` loop at
  ~268 req/s, `perf record -F 999 -g`; 11.9% `sort::unstable`, 7.8% `hash_items`,
  3.3% `encode_bulk`, ~59% libc self, ~15% kernel. `LRANGE` on the same data is
  ~46% kernel with no sort. Diagnostic only; raw perf data retained in the
  development environment (untracked).
- Sorted-set cohort (section 3.2, candidate 4.30): 10,000-member zset (6-byte
  members, 3-digit scores), 4 clients, 250 requests each (1,000 `ZSCORE` per
  client), three rotations. `ZSCORE` 6,913 (p50/p99 0.37/2.95), Redis 8,607
  (0.32/2.17), Valkey 10,019 (0.28/1.58). `ZRANGE 0 9` 528 (4.61/13.9), Redis
  10,311 (0.24/1.49), Valkey 10,771 (0.28/1.37). `ZRANGE 0 9999` 340 (11.3/20.3),
  Redis 1,581 (2.30/6.04), Valkey 1,036 (3.58/7.61). Controls at 128 members
  (inside the host spread) and 1,024 members (17–20% behind on the small range).
  Raw JSON retained (untracked).
- Sorted-set perf attribution (section 3.2): single-client 10k `ZRANGE 0 9` loop
  at ~301 req/s, `perf record -F 999 -g`; 21.5% sort code, 21.2% libc tuple
  moves in the sort, 6.4% `zset_items`, ~59% libc self and ~10% kernel.
  Diagnostic only; raw perf data retained (untracked).
- Cursor-iteration cohort (section 3.16, candidate 4.31): 100,000 keys
  (`key:%06d`, 16-byte values) plus a 10,000-field hash, one connection, 300
  spread-cursor `SCAN COUNT 10` calls, a full `SCAN COUNT 1000` traversal and a
  full `HSCAN COUNT 10` traversal, three rotations. Per-call p50 42.64/46.76/42.58
  ms versus Redis 0.26/0.32/0.24 and Valkey 0.10/0.16/0.22; full scan 4.81 s
  (101 calls) versus 0.239/0.167 s; full HSCAN 5.05 s (1,000 calls) versus
  0.257/0.247 s. PING floor 0.21/0.16/0.10 ms. Raw JSON retained (untracked).
- Large-collection drop cohort (section 3.17, candidates 4.32/4.33): three
  200,000-element lists of 8-byte values plus five 100-element controls, and ten
  20,000-key flush rounds, three rotations. `DEL` large 7.28 ms versus
  0.166/0.130; `UNLINK` large 6.21 (max 9.8) versus 0.316/0.235; small control
  0.28 versus 0.13/0.12; `FLUSHALL ASYNC` 9.20 (max 17.4) versus 0.886/0.568;
  `FLUSHALL` 10.12 versus 3.44/1.52. RSS delta for one 200k list 27.8–28.0 MB
  (142–143 B/element) versus 1.47–1.54 MB (7.5–7.9 B). Raw JSON and `/proc`
  samples retained (untracked).
- String-mutation cohort (section 3.18, candidate 4.34): one 1 MB value set
  directly and one 32 MB value built from 512 KiB `APPEND`s; 200 repetitions at
  1 MB and 100 at 32 MB, three rotations. 32 MB `APPEND` p50 6.81/8.86/7.35 ms
  versus Redis 0.14/0.18/0.12 and Valkey 0.13/0.12/0.24; `SETRANGE` p50
  6.98/8.86/7.19 versus 0.12/0.18/0.21 and 0.10/0.06/0.18; 64 B controls
  0.080/0.135 versus 0.12–0.17; `GETRANGE 0 15` 0.118 versus 0.195/0.243. Raw
  JSON retained (untracked).
- Collection-write cohort (section 3.19, candidate 4.35): a 10,000-field hash, a
  10,000-member zset, a 10,000-member set and a 200,000-element list of 8-byte
  values, plus 10-element controls; 100 repetitions per row, three rotations.
  `HSET` p50 7.00/6.45/6.21 ms versus Redis 0.09/0.58/0.11 and Valkey
  0.14/0.09/0.08; `RPUSH` 19.86/18.34/17.36 versus 0.08/0.09/0.07 and
  0.06/0.11/0.07; `LPOP` 18.08/17.21/17.82 versus 0.34/0.19/0.53 and
  0.07/0.21/0.07; `ZADD`/`SADD` medians 4.21/1.23 versus 0.08–0.22/0.08–0.35;
  controls 0.11–0.23. Raw JSON retained (untracked).
  The 100k extension (20 repetitions) uses 100,000-member hash/zset/set plus
  10-member controls and adds update/remove rows: `HSET` new/existing
  92.7/92.8 ms versus 0.12–0.32/0.08–0.21, `HDEL` 99.4 versus 0.15/0.15,
  `ZADD`/`ZINCRBY`/`ZREM` 61.4/57.3/63.9 versus 0.10–0.23/0.21–0.26/0.10–0.13,
  `SADD`/`SREM` 11.2/10.7 versus 0.25–0.60/0.08–0.20; controls 0.15–0.23. Raw
  JSON retained (untracked).
- Random-member cohort (section 3.20, candidate 4.36): 10k and 100k sets/hashes/
  zsets plus 10-element controls; 100 repetitions at 10k and 50 at 100k, three
  rotations. 100k `SRANDMEMBER` p50 11.68/11.54/11.24 ms versus Redis
  0.20/0.24/0.18 and Valkey 0.12/0.33/0.41; `HRANDFIELD` 33.66/34.29/30.77
  versus 0.34/0.14/0.40 and 0.11/0.26/0.10; `ZRANDMEMBER` 21.32/22.76/20.61
  versus 0.25/0.34/0.20 and 0.16/0.20/0.10; `SPOP` 11.11 versus 0.304/0.126.
  The 10k rows are 1.0–2.6 ms and controls 0.13–0.31. Raw JSON retained
  (untracked).
- Bitmap cohort (section 3.21, candidate 4.37): one 1 MB and one 16 MB `0xff`
  bitmap (the 16 MB built by 512 KiB `APPEND`s), 100/30 repetitions, three
  rotations. 16 MB `SETBIT` p50 5.32/5.58/4.85 ms versus Redis 0.09/0.71/0.35
  and Valkey 0.31/0.53/0.22; `BITCOUNT` 22.48/23.24/23.01 versus 3.52/3.13/3.37
  and 3.77/4.56/5.70; 1 MB `SETBIT`/`BITCOUNT` 0.331/1.849 versus
  0.092/0.259 and 0.172/0.305; `GETBIT`, `STRLEN` and 64 B controls at the
  floor. Raw JSON retained (untracked).
- Set-algebra probe (section 2.24, no candidate): two overlapping sets of 10k
  and 100k members, 50/20 repetitions, three rotations. At 100k, `SINTER`
  52.5 versus 38.9/23.2 ms, `SINTERCARD` 52.0 versus 35.7/15.3, `SDIFF` 49.9
  versus 66.9/40.1, `SUNION` 76.8 versus 105.4/78.8 (Rill leads Redis); at 10k
  the same commands are 5.4–7.8 versus 1.4–9.8 ms. Raw JSON retained
  (untracked).
- Sorted-set rank cohort (section 3.22, candidates 4.30/4.38): a 10,000- and a
  100,000-member sorted set (7-byte members, scores 100–899), one connection,
  50/20 repetitions, three rotations. 100k `ZRANK` middle p50 58.56/100.91/57.21
  ms versus Redis 0.36/0.06/0.16 and Valkey 0.07/0.07/0.23; `ZRANK` last 72.5,
  `ZREVRANK` 71.4, `ZRANGEBYSCORE ... LIMIT 0 10` 62.4, deep `ZRANGE` 63.7,
  `ZRANGE 0 9` 57.4, `ZSCORE` 3.27, versus 0.07–0.17 for both pins; 10k rows
  2.4–3.1 ms; 10-element controls 0.28/0.11 versus 0.10–0.14. Raw JSON retained
  (untracked).
- Pub/sub fan-out cohort (section 3.23, candidate 4.39): one subscriber with a
  reader thread plus a single pipeline of 2,000 `PUBLISH` (32-byte payload), and
  a subscriber on 1 / 1,000 / 10,000 channels with 500 / 500 / 200 timed
  publishes, three rotations. Burst: Rill delivered 1,024/2,000 in all three
  rounds, receiver-count sum 1,024, `PUBSUB NUMSUB` 0 after; Redis and Valkey
  delivered 2,000/2,000 with NUMSUB 1. `PUBLISH` p50 1 channel: 0.12–0.22 versus
  0.13–0.30/0.20–0.30; 1,000 channels: 0.13 versus 0.16–0.23/0.21–0.30; 10,000
  channels: 0.38–0.89 versus 0.14–0.21/0.12–0.31. Raw JSON retained (untracked).
- Cardinality/membership cohort (section 3.24, candidate 4.40): a 100k
  set/hash/zset and a 200k list, 100 repetitions, three rotations. `SISMEMBER`
  hit 8.45/7.70/8.04 ms versus Redis 0.19/0.07/0.40 and Valkey 0.22/0.13/0.27;
  miss 8.67 versus 0.17/0.21; `SCARD` 8.18/7.02/8.08 versus 0.14/0.20/0.37 and
  0.18/0.15/0.08; `HLEN` 2.24 versus 0.15/0.19; `ZCARD` 1.57 versus 0.12/0.16;
  `LLEN` 1.55 versus 0.20/0.14; `HEXISTS` 0.18 versus 0.29/0.14; `STRLEN` 0.13
  versus 0.16/0.16; 10-element controls 0.12–0.18. Raw JSON retained (untracked).
- List random-access cohort (section 3.25, candidate 4.41): a 200,000-element
  8-byte list, 100 repetitions, three rotations. `LINDEX 0` p50 1.473 versus
  Redis 0.107 and Valkey 0.068; middle 1.404 versus 0.104/0.081; last 1.289
  versus 0.102/0.142; `LPOS` hit-first 1.256 versus 0.095/0.127; `LPOS` missing
  1.806 versus 5.019/5.854; `LLEN` 1.364 versus 0.138/0.117; 10-element controls
  0.097/0.097 versus 0.09–0.12. Raw JSON retained (untracked).
- Stream cohort (section 3.26, candidate 4.42): a 1,000- and a 5,000-entry stream
  of one 8-byte field pair, 30/50 repetitions, three rotations. 5k `XADD` p50
  1.36/1.53/1.22 ms (p99 11.6) versus Redis 0.30/0.10/0.07 and Valkey
  0.06/0.08/0.14 (p99 1.87/1.58); 1k 0.25/0.43/0.26 versus 0.10/0.71/0.15 and
  0.08/0.10/0.08; `XLEN` 0.072 versus 0.156/0.103; `XRANGE` first/mid
  0.121/0.120 versus 0.099–0.307/0.187–0.288; `XREVRANGE` 0.118 versus
  0.117/0.136; 10-entry control 0.077/0.118 versus 0.078/0.103 and 0.231/0.143.
  Raw JSON retained (untracked).
- Geospatial cohort (section 3.27, candidate 4.43): a 50,000-member geo grid
  over ~0.5°, 50 repetitions, three rotations. 1 km `GEOSEARCH` p50
  4.34/4.40/3.85 ms versus Redis 0.12/0.16/0.16 and Valkey 0.14/0.17/0.23;
  2×2 km box 4.49 versus 0.21/0.19; 5 km (all members) 10.38 versus
  10.09/10.91; `GEOPOS` 1 3.03/1.32/1.99 versus 0.14/0.19 and 100
  26.86/24.86/22.33 versus 0.47/0.61; `GEODIST` 0.86/3.77/3.12 versus
  0.168/0.167; 10-member controls 0.149/0.139 versus 0.24/0.165 and 0.18/0.264.
  Raw JSON retained (untracked).
- Multi-member cohort (section 3.28, candidate 4.44): a 100k set/hash/zset, 50
  repetitions, three rotations. `SMISMEMBER` 1/10/100 p50
  7.90/8.49/12.73, 8.66/8.25/11.75, 7.90/7.42/10.90 versus Redis
  0.094/0.227/0.394 and Valkey 0.075/0.077/0.211; `ZMSCORE`
  11.77/23.42/273.23, 5.35/43.97/336.85, 8.02/33.37/221.99 versus
  0.102/0.422/0.630 and 0.112/0.328/0.187; `HMGET` 1/10/100
  0.23/0.36/0.32 versus 0.47/0.30/0.52 and 0.11/0.07/0.17; controls
  0.33/0.26/0.26. Raw JSON retained (untracked).
- Mass-expiry cohort (section 3.29, candidate 4.45): 30,000 keys with `PX 400`,
  a 0.7 s wait, one command per fresh wave, two waves per command, three
  rotations. `MGET` p50 36.29 ms (per-wave 29.31/51.17, 27.95/43.26,
  26.94/51.35) versus Redis 0.69 and Valkey 0.98; `KEYS` 43.58 versus 0.63/0.80;
  `RANDOMKEY` 45.45 versus 0.65/0.60; `DBSIZE` 41.21 versus 0.58/0.83; lazy
  `GET` 0.92 versus 0.75/0.72. Raw JSON retained (untracked).
- SORT cohort (section 3.30, candidate 4.46): a 100,000-element integer list plus
  `w<i>` weight keys, 50 repetitions, three rotations. `BY nosort LIMIT 0 10`
  p50 7.8/8.0/8.7 ms (p99 17.2) versus Redis 0.12 (1.17) and Valkey 0.16 (0.77);
  sorted `SORT` 92.5 versus 84.8/105.6; `LIMIT 0 10` 32.4 versus 26.0/29.0;
  ALPHA 90.2 versus 108.1/106.8; DESC 33.5 versus 28.2/28.1; `BY w*` 117.5
  versus 92.1/80.4; `GET #` 30.0 versus 27.3/26.9; 100-element controls
  0.21/0.27 versus 0.15/0.26 and 0.26/0.23. Raw JSON retained (untracked).
- Large-hash/keyspace listing cohort (section 3.31, extending 4.29/4.31): a
  100,000-field hash (7-byte fields, 16-byte values) and a 100,000-key keyspace,
  30 repetitions, three rotations. `HGETALL` 132.3/112.4/132.6 ms versus Redis
  25.6/26.2/24.0 and Valkey 40.9/37.8/35.5; `HKEYS` 116.3/98.2/98.5 versus
  17.0/16.4/15.6 and 15.3/15.8/19.2; `HVALS` 101.6 median versus 19.7/17.9;
  `KEYS key:*` 96.5/91.1/80.1 versus 49.4/42.8/42.5 and 21.1/23.0/20.7;
  `KEYS key:0000*` 32.7 versus 35.1/7.0; `KEYS nope*` 23.7 versus 26.7/5.5;
  `RANDOMKEY` 0.52 versus 0.21/0.29; `HMGET` 100 0.49 versus 0.47/0.22;
  10-member controls 0.40/0.25. Raw JSON retained (untracked).
- HyperLogLog cohort (section 3.32, candidate 4.47): 100,000 distinct elements
  added in 2,000-command pipelines, then 10,000 `PFCOUNT` calls, three rotations.
  `PFADD` 1,709/1,664/1,696 ms (16.96 µs/add) versus Redis 410/421/367 (4.10) and
  Valkey 389/487/392 (3.92); `PFCOUNT` 2,655/2,771/2,646 ms (265.5 µs/call)
  versus 35.7 (3.57) and 33.1 (3.31); single `PFADD`/`PFCOUNT` p50
  0.187/0.484 versus 0.151/0.169 and 0.302/0.059; 10-element controls
  0.164/0.185 versus 0.195/0.228 and 0.073/0.123. Raw JSON retained (untracked).
- BITPOS cohort (section 3.33, extending 4.37): one 16 MB `0xff` bitmap (built by
  512 KiB `APPEND`s), 10 repetitions, three rotations. `BITPOS 0` 200.1/201.6/
  206.7 ms versus Redis 5.3/5.1/4.9 and Valkey 5.7/4.6/5.1; `BITPOS 0 0 -1`
  209.4 versus 4.79/5.96; `BITPOS 1` 0.12 versus 0.60/0.09; `BITCOUNT` 23.46
  versus 4.04/4.78; 64 B `BITPOS 0` 0.32 versus 0.55/0.10. Raw JSON retained
  (untracked).
- Stream-group cohort (section 3.34, candidate 4.48): a 5,000-entry stream plus
  a 10-entry control, 20 unique groups each, three rotations. `XGROUP CREATE`
  8.26 ms (7.47/13.1/8.26) versus Redis 0.083 (0.07/0.10/0.08) and Valkey 0.094
  (0.09/0.13/0.09); 10-entry control 0.338 versus 0.120/0.097;
  `CREATECONSUMER` 0.123 versus 0.227/0.127; `SETID` 0.099 versus 0.283/0.099;
  `DESTROY` 0.109 versus 0.097/0.109; 10-entry `CREATECONSUMER` 0.169 versus
  0.086/0.069. Raw JSON retained (untracked).
- Sorted-set pop cohort (section 3.35, candidate 4.49): a 100,000-member sorted
  set, 20 pops, three rotations, 100-member control. `ZPOPMIN` 156.7 ms
  (169.3/156.7/156.4) versus Redis 0.42 (0.4/0.7/0.2) and Valkey 0.34
  (0.3/0.3/0.3); `ZPOPMAX` 159.9 versus 0.21/0.29; `ZPOPMIN COUNT 10` 146.6
  versus 0.35/0.16; `ZMPOP 1 z MIN` 155.8 versus 0.16/0.20; 100-member control
  0.11 versus 0.16/0.24. Raw JSON retained (untracked).
- Transaction cohort (section 3.36, measured null): one pipelined `MULTI ... EXEC`
  per sample, 20 repetitions, three rotations. `MULTI`+100 `SET`+`EXEC` 0.764 ms
  (0.57/1.16/0.76) versus Redis 0.448 (1.04/0.44/0.45) and Valkey 0.742
  (1.56/0.45/0.74); 1,000 commands 3.02 versus 2.76/2.48; 100×1 KB 0.873 versus
  1.042/1.077; queue-only `DISCARD` 1.37 versus 1.30/1.77; plain 1,000 `SET`
  control 1.42 versus 2.20/2.59; second 1,000-command tx 2.85 versus 2.75/2.59.
  Raw JSON retained (untracked).
- MONITOR cohort (section 3.37, extending 4.39): one 2,000-`PING` pipeline with
  0/1/10 attached monitors, two bursts per configuration, three rotations. Lines
  per monitor: Rill 1,024 in all six samples; Redis/Valkey 2,000 in all six.
  Publisher p50 0 monitors 5.18 versus 1.88/2.37 ms; 1 monitor 3.75 versus
  5.66/16.74; 10 monitors 5.99 versus 19.78/27.22. Raw JSON retained
  (untracked).
- Lex-range cohort (section 3.38, candidate 4.50): one 100,000-member zset with
  score 0 and members `m000000`–`m099999`, a 10-member control, 20 repetitions
  per small reply and five per full reply, three rotations. `ZRANGEBYLEX - +
  LIMIT 0 10` 51.0/56.0/66.0 ms versus Redis 0.18/0.22/0.16 and Valkey
  0.15/0.80/0.27; mid-range 63.2/70.5/69.8 versus 0.14–0.20/0.18–0.54; reverse
  55.1/60.1/62.5 versus 0.13–0.69/0.32–1.02; `ZLEXCOUNT` all 1.88/1.13/2.28
  versus 0.17–0.37/0.07–0.29 and narrow 17.3/13.0/15.0 versus
  0.21–0.42/0.07–0.34; `ZCOUNT` 1.07–2.45 versus 0.14–0.78/0.18–1.20; full
  100k lex reply 137.9/159.3/144.8 versus 85.1–91.0 and 99.2–110.5; 10-member
  controls 0.25–0.47 versus 0.09–0.33/0.17–0.22. Raw JSON retained (untracked).
- Stream consumer-group cohort (section 3.39, measured null): one 10,000-entry
  stream with one 16-byte field, one group owning all 10,000 entries, 20
  repetitions per row (40 fresh groups for the `NEW` rows), three rotations.
  History 100 0.61/1.26/0.91 ms versus Redis 1.14/0.88/1.50 and Valkey
  0.50/0.72/0.62; history 1000 5.90/7.68/7.00 versus 7.21/7.40/11.10 and
  4.76/5.80/6.64; new 100 0.84/1.14/0.92 versus 0.55/0.57/1.32 and
  0.69/0.75/0.62; new 1000 7.96/8.99/9.36 versus 4.73/5.19/7.51 and
  6.55/5.84/6.43; `XPENDING` extended 1000 3.57/4.36/3.25 versus 2.55/2.93/3.93
  and 2.96/3.72/4.49; `XAUTOCLAIM` 1000 9.84/10.54/9.05 versus 4.67/4.44/5.66
  and 6.13/8.16/7.50; `XCLAIM` 100 at parity; `XLEN`/`XADD` controls 0.18–0.43/
  0.19–0.39. Preload (10,000 `XADD`) took 16.2 s versus 0.1–0.2 s, the 4.42
  clone path. Raw JSON retained (untracked).
- BITFIELD cohort (section 3.40, extending 4.37): one 16 MB `0x01` value, one
  1 MB value and a 64 B control, 20 repetitions, three rotations. 16 MB
  `BITFIELD SET u8` 4.38/6.09/4.56 ms versus Redis 0.24/0.06/0.09 and Valkey
  0.42/0.09/0.07; `INCRBY u8` 4.69/3.83/4.68 versus 0.24/0.08/0.12 and
  0.55/0.11/0.07; `SET u63` 4.38/4.71/4.39 versus 0.11/0.11/0.30 and
  0.12/0.19/0.10; 100× `INCRBY u8` 3.83/4.89/4.58 versus 0.31/0.24/0.41 and
  0.31/0.54/0.25; `BITFIELD_RO GET` 0.21/0.47/0.33 versus 0.28/0.25/0.14 and
  0.20/0.09/0.14; 1 MB `SET u8` 0.14/0.59/0.51 versus 0.34/0.40/0.13 and
  0.17/0.63/0.08; 64 B control 0.06/0.30/0.19. The same pass re-measured
  `SETBIT` 4.08/4.37/4.96 versus 0.07/0.38/0.26 and 0.15/0.27/0.06, `GETBIT`,
  `GETRANGE` and `BITFIELD` all-`GET` at the floor. Raw JSON retained
  (untracked).
- Zset-algebra cohort (section 3.41, candidate 4.51): two 100,000-member
  collections (`a`=`m000000`–`m099999` score 1, `b`=`m050000`–`m149999` score 2),
  10 repetitions, three rotations, 10-member controls. `ZINTERCARD`
  222.1/215.6/220.6 ms versus Redis 41.3/34.7/40.4 and Valkey 17.1/15.1/23.4;
  `ZINTERSTORE` 305.1/306.6/373.1 versus 125.5/92.5/110.5 and 65.3/57.3/61.6;
  `ZDIFFSTORE` 331.3/306.1/345.7 versus 99.8/102.0/103.3 and 61.4/59.6/66.0;
  `ZUNIONSTORE` 458.0/477.6/491.4 versus 356.0/373.3/366.6 and
  280.3/286.2/268.3; `SINTERSTORE` 44.3/42.0/50.5 versus 66.9/57.2/67.3 and
  36.5/29.4/39.4; `SUNIONSTORE` 65.8/60.3/77.5; `SDIFFSTORE` 38.1/39.6/52.1;
  `SINTERCARD` 52.6/40.8/41.5 versus 39.0/29.6/29.2 and 11.4/15.0/13.6;
  10-member controls 0.16–0.25. Preload (four 100k collections) took 16.0 s
  versus 1.5–1.7 s, the 4.35 write path. Raw JSON retained (untracked).
- INFO-keyspace cohort (section 3.42, candidate 4.52): 10k, 100k and 500k plain
  string keys with no TTLs, 20 repetitions, three rotations. `INFO keyspace`
  0.15/0.41/0.21 ms at 10k versus Redis 0.19/0.10/0.22 and Valkey
  0.53/0.14/0.09; 2.94/2.79/2.70 at 100k versus 0.11/0.17/0.19 and
  0.13/0.12/0.22; 10.93/10.00/9.14 at 500k versus 0.13/0.25/0.28 and
  0.13/0.30/0.38. Plain `INFO` 10.19/10.15/9.40 at 500k versus 0.35/0.36/0.48
  and 0.22/0.37/0.26; `INFO stats` 0.11–0.30 and `DBSIZE` 0.10–0.43 at 500k.
  Preload (500k pipelined `SET`) 2.7 s versus 2.1–2.7 s. Raw JSON retained
  (untracked).
- Two-key-move cohort (section 3.43, candidate 4.53): two 100,000-member sets and
  two 200,000-element 16-byte lists, 20 repetitions (each `SMOVE` consuming a
  distinct member), three rotations. `SMOVE` 25.49/23.74/26.39 ms versus Redis
  0.40/0.68/0.38 and Valkey 0.28/0.16/0.36; `LMOVE` 40.64/36.79/39.15 versus
  0.34/0.31/0.47 and 0.08/0.42/0.07; `RPOPLPUSH` 43.53/36.40/41.16 versus
  0.23/0.33/0.85 and 0.15/0.66/0.19; same-key rotate 20.51/22.87/19.93 versus
  0.24/0.26/0.44 and 0.26/0.21/0.10; 10-member `SMOVE` 0.31/0.38/0.23;
  `LPUSH`/`SADD` controls 0.18/0.22; `SISMEMBER` control 8.69/9.39/10.55 (the
  known 4.40 path). Preload 13.4 s versus 1.4–1.6 s, the 4.35 write path. Raw
  JSON retained (untracked).
- Cardinality-LIMIT cohort (section 3.44, extending 4.51): two 100k sets and two
  100k zsets with identical membership, 20 repetitions, three rotations.
  `SINTERCARD` 56.75/49.31/53.26 ms versus Redis 38.39/40.10/46.63 and Valkey
  25.44/30.00/31.03; `SINTERCARD LIMIT 10` 50.72/59.09/49.50 versus
  0.26/0.15/0.20 and 0.21/0.25/0.28; `SINTERCARD LIMIT 0` 55.24/49.57/56.67;
  `ZINTERCARD` 267.27/267.35/270.79 versus 41.26/43.35/47.73 and
  24.45/24.91/22.94; `ZINTERCARD LIMIT 10` 313.76/259.46/267.22 versus
  0.22/0.18/0.33 and 0.19/0.49/0.08; `ZINTERCARD LIMIT 0`
  299.88/291.02/275.14; 10-member `LIMIT 10` controls 0.12–0.27; `EXISTS`/`PING`
  0.11–0.25. Preload 16.0 s versus 1.5–1.7 s, the 4.35 write path. Raw JSON
  retained (untracked).
- List-edit cohort (section 3.45, candidate 4.54): one 200,000-element list of
  distinct 8-byte values, 20 repetitions, three rotations. `LINSERT` head
  19.86/16.79/19.06 ms versus Redis 0.09/0.21/0.07 and Valkey 0.65/0.18/0.24;
  `LSET` index 100k 21.98/16.46/15.22 versus 0.09/0.34/0.14 and
  0.17/0.28/0.96; `LINSERT` middle 21.70/16.35/21.04 versus 3.51/3.69/3.13 and
  4.48/4.33/4.10; `LREM` tail 22.88/19.90/21.09 versus 6.97/7.12/6.29 and
  6.57/8.15/7.28; `LPOS RANK -1` 1.87/1.59/1.31 versus 0.09/0.30/0.16 and
  0.27/0.08/0.36; 10-element controls 0.20–0.61. Preload 3.9 s versus 0.3–0.5 s,
  the 4.35 write path. Raw JSON retained (untracked).
- SMEMBERS cohort (section 3.46, measured lead/null): sets of 1,000, 10,000 and
  100,000 8-byte members, 20 repetitions, three rotations. `SMEMBERS` 100k
  84.75/82.70/103.65 ms versus Redis 83.36/88.71/96.22 and Valkey
  83.05/88.66/86.37; 10k 9.95/9.28/12.86 versus 9.60/9.33/8.52 and
  8.79/9.60/9.58; 1k 1.47/0.97/0.91 versus 1.56/1.11/0.79 and
  1.24/1.40/0.89; 10-member control 0.08–0.34. Preload 1.9 s versus 0.3–0.4 s,
  the 4.35 write path. Raw JSON retained (untracked).
- Multi-key cohort (section 3.47, candidate 4.55): 120,000-key keyspace, 1,000
  keys per command (`MSET` 500 pairs), 20 repetitions, three rotations. `MGET`
  2.30/1.74/1.12 ms versus Redis 1.65/2.23/2.84 and Valkey 2.63/2.34/2.09;
  `MSET` 500 pairs 2.30/1.11/1.18 versus 0.38/0.85/0.58 and 0.53/1.25/1.08;
  `EXISTS` 0.85/0.94/0.88 versus 0.30/0.42/1.72 and 1.47/0.40/0.95; `TOUCH`
  0.55/0.99/0.98 versus 0.32/0.93/1.38 and 0.62/0.66/0.48; `DEL` 1,000 fresh
  keys 1.63/2.01/1.27 versus 0.75/0.98/1.04 and 0.77/1.03/1.01; 10-key controls
  0.11–0.65. Boundary probe: Rill accepts `MGET` 1,023 keys and rejects 1,024
  with `-ERR Protocol error: too many arguments`; Redis/Valkey accept `MGET`
  10,000, `MSET` 600 pairs and `DEL` 2,000. Raw JSON retained (untracked).
- Post-fix string cohort (section 3.18, PR #258): the same 1 MB/32 MB harness on
  the merged head, three rotations. 32 MB `APPEND` p50 0.131 (p99 1.22) versus
  Redis 0.142 (1.21) and Valkey 0.219 (2.24); `SETRANGE` 0.185 (1.32) versus
  0.222 (1.69) and 0.289 (3.31); 1 MB `APPEND`/`SETRANGE` 0.124/0.217 versus
  0.238/0.146 and 0.208/0.068; 64 B controls and `GETRANGE` at the floor.
  Pre-fix medians were 7.35/7.19 ms. Raw JSON retained (untracked).
- Keyspace-growth attempt (section 4.18): 262,144 keys × 64 B, P16, a fresh
  growth run (500k SET) then a post-growth run (200k SET) on the same server,
  three rotations. Rill 192k/124k req/s, p99 5.0/10.1 ms, max 64/81 ms; Redis
  150k/172k, 6.3/4.6, 80/28; Valkey 144k/116k, 5.8/10.2, 50/125. Post was not
  faster and maxima reached 371 ms; host-dominated and inconclusive, so 4.18
  stays deferred. Raw JSON retained (untracked).
- TTL-heavy diagnostic (section 3.7): 8 Python pipelined clients (16 deep), 1024
  keys, 64 B, 160k commands per run, three rotations, plain vs 10,000 s TTL.
  Medians: Rill SET 117k/125k, GET 112k/113k; Redis SET 111k/106k, GET 91k/125k;
  Valkey SET 110k/122k, GET 120k/111k. Differences are inside the host spread and
  the client bounds the rate; it supports keeping 4.9 closed, not a production
  claim. Raw JSON retained (untracked).
- Expiry and eviction comparison (section 3.10): Redis `expire.c` constants and
  `activeExpireCycle`, Valkey `active-expire-effort`, and the 16-entry eviction
  pool (`redis evict.c#L36`, `valkey evict.c#L54`) against our
  `purge_expired(128)` per 100 ms tick and bounded candidate scans.
- Eviction measurement (section 3.10, candidate 4.26): 100k SET requests over a
  100k-key space, 64 B, 8 clients, P1/P16, `allkeys-lru`, controls without a
  memory limit; three rotated rounds. Rill 500 KB payload (7,073 evictions),
  Redis/Valkey 2 MB (~4,000). P16 SET 144,365 no-eviction vs 20,498 evicted
  (−85.8%, p99 8.01/64.51 ms); P1 14,895 vs 8,339 (−44.0%). Redis P16 130,085 vs
  171,696 and Valkey 117,044 vs 137,789 (noisy, no penalty). A policy A/B shows
  Rill LRU 27.1–31.3k vs random 25.3–27.6k at P16, both far below control, while
  Redis shows no policy penalty. Shared-host diagnostic; raw JSON retained
  (untracked).
- Encoding comparison (section 3.11): Redis embstr limit 44 B and
  `tryObjectEncoding`, listpack/intset thresholds (`redis config.c#L3511`,
  `#L3593`, `#L3603`), against our `Entry` `Vec` value and the duplicated key in
  `expirations`/`versions`. Code citations; memory is only measured by the RSS
  column of the 4.16 rows.
- Blocking-wake comparison (section 3.12): Redis `signalKeyAsReady` from the
  mutation path (`redis db.c#L445`, `#L790`) and
  `handleClientsBlockedOnKeys` immediately after `call()` (`server.c#L4846`);
  Valkey the same (`db.c#L223`, `server.c#L4647`); against our 1 ms
  `ASYNC_TICK` re-enqueue without a ready-key signal (`reactor.rs#L20`,
  `#L467`). Measured 2026-11-12: 300 BLPOP/LPUSH wakeups per run, three rotated
  runs per server (900 each), randomized 0.5–2.5 ms push delay; after-push p50/p99
  were 1.16/1.92 ms (Rillcache), 0.031/0.221 (Redis 8.10.2) and 0.025/0.220
  (Valkey 9.1.2). Shared-host diagnostic; raw JSON retained in the development
  environment (untracked). Idle-CPU run (same day): 5 s process-CPU windows with
  0/1/10/50/100/1000 idle BLPOP clients; Rillcache 0.6/5.6/6.8/13.0/17.8/22.2%,
  Redis 0.4/—/—/—/0.2/0.6%, Valkey 0.4/—/—/—/0.4/0.6%. Raw JSON retained; not a
  gate.
- Async-client idle CPU (section 3.12, candidate 4.24): 5 s process-CPU windows
  with 0/1/100/1000 idle SUBSCRIBE clients: Rillcache 0.8/6.0/17.8/22.2%,
  Redis 0.2/0.2/0.6/0.4%, Valkey 0.4/0.4/0.6/0.4%; MONITOR: Rillcache
  0.6/5.8/20.8/27.4%, Redis 1.6/0.4/0.4/0.4%, Valkey 0.4/0.4/0.4/0.8%.
  Shared-host diagnostic; raw JSON retained (untracked), not a gate.
- Post-#226 idle re-measurement (section 3.12): current binary, 5 s CPU windows.
  Blocked: 0.6/5.0/6.6/12.4/20.4/21.4% at 0/1/10/50/100/1000; SUBSCRIBE:
  0.8/4.6/13.6/22.4% at 0/1/100/1000; MONITOR: 1.0/4.2/20.8/26.6%. Values are
  within pre-fix noise; the per-client scan remains. Raw JSON retained.
- Async delivery latency (section 3.12, candidate 4.24): 300 publishes/PINGs per
  run, three rotated runs, randomized phase; after-PUBLISH p50/p99 1.05/3.60 ms
  (Rillcache), 0.075/1.19 (Redis 8.10.2), 0.064/0.56 (Valkey 9.1.2); after-PING
  MONITOR line 0.37/6.15, 0.050/0.64 and 0.048/0.49. Shared-host diagnostic; raw
  JSON retained (untracked).
- Rewrite scheduling comparison (section 3.13): Redis/Valkey
  `rewriteAppendOnlyFileBackground` forks a COW child
  (`redis aof.c#L3239`, `valkey aof.c#L2595`); our cooperative slices and the
  measured 130 ms → 9–20 ms start pause are documented in
  [persistence.md](../persistence.md) on the 20k-key/2.5 MB fixture. The numbers
  are dev-host diagnostics, not a gate result; 4.23 is the conditional start-pause
  candidate.
- Current-head rewrite reproduction (section 3.13): local runs 2026-11-12 on the
  shared k3s host. 20k keys × 64 B: start-pause medians 9.1–14.2 ms (range
  5.5–45.7), full pass 4.85 s, concurrent GET p99 10.8 / max 88.5 over 4,187
  requests. 200k keys: start-pause medians 90.7–106.9 ms (range 83.3–116.0),
  full pass 43.4 s, concurrent GET p99 18.8 / max 338 over 23,791 requests.
  Raw results retained in the development environment (untracked); diagnostics,
  not cross-server cohorts. The maxima include shared-host scheduling beyond the
  rewrite.
- Connection-setup comparison (section 3.14): Redis/Valkey `tcp-backlog` 511 and
  `tcp-keepalive` 300 s with `anetKeepAlive`
  ([`redis config.c#L3526`](https://github.com/redis/redis/blob/8.10.2/src/config.c#L3526),
  [`anet.c#L115`](https://github.com/redis/redis/blob/8.10.2/src/anet.c#L115));
  a local `strace` of this build shows `SO_REUSEADDR` and `listen(..., 128)` with
  no keepalive option. Source/trace evidence. A local connect/PING storm attempt
  (section 3.14) was client-bound and inconclusive: 2,232 conn/s for Rill,
  3,796 for Redis, 2,189 for Valkey (p99 30.9/7.9/26.8 ms), so no direction is
  claimed; a kernel-level client is still needed.
- Gate artifacts retain raw medians, spreads, p99, CPU and hashes; read the
  manifest's `candidate_source`, `complete` and `gate_passed` before citing.
- Valkey four-CPU gate: five alternating rounds, wins/5 and a deterministic
  paired-bootstrap 95% interval (2000 resamples, fixed seed); the interval is
  diagnostic only and never passes or fails a row. Dragonfly remains three
  rounds. Historical cohorts above used three rounds and are marked as such.
- Five-round cohorts (section 2.17): run 41936225853 on head `927ccef` passed
  Valkey 10/10 and Dragonfly 10/10; P1 ping was 4/5 with ratio 1.0254
  [0.9703–1.0661]. Run 41940385826 on head `90c1768` passed Valkey 10/10
  (P1 ping 5/5, ratio 1.1876 [1.0206–1.2964]) but failed Dragonfly 9/10 on
  P1 ping (−1.47%). Artifacts under `/root/perf-evidence/pr189/four-cpu/` and
  `/root/perf-evidence/pr190/four-cpu/`.
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
- Adaptive-poll direction (section 4.13): **closed**. The merged 20 µs window is
  the baseline; a miss-based shrink and shorter fixed window are rejected
  analytically, and the hybrid 5 µs busy + 15 µs timed wait was measured and
  rejected (run 41612942833: CPU 95.5%, P1 p99 0.959 vs 0.231 ms). The ~95% P1
  CPU is the deliberate price of the P1 win; reducing it needs a kernel batching
  or busy-poll mechanism outside the safe-Rust/Mio boundary. Do not reopen with
  another window length.
- Manual cohort (section 2.11): run 41477114469 against source `9692e07`,
  180 validated records, Redis/Valkey/Rillcache five workloads at P1/P16 and
  one-core; deterministic tables retained at
  `/root/perf-evidence/manual-41477114469/report.md`. The generated report is now
  merged as `63e1f20` (PR #168); it is not a gate, and this record still uses the
  run artifact rather than the report as performance evidence.
- Hosted reactor diagnostic (section 4.11): runs 41500430375 (72 samples) and
  41507264904 (108) completed with frozen binaries and matched-thread /proc
  counters. Both selector variants were rejected, with losses retained. They are
  distinct from the later bounded-poll mechanism and its protected evidence.
- Clean-runner upstream cohort (section 2.14): run 41563160515 against source
  `c8b760a`, 180/180 valid samples, six rotated repetitions, 16 clients,
  disjoint server/client CPUs, pinned Redis 8.10.2 `redis-benchmark`; artifacts
  under `/root/perf-evidence/upstream-41563160515/`. Advisory only, no gate
  substitution and no report PR.
- Clean-runner repeat (sections 2.14–2.15): run 41633321852 against source
  `d5da908`, same harness and sample count; artifacts under
  `/root/perf-evidence/upstream-41633321852/`. Confirms the small ping-specific
  per-command CPU gap (~5%) and the small 16-client p99 disadvantage.
- Wakeup/spin diagnostic (section 4.11): superseded by the merged bounded-poll
  implementation in 4.12; the diagnostic's scheduler/syscall protocol remains the
  template for any future wakeup change.
- Cross-candidate count evidence (section 2.12): PR #167 one reactor, four-CPU run
  41518853948 (Valkey 8/10, P1 ping −0.80%, P1 GET −0.11%; Dragonfly 10/10);
  PR #172 two reactors, run 41544908472 (Valkey 9/10, P1 ping −1.42%; Dragonfly
  10/10). Artifacts retained under `/root/perf-evidence/pr167/` and `pr172/`.
- Merged owner-local reactor + bounded poll (section 4.12): PR #172 final head
  `c4d4698`, four-CPU run 41552051338 PASS Valkey 10/10 (P1 ping +3.13%) and
  Dragonfly 10/10 (+18.32%); original gate run 41552051280 PASS; merged as
  `21ed301`. Artifacts under `/root/perf-evidence/pr172/final/`.
- Post-merge docs cohort (sections 2.14–2.15): PR #175 head `e75036b`, four-CPU
  run 41564296997 PASS Valkey 10/10 (P1 ping 170,807.9 vs 140,275.0, +21.77%;
  p99 0.119/0.087) and Dragonfly 10/10 (178,931.8 vs 170,517.6; p99
  0.119/0.055); artifacts under `/root/perf-evidence/pr175/four-cpu/`.
- Later tail cohorts (section 2.15): #176 run 41566914311 (p99 0.071/0.087
  Valkey, 0.071/0.071 Dragonfly, rate +0.44% vs Valkey; artifacts under
  `/root/perf-evidence/pr176/four-cpu/`) and #179 run 41582234348 (p99
  0.119/0.087 Valkey, 0.127/0.063 Dragonfly, rate +23.54%; artifacts under
  `/root/perf-evidence/pr179/four-cpu/`). Both docs-only heads.
- Owner-local reactor and bounded poll (section 4.12): candidate default is one
  reactor plus a 20 µs zero-timeout window. Local frozen-binary diagnostic
  samples and the official `valkey-benchmark` screen are retained under
  `/root/perf-evidence/ping-max/`; they are shared-host mechanism evidence, not
  gate results. Fresh original and four-CPU gates on the exact head decide
  acceptance. The failed `ceil(CPUs/2)` head is preserved there with its raw
  four-CPU log.
- Development loop: `make dev-check` for editing feedback, `make test` after a
  complete logical change and `make ci` before review, per
  [development-cycle](../development-cycle.md). Local screens never replace the
  protected gates.
- Local paired diagnostic used in 2.4 (diagnostic only):
  `perf stat -p <server-pid> -e 'syscalls:sys_enter_recvfrom,...' -- sleep <n>`
  around a `valkey-benchmark` run with both servers pinned to the same CPUs.
- Local syscall diagnostic used in 2.16: merged default `d5da908`, server pinned
  CPU 0, clients CPUs 1–2, `perf stat` tracepoints over 6.088 s while
  `rillcache-bench` ran 20k P1 ping; raw files and the per-command table under
  `/root/perf-evidence/syscalls-current/`. Diagnostic only; the host ran at
  ~1.2k ops/s, so these counts are not gate predictions.
- Store-path and instruction A/B: `scripts/compare-builds.py` and the documented
  hot-path study, with frozen binaries and retained raw JSON.
- Dependency verification: `make ci` runs the policy test, which executes
  `cargo tree` for all targets/features and rejects non-allowlisted packages.

## 10. Limits and open questions

- The protected gate uses small strings, cyclic/random keys, eight clients and
  short five-sample Valkey medians (three for Dragonfly and historical cohorts).
  It is a merge policy, not a statistical proof or a production SLO; mixed TTLs,
  large values, saturation and tail SLOs remain outside it. The paired-bootstrap
  interval is diagnostic and does not change the strict median rule. It compares
  Valkey and Dragonfly; Redis 8.10.2 is pinned in the source analysis and the
  manual cohort (section 2.11), not in the required four-CPU comparison, so a
  Redis four-CPU loss would not be visible to the gate.
  The manual cohort shows Rillcache ahead on all five measured workloads at
  P1/P16, but it is not a gate and does not cover unseen commands or mixed
  workloads. The clean-runner upstream cohort confirms the same direction on the
  merged default (section 2.14); the first daily five-server cohort leads every
  row with only four P1 rows inside its advisory band, and the second daily
  cohort on identical code leads every row by 21.7–32.6% P1 against Redis/Valkey
  while all five P1 p99 are worse (section 2.19). The remaining measured gaps
  against Redis/Valkey are a small P1 per-command CPU difference whose sign is
  host-dependent, host-dependent P1 tail variance, the large-value reply copy
  and input-growth copies that the 64-byte gate cannot see — **1 MiB GET −42% to
  −45%, SET −28% to −32%** (sections 2.20–2.22) — the large multi-bulk hash
  reply (10k `HGETALL` **−43% vs Redis**, section 3.2) and the persistence-on
  journal write shape: **−66.5% P1 / −90.2% P16** SET with AOF on versus 8–34%
  for the C servers (section 3.9). The gate runs with persistence off.
- Since #198 the four-CPU gate measures each workload row in its own partial job
  on its own allocated runner of the same machine type. That is still a valid
  within-row alternating comparison, but cross-row absolute rates are not
  comparable, and a per-row host shift can move a margin. Earlier cohorts that
  used one shared runner are labeled as such in their sections.
- The gate scope is 8 clients at P1/P16. A local 8-client P64/P256 and 64-client
  P16 check (section 2.25) leads the C servers by 29–122% in every deep row, with
  P1 SET at +2.5–3.0%; no scaling loss is measured. The absolute rates are
  shared-host diagnostics and do not substitute for a gate row.
- The P1 ping row is host-sensitive. The refreshed runtime passed both four-CPU
  comparisons twice (section 2.10), but earlier identical binaries also lost that
  row by up to 4.32% on the same shared runner (section 2.9). Readiness was
  necessary but not sufficient, and the rejected drain probe shows that syscall
  counts alone are not a throughput proxy. Round-robin at four reactors improved
  Valkey but still lost Dragonfly PING; the eight-reactor response then lost P1
  ping to both and is rejected. Candidate 4.12 restored the single owner-local
  reactor and removed the blocking wakeup from the pipeline-1 round trip; it
  merged as PR #172 after passing both four-CPU gates (section 2.13). The
  remaining P1 question is host-sensitive measurement and other workloads, not a
  larger thread count.
- The same docs-only runtime lost the P1 ping median in #216 (−0.67%) and #218
  (−1.35%) and won it in #217 (+15.4%, interval excluding 1.0) within three hours
  on shared four-CPU runners. That −1.35% to +15.4% swing for identical code is
  the clearest measurement-risk evidence in the record; judge a change on its
  own fresh cohort, never by picking the favorable one.
- Expiry is outside the gate: our fixed 1,280 due reaps/s is bounded and ordered,
  but sustained high-churn TTL under memory pressure could leave stale keys longer
  than a 25%-CPU burst cycle; that remains unmeasured (4.16 row 1). A limited TTL
  diagnostic (section 3.7) found no consistent TTL-vs-plain penalty, but it is
  client- and host-bound and does not replace a clean TTL cohort. Eviction is
  now measured (section 3.10): under continuous pressure P16 SET loses **85.8%**
  and p99 rises to 64.5 vs 8.0 ms, policy-independent, while the C servers show
  little penalty; 4.26/AC29 cover it. Budgets differ (payload vs RSS) and the host
  is shared, so the absolute rates are diagnostic.
- Value and key representation is a memory axis, not a gate row (section 3.11).
  A TTL key that has been written can own four buffers (main key, value, expiry
  key, version key) where the C servers share one key object; integer values
  always allocate a `Vec`. No encoding rewrite is justified without the TTL or
  eviction RSS cohort, and no rate/tail claim follows from the gate's 64-byte
  rows.
- Blocking commands are the largest **measured** latency and idle-CPU gap in this
  review (section 3.12): after the push reply, blocked clients wake in 1.16 ms p50
  / 1.92 ms p99 versus 0.03/0.22 ms for Redis/Valkey on the same host, and one
  idle blocked client costs 5.6% of a core (22.2% at 1000) versus <1%; the
  post-#226 re-measurement is 5.0/21.4%, so the spin fix left the per-client scan
  intact. Idle
  subscribers and monitors show the same tick cost (6.0% at one, 22–27% at 1000;
  post-#226 4.6/22.4% and 4.2/26.6%)
  and delivery waits for the tick too: pub/sub 1.05/3.60 ms p50/p99 and MONITOR
  0.37/6.15 ms versus ≤0.08/1.19 ms for the C servers; both are covered by 4.24.
  The protected gate has no such client, so no gate row sees either gap;
  4.22/4.24 are admitted and AC25/AC27 gate the implementation.
- Rewrite scheduling has measured start-pause medians of **9–14 ms at 20k keys**
  and **91–107 ms at 200k** (section 3.13), with 4.85 s / 43.4 s full passes,
  where the C servers fork a copy-on-write child; the gate runs with persistence
  off, so no gate row sees it. The figures are dev-host diagnostics from
  [persistence.md](../persistence.md) and the current-head reproduction, not a
  cross-server cohort; 4.23 is the only candidate and needs a rewrite-latency
  measurement (AC26).
- Connection setup differs from the C servers: local `strace` shows a listen
  backlog of 128 and no TCP keepalive, where Redis/Valkey default to 511 and
  300 s (section 3.14). The gate's eight persistent connections do not exercise
  this; a Python connect-storm attempt was client-bound and inconclusive
  (2,232 vs 2,189/3,796 conn/s), so a kernel-level cohort is still needed and no
  production claim is made.
- P1 p99 is not consistently better under the merged bounded-poll default
  (section 2.15). The throughput gate passes on medians, but AC5 remains a design
  criterion: a future wakeup/poll change must measure p99 and reject a regression
  beyond AC5 even when throughput improves. Do not claim a tail win from
  throughput evidence.
- The #212 samples refine that: typical P1 latency (p50/p95) is lower in every
  command, while SET/XX p99 is ~50% higher on that host and lower on the daily
  host (section 2.22). The sign is host-dependent and unisolated; 4.19 is the
  measurement that must come before any runtime change. Per-command CPU also
  flips sign between the clean cohorts and the #212 process-CPU/command ratio, so
  neither is a stable deficit.
- Several earlier single-runner cohorts (through #179) had `client_cpus` equal to
  `server_cpus` (`[0,1,2,3]`), so the load generator shared the server CPUs.
  That biases a multi-reactor server against a single-thread competitor and put
  the P1 ping row inside a +2.9% to −4.3% run-to-run band for an unchanged
  runtime (section 2.9). The gate states disjoint client CPUs "when available",
  but every recent cohort reports `[0,1,2,3]` for both because the runner has
  exactly four logical CPUs; a disjoint control therefore needs a different
  runner type, not a configuration change. Changing the runner is a CI/policy
  decision, not a server tuning knob.
- Cross-client write coalescing (4.5) is closed: it has no target row, and the
  current per-client batch already leads every P16 row (sections 2.19 and 4.5).
  The resumable parser (4.2) is closed by bound: incomplete frames rescan only
  length headers, and a multi-megabyte workload would be needed to reopen it.
  The cached clock (4.9) and incremental rehash (4.18) are closed/deferred for
  the gate's keyspace and workload; only a matched TTL-heavy or large-keyspace
  cohort can reopen them. A local 262k-key growth attempt (4.18) was
  host-dominated and inconclusive (post-growth not faster; maxima to 371 ms), so
  the rehash question remains open for a clean cohort.
- The 128-byte command-counter padding is inert on the current one-reactor
  default and must not be cited as a cause of its P1 margins; a multi-reactor
  layout claim needs the A/B in section 4.10. The one structural difference this
  refresh did find is large-bulk reply copy avoidance and large-frame input
  growth, both measured on 2026-11-12 and now admitted as 4.15/4.25, not
  implemented changes.
- Watch-free mutation bookkeeping (4.20) is a real source difference with a
  below-noise throughput bound (~40–100 ns per mutation). It stays conditional on
  4.19 isolating a server-side mutation tail; do not remove `WATCH` tracking
  without the AC22 correctness proof and tests.
- Persistence-on behavior remains outside the protected gate but is now measured
  (section 3.9): AOF-on SET loses **66.5% at P1 and 90.2% at P16** (P16 p99 15.1
  vs 3.1 ms), against 8.5–33.5% for the C servers. The journal writes one framed
  transaction per command while the C servers write a buffered batch per loop;
  4.21 is admitted and AC23 sets the implementation targets. Absolute rates are
  shared-host diagnostics, not a gate result. The `always` policy is source-only:
  our per-append `sync_data` versus the C servers' per-loop fsync, with a local
  attempt that failed under host load (section 3.9); 4.21 requires one fsync per
  batch.
- Startup replay is also outside the gate but measured (section 3.15):
  time-to-PING is 0.258 s at 50k keys and 1.10–1.24 s at 200k, versus
  0.011–0.061 s for the C servers. The source shows a separate checksum parse
  pass plus a reparse and normal execute; 4.27/AC30 cover it. Shared-host
  diagnostic; DBSIZE was verified at 200k for Rill and 50k for Redis.
- AOF HMAC adds a measured **−10.2% P1 / −23.6% P16** append cost and a 3.6×
  50k replay (0.21 → 0.75 s). That is the price of per-transaction
  authentication; the append cost is accepted, and only the existing
  verification is folded into the 4.27 replay pass. The C servers have no
  equivalent, so this is an internal A/B, not a cross-server gap claim.
- The hosted profile (section 2.23) is instrumented evidence: tracepoints and
  CPU-clock sampling perturb the workload, its rates are quarantined, and the
  per-reply syscall counts are neither latency nor throughput. It corrects the
  local ~8-epoll figure (0.20–0.36 per reply at gate rates) and shows lower
  context switches, but it does not replace a gate. The drain-probe excess
  (+12%/+37% total syscalls) is real per reply, and its removal still lowered P1
  medians in PR #155, so it stays rejected.
- The reviewed dependency exceptions are rustls, Tokio, Mio and
  `core_affinity`, merged through their own PRs. A future head still needs fresh
  gates, and any further dependency needs its own review. The portable fallback
  remains the documented boundary for TLS, fixed workers and non-Unix builds.
- The io_uring send-batching attempt (section 4.14) is closed: it was implemented
  behind the safe-API `crates/rillcache-io` boundary, passed the full local suite,
  and then measured 5-14% slower with identical server CPU while its stress test
  still showed reply aliasing. It is evidence, not a pending task; do not revive
  it without a mechanism that reduces syscall or TCP work rather than adding a
  completion round trip.
- The short-read elision is a rejected experiment, not an unfinished task: its
  branch and gate artifacts are retained, and it must not be revived without a
  different readiness mechanism and fresh evidence.
- Redis 8.10.2 and Valkey 9.1.2 are pins, not an evergreen "latest" claim; source
  links are immutable tags.
- The sorted-set cohort (section 3.2) measures only rank ranges starting at 0,
  plain `ZRANGE`, and `ZSCORE`, on one 10,000-member set with duplicate scores.
  It does not measure deep-start ranks, `ZRANGEBYSCORE`/`BYLEX`, `WITHSCORES`,
  `ZADD` write cost, memory/RSS of an ordered index, or the 1,024–10,000 knee
  precisely. The 128- and 1,024-member controls bound the size regime; the
  mechanism attribution is a single-client perf sample on a shared host, so it
  identifies cost, not a gate-rate win or loss.
- The cursor cohort (section 3.16) measures single-connection per-call cost at
  `COUNT 10`/`COUNT 1000` and full traversals on a stable 100k-key keyspace. It
  does not measure concurrent SCAN with writes, `MATCH`/`TYPE` selectivity,
  `SSCAN`/`ZSCAN` directly, TTL keys (the unbounded `reap(now, usize::MAX)` is
  source-only), the cursor's compatibility cost, or the memory/RSS and mutation
  cost of a key index; those are implementation-PR evidence under AC33.
- The large-collection cohort (section 3.17) measures single-connection command
  latency for 8-byte list values, one flush size and `/proc` RSS deltas on one
  shared host. It does not measure large elements, other collection types, TTL
  keys, concurrent load, the total CPU of deferred freeing, queue-drain behavior,
  or allocator/fragmentation effects; the ASYNC flush sample spread is wide
  (Redis 0.17–6.8 ms). Those are implementation-PR evidence under AC34/AC35, and
  the RSS figure includes allocator arenas rather than exact object bytes.
- The string-mutation cohort (section 3.18) measures single-connection command
  latency for 1 MB and 32 MB values built by `APPEND`, with 64 B controls and a
  small `GETRANGE`. It does not measure concurrent mutation, persistence-on
  appends, the full 512 MiB ceiling, capacity retained after shrink, or the exact
  bytes copied. 4.34 landed in #258 and its post-fix cohort is now the regression
  evidence; small- and large-value `SET` copies remain 4.15/4.25 territory.
- The collection-write cohort (section 3.19) measures single-connection latency
  for one write into preloaded collections, with 10-element controls. It does
  not measure pipelined/batched writes (e.g. `HSET` with 500 pairs), concurrent
  readers, pack↔large threshold crossings, field-TTL paths, or memory; those are
  implementation-PR evidence under AC37. The 100k extension covers hash/zset/set
  only, not the list rows, and removes one field or member per sample, so it does
  not measure repeated shrink/grow cycles or empty-collection deletion. The Valkey
  `SADD`/`ZADD` control values show the shared host's spread.
- The random-member cohort (section 3.20) measures single-connection command
  latency at 10k and 100k for one collection type per command, with 10-element
  controls. It does not measure the sampling distribution statistically, very
  large (1M) collections, negative `COUNT` with repetition, `SPOP` with count, or
  the index's memory; those are implementation-PR evidence under AC38. The C
  server medians include the ~0.1–0.2 ms client floor.
- The bitmap cohort (section 3.21) measures one 1 MB and one 16 MB bitmap of
  `0xff` bytes, so the popcount is a best case and the write is at offset 0; it
  does not measure sparse bitmaps, BYTE/BIT sub-ranges, negative indices,
  `BITPOS`/`BITFIELD`, offsets near the 512 MiB cap, or concurrency. Those are
  implementation-PR evidence under AC39.
- The set-algebra probe (section 2.24) is a null result: at 100k, `SINTER`,
  `SINTERCARD` and `SDIFF` are 1.2–3.4× behind the faster C server, `SUNION`
  leads Redis, and the replies are large enough that encoding is a material part
  of every row. It does not justify a candidate; a smaller result or a
  store-variant cohort would be needed.
- The sorted-set rank cohort (section 3.22) measures single-connection reads on
  one 10k/100k set with duplicate scores; it does not measure `ZRANGEBYLEX`,
  `ZRANGESTORE`, `ZREMRANGEBYRANK`, negative/reverse deep ranges, concurrent
  writes, or the rank index's memory and `ZADD` cost. Those are
  implementation-PR evidence under AC32/AC40.
- The pub/sub cohort (section 3.23) measures one connection with a reader thread,
  a 2,000-message pipeline, and a single subscriber subscribed to up to 10,000
  channels. It does not measure 100-subscriber fan-out, slow/dead subscribers,
  pattern subscriptions, RESP3 frames, or memory under sustained fan-out; those
  are implementation-PR evidence under AC41. The burst depends on the 1 ms tick
  and the 1,024-entry queue, both source-confirmed.
- The cardinality/membership cohort (section 3.24) measures single-connection
  commands on 100k/200k collections with 10-element controls. It does not measure
  `SMISMEMBER`, `SINTERCARD`, `HSTRLEN`, missing-key paths at scale, or `LLEN`
  after heavy list mutation; those are implementation-PR evidence under AC42.
  The `SetInt`/`SetPack` probe semantics are source-only here.
- The list random-access cohort (section 3.25) measures a 200k list of 8-byte
  values with 10-element controls. It does not measure deep `RANK`/`MAXLEN`
  combinations, `LPOS` with `COUNT`, lists built from `LINSERT`/`LREM` or very
  large elements; those are implementation-PR evidence under AC43.
- The stream cohort (section 3.26) measures a single-field entry on one stream at
  1k/5k; it does not measure multi-field entries, trimming cost, groups/PEL,
  `XREAD` blocking, or 50k+ streams (the preload itself is O(n²) in the current
  implementation). Those are implementation-PR evidence under AC44.
- The geospatial cohort (section 3.27) measures a grid over ~0.5° with no
  boundary/dateline members; it does not measure `GEORADIUS` legacy variants,
  `WITHCOORD`/`WITHDIST`, `STORE`, or the neighbour-cell edge cases, which are
  correctness fixtures for the implementation PR under AC45. `GEODIST` showed
  host spread (0.86–3.77 ms).
- The multi-member cohort (section 3.28) measures one set/hash/zset at 100k with
  up to 100 requested members; it does not measure duplicate or non-integer
  members, `SetInt` encodings, `ZSetPack`, or larger member counts. Those are
  implementation-PR evidence under AC46; the ratio is expected to grow linearly
  with the member count because the scan is repeated.
- The mass-expiry cohort (section 3.29) measures one command after a single
  uniform expiry wave; it does not measure staggered expiry, admission under
  `maxmemory`, the periodic purge rate, or commands that touch few expired keys.
  Those are implementation-PR evidence under AC47. The C-server MGET samples
  include one 14.7 ms outlier; the median is used.
- The SORT cohort (section 3.30) measures one 100k integer list with weight keys;
  it does not measure `STORE`, set/zset sources, `BY hash*->field` patterns, or
  large `LIMIT` windows. Those are implementation-PR evidence under AC48. The
  sorted rows show the host's spread (ALPHA 90–108 ms across servers).
- The listing cohort (section 3.31) measures a 100k hash and 100k keyspace with
  fixed-size reply accounting; it does not measure `HSCAN`-order interactions,
  hash field TTLs, `KEYS` with expensive glob patterns, or the ordered-index
  memory. Those are implementation-PR evidence under AC31/AC33. The C-server
  `KEYS` prefix/none rows show a wide Redis–Valkey spread (5.5–35 ms).
- The HyperLogLog cohort (section 3.32) measures one key with 100k distinct
  elements and repeated counts in pipelines; it does not measure sparse
  encodings, `PFMERGE`, `PFCOUNT` over multiple keys, serialization/rewrite
  compatibility or the exact↔dense threshold. Those are implementation-PR
  evidence under AC49.
- The BITPOS cohort (section 3.33) measures a dense 16 MB bitmap of `0xff`; it
  does not measure sparse bitmaps (early hits), BYTE/BIT sub-ranges with
  offsets, or `BITFIELD`. Those are implementation-PR evidence under AC39.
- The stream-group cohort (section 3.34) measures one 5k-entry stream and a
  10-entry control; it does not measure 100k-entry streams, `MKSTREAM`,
  `BUSYGROUP`, PEL/consumer interactions or `XREADGROUP` after group creation.
  Those are implementation-PR evidence under AC50; the 100k extrapolation is
  linear from the 5k row.
- The zset-pop cohort (section 3.35) measures 20 pops on one 100k set; it does
  not measure `BZPOPMIN`/`BZPOPMAX`/`BZMPOP`, `COUNT` larger than 10, or the
  pop-min under WATCH/transactions. Those are implementation-PR evidence under
  AC51.
- The transaction cohort (section 3.36) measures pipelined transactions up to
  1,000 commands with one value size; it does not measure `WATCH` aborts, 10,000
  commands, or multi-owner transaction routing. It is a null result at the
  measured shape, not universal parity.
- The MONITOR cohort (section 3.37) measures a 2,000-command burst with up to 10
  monitors on one connection; it does not measure sustained monitor load, slow
  readers, MONITOR with large replies, or the proposed byte-bounded drain. The
  delivered-line truncation is deterministic across the six samples; publisher
  timings are host-noisy (0-monitor Rill 5.18 ms versus 3.75 at 1 monitor) and
  are not used as a rate claim.
- The lex-range cohort (section 3.38) keeps every member at score 0, so it
  measures the defined `BYLEX` domain but not mixed scores, where Rill's
  global lex filter and the C servers' skiplist traversal can differ (that
  behavior is an implementation obligation for 4.50, not a measured claim). It
  covers a 100k set only, one connection, and no `ZRANGESTORE`/`ZREMRANGEBY*`
  rows, slow readers or fragmentation; the full 100k reply is reply-bound and
  its 1.4–1.7× is reported as that, not as a seek cost.
- The stream consumer-group cohort (section 3.39) uses one 16-byte field per
  entry, one group and one owning consumer, and a 10k-entry PEL on one
  connection. It does not measure multi-field or large-field entries (where the
  per-entry clone share could grow), 100k-entry PELs, many consumers/groups,
  blocking `XREADGROUP`, or `XREAD`/`XRANGE` reply construction. The 1.3–2.1×
  residuals are single-cohort observations on a shared host, not a candidate.
- The BITFIELD cohort (section 3.40) covers one 16 MB and one 1 MB value on one
  connection, with u8/u63 widths, `WRAP` overflow and no `FAIL`/`SAT` or signed
  widths; it does not measure multi-key `BITFIELD`, values near the 512 MiB cap,
  or a value that grows past its current end under concurrent readers. The write
  timing is dominated by two full-value copies and scales with value size, which
  the 1 MB and 64 B rows confirm; it is not a per-subcommand claim.
- The zset-algebra cohort (section 3.41) uses two 100k zsets with disjoint-by-half
  membership, integer scores and default weights/`SUM`; it does not measure
  `WEIGHTS`, `AGGREGATE MIN/MAX`, NaN/±infinity scores, more than two sources,
  destination-as-source, `ZSetPack` sources or `SET` inputs, which are
  implementation obligations for 4.51 rather than measured claims. It also
  re-confirms that the set-store variants lead; the `SCARD` control's 43.5 ms is
  the admitted 4.40 cardinality path, not a floor.
- The INFO-keyspace cohort (section 3.42) uses string keys with no TTLs, no field
  TTLs and one database on one connection, so it isolates the walk; it does not
  measure a large expiring set (the 4.45 mass-expiry path), `INFO all` reply
  formatting cost at much larger section counts, or concurrent writers during
  `INFO`. The candidate's correctness burden — counters equal to a full scan
  under churn — is an implementation obligation, not measured here.
- The two-key-move cohort (section 3.43) uses 16-byte list elements, one element
  per move and a single connection; it does not measure multi-element moves
  (none exist), blocking `BLMOVE` latency, chunk-boundary split/merge behaviour,
  field-TTL collections, or moves between a pack and a large container. The
  `SISMEMBER` row is the admitted 4.40 cardinality path, not a move control.
- The cardinality-LIMIT cohort (section 3.44) uses identical membership (every
  member common) so the first ten matches are found immediately by an early-exit
  implementation; it does not measure a sparse overlap where the probe must skip
  many non-matches, more than two inputs, `SetInt`/`SetPack`/`ZSetPack` mixes, or
  approximate cardinality (not implemented). The `LIMIT 0` rows confirm the
  unlimited contract, not the early-exit path.
- The list-edit cohort (section 3.45) uses distinct 8-byte values on one 200k
  list, with one edit per command and 20 repetitions; it does not measure
  duplicate-value `LREM` counts, `LREM` negative counts, index near the chunk
  boundary, `ListPack`/`ListQuick` threshold transitions, or blocking reads
  concurrent with edits. The middle-insert and tail-removal rows are scan-bound
  on all three servers, so their ratios understate the rebuild share.
- The SMEMBERS cohort (section 3.46) uses one connection, 8-byte members and no
  pipelining; the 100k row is reply-bound on a loopback socket, so it measures
  the full collect+sort+encode+write path, not the sort alone. It does not
  measure a slow reader with `CLIENT PAUSE`/backpressure, a set that changes
  during the reply, `SetInt`/`SetPack` mixed encodings, or a 1M-member set; the
  conclusion is scoped to "do not extend 4.29 to sets", not a per-byte claim
  across sizes.
- The multi-key cohort (section 3.47) stays below the current argument cap
  (`MSET` 500 pairs), uses one connection and 10-key controls, and does not
  measure pipelined batches, 2,000+-key commands (currently rejected), or a
  frame-boundary argument set; the boundary probe issues one command per
  connection, so it observes the rejection reply, not how a pipelined batch
  behaves after it.
