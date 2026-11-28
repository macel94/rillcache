# Durability-mode performance profile

**Scope: one development host, matched workload, advisory only.** Every row was
produced by the same command against the same release binary, so the modes are
matched with each other; they are not matched with published benchmarks and are
not an SLO. Reproduce with:

```sh
python3 scripts/durability_profile.py --binary target/release/rillcache \
    --writes 20000 --clients 8
```

Workload: 8 concurrent Python clients, pipeline depth 1, 64-byte values, plain TCP,
one owner worker, no TLS, no eviction pressure, no rewrite in progress. The
generator is a single-CPU Python process and is **the bottleneck for the
`none` row**, so absolute throughput is a ceiling of this harness, not of the
server. Only the *relative* cost of durability is interpreted.

| Durability mode | Writes | req/s | p50 ms | p99 ms | p99.9 ms | Server CPU s | CPU s/write | RSS KiB | Restart to ready s | Key recovered |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| none | 20000 | 3922 | 0.685 | 20.438 | 80.811 | 2.25 | 0.000112 | 18079744 | n/a | n/a |
| always | 20000 | 1883 | 2.183 | 37.768 | 110.138 | 5.74 | 0.000287 | 18640896 | 0.002 | yes |
| everysec | 20000 | 2125 | 2.089 | 36.573 | 109.352 | 5.33 | 0.000267 | 18677760 | 0.001 | yes |
| no-fsync | 20000 | 2163 | 2.038 | 37.731 | 96.264 | 5.51 | 0.000275 | 18591744 | 0.002 | yes |

## What the numbers show

- Turning the journal on costs roughly half the client-bound throughput
  (3922 → 1883–2163 req/s) and roughly **2.4–2.6× the server CPU seconds per
  write**, with p50 rising from 0.69 ms to ~2.1 ms.
- `always`, `everysec` and `no` are within ~15 % of each other in this harness and
  their tails overlap (p99 36–38 ms, p99.9 96–110 ms). This harness cannot resolve
  a difference between them: the generator and the loopback round trip dominate,
  exactly as the gate policy warns. It does **not** support a claim that one policy
  is faster.
- Restart-to-ready after a clean shutdown is milliseconds for every persistent mode
  (0.001–0.002 s) and an acknowledged key is recovered in all three, so the
  recovery *path* works at this size. Large-journal replay time is not measured
  here; `scripts/upgrade_drill.py` and `scripts/restore_drill.py` cover rollback and
  restore instead.
- RSS is flat across modes (18.1–18.7 MiB) at 20k keys, i.e. the journal path adds
  no measurable resident overhead at this size.

## Retained limits

- One host, one generator process, 8 clients, 64-byte values: no controlled
  hardware, no second generation, no TLS, no mixed workload (`P2`, `P5`, `P6`).
- Replication lag, failover time and cluster migration are **not applicable**:
  those topologies are explicit exclusions (`A1`, `A2`), not unimplemented rows.
- Rewrite pause is not measured here; `BGREWRITEAOF` failure behaviour is covered by
  the storage-fault drills, and its duration under a large journal is still open
  (`D10`).
- No accepted SLO follows from this table. It is matched evidence for P4, not a
  capacity plan.
