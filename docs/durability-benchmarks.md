# Durability policy measurements (D10)

**Status: measurement harness and evidence record. The acceptance budgets below
are a proposal; the release owner must accept them before D10 can close.**

`scripts/durability_measure.py` measures the three supported journal policies on
the same host, workload and client:

```sh
make bench-threads                      # builds target/release/rillcache + rillcache-bench
python3 scripts/durability_measure.py \
  --binary target/release/rillcache \
  --bench target/release/rillcache-bench \
  --requests 20000 --clients 8 --pipeline 1 --runs 3 \
  --json target/benchmarks/durability.json | tee target/benchmarks/durability.md
```

For each policy (`no`, `everysec`, `always`) it starts a fresh disposable server
with `--appendonly yes`, measures a matched `set` workload (throughput median,
batch p99 median), samples `PING` round trips before the rewrite, issues
`BGREWRITEAOF`, samples `PING` again during the rewrite, and records:

| Recorded field | Meaning |
| --- | --- |
| `throughput_median_ops` | median requests/second of the measured runs |
| `batch_p99_median_us` | median whole-batch p99 from the upstream-style bench |
| `ping_before` | `PING` p50/p99/max before the rewrite (control) |
| `ping_during_rewrite`, `rewrite_pause_p99_ms` | `PING` p50/p99/max while the journal rewrites |
| `rewrite_reply` | the server's rewrite acknowledgement (`(no journal)` for the baseline) |
| `contention_ratio_vs_baseline` | policy throughput divided by the AOF-off baseline (the persistence serialization cost) |

The matching properties are measured, not assumed: the same binary, keyspace,
value size and client counts are used for every policy, the policy is set only
through the documented startup flag, and the server is disposable.

## Proposed acceptance budgets

These are the numbers a release owner would accept; the harness records the
measured values and the deltas, it does not self-accept:

1. `always` must not lose acknowledged writes (already covered by the crash
   drills) and its batch p99 must stay within 3× the `no` policy on the same host.
2. `everysec` throughput must stay within 10% of `no` for the matched workload.
3. The rewrite pause (`PING` p99 during the rewrite) must stay under 250 ms on the
   reference runner, with the measured value recorded even when it exceeds it.
4. No policy may fail the workload or leave the server unable to answer `PING`
   after the rewrite.

## Persistence contention (R6)

The `baseline` row is the matched control for the exclusive transaction gate that
persistence takes: `contention_ratio_vs_baseline` is the fraction of AOF-off
throughput the policy retains on the same host and workload. Proposed budget for
the release owner:

1. `no` must retain at least 90% of the AOF-off baseline throughput.
2. `everysec` must retain at least 75%.
3. `always` must retain at least 40% and must not lose acknowledged writes (the
   crash drills own that property).

The harness records the measured ratios either way; it does not fail on them,
because the ratios are the SLO that has not been accepted yet.

## Measured result (2026-11-28, hosted shared runner)

Run `46135095874` on commit `127253a9` (scheduled benchmark runner, 2 vCPU, 8
clients, 20,000 `set` requests per run, 3 runs, ext4 on the runner disk). Raw
records: [`benchmark-reports/durability-2026-11-28-hosted.json`](benchmark-reports/durability-2026-11-28-hosted.json).

| Policy | Throughput median req/s | vs AOF-off | batch p99 median us | PING p99 during rewrite ms |
| --- | ---: | ---: | ---: | ---: |
| AOF off (baseline) | 214430.36 | 1.000 | 51.32 | 0.557 |
| `no` | 32272.06 | 0.151 | 307.918 | 0.578 |
| `everysec` | 31440.09 | 0.147 | 313.653 | 0.544 |
| `always` | 7509.57 | 0.035 | 1396.206 | 0.496 |

What this measures and what it does not:

- The exclusive persistence gate costs ~85% of AOF-off throughput for `no` and
  `everysec`, and ~96% for `always`, on this runner and workload. That is the
  measured cost of the current design, not a target.
- The client-observed `PING` p99 while the journal rewrites stays far inside the
  one explicitly accepted bound in [persistence performance](persistence-performance.md)
  (rewrite start pause <= 1 s), but at 20,000 keys, not at the million-key ceiling
  measured there.
- The proposed budgets above (90%/75%/40%) are **not met** by this run. They were
  a proposal; the data rejects them as realistic for this design, and they are not
  claimed as accepted.
- No throughput or latency SLO for the persistence path is accepted anywhere in
  the repository; `docs/persistence-performance.md` says so explicitly and keeps
  R6 open. Deciding the acceptable persistence cost (or choosing the
  concurrent/ordered design instead) is the release owner's decision.

## Evidence retention

`target/benchmarks/durability.json` (raw records) and `durability.md` (summary
table) are written by the harness. The scheduled `Comparative benchmarks`
workflow runs the harness after building the release binary and keeps both files
in its benchmark artifact, so each run retains its own measurements and failures
instead of an attestation.

**Owner decision outstanding:** accept, amend or reject the four budgets above.
Until then D10 stays open even when a run is green.
