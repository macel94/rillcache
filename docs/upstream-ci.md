# Original upstream TCP compatibility and native performance evidence

Two **independent**, read-only workflows use `ubuntu-latest`. They do not replace,
relax or feed historical passes into the existing **Tests and offline CI** and
**Valkey performance gate** protections. The latter still requires all 20 scoped
medians to strictly beat Valkey. These new workflows do not grant full Redis or
Valkey compatibility, production readiness, or a universal performance claim.

## Run and inspect

- `.github/workflows/upstream-compatibility.yml`: every PR and manual dispatch.
- `.github/workflows/upstream-performance.yml`: **manual dispatch only**, separate
  from compatibility. No matrix splits the three-server benchmark across runners.
- Actions UI: select the workflow, **Run workflow**, choose the task branch.
  With an authenticated CLI: `gh workflow run upstream-compatibility.yml --ref BRANCH`
  or `gh workflow run upstream-performance.yml --ref BRANCH`. A new manual-only
  workflow may not be dispatchable before its definition exists on the default
  branch; a 404/422 is not a test result. Do not bypass branch protection to test it.
- Inspect completed runs with `gh run view RUN_ID --log` and
  `gh run download RUN_ID`. Summaries are convenience views; JSON and original
  logs are the evidence. Never claim a pass from a queued/running run.
- Build/suite/performance logs upload on failure with `always()`, retained 30
  days; same-run build bundle retained 7 days. Cancellation/runner loss can still
  prevent final artifact upload. No blanket `continue-on-error`, masked pipeline
  statuses, performance retry-until-green, local daemon or secret is needed.
- Expensive work is Actions-only. Local validation is limited to
  `python3 -m unittest discover -s scripts/upstream -p 'test_*.py' -v` and static
  syntax/policy checks. No local server build, upstream suite or benchmark is
  necessary. Python helpers use only its standard library, not Rust dependencies.

## Pins, build and cache

`ci/upstream/pins.json` records Rust **1.99.1**, Redis **8.10.2**, Valkey **9.1.2**,
archive SHA-256s and tag commit identities. Version/checksum pins are checked
against `benchmarks/targets.mk`; Rust must match `rust-toolchain.toml`. The build
is the existing `rillcache` Cargo binary with default features:
`cargo build --release --locked --offline --bin rillcache`. Cargo.lock is kept;
no workspace/feature/manifest rewrite or extra crate is introduced.

Native core servers, CLIs and Redis's **single pinned `redis-benchmark` executable**
are built from checksum-verified original source (`make -C src -j2 MALLOC=libc
BUILD_TLS=no OPTIMIZATION=-O3 ...`). Optional bundled modules are not built or
loaded. Native build prerequisites are installed explicitly. Allocator, TLS and
optimization flags are identical for both references. This is external test
infrastructure, not a C/C++ dependency or FFI in the Rust server. Native C unit,
module API, allocator/layout and implementation integration suites are not run
against the Rust candidate.

Only source archives are cached, keyed by the complete pin file, and checksums
are revalidated on every use. Extracted/compiled native trees and Cargo target
output are **not reused** across runner images, architectures, compilers,
features, source heads or flags. There are no external Rust dependencies to make
registry caching useful. Compatibility builds once and shares a same-workflow
artifact (never a previous-run or privileged-workflow artifact); performance
builds and measures all three servers in its own single job. ImageOS/ImageVersion,
OS/kernel, CPU/affinity, memory, toolchain/compiler/linker/libc/Tcl versions,
source head and built checkout (including PR merge SHA), commands, options,
archive and executable hashes are retained.

To update pins, independently verify the upstream tag commit and archive hash,
change both pin files together, and verify action tag-to-commit identities with
GitHub's API. Every `uses:` is a full verified commit SHA; action identities are
also recorded in `pins.json`. Preserve old run evidence and disclose changes.
Never reuse compiled native sources with different make flags.

## Compatibility contract and exclusions

Each independent matrix job selects one original upstream Tcl file:
`unit/type/string`, `unit/type/incr`, or `unit/keyspace`, separately for Redis and
Valkey. Matrix fail-fast is disabled. An original native control runs **first**
against that upstream's external TCP server; the identical file, options and
filter selection then run against a fresh Rillcache instance. The endpoints
are loopback TCP, not linked libraries. Native results are saved before candidate
execution and remain available even if the candidate crashes or setup fails.
A failed native control invalidates attribution, but candidate diagnostics still
run; it is not silently retried or treated as candidate conformance evidence.

