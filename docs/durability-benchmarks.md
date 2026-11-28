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

## Evidence retention

`target/benchmarks/durability.json` (raw records) and `durability.md` (summary
table) are written by the harness. The scheduled `Comparative benchmarks`
workflow runs the harness after building the release binary and keeps both files
in its benchmark artifact, so each run retains its own measurements and failures
instead of an attestation.

**Owner decision outstanding:** accept, amend or reject the four budgets above.
Until then D10 stays open even when a run is green.
