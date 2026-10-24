# Profiled GitHub CI evidence: 10d56ec

Unmodified artifacts downloaded from:

- [CI 40114110305](https://github.com/macel94/rillcache/actions/runs/40114110305): `smoke/`.
- [Comparisons 40114117073](https://github.com/macel94/rillcache/actions/runs/40114117073): four comparison directories and `reference-binaries.sha256`.

Tested source: `10d56eca165f14a2b5fb043e36d2c33cc52ed762`. The subsequent
review/evidence commit changes documentation/data only, not the tested Rust code
or workflow. Both workflows succeeded: **59 fast tests**, eight reference
conformance invocations, **180 active records and 12 idle records**. No binaries
or credentials are included here.

All records were independently parsed and checked for exact workload/batch
counts, ordered percentiles, CPU tick/time/percentage consistency, positive RSS,
matching PID/start-time identity and observation windows bracketing request timers.
All endpoint thread sets were unchanged: active samples matched nine Rillcache,
six Redis or seven Valkey threads. This is matched-thread accounting, not proof
that no intermediate thread could have been created and destroyed.

The delivered release archive's checksum was verified. Its benchmark executable
passed all five profiled workloads, and its server passed independent binary-safe
NX/XX, syntax-error preservation and QUIT/pipeline checks. The delivered server
SHA-256 matches the b654fa6 CI server exactly; no server implementation changed.

## Parameters and provenance

- AMD EPYC 7763, four logical CPUs, Rust 1.99.1; exact metadata in each profile.
- Five workloads: PING, SET, GET, NX-existing rejection and XX-existing overwrite.
- Eight clients, 1024 cyclic keys, 64-byte values, 1000 warmup commands/client,
  three repetitions. Pipeline 1: 100,000 commands; pipeline 16: 1,000,000.
- Single-core profiles: server CPU 0, clients CPUs 1–3. Other profiles inherit
  affinity. Order rotates, and **only one comparison server is alive at a time**.
- Optional Linux procfs accounting is enabled, with one second of idle observation
  before each server's benchmark connections. Ordinary fast-CI smoke is unprofiled.
- Reference versions remain pinned Redis 8.10.2 and Valkey 9.1.2, with persistence
  disabled and default I/O-thread settings. Logs show exactly one link per server;
  binary digests remained identical across all four profiles.

The checksum file preserves original runner paths. It is executable provenance,
not a source lockfile, signed-commit verification or an upstream signature claim.

Read [the review](../../../docs/ci-benchmark-review.md) and
[accounting methodology](../../../docs/benchmarks.md#optional-linux-server-accounting)
before drawing conclusions. CPU counters are tick-quantized, RSS is approximate,
VmHWM is a lifetime peak, and context-switch counts cover matched thread lifetimes.
One-second idle samples cannot prove zero CPU cost. Shared-VM and feature-scope
limitations remain; no general Redis/Valkey performance victory is established.
