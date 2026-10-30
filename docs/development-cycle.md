# Bounded development and performance experiments

## S-DEVLOOP (small, independently useful tooling story)

As a contributor, I can reject an incorrect or unhelpful experiment cheaply,
without rebuilding in a second worktree or substituting focused tests for the
protected delivery checks. This changes development tooling, not runtime policy
or the definition of a passing performance gate.

- **R-DEV1:** When `make dev-check` runs, it shall check formatting, all-target/
  all-feature Clippy and one offline library-test pass. Full `check`/`ci`, all
  three worker-mode passes and constrained tests shall remain unchanged.
- **R-DEV2:** When screening is requested, it shall require distinct exact source
  SHAs and a preselected hardware/software fingerprint before expensive builds.
  Missing/mismatched hardware shall fail, not silently use another host.
- **R-DEV3:** When a normal study completes, it shall retain three Latin-rotated
  repetitions of baseline/candidate/Valkey at P1 and P16, fresh distinct Rust
  binaries, official reference identity and all raw evidence. Profiles shall be
  opt-in and excluded from normal throughput claims.
- **R-DEV4:** If a cached reference binary/identity changes, stale application
  compilation occurs, evidence is incomplete/corrupt or a process fails, the
  study shall fail and retain evidence. No automatic cleanup or retry-to-green.

## Editing, screening, promotion

Use the same existing non-main worktree for serial edits; retain its build cache.
Do not create another worktree just to rebuild an identical source tree. Parallel
agents still require their own independent worktrees/branches and target caches.
Never delete existing worktrees, branches, logs, binaries or unpushed work as
an automatic optimization.

```sh
export PATH=/root/.cargo/bin:$PATH
# Substitute a real test filter/target; confirm the reported matching-test count.
cargo test --locked --offline --lib server::handoff
cargo test --locked --offline --test server
make dev-check
# After a complete logical change; make ci includes this, so do not duplicate it
# immediately before a ready-candidate make ci run.
make test
# Before requesting review: unchanged full delivery validation.
make ci
```

Use the pinned Rust toolchain. A focused pass is not full validation. Runtime
candidates additionally need the repository's release and actual-four-CPU/mode
regressions. Develop directly on the existing owning branch when requested;
do not rebuild identical source in a second worktree just for integration.
Never reuse old protected passes for a changed head.

Before each performance change record its mechanism, predicted effect, matched
baseline, controls and rejection condition. Two unsuccessful experiments or 30
minutes without new evidence trigger diagnosis review before another candidate.
Fixing a separate correctness bug is valuable but is not a throughput improvement.

Keep **one** canonical checkpoint (about 300–600 words) and one append-only JSONL
experiment ledger outside tracked source, linking raw evidence and historical
failures. Keep old checkpoints as historical, not competing current state.

## Fixed-host advisory screening

`scripts/dev-performance.py` reuses the tested owned-process diagnostic. It is
Actions-only, manual-only and requires an isolated self-hosted Linux x64 runner
with label `rillcache-perf`. **No such runner was configured during this rollout.**
The source implementation is not a claim that the workflow or a dedicated host
is deployed. Do not register a credential-bearing development machine as a public
repository runner; use an ephemeral/resettable isolated host without user tokens,
SSH keys or access to private networks. Provisioning remains an external blocker.

Prepare the host with the pinned Rust toolchain, Python 3.12+, GCC/Make, curl,
util-linux/taskset, Tcl and standard Ubuntu native build prerequisites. Perf is
needed only for optional profiles; no host sysctl/security/network/SMT tuning is
permitted. Choose host hardware independently of winning results.

On that host, from the repository with the workflow's build environment:

```sh
CARGO_BUILD_JOBS=2 python3 scripts/dev_hardware.py
```

Record the fingerprint **before** candidate measurements. It includes CPU model,
topology/allowed CPUs, kernel/OS, Rust/native toolchains and build environment.
It does not prove physical-host dedication, idle neighbors, constant frequency
or statistical superiority. Hardware changes require a deliberate new contract,
not automated acceptance of whichever runner wins.

Use the existing `upstream-performance.yml` manual workflow with `study=true`,
`study_base=<40-character SHA>`, `study_candidate=<40-character SHA>` and
`study_hardware_sha256=<preselected fingerprint>`. The default remains the
existing hosted three-server comparison; no required workflow is changed. An
unmerged workflow change is not active on main. Only enable `study_profiles`
when there is a specific unresolved profiling question.

Screening builds both Rust revisions at one absolute temporary source path,
package-cleans release artifacts before each build, requires real compilation,
and freezes distinct binaries. Unchanged dependencies remain cached within the
build. Source-file deletions are explicitly rejected to prevent archive-overlay
contamination; do not hide that unsupported case. Reference caching verifies
source checksum, hardware/build identity and both binary hashes. It never caches
performance results or substitutes an old application binary.

Normal scope: PING, P1/P16, 8 clients, 2 generator threads, 8000 warmups,
10-second official-generator loads, 3 rotating repetitions, default servers,
shared four logical CPUs and one owned server at a time. Every runtime child
receives only PATH/LANG/LC_ALL; failures and joined cleanup are recorded. Optional
P1 profiles use separate 20-second loads/8-second captures and 2-second idle
captures. Instrumented CPU/syscalls do not prove normal throughput, energy or
per-request tails. There is no native profiling on the agent host.

Artifacts include frozen binaries, compiler/source/reference identity, raw CSV,
commands, resources and process lifecycle. Revalidate a downloaded artifact:

```sh
python3 scripts/dev-performance.py --check /path/to/dev-performance
python3 -m unittest discover -s scripts/upstream -p 'test_*.py' -v
```

Subset wins are advisory, not extra prerequisites or acceptance passes. A ready
runtime candidate still needs **all 14 current-head protected checks**, all
**20 original** and **10 official four-CPU** medians strictly above Valkey, a
current base and configured code-owner review. Preserve every loss; do not push
tooling-only changes to obtain another run of an unchanged failed runtime.
