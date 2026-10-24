# Contributing

Start with the [README](README.md), [requirements](docs/requirements.md) and
[benchmark methodology](docs/benchmarks.md). All project code must be AI-generated;
human product direction, review and test-based validation remain required.

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
5. Capture the relevant `make bench`/`make compare` evidence. Do not introduce
   shared-runner timing assertions or trade away safety for benchmark numbers.
6. Update compatibility limits and mark only behavior actually verified by tests.

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
production deployment. Heavier reference builds/comparisons run manually/weekly.

When updating Rust or GitHub Actions, verify official upstream releases and update
pins deliberately. Actions are pinned by commit SHA. Normal CI, benchmarking and
inference jobs use read-only repository permissions and do not persist checkout
credentials. The inference job additionally grants `copilot-requests: write` to
its short-lived workflow token; no personal secret is required.
Only the separate default-branch benchmark-report publication job has contents
write permission and persisted credentials; it stages generated report paths only
and never runs inference. Fork PRs never receive a privileged
`pull_request_target` execution path. See [automatic reporting](docs/benchmark-reports.md).
Do not commit credentials, benchmark build sources, generated target output or
private environment files.
