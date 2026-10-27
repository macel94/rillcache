# Current bottlenecks: collection scaling and administrative scans

Measured 2026-10-27/28 against Rust application source
`f33b44c7703f71477fec8755001ce3946c0054ba`. Its `src/`, Cargo manifests/lockfile
and pinned toolchain are unchanged at `60b5ee399bb90b097a2d5d00bf747ca7d3ca6233`
(the main snapshot checked after collection). The TLS sidecar is **not** measured.
These are source-pinned findings, not evergreen claims about later main.

**This is an investigation, not an optimization PR.** No server behavior,
allocator, dependency, hashing policy, release profile or merge gate changes.
The old [hot-path study](../../../docs/hot-paths.md) describes older revisions;
its percentages must not be treated as current profiles.

## What is slow in the tested shapes

Seven operations, four sizes (128/1,024/8,192/32,768), three process repetitions:
**84 validated samples**, not best-of selection. The table below shows the
median of each repetition's handler p50, in microseconds. The
[complete 28-row table](component/summary.md) includes every size, repetition
spread, p99 and whole-loop cost; [all 84 records](component/samples.json) remain
available. Cardinalities increase 256-fold between these endpoints.

| Operation | 128 elements/keys, µs | 32,768 elements/keys, µs | Interpretation |
|---|---:|---:|---|
| PING | 0.060 | 0.060 | no-key-map control stays flat |
| HGET existing field | 0.350 | 0.380 | direct field lookup stays nearly flat |
| HSET existing field | 37.911 | 27,301.324 | full hash reconstruction, not constant-width in-place update |
| HLEN | 0.630 | 182.216 | builds a borrowed-pair vector to count fields |
| SCARD | 7.640 | 2,418.652 | clones/materializes members merely to count them |
| INFO server | 1.390 | 2,690.191 | scans the keyspace for sections not requested |
| MEMORY STATS | 1.160 | 612.450 | live-key scan despite mostly scalar output |

Do not compare these nanosecond-scale controls directly to TCP latency. Each
handler timer includes Instant/black-box/assertion overhead. Whole-loop timing
also includes reply validation; it is **not** identical to handler time. The
shared six-vCPU development VM has extreme tails: at 32,768 fields HSET's median
across the three reported handler p99 values is **532 ms**. The complete samples
retain worse/better runs; these observations are not stable-hardware SLOs.

### 1. HSET rebuilds an entire hash under the store write lock

