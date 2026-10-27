## Story and requirements

Story ID and EARS acceptance criteria; describe only the increment delivered.

## Lane and shared surfaces

- Lane: `streams` / `collections` / `strings-keyspace` / `security-clients` /
  `persistence` / `observability-admin` / `pubsub` / `io-server` / `surface` /
  `tooling` (see [the lane map](../docs/parallel-work-lanes.md))
- Shared-surface files touched: none / list them and explain why
- [ ] `python3 scripts/lane_check.py` reviewed; accidental shared-surface spill
      was removed or moved to a surface-lane increment

## Evidence

- [ ] All project code was AI-generated and is ready for human validation.
- [ ] No unsafe code, FFI, build scripts or external dependencies were introduced.
- [ ] Binary/empty/missing values, invalid input, limits and concurrency are tested.
- [ ] `make ci` passes with the pinned toolchain and lockfile.
- [ ] Required **Tests and offline CI** and **Valkey performance gate** pass on the current, up-to-date PR head.
- [ ] All 20 scoped median throughputs strictly beat Valkey; ties/losses/incomplete evidence are not waived.
- [ ] This change enters main only through a protected squash-merged PR, without admin bypass.
- [ ] Relevant benchmark evidence is attached, with parameters and environment.
- [ ] Compatibility boundaries and deferred behavior are documented accurately.

## Benchmark / behavior observations

Include raw evidence; do not claim that Rust alone makes this faster than Redis
or Valkey. Note untested behavior and measurement limitations.