`ci/upstream/scope.json` is the capability/exclusion manifest. There are no
hand-copied or Rust-reimplemented upstream assertions and no test patches,
protocol proxy, shim server, injected Tcl callbacks or renamed expectations.
All original Tcl/runtest hashes are recorded before build and checked before and
after execution. The original `tests/test_helper.tcl` coordinator and support
files are used unchanged with upstream external-server switches. Tcl manages
its own test client processes; `--clients 1` serializes destructive commands on
the endpoint. Native and candidate never share state or run simultaneously.

Portable scope limits are applied **identically**:

- `--singledb`: no automatic SELECT 9; upstream `singledb:skip` tests are omitted.
- `--ignore-encoding`, `--ignore-digest`: disable upstream encoding/debug-digest
  validations; no object layout or DEBUG digest equivalence claim.
- Denied tags: `needs:debug`, `needs:repl`, `slow`, with reasons in the manifest.
  No individual failing test is silently removed. Unsupported public commands
  reached inside selected tests remain failures.
- External-mode upstream `external:skip` behavior is automatic, not evidence of
  candidate support. Protocol's main block is external-only-inapplicable; it is
  explicitly excluded rather than reporting a zero-test green result. Other
  units, modules, cluster and integration scopes remain untested; every pinned
  source unit receives an inclusion/exclusion reason in `unit-inventory.json`.
- Upstream may conditionally omit tests without emitting events. Thus skip/ignore
  **events** (especially group-level ignores) are not exact skipped assertion
  counts. Native observed names provide an auditable lower-bound universe, not
  a static total inferred from grep. `unreached_native_observed_tests` lists
  names seen in control but not passed/failed/individually skipped on candidate;
  unobserved dynamic tests are explicitly unknown. Full original logs remain.

### Current expected blocker — not a pass

Unchanged external setup itself requires `FLUSHALL`, `FUNCTION FLUSH` and, for
Redis, `SCRIPT FLUSH` and CONFIG operations before test assertions. Rillcache
currently lacks this administrative surface. Diagnostic prerequisite probes
record unsupported commands, **then still invoke the original harness**. A
candidate may therefore be `prerequisite_blocked` with **zero assertions run**.
That is a failed compatibility job, not a skipped test, server assertion, green
supported subset, or reason to alter upstream Tcl or emulate missing commands.

`--durable` lets upstream continue after many assertion/command errors. It does
not guarantee continuation after setup or file-level exceptions. The report
separates upstream test failure events (assertions **or** command exceptions),
harness/setup exceptions, explicit skip/ignore events, timeouts, server exits,
zero-test failures and native-observed unreached names. Any nonzero original exit,
missing prerequisite, incomplete unit or zero executed tests cannot pass.
Wrapper exit 1 means candidate failure; exit 2 means invalid native control or
infrastructure. Raw child exit codes are independently retained in command JSON.

## Manual matched performance contract

The same pinned Redis `redis-benchmark` binary is used unchanged for every server,
not Rillcache's Rust benchmark client and not one client per reference. Fixed
settings in the scope manifest are 16 keepalive RESP2 connections, uniform 1,024
key space, 64-byte string values, pipelines 1 and 16, fixed RNG seed, 30,000 warmup
operations and 300,000 measured operations. PING, SET, GET-hit, GET-miss and INCR
make 10 scoped conditions. Six repetitions rotate order Rillcache/Redis/Valkey,
Redis/Valkey/Rillcache, Valkey/Rillcache/Redis, repeated twice. These fixed repeats
are the experiment, **not retries after failures**.

Each sample starts one new isolated instance and directory, seeds every possible
hit key, validates **all 1,024 values**, warms up, validates again, measures, then
validates again. INCR sums all counters against exact warmup/measured operation
counts; misses intentionally use a separate absent namespace. PING is checked;
all preload SET replies and all sampled generator replies must be successful.
The upstream generator aborts at its first command/connection error and only
prints a successful result after its completion count is reached. CSV must
contain exactly one expected command, finite positive throughput and monotonic
latency percentiles. Unknown stderr, zero output, timeouts, crashes, bad data and
partial samples fail validation. A known unsupported `CONFIG` **metadata probe**
warning is retained separately; it does not hide workload errors. A failed
process has unknown total error count, never zero.

