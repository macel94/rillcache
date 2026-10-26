# Profile-guided benchmark studies

## Client I/O handoffs: `6cea8cc` → `0157177`

[Latest measured results, design and limits](client-handoffs/README.md): the
current five-workload reference cohort leads Valkey in all 20 measured rate and
batch-p99 conditions, including equal-one-CPU budgets. Two separate A/B studies
retain 720 raw records and all regressions; the permanent 180-record reference
archive regenerates its numeric tables offline. Sparse/epoch guards preserve
managed thread counts, bounded ownership and joined shutdown.

## Kernel-guided I/O: `7b5f08b` → `29bfb20`

[Earlier findings and all v1/v2 comparisons](io-scheduling/README.md): kernel
profiling, same-pass receive/dispatch, guarded reply yields and a versioned
cooperative run budget. Includes 720 native records, substantial worker-tail
reductions, CPU/spreads, and every measured regression. These are separate from
the automatic current-reference/Copilot cohort.

## Hot paths: `3dca0c7` → `d57cb2e`

- [Before/after findings and all 60 native comparisons](d57cb2e/README.md).
- [Generated throughput/spread, batch-p99 and CPU table](d57cb2e/summary.md).
- [Raw evidence/provenance](d57cb2e/manifest.json): 360 validated records,
  frozen baseline client, three runtime modes, two CPU budgets, two pipelines,
  five workloads, three alternating-order repetitions.
- [User-instruction diagnostics](d57cb2e/pmu/manifest.json): 30 separate perf
  captures; instrumented throughput is deliberately not performance evidence.
- [Exploratory local microbenchmarks](hotpaths-local/README.md), including spreads
  and the discarded stale-executable measurement mistake.

These studies are separate from the [current Redis/Valkey comparison and genuine
Copilot assessment](../README.md). Source/run identities and limitations matter:
lower user instructions, higher throughput or more workers do not guarantee
better tails, lower total CPU or an all-workload victory.
