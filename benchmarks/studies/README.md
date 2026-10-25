# Profile-guided benchmark studies

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