Only **one comparison server at a time** runs in this one job/runner. Server and
client use fixed disjoint allowed CPU affinities; a runner with fewer than two
allowed CPUs is an infrastructure failure. Candidate uses `--threads 1`, native
references `--io-threads 1`. Native background threads/candidate control threads
still exist, so this is an equal CPU-affinity budget, not identical architecture.
All disable persistence, TLS/auth, eviction and clustering; loopback bind,
maxclients and 256 MiB configured data/memory limits are fixed. Candidate's
payload budget is **not** native RSS/maxmemory accounting equivalence; the bounded
small dataset must stay below both. Server CONFIG/debug changes made by Tcl
suites never leak into performance's fresh instances.

Reports include median/min/max throughput and coefficient of variation; medians
of mean/p50/p95/p99 latency summaries; command/connection errors; server process
CPU delta and one-core percentage; post-load RSS and fresh-process lifetime peak
RSS; generator CPU and peak RSS. `/proc` CPU deltas exclude setup/warmup; resource
wall time includes generator startup, CSV parsing and wrapper overhead. Peak RSS
includes preload/warmup and is **not** an isolated measured-window peak. GNU time
accounts generator resources separately. Pipeline latency is the upstream
client's pipeline/request timing, **not independently measured per-command**
latency; medians of percentiles are not pooled percentiles.

An incomplete/invalid condition is retained in tables and invalidates the entire
matched cohort: no ratio/winner is reported for that cohort. All raw samples,
failures, run order and commands persist. Deltas within max(5%, twice either
throughput CV) are explicitly inconclusive; larger differences are descriptive,
not statistical significance. This workflow fails on invalid infrastructure/data,
not on small or negative speed differences. It is exploratory shared-runner
evidence, not a replacement for the protected all-20-case Valkey gate, dedicated
hardware, production workloads, client saturation analysis or full compatibility.

## Lifecycles and failure diagnosis

Each endpoint has a bounded TCP PING readiness loop **and** live-process check.
Socket I/O has deadlines and bounded RESP sizes. Each child command has its own
wall deadline; Tcl also has an upstream no-progress timeout. Process groups are
owned by the harness, terminated in `finally`, waited for, and forcibly killed
only after bounded waits; no system-wide pkill. Temporary storage and ports are
unique; no shared service or concurrently destructive suite uses an endpoint.
SIGINT/SIGTERM runs cleanup where the OS permits. Rillcache's standalone binary
has no portable signal-handler shutdown API; the harness records termination
rather than claiming SIGTERM exercised its in-process joined-shutdown contract.

When a run fails, inspect in order: build prerequisites/checksums and command
status; server startup log/readiness and process status; prerequisite JSON; native
control; candidate Tcl events; unreached names; benchmark warmup/data validation;
raw generator stderr/CSV; affinity/environment and per-sample resource metrics.
Fix genuine infrastructure defects and preserve original failed runs. Candidate
compatibility defects remain red until real server behavior changes; do not add
exclusions, rerun unchanged performance gates until green, or report missing
coverage as support. Workflow changes require configured code-owner review.

## Upstream references

- [Pinned Redis build instructions](https://github.com/redis/redis/blob/498ecd0d6d007db11ddb3aea9428552598a78622/README.md)
- [Pinned Redis Tcl helper](https://github.com/redis/redis/blob/498ecd0d6d007db11ddb3aea9428552598a78622/tests/test_helper.tcl)
- [Pinned Valkey build instructions](https://github.com/valkey-io/valkey/blob/7f1dffedff6de73058b2c2a389422b6ecd56c8fb/README.md)
- [Pinned Valkey test instructions](https://github.com/valkey-io/valkey/blob/7f1dffedff6de73058b2c2a389422b6ecd56c8fb/tests/README.md)
- [Redis benchmark methodology and limitations](https://redis.io/docs/latest/operate/oss_and_stack/management/optimization/benchmarks/)
- [GitHub hosted runner labels/images](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)
