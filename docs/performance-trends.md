# Repeated-round performance trend

**Scope: one development host, advisory, not an SLO.** The merge gates use three
short repetitions per condition, which cannot show a trend. This harness runs many
rounds back to back against one long-lived server and fits a least-squares slope per
round, so a warm-up, a drift or a leak becomes visible instead of being hidden in a
single median.

```sh
python3 scripts/performance_trend.py --binary target/release/rillcache \
    --rounds 10 --seconds 45
```

Workload: 8 clients, pipeline 1, 64-byte values, a 512-key working set, one `SET`
per five operations, plain TCP, one owner worker. The generator is a single Python
process, so absolute numbers are a harness ceiling.

## Measured run (450 s total, 10 rounds of 45 s)

| Round | s | ops/s | p50 ms | p99 ms | RSS KiB | Errors |
|---:|---:|---:|---:|---:|---:|---:|
|---:|---:|---:|---:|---:|---:|---:|
| 1 | 45.02 | 4217 | 0.944 | 14.230 | 6275072 | 0 |
| 2 | 45.04 | 4014 | 0.997 | 15.213 | 6311936 | 0 |
| 3 | 45.07 | 4076 | 1.196 | 12.303 | 6311936 | 0 |
| 4 | 45.02 | 3868 | 0.924 | 16.981 | 6311936 | 0 |
| 5 | 45.12 | 3997 | 1.030 | 14.331 | 6316032 | 0 |
| 6 | 45.02 | 3588 | 0.751 | 22.165 | 6328320 | 0 |
| 7 | 45.02 | 4373 | 1.050 | 12.341 | 6328320 | 0 |
| 8 | 45.02 | 4251 | 0.855 | 16.136 | 6328320 | 0 |
| 9 | 45.03 | 3815 | 0.796 | 20.155 | 6328320 | 0 |

Least-squares slopes per round: **throughput −1.08 ops/s**, **p99 +0.32 ms**,
**RSS +4 468 KiB**. Zero protocol errors across 39 000+ operations.

## What this shows, and what it does not

- Throughput is **flat** over 7.5 minutes: the slope is −1.08 ops/s per round
  against a round-to-round spread of ~3 600–4 400 ops/s, i.e. inside the noise.
- p99 oscillates between 12.3 ms and 22.2 ms with a slope of +0.32 ms per round —
  it does not trend up within this run.
- RSS rose from 6 275 072 KiB to 6 328 320 KiB (~53 MiB over 450 s), a positive
  slope. This run **does not** attribute that to a leak: the keyspace is bounded,
  the harness opens and closes 8 connections per round, and no allocation tracing
  was done. It is retained here as an open observation for the soak work (`O9`)
  rather than reported as a defect or dismissed.

## Retained limits

- Single development host, Python generator, one worker, no TLS, no persistence,
  no controlled hardware (`P2`, `P5`). Minutes, not hours: a multi-hour stability
  run and allocation-level leak analysis remain open under `O9`.
- No regression budget is accepted from this trend. A budget needs an owner-accepted
  SLO on controlled hardware; this page is evidence of the run, not a gate.
- `docs/gate-scope.md` remains authoritative: no required context runs this harness.