At the pinned source, [`hash_set`](../../../src/engine/collections.rs#L258)
materializes `hash_items`, copies every field and value into a new HashMap,
updates the requested field, recomputes payload size and stages/replaces the
collection. This happens while the shared store write guard is held.

The probe updates **one existing field**, alternating different 64-byte values;
field count remains constant and the final target value is checked. It is not
an insert-growth benchmark or an unchanged-value shortcut. HGET on the same
shape is the lookup control. HSET's median handler p50 grows about **720×**
across the 256× field-count increase.

The separate whole-process HSET CPU profile records 2,520 user-cycle samples,
zero reported lost samples, and throttling. Among retained sampled periods,
the two memmove symbols account for **25.80%** self overhead; `_int_malloc`
16.13%, `cfree` 16.14%, and `malloc_consolidate` 12.17%. These are exclusive
symbol costs, not an end-to-end speedup prediction. Setup/warmup/validation are
included in this profile, unlike the uninstrumented handler timers.

**Bounded follow-up:** preserve staged failure atomicity and payload admission,
but avoid cloning all untouched fields on an admitted existing-field update.
Require binary/duplicate fields, field and key TTLs, OOM preservation,
WATCH/MULTI, encoding transitions, journal/notifications and all worker modes.
Do not simply mutate first and discover admission failure afterward.

### 2. HLEN and SCARD materialize collection data to return a scalar

[`hash_read`](../../../src/engine/collections.rs#L327) uses
`collection.hash_items().len()` for HLEN: it allocates a vector of **borrowed
pairs**, not deep copies of all values. `set_read` calls `set_items()` before
branching to SCARD, cloning the member representation unnecessarily. Both
paths take the store's write guard for expiry-aware reads.

Measured handler p50 grows about **289×** for HLEN and **317×** for SCARD.
The HLEN whole-process profile has 44.78% self overhead in the hash-items
vector-construction symbol; SCARD shows member traversal/cloning and allocator
work. Those captures have lost samples and include seeding; do not interpret
such percentages as clean steady-state attribution.

**Bounded follow-up:** representation-aware cardinality without materializing
members/pairs. Preserve lazy expiry visibility and integer/set/listpack/hash
encodings. Test missing/wrong types, empty collections, field expiry, threshold
transitions and counts after mutation. Source inspection suggests similar
opportunities in ZCARD/LLEN; **neither is measured here**, so they are not ranked
as verified bottlenecks.

### 3. Narrow INFO still eagerly scans the keyspace; MEMORY STATS scans too

The pinned `Store::info` computes live keys, expiring keys, average TTL and
logical memory data before deciding which requested section to emit. Thus
`INFO server`, which returns no keyspace section, pays for whole-map traversal.
Its measured p50 rises about **1,935×**. In the separate whole-process profile,
four entry-iteration/fold symbols account for **97.77%** self overhead among
retained sampled periods. That capture lost 16 samples; it is not kernel/wall
clock attribution.

[`memory_snapshot`](../../../src/engine/administration.rs#L145) scans all entries
under a read guard to count currently live/deadline-bearing keys. MEMORY STATS
p50 rises about **528×**; `memory_snapshot` accounts for 96.25% of retained
whole-process sampled user-cycle periods, with 23 lost samples.

**Bounded follow-up:** compute INFO sections lazily, starting with a server-only
query that does not require keyspace data. Treat scalar counter caching for
expiry-sensitive memory/keyspace data as a separate correctness problem; stale
counters are not an acceptable optimization. Preserve INFO default/all and
section combinations, commandstats/errorstats, expiry and logical-memory scope.

### What the measurements do not establish

- These single-threaded Store calls have **no network, Session observers, ACLs,
  TLS, concurrent clients, TTL population, eviction or journal I/O**. Default
  collection encodings are used, with noninteger 15-byte member names and 64-byte
  hash/string values. No claim covers all representations/data distributions.
- Source proves the expensive work holds a shared-store guard. **Cross-client
  head-of-line blocking is a risk, not measured contention latency here.** A
  concurrent small-key/large-collection TCP experiment is a follow-up, not a
  fabricated result.
- The priority above reflects these measured shapes, not a production traffic
  mix or a ranking of every command. It does not establish that the global lock
  is the universal problem; flat PING specifically provides no evidence for that.
- The required native Valkey gate covers PING/SET/GET/NX/XX with its own fixed
  keyspace, CPU budgets and pipelines. Passing its 20 rows cannot exclude these
  collection/administrative scaling problems. This PR still needs its **own**
  fresh exact-head protected checks; no old pass or component result substitutes.
- No before/after server change is measured and **no improvement is claimed**.
  Optimizing these paths may still regress TCP workloads, CPU use or tails.

## Evidence and collection method

[Component provenance](component/manifest.json) records source, binary SHA-256s,
CPU 0 affinity, diagnostic source hashes, every measurement's order and hashes
for the records, generated table and selected public hardware/toolchain metadata.
Each sample is a fresh process. Repetition order is forward, reverse, then
rotated. Seeds and 16 checked warmup calls are outside both timers; argument
vectors and reply capacity are reused, and the handler sample vector is reserved.
Every timed reply is checked; count commands match the seeded cardinality,
HGET matches bytes, HSET returns zero additions and checks final count/value,
INFO checks bulk framing and its section/version; MEMORY STATS checks its fixed
array prefix, terminal CRLF, key count and scope (not every map value). This is
workload-result validation, not a new general RESP conformance parser.
Seeding is checked as well. Assertions remain enabled in the release probe.

[Profile evidence](profiles/manifest.json) records the same probe's identity and
commands, raw recording digests, sample/loss/throttling counts and hashes for
all exported tables/logs. Published perf exports strip trailing ASCII spaces and
redundant terminal blank lines; original export hashes are retained too.
No data rows, numbers, symbols or warnings are dropped.
Full self/inclusive symbol tables are retained;
inclusive/children percentages overlap and must **not** be added together.
Raw binary recordings are retained in the owner's evidence directory rather than
committed as large ELF/perf build output. The portable text exports are committed
and all captures can be reproduced with the commands below. Raw perf records are
not an acceptance artifact and their local retention is not a public download.

| Whole-process profile | Retained samples | Reported lost samples | Throttle events |
|---|---:|---:|---:|
| HSET | 2,520 | 0 | 2,520 |
| SCARD | 1,470 | 28 | 1,498 |
| HLEN | 651 | 47 | 677 |
| INFO server | 1,168 | 16 | 1,184 |
| MEMORY STATS | 499 | 23 | 522 |

Profiles request 249 Hz `cycles:u` under a kernel maximum of 250 Hz. Actual
sampling is throttled and incomplete where losses are reported. Percentages are
period-weighted user-space self overhead, **not** sample-count percentages,
system CPU, elapsed latency or predicted savings. `perf report --stats` exports
only counters in this installed version; symbols were subsequently exported
from the **same** recordings without that option. Initial stats exports and all
recording warnings remain in the owner's evidence directory. No capture was
rerun to hide losses or obtain a preferred attribution. Instrumented probe timing
is deliberately not used in the component table.

## Reproduce

Linux, the repository-pinned Rust toolchain, Python standard library, `taskset`,
and optional Linux perf; no new package/Rust dependency. Run from the repository
root, with disposable output directories. Profile permissions vary by host.

For a later checkout whose application inputs differ, first create a disposable
worktree at `f33b44c7703f71477fec8755001ce3946c0054ba`, copy this study directory,
`scripts/bottleneck-study.py` and `tests/bottleneck_study.rs` into the corresponding
paths there, and run the commands from that pinned worktree. The collector rejects
changed inputs; do not remove that check to recreate this source-pinned result.

```sh
export PATH=/root/.cargo/bin:$PATH CARGO_BUILD_JOBS=2
cargo build --locked --offline --release --lib
mkdir -p target/bottleneck-frozen
cp target/release/librillcache.rlib target/bottleneck-frozen/
rustc --edition=2024 -O benchmarks/studies/current-bottlenecks/probe.rs \
  --extern rillcache=target/bottleneck-frozen/librillcache.rlib \
  -L dependency=target/release/deps -o target/bottleneck-frozen/probe
python3 scripts/bottleneck-study.py --self-test
cargo test --locked --offline --test bottleneck_study
python3 scripts/bottleneck-study.py \
  --probe target/bottleneck-frozen/probe \
  --library target/bottleneck-frozen/librillcache.rlib \
  --source f33b44c7703f71477fec8755001ce3946c0054ba \
  --out target/new-bottleneck-study
python3 scripts/bottleneck-study.py --check target/new-bottleneck-study
python3 scripts/bottleneck-study.py --check benchmarks/studies/current-bottlenecks/component
```

The collector rejects existing output, changed application inputs/binaries,
invalid replies, absent/duplicate conditions, bad parameters, nonfinite JSON and
unordered percentiles. The offline checker verifies evidence hashes and regenerates
the exact table; it cannot authenticate a recording or prove a supplied binary
was honestly compiled. Build recipe/source/digests are provenance, not signatures.
Do not substitute another library or alter normal application release flags.

For optional separate profiling, choose an **allowed** CPU (0 was used here):

```sh
perf record -F 249 -e cycles:u --call-graph dwarf -o target/hset.data -- \
  taskset -c 0 target/bottleneck-frozen/probe hset 32768 300
perf report --stdio --show-nr-samples --show-total-period --sort symbol \
  --call-graph none --no-children -i target/hset.data
perf report --stdio --stats --header -i target/hset.data
```

Repeat independently for `scard 32768 2000`, `hlen 32768 4000`,
`info-server 32768 1500`, and `memory-stats 32768 3000`; never use these
instrumented elapsed values as the uninstrumented cohort. Consult
[perf record](https://man7.org/linux/man-pages/man1/perf-record.1.html) and
[perf report overhead semantics](https://man7.org/linux/man-pages/man1/perf-report.1.html).

## Bounded INVEST story / EARS acceptance

**S-BD:** As a maintainer, identify reproducible current-source scaling costs
before selecting a safe, independently testable optimization. The deliverable is
the usable diagnostic, validated records and findings—not an unfinished server
feature disguised as a plan.

- **R-BD1:** When the collector succeeds, it shall retain all 84 checked records,
  all 28 conditions, order, source/binary identity and deterministic summaries.
- **R-BD2:** If evidence is incomplete, duplicated, changed, nonfinite or has
  invalid bounds/replies, then validation shall fail rather than report success.
- **R-BD3:** When profiling is reported, instrumentation, setup scope, sample loss,
  throttling and self/inclusive overhead shall remain distinct from elapsed data.
- **R-BD4:** While investigating, the server, safe-Rust/dependency rules and
  protected native gate shall remain unchanged. Hypotheses and unmeasured behavior
  shall not be relabeled as demonstrated gains.

The four Rust probe tests are included in ordinary Cargo/CI through
`tests/bottleneck_study.rs`; the five Python standard-library self-tests cover
matrix/order/hash integrity, malformed numbers and incomplete output.
