---
type: workflow
title: Testing and delivery
description: The local feedback loop, test pyramid, protected GitHub checks, benchmark gates and delivery rules Rillcache contributors must preserve.
tags: [testing, ci, benchmarks, delivery, pull-requests, workflow]
verified:
  - by: openwiki/0.7.3
    at: 2026-11-02T15:32:39.597Z
sources:
  - id: openwiki-source-8037e2358a2c4f9b2c722a11
    resource: repo://AGENTS.md
  - id: openwiki-source-f317ee207e1653d2033c81a4
    resource: repo://CONTRIBUTING.md
  - id: openwiki-source-796a6be7cd7592c1844f8094
    resource: repo://docs/actions-gate-audit.md
  - id: openwiki-source-e600869a83d04cf90374b713
    resource: repo://docs/pr-quality-gate.md
  - id: openwiki-source-3989644b876c3ddd6e4b127e
    resource: repo://docs/upstream-ci.md
  - id: openwiki-source-012f2c78e3b1446dfc35803f
    resource: repo://Makefile
  - id: openwiki-source-09a820a7145b3cd5f8ae5126
    resource: repo://scripts/pr_gate_policy.py
generated: { by: "opencode", at: "2026-11-02T15:32:39.597Z" }
---

# Testing and delivery

Rillcache is developed in small increments and delivered through protected pull
requests. The fast local loop is deliberately narrower than the merge gate; a
green focused test is evidence for one change, not permission to merge.

## Local feedback loop

The pinned Rust toolchain, lockfile and vendored crates make the common path
offline. All Cargo commands use `--locked --offline`.

```sh
export PATH=/root/.cargo/bin:$PATH

# During editing: formatting, strict Clippy and one library-test pass.
make dev-check

# After a complete logical change: every worker mode and the TCP suites.
make test

# Before review: the same fast verification used by GitHub, plus release smoke.
make ci
```

The targets have distinct jobs:

| Target | What it runs |
| --- | --- |
| `make fmt` / `fmt-check` | Apply or verify `cargo fmt` |
| `make lint` | All targets/all features Clippy with warnings denied |
| `make test-unit` | One offline library-test pass only |
| `make dev-check` | `fmt-check`, `lint`, `test-unit` |
| `make test` | Full suite with `RILLCACHE_TEST_THREADS` set to 0, 1 and 4 |
| `make test-constrained` | Default TCP suite on one allowed CPU via `taskset` when available |
| `make test-async` | Transport and readiness suites through the opt-in Tokio engine |
| `make check` | `fmt-check`, `lint`, `test`, constrained and async suites |
| `make ci` | `check`, release smoke and core benchmark |

`make test` starts servers on ephemeral ports through managed handles. Tests do
not use fixed sleeps for readiness, and a server is shut down and joined before
the test returns. The three worker modes cover thread-per-client, one fixed worker
and four fixed workers. The async target is separate because Tokio is opt-in.

The release smoke runs 2,000 requests at four clients and pipeline eight, plus
1/4-worker variants. It validates replies, not just process exit. The core bench
is a store-level measurement and does not replace the network gates.

## Test layers

- **Library tests** cover parsing, data structures, expiry, accounting, eviction,
  journal replay, transactions, auth, routing and owner transitions.
- **Wire tests** start a real server and send RESP over TCP. They cover binary
  payloads, fragmentation, pipelining, worker handoff, TLS, blocking, pub/sub,
  shutdown and error dialects.
- **Reference tests** compare a focused subset with Redis or Valkey when an
  external disposable endpoint is supplied. They are optional and do not run in
  the offline fast path.
- **Upstream Tcl tests** run three original Redis/Valkey files against pinned
  binaries in GitHub Actions. They are required PR checks, not a local
  prerequisite.
- **Policy and documentation tests** verify workflow inventory, gate identity,
  crate boundaries, source hygiene and documentation links.

A behavior change needs a test that fails before the implementation and passes
after it. A performance change additionally needs a mechanism, a measurable
prediction, a matched baseline and a rejection condition recorded before the
experiment. Two unsuccessful experiments or 30 minutes without new evidence
trigger a diagnosis review, not more tuning.

## Protected main

`main` is PR-only. Direct pushes, force pushes, deletion and administrator bypass
are not delivery paths. The active GitHub ruleset requires:

- a pull request with squash merging only;
- review-thread resolution and configured code-owner review;
- strict up-to-date required checks on the exact head;
- the GitHub Actions integration as the producer of every required context;
- no bypass actors in a full operator audit.

The repository currently has **14 app-pinned required contexts** across six
PR-triggered workflows:

