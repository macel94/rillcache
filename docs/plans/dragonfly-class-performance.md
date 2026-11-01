# Toward Dragonfly-class performance

Status: **research plus one implemented comparison gate**. This document separates
the published Dragonfly headline from an equal-setup comparison, records the
current gap, and defines the implementation increments needed to close it. It
does not claim 64-core parity or change the default execution mode.

## What the published numbers actually compare

Dragonfly's README compares against a single-threaded Redis process. Its own
results show the scale effect:

| Instance | Dragonfly | Redis | Rough gap |
| --- | ---: | ---: | ---: |
| `m5.large` (2 vCPU), SET | 173K QPS | 159K QPS | ~1.1x |
| `m5.large`, GET | 191K QPS | 194K QPS | ~equal |
| `m5.xlarge` (4 vCPU), SET | 279K QPS | 190K QPS | ~1.5x |
| `m5.xlarge`, GET | 305K QPS | 220K QPS | ~1.4x |
| `c6gn.16xlarge` (64 vCPU), SET/GET | 3.8M QPS | one Redis process | ~25x |

The 25x figure is therefore mostly **many-core scale-out plus a single-threaded
opponent**, not a 25x per-core engine advantage. Dragonfly also publishes
10M SET / 15M GET QPS at pipeline 30 on that 64-vCPU machine, with a separate
load-generator host and snapshots disabled.

A fairer same-host screen on the throttled development container used the same
four server CPUs, the same 8-client Rust client, 1024 keys and 64-byte values.
Dragonfly v2.0.2 was run with four proactors, `--dbfilename=`,
`--conn_use_incoming_cpu`; Rillcache used its default one-owner path:

| Workload | Pipeline | Dragonfly v2.0.2 | Rillcache default |
| --- | ---: | ---: | ---: |
| SET | 1 | ~5.7K ops/s | ~23.7K ops/s |
| SET | 16 | ~40.7K ops/s | ~98.3K ops/s |
| GET | 1 | ~6.3K ops/s | ~24.8K ops/s |
| GET | 16 | ~86.2K ops/s | ~225K ops/s |

This screen is advisory, not a CI result: the client was co-located, the host was
loaded and Dragonfly was not tuned per instance. It shows that the published
absolute numbers cannot be transferred to a four-CPU comparison, and that
Rillcache is already competitive at the same core count.

## Implemented: same-setup Dragonfly comparison in CI

The required `Valkey 4-CPU performance gate` job now also downloads the official
checksum-pinned Dragonfly `v2.0.2` x86_64 release and runs the **same** five
commands at P1/P16, 8 clients, 1024 keys, 64-byte values, 8,000 warmups and
official ten-second samples. Rillcache must strictly beat Dragonfly in all ten
medians. The check name and required-context inventory remain unchanged, so no
new live ruleset bypass or advisory-only workflow is introduced.

The same change adds a plain-state bare-`PING` fast path in the session boundary.
It retains command accounting, MONITOR delivery, pause handling and SLOWLOG/
latency/commandstats observation, but skips the generic command dispatcher and
connection-control probes when authentication/ACL restrictions, subscriptions,
transactions, blocking and pause are all absent. This targets the only row that
was inside shared-runner noise in the first Dragonfly cohort without weakening
the comparison.

Dragonfly is an optional external benchmark target under the Business Source
License 1.1. It is not a Cargo dependency, vendored source, release-archive file
or container component. CI downloads the official binary over HTTPS, verifies
the pinned SHA-256, executes it only for non-production benchmarking, records the
license notice and hashes with the evidence, and keeps the binary out of tracked
source and artifacts. The license text is in
[`benchmarks/notices/Dragonfly-BSL-1.1.txt`](../../benchmarks/notices/Dragonfly-BSL-1.1.txt).

## Why Rillcache does not use 64 cores today

The protected default is deliberately one owner: data lives on one thread and
commands never leave it. That is currently the fastest verified mode, including
after the deferred owner-batch work in PR #152. The remaining gaps are structural:

1. **Owner-hop latency.** A remote command still costs a cross-thread mailbox
   wake and reply; on the development host that is hundreds of microseconds,
   while a local command is tens of nanoseconds. Batching amortises the hop but
   does not remove its latency.
2. **One socket reactor.** The readiness loop has one thread. More owner threads
   do not help if the socket thread is the serialization point.
3. **One active run per connection.** A pipeline is parsed into one run and the
   next run waits for every reply, even when other owners could accept work.
4. **No cross-client coalescing.** Each connection submits its own owner batch;
   under light pipelines the mailbox sees many small messages.
5. **Hash/data-structure cost.** The default maps use the standard seeded hasher;
   Dragonfly uses a custom sharded table and allocator. Replacing these safely
   requires benchmarked, security-preserving implementations.
6. **Unsafe/low-level engineering.** Dragonfly uses C++, custom allocators and
   io_uring. Rillcache forbids unsafe code, FFI, custom allocators and new
   dependencies outside the reviewed vendored set, so equivalent mechanisms must
   be built safely or avoided.

## Implementation roadmap

Each step must preserve the three protected gates, strict all-20 and all-10
medians, and the default one-owner path until a replacement demonstrably wins on
the same host.

### Phase 1: make multi-owner beat one owner at P16

- Replace the Tokio owner wake with a bounded, fair, low-latency synchronous
  handoff (for example a mutex/condition-variable queue with explicit credit
  accounting, or a reviewed safe queue), measured against the current mailbox.
- Allow more than one admitted run per connection while preserving per-owner
  order and reply order.
- Coalesce same-owner work across ready connections into one owner message, with
  a bounded delay that never exceeds the existing reply-order contract.
- Acceptance: on four server CPUs and independent keys, 2 and 4 owners strictly
  beat 1 owner at P1 and P16 in at least three alternating repetitions; publish
  all rows, including losses. Keep one owner as the default until this passes.

### Phase 2: remove the single socket-thread ceiling

- Evaluate multiple readiness workers with a shared listener or safe
  `SO_REUSEPORT`-style distribution available through the existing Mio surface;
  no new dependency without review.
- Keep connection ownership exclusive, preserve shutdown joining and bounded
  admission, and never split one connection's reply ordering across workers.
- Acceptance: total throughput scales with 1, 2, 4 and 8 socket workers on fixed
  owner counts, with p99 and CPU per command reported.

### Phase 3: reduce per-command data cost

- Benchmark a seeded, collision-resistant fast hasher for the hot key maps
  against the standard hasher on GET/SET/P16; only adopt it if it is both faster
  and resistant to adversarial key sets.
- Consider a resumable RESP parser only if a large-frame workload shows quadratic
  rescans matter; ordinary small commands do not justify the risk.
- Do not add a custom allocator, FFI or unsafe code to chase Dragonfly's C++
  implementation.

### Phase 4: honest many-core evidence

- Run Rillcache and Dragonfly on the same many-core host, separate generator,
  same pipeline depth, value size and disabled persistence, and report per-core
  throughput plus p99.
- Only claim parity or superiority for the measured host, core budget, pipeline
  and command set. Keep the 4-CPU gate as the mandatory merge authority.

## Limits and open decisions

- No 64-core Rillcache evidence exists yet; no claim is made that Phase 1 alone
  matches Dragonfly's total throughput.
- The Dragonfly gate is scoped to the same four logical CPUs and small-string
  SET/GET workloads as the existing Valkey gate. It is not a production SLO or a
  universal compatibility claim.
- The BSL target is non-production-only and pinned to an official release. A
  future license or release change requires a new reviewed pin, not a silent
  download.
- Benchmark-specific server modes are not permitted. All phases must improve the
  same default/native paths users run.
