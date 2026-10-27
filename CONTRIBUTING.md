# Contributing

Start with the [README](README.md), [requirements](docs/requirements.md) and
[benchmark methodology](docs/benchmarks.md). All project code must be AI-generated;
human product direction, review and test-based validation remain required.

## PR-only main and required quality gates

Never commit on or push directly to `main`, including documentation and generated
reports. Work on a branch and open a PR. Merge only after **Tests and offline CI**
and **Valkey performance gate** pass on an up-to-date head: all 20 scoped median
throughputs must strictly exceed Valkey. Ties, slower cases and invalid/incomplete
evidence block merging. No administrator/bot bypass, fastest-sample selection or
threshold relaxation. CI/policy files additionally require code-owner review;
ordinary implementation PRs have no blanket approving-review quota. See the
[enforced policy, exact benchmark scope and automation](docs/pr-quality-gate.md).

## Small, verifiable changes

1. Write an independently useful, negotiable, valuable, estimable, small and
   testable (INVEST) story. Avoid implementing a large command family at once.
2. Add EARS acceptance criteria: **When** an event occurs, the server **shall**
   respond; **If** invalid behavior occurs, **then** the server **shall** handle it.
   Specify binary data, invalid arguments, limits, concurrency and failure cases.
3. Add unit and wire-level tests alongside the implementation. Use ephemeral
   ports and managed server handles; do not use fixed sleeps for readiness.
4. Run `make test` during iteration, then `make ci` before requesting review.
   `make fmt` applies formatting. All Cargo checks/builds use the pinned toolchain,
   lockfile and offline mode. Zero external dependencies applies to every target.
5. Capture the relevant `make bench`/`make compare` evidence. The separate mandatory
   PR gate applies a scoped shared-runner throughput threshold, not a stable-hardware
   SLO or universal win. Preserve uncertainty, CPU/tails and losses; never trade
   away safety or change the acceptance threshold to obtain a passing result.
6. Update compatibility limits and mark only behavior actually verified by tests.

## Autonomous agent completion

For requested parallel work, each agent independently owns a bounded INVEST
story, EARS acceptance tests, its worktree/branch, complete integration and its
own ready-for-review PR. Derive and implement routine design decisions without
asking the user to approve proposals. Do not substitute plan-only PRs or stubs
for requested behavior; documentation-only work is appropriate when requested.

Create the finished PR ready for review and enable protected squash auto-merge
while GitHub checks run. Draft/WIP status requires an explicit user request;
pending CI or an invented manual-review checkpoint is not a draft reason.
Verify the actual ready/auto-merge state, not just successful PR creation. Actual
required checks and configured reviews remain mandatory; report genuine blockers
without claiming completion or bypassing them. See the [delivery contract,
draft root cause, INVEST/EARS examples and stale-PR solution](docs/parallel-pr-delivery.md),
the [parallel work lanes and advisory checker](docs/parallel-work-lanes.md), and
the auto-loaded [project instructions](AGENTS.md).

## Safety is not negotiable

`unsafe_code = "forbid"` applies package-wide, and each crate root also contains
`#![forbid(unsafe_code)]`. No unsafe code, FFI, native extensions, dependency
exceptions, build scripts or alternative allocators. Do not relax/remove the
policy to make an optimization work. Safe APIs of the Rust standard library and
runtime are permitted; their implementation internals are outside this policy.

## Feedback and delivery

`make ci` is identical locally and on GitHub: formatting, all-target Clippy,
unit/integration/policy tests, release build, validated TCP smoke, core benchmarks.
The fast path has no third-party crates, containers, services or secrets to set
up. No target cache is transferred: for this tiny dependency-free project that
would add overhead; revisit only with measured evidence.

GitHub main builds deliver checksummed Linux binaries as short-lived Actions
artifacts. Verify the archive using its adjacent SHA-256 file, extract it, then
run the binary on loopback. This is continuous delivery for testing, not automatic public
production deployment. A separate [container workflow](.github/workflows/container.yml)
tests native amd64/arm64 scratch images and publishes the exact tested archives to
GHCR using only its workflow token. Only its trusted publisher gets `packages: write`;
fresh unprivileged jobs must prove anonymous pull/run. Docker/Kubernetes examples
and the one-time GitHub public-visibility setting are in [container delivery](docs/containers.md).
`make container` / `make container-smoke` are optional: container tooling and network
access remain outside the offline fast path. Full Redis/Valkey/Copilot comparisons
run manually/weekly; a separate native Valkey comparison is mandatory on every PR,
including docs/report-only changes.

When updating Rust or GitHub Actions, verify official upstream releases and update
pins deliberately. Actions are pinned by commit SHA. Normal CI, benchmarking and
inference jobs use read-only repository permissions and do not persist checkout
credentials. The inference job additionally grants `copilot-requests: write` to
its short-lived workflow token; no personal secret is required.
The default-branch benchmark-report publisher and the trusted-main PR branch
updater are the only jobs with repository write permissions. The publisher stages
generated report paths only, pushes an automation branch and opens a PR, never
pushes main or runs inference. The updater uses contents/pull-request/action
permissions only to update opted-in same-repository PR branches and explicitly
dispatch the existing read-only required checks; it never checks out PR code or
merges a PR. The publisher's narrowly scoped pull-request/action permissions create
that PR and dispatch
the two read-only required checks on its head; no new stored token is needed. Fork PRs never receive a privileged
`pull_request_target` execution path. See [automatic reporting](docs/benchmark-reports.md).
Do not commit credentials, benchmark build sources, generated target output or
private environment files.