| Workflow | Required context(s) |
| --- | --- |
| `ci.yml` | `Tests and offline CI` |
| `pr-quality.yml` | `Valkey performance gate` |
| `pr-multicpu-quality.yml` | `Valkey 4-CPU performance gate` |
| `container.yml` | `verify`, native amd64 build, native arm64 build |
| `upstream-compatibility.yml` | pinned tool build plus six Redis/Valkey Tcl matrix checks |
| `pr-branch-updater.yml` | `read-only-self-tests` |

The machine-readable inventory is `.github/pr-workflow-gates.json`. A tracked
JSON file is not proof of live enforcement, so operators run:

```sh
python3 scripts/pr_gate_policy.py
python3 scripts/pr_gate_policy.py --live
```

`--live` performs bounded read-only GitHub queries and checks the actual ruleset,
effective branch rules, app identity, strict freshness, active required workflows
and visible bypass actors. In read-only automation, GitHub may redact the bypass
list; the checker reports that as `UNVERIFIED`, never as a clean empty list.

## Performance gates

Both Valkey gates compare the exact PR head against a pinned Valkey build. They do
not accept ties, averages, historical passes, path exemptions or a successful
no-op.

**Original gate — 20 comparisons.** Five workloads (PING, SET overwrite, GET hit,
NX-existing rejection, XX-existing overwrite), two CPU profiles (inherited and
one logical CPU for the server), pipelines 1 and 16. The run uses 8 clients, 1,024
keys, 64-byte values, 1,000 warmup commands per client and three alternating
repetitions. Every one of the 20 medians must be strictly higher than Valkey.

**Official four-CPU gate — 10 comparisons.** The same five workloads at pipelines
1 and 16 run with the official `valkey-benchmark` binary, eight connections, two
generator threads, 1,024 random keys, 64-byte values, 8,000 warmups, ten-second
samples, three repetitions and a fixed seed. Both servers receive the same four
allowed logical CPUs. All ten medians must strictly beat Valkey. The same required
job also compares the frozen Rillcache binary with Dragonfly on the identical
traffic and requires ten more strict wins.

Both gates validate raw files, argument/timer identity, binary hashes, source
identity, reply/data integrity and complete cohorts before applying the threshold.
A completed loss is valid evidence but fails. An incomplete cohort fails. The
four-CPU check is not replaced by the original gate, and neither gate is proof of
realistic mixed-workload behavior or a production SLO.

## Upstream Tcl compatibility

The upstream workflow builds pinned tools from checksum-verified archives, then
runs the original `unit/type/string`, `unit/type/incr` and `unit/keyspace` files
for Redis and Valkey. A native control runs first. Candidate failures, native
control failures and setup blockers are real failures; zero assertions executed
is not a pass. Tests requiring debug, replication or the slow tag are excluded
with recorded reasons. All other matrix members are live-required.

The harness does not patch upstream Tcl, proxy protocol frames or emulate missing
feedback. A prerequisite probe records support, but the original file still runs.
Partial or unobserved results remain visible in the JSON and logs.

## Delivery rules

1. Work on a branch and open a pull request; never commit to `main`.
2. Keep one task, one branch and one PR. Declare the affected lane in the PR body.
3. Implement complete behavior, wiring and tests; do not substitute a plan, stub
   or sibling-dependent patch for a requested feature.
4. Preserve failed benchmark evidence. Do not remove a losing workload, lower a
   threshold, select the best sample or rerun an unchanged failed candidate until
   it passes.
5. Do not self-approve, use administrator bypass or disable a gate.
6. Open finished PRs ready for review. Pending CI is not a reason to hide a
   finished change as a draft.
7. If protected auto-merge is requested, pin it to the reviewed head with
   `gh pr merge <PR> --auto --squash --match-head-commit <SHA>` and verify the
   request or merged state.
8. Required reviews, failed checks and conflicts remain visible blockers. Fix
   them in the owning branch; do not hand routine decisions back as a proposal.

CI, policy, pin and safety files require the configured code-owner review. A
workflow change is active only after its protected PR merges to `main`; an
unmerged workflow file is not deployed behavior.

## Evidence limits

Shared GitHub runners are not controlled hardware. A three-sample median is the
owner's merge policy, not statistical proof of superiority. The original gate
measures small strings and cyclic keys; the four-CPU gate uses a fixed official
generator. Neither covers persistence, TLS, eviction, large values, mixed TTLs,
client saturation or long-running failure modes. Those limits belong in the
report, not in an optimistic claim.

The full compatibility boundary is in
[compatibility and production gaps](../boundaries/compatibility-and-production-gaps.md).
The command surface is described in
[data types and commands](../features/data-types-and-commands.md).
