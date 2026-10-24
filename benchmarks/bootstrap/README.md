# Bootstrap harness-validation evidence

These raw runs establish that benchmarks exist from the first implementation.
They are **not a controlled performance baseline or evidence that the project has
achieved its Redis/Valkey performance objective**.

## Captured runs

- Redis 8.10.2 and Valkey 9.1.2: version/checksum pins in `../targets.mk`.
- Safe, dependency-free Rillcache 0.1.0 and the same Rust TCP client for all servers.
- Separate native server processes; no RDB/AOF; upstream default optimizations,
  allocator and I/O-thread configuration for competitors; loopback TCP.
- 10,000 measured requests per workload, 8 clients, 1024 cyclic keys, 64-byte
  values, 100 untimed warmup commands/client, 3 runs each of PING, SET and GET.
- Both pipeline 1 and pipeline 16. Each directory includes the three unmodified
  result JSON files and its environment/version/configuration/binary-checksum log.
- The supported-subset wire-conformance test passed against both reference
  servers before each comparison. Every measured response was validated, all
  counts and JSON percentiles were checked, and process cleanup was verified.

## Interpretation limits

The shared VM was noisy, without isolated CPU affinity, normalized CPU budgets or
controlled frequency. Server order was fixed. Rillcache's limited cache-only
feature set is not Redis/Valkey's full functionality. Some first-attempt longer
runs were interrupted and **are not included**. The shorter completed runs here
validate the comparison tooling; throughput ratios must not be promoted as a
performance win or used as CI thresholds.

See [methodology and next measurements](../../docs/benchmarks.md) for the work
required before a defensible performance claim.

## Source identity

Measurements were collected from the bootstrap working tree before its first
implementation commit. The environment files accurately show the preceding
README-only commit and dirty/untracked files; this was not a clean release.
`source.sha256` identifies the exact Rust, Cargo, toolchain, Make and target-pin
files measured, preserved in bootstrap commit `ed15e2f`. From that revision's
repository root (not a later feature revision), verify them with:

```sh
sha256sum --check benchmarks/bootstrap/source.sha256
```

Upstream source-archive builds have no upstream Git checkout. Their built-in
`sha=00000000`/dirty fields are not provenance; use the pinned archive SHA-256 and
recorded binary checksums. Checksums are not digital signatures.

Reproduce the workload (new results will reflect the new host/load):

```sh
make compare BENCH_ARGS='--requests 10000 --clients 8 --pipeline 1 --keyspace 1024 --value-size 64 --warmup 100 --runs 3'
make compare BENCH_ARGS='--requests 10000 --clients 8 --pipeline 16 --keyspace 1024 --value-size 64 --warmup 100 --runs 3'
```

Each command overwrites `target/benchmarks/comparison/`; copy it between runs to
preserve both, as the GitHub comparison workflow does.
