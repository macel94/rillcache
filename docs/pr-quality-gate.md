# Main is PR-only: tests and Valkey performance

All future changes to `main`, including generated benchmark reports and docs,
must come through a pull request. No direct commits/pushes, administrator bypass,
force pushes or branch deletion. The active
[GitHub ruleset](https://github.com/macel94/rillcache/rules/25863639) is recorded in
[`.github/main-ruleset.json`](../.github/main-ruleset.json). A JSON file alone does
not enforce GitHub settings; the ruleset must remain active with no bypass actors.
Administrators can edit repository settings; doing so to land a failing PR is not
an accepted delivery path.

## Required checks

- **Tests and offline CI**: the existing `make ci`, including formatting, strict
  all-target Clippy, dependency/safety tests, all three execution modes, constrained
  default TCP tests and reply-validated release smoke/core benchmarks.
- **Valkey performance gate**: the PR workflow must complete and Rillcache must
  have **strictly higher median throughput than pinned Valkey in all 20 cases**.
  Ties and even one slower median fail. No averaged score, historical-result reuse,
  path exemption, successful no-op or manual override qualifies as a pass.
- **Valkey 4-CPU performance gate**: a separate workflow runs on every main PR
  using the same unchanged official `valkey-benchmark` executable for both servers.
  Both servers receive exactly four logical CPUs; all 10 workload/pipeline medians
  must strictly beat Valkey. Fewer than four allowed CPUs fails, never skips.
  This context is now live-required; its earlier omission allowed PR #131 to merge
  while it failed. See the [verified audit and correction](actions-gate-audit.md).
- **Container verification and both native builds**: `verify`,
  `build (amd64, ubuntu-24.04)` and `build (arm64, ubuntu-24.04-arm)` must all pass.
- **Original Tcl compatibility**: the pinned tool build and every Redis/Valkey
  string/incr/keyspace matrix check must pass; native-control/setup failures count.
- **PR policy self-tests**: `read-only-self-tests` must pass. The audit branch adds
  actual live-rule verification here; that source rollout requires its protected PR.

That is 14 app-pinned contexts across six PR-triggered workflows. The complete
[workflow/job inventory](../.github/pr-workflow-gates.json) accounts for every
repository workflow and explicitly excludes only non-PR publication jobs and
manual/scheduled studies. `python3 scripts/pr_gate_policy.py --live` verifies
actual effective main protection; a tracked JSON file is never proof by itself.
Full operator mode requires visible bypass actors. Actions uses explicit
`--read-only-view` because GitHub hides that field without ruleset-write access;
redaction is reported as UNVERIFIED, never as an empty/no-bypass list. All fourteen
contexts and the other readable controls remain mandatory in that mode.

Required contexts are pinned to the GitHub Actions integration, not arbitrary
commit-status writers. Test CI has a unique check name to avoid collision with
container workflow jobs. Branches must include the latest `main`; changed heads
need fresh checks. Only squash merging is allowed and review threads must be
resolved. CI, benchmark policy/tooling, pins and safety/build settings are owned
by `@macel94` through CODEOWNERS and require owner review when changed. Ordinary
implementation PRs have no blanket approving-review quota. GitHub does not allow
an author to approve their own PR; policy changes need an eligible owner review.
CODEOWNERS is evaluated from the PR's base branch.

## Exact performance acceptance scope

| Dimension | Required setting |
| --- | --- |
| Workloads | PING, SET overwrite, GET hit, NX-existing rejection, XX-existing overwrite |
| Server modes | Default plain-TCP Rillcache; default Valkey, no persistence/TLS |
| CPU profiles | Inherited affinity; equal one-logical-CPU servers with clients on other CPUs |
| Pipelines | 1 and 16 |
| Requests/workload/repetition | 100,000 at pipeline 1; 1,000,000 at pipeline 16 |
| Clients/keyspace/value | 8 clients, 1024 cyclic keys, 64-byte values |
| Warmup/repetitions | 1000 commands/client; 3 alternating native server repetitions |
| Gate | Rillcache median cmd/s > Valkey median cmd/s in every workload/profile/pipeline |

The original gate has 20 comparison rows, 120 validated native records and 24 raw
JSON files. It is unchanged by the additional four-CPU gate below.
Only one owned server is alive at a time; readiness is reply-validated and cleanup
waits/reaps on errors/timeouts. The same frozen base-branch Rust client validates
every measured reply. The protected base also supplies the harness, Valkey source
version/archive checksum and compiler. Candidate builds use normal release flags;
there is no benchmark-specific server mode. The installing PR alone may bootstrap
the new harness against the explicitly pinned pre-policy base `19f61b0`; subsequent
PRs use the base branch's existing harness, never a permissive missing-file fallback.
A PR proposing a policy/pin change is therefore measured with the previous policy.

Missing/corrupt/duplicate/nonfinite data, mismatched parameters/timers/CPU or PID
identity, changed binary/evidence hashes, process/client errors and incomplete
cohorts cannot pass. The check independently reloads all raw files and regenerates
the table before applying the threshold. Complete slower cohorts remain valid
numeric evidence but have `gate_passed: false`; incomplete cohorts remain
`complete: false`. Both fail the required job.

The Actions summary and 30-day artifact retain every median, min/max spread,
whole-batch p99, sampled CPU cost, hashes, source identities, order and environment.
p99 is **whole pipeline-batch completion**, not per-request latency. CPU observation
windows bracket request timers; matched-thread accounting and quantization limits
remain in the raw records. Inherited Rillcache can use more than one CPU.

This is the owner's explicit hard **merge policy on shared CI**, not a claim that
shared runners are stable hardware or that a three-sample median proves statistical
superiority. Noise can block a PR. Investigate failures; do not remove a losing
workload, lower the threshold, choose the fastest sample or rerun until a preferred
answer appears. The suite is RESP2, small strings and cyclic keys, not realistic
mixed traffic, complete compatibility or a production SLO. Batch tails and CPU are
reported, not additional throughput gate criteria.

## Independent official-Valkey four-CPU gate

The [additional PR workflow](../.github/workflows/pr-multicpu-quality.yml) builds
`valkey-server` and **`valkey-benchmark` from the same checksum-verified source**
using protected-base pins/build tooling. The benchmark is unmodified: custom
command mode is an [official supported interface](https://valkey.io/topics/benchmark/),
also exercised in Valkey's `tests/integration/valkey-benchmark.tcl`. No Rust load
generator, rewritten upstream assertion, or custom timed loop is used here.

Five commands (PING, SET overwrite, GET hit, NX-existing rejection, XX-existing
overwrite) at P=1 and P=16 give **10 strict median comparisons and 60 fresh native
server samples**. Each sample has 8000 warmup commands, 8 connections, 2 generator
threads, 1024 random keys, 64-byte values and seed 42. Every measured sample uses
the official **`--duration 10`** mode (not `-n`); three repetitions alternate server
order. The wrapper requires at least ten seconds of observed generator wall time. Default server modes,
loopback TCP and persistence/TLS-off are retained. There is no aggregate-score
escape, tie pass, retry-until-green or substitution for the original 20-case gate.

Both servers are pinned to the same first four allowed logical CPUs; the generator
uses remaining CPUs when available, otherwise shares those four. Affinity is
checked before/after load; exact placement and overlap are recorded. These are
logical CPU budgets, not proof of four physical cores, server worker counts or
client-independent saturation. The wrapper checks every possible hit key before
and after warmup/load and probes conditional SET semantics. The official generator
rejects workload error replies but does **not** validate every non-error payload;
this is not the original Rust client's every-reply contract. Unknown stderr,
nonzero exits, timeouts, malformed/nonfinite CSV and incomplete cohorts fail.

The protected base supplies the orchestration wrapper and shared process/CSV
helpers, not candidate replacements. Only installation against exact protected
base `d94bd4e97906e517c30d056ddd2be24c0714f315` may bootstrap the new wrapper;
future missing base wrappers fail closed. The same official generator binary is
hashed and used for every sample. `--check` independently reloads raw CSV, validates
exact argv/CPU/source/process identity, statuses, hashes, dataset checks and the
regenerated table. Complete losses remain complete numeric evidence but fail.
The initial request-count experiment on head `d429e62` is retained in Actions run
[`40658513184`](https://github.com/macel94/rillcache/actions/runs/40658513184): it
completed with **9/10**, including a failing PING/P=1 median tie at 133,155.80 req/s.
No retry or pass is claimed for that head. Pinned Valkey `benchmarkSequence()`
includes thread shutdown in elapsed time; `showThroughput()` polls shutdown every
250 ms, quantizing the sub-second count-based samples into large throughput steps.
The official duration mode is exercised by its unchanged integration tests; fixed
ten-second measurements span 40 polling ticks rather than only 3–4. Residual timer
quantization/shared-runner limits remain. Schema 2 additionally validates duration
and rejects schema-1 count-based cohorts, preventing relabeling of failed evidence.
This is a measurement-method correction, not a weaker threshold: ties/losses still
fail, every workload remains, and the original 20-case gate is unchanged.

Original stdout/stderr, command/server/sample JSON, resource observations, source
and binary hashes, environment and summaries survive failures in 30-day artifacts.
Offline validation: `python3 scripts/test-multicpu-performance.py`. Actual comparative
performance remains Actions-only; mocked/replayed tests are not performance wins.

## Automated report PRs

The manual/weekly comparison and genuine Copilot review retain their separate
read-only measurement/inference jobs. Only the trusted default-branch publication
job has `contents: write`, `pull-requests: write` and `actions: write`. It:

1. Validates immutable report identity/order using the existing Rust publisher.
2. Creates `automation/benchmark-report-<run>-<attempt>` from current `main`.
3. Commits only generated dashboard/history paths and opens a PR, or reuses an
   existing PR/no-ops an already-published report. It never pushes `main`.
4. Observes exact-head CI, both Valkey gates and original Tcl runs, dispatching only missing
   read-only workflows after checking their definitions against protected main.
   Approval-held events are not test evidence. Any real exact-head failure stops
   publication without retries, including an earlier failure followed by success.
5. Requests protected squash auto-merge only on its newly created ready PR, with
   `--match-head-commit`, and verifies the request or normal merged state. Existing
   disabled merge intent is preserved on retries. No automatic bypass or exemption
   applies to report-only PRs; all required checks above apply equally.
   Stale report branches must be updated/rechecked; immutable history is retained.

Repository Actions defaults remain read-only. GitHub's setting permitting Actions
to create PRs is enabled. There is no new PAT, secret, privileged PR execution,
`pull_request_target` workflow, or Copilot token in the PR benchmark process.
Reports remain immediately readable in the review summary/artifact while their
PR waits for checks/review/merge. New `main` commits trigger normal release/image
workflows when merged through the ordinary user-authorized PR path.

## Verification and maintenance

Offline optional tooling checks (Python standard library only):

```sh
python3 scripts/pr-performance-gate.py --self-test
python3 scripts/test-pr-policy.py
python3 -m unittest scripts.test_pr_gate_policy -v
python3 scripts/pr_gate_policy.py --live
python3 scripts/test-multicpu-performance.py
python3 scripts/pr-performance-gate.py --check /path/to/original-gate
python3 scripts/pr-multicpu-performance.py --check /path/to/four-cpu-gate
```

The first suite includes archived real-cohort replay, single loss/tie rejection,
median-vs-best-sample, missing/duplicate/nonfinite samples, checksum/outcome/table
corruption and owned-process cleanup tests. Publisher tests mock Git/gh and reject
main commits/pushes, invalid context, hidden dispatch failure and duplicate/no-op
publication errors. Rust policy tests keep workflow boundaries in ordinary CI.
Python/native Valkey builds remain outside Cargo and the ordinary offline fast path.

To audit live enforcement, read `repos/macel94/rillcache/rulesets/25863639` and
`repos/macel94/rillcache/rules/branches/main` with `gh api`, and compare with the
tracked policy. A policy change requires an explicit reviewed PR and corresponding
verified GitHub configuration update, not merely editing JSON. Never temporarily
turn off enforcement to merge its own implementation.

Sources: [available GitHub rules](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/available-rules-for-rulesets),
[ruleset REST API](https://docs.github.com/en/rest/repos/rules),
[workflow-token events](https://docs.github.com/en/actions/how-tos/writing-workflows/choosing-when-your-workflow-runs/triggering-a-workflow),
[workflow dispatch API](https://docs.github.com/en/rest/actions/workflows#create-a-workflow-dispatch-event),
[CODEOWNERS](https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/customizing-your-repository/about-code-owners).
