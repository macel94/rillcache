# Corrected GitHub CI evidence: b654fa6

Unmodified result files downloaded from:

- [CI 40103835947](https://github.com/macel94/rillcache/actions/runs/40103835947): `smoke/`.
- [Comparisons 40103838600](https://github.com/macel94/rillcache/actions/runs/40103838600): four comparison directories and `reference-binaries.sha256`.

Tested source: `b654fa651d5eb851acc0928e5a5037b13ca87205`. The subsequent evidence/
review commit changes documentation/data only, not the tested Rust implementation
or workflow. Both workflows succeeded; 47 fast tests, eight reference checks and
all 180 external result records were validated. No binaries or credentials are
included here.

Each comparison includes three runs of PING, SET, GET, SET NX against existing
keys (rejected writes), and SET XX against existing keys (admitted overwrites).
All use 8 clients, 1024 cyclic keys, 64-byte values and 1000 warmup commands/client.
Pipeline 1 uses 100,000 measured commands/workload; pipeline 16 uses 1,000,000.
The single-core profiles pin every server to logical CPU 0 and clients to 1–3;
other profiles inherit the runner's allowed CPUs. Server order rotates.

All four profiles used exactly the same reference executables. The saved checksum
file records those original runner paths; it is not a source lockfile or a claim
of signature verification. Environment files include source/compiler/hardware,
requested and actual affinities, ordering, versions and executable digests.

Read [the review](../../../docs/ci-benchmark-review.md) before drawing conclusions.
The one-CPU profiles expose weaker unpipelined throughput and tail latency despite
pipelined-throughput strengths. This is a limited, shared-VM workload, not a
production SLA or a proof of a general Redis/Valkey performance win.
