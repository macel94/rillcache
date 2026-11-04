# Performance

Rillcache aims to make Redis-style workloads substantially faster without
changing the way applications use a cache. Results come from repeated comparisons
with named workloads and CPU budgets, not from the choice of language alone.

## A recorded four-CPU comparison

This **2 November 2026** snapshot comes from the accepted [PR162 native gate](https://github.com/macel94/rillcache/actions/runs/41430198380).
It covers every workload/pipeline pair in that comparison, not a selection of
winning commands. Ratios are median Rillcache throughput divided by the other
server's median; `1.20x` means about 20% more requests per second in that case.

| Workload | Pipeline | Rillcache / Valkey | Rillcache / Dragonfly |
| --- | ---: | ---: | ---: |
| ping | 1 | 1.20x | 1.03x |
| ping | 16 | 1.62x | 1.06x |
| set | 1 | 1.19x | 1.43x |
| set | 16 | 1.79x | 1.70x |
| get | 1 | 1.27x | 1.43x |
| get | 16 | 1.86x | 1.86x |
| set-nx-existing | 1 | 1.26x | 1.48x |
| set-nx-existing | 16 | 1.85x | 1.89x |
| set-xx-existing | 1 | 1.30x | 1.38x |
| set-xx-existing | 16 | 1.87x | 1.64x |

Both servers in each comparison received the same **four logical CPUs**, with
8 clients, 2 generator threads, 1,024 random keys, 64-byte values, 8,000 warmup
commands, ten-second measured samples and alternating repetitions (three in the
cohorts below; the Valkey four-CPU gate now runs five and reports a paired
bootstrap interval as a diagnostic, per [the quality gate](pr-quality-gate.md)). The
same unmodified official `valkey-benchmark` drove both servers, including the
Rillcache-versus-Dragonfly comparison. Generator CPUs were separate when
available and otherwise shared, as recorded in the artifact.

The Valkey and Dragonfly comparisons are separate matched cohorts: each ratio
uses its own Rillcache/control medians. Values above are rounded. Raw CSV,
min/max spreads, latency, CPU placement, exact commands, versions, pins and
hashes remain in the linked run. The native executable was `rillcache 0.1.0`,
SHA-256 `54b64f5c883412d8d0513bf777998afaff19fb8cbaece8b8218e92401325bb6e`.
These are native-process results, not container-throughput measurements.

### The full acceptance scope

The same application binary also passed the separate [original comparison](https://github.com/macel94/rillcache/actions/runs/41430198230):

| Comparison | Cases | Recorded outcome |
| --- | ---: | --- |
| Original Valkey comparison, inherited and equal-one-CPU profiles | 20 | 20 strict median wins; 120 validated records |
| Official four-logical-CPU Valkey comparison | 10 | 10 strict median wins; 60 samples |
| Official four-logical-CPU Dragonfly comparison | 10 | 10 strict median wins; 60 samples |

This is a dated result, **not a promise that every workload or machine will see
these gains**. The scope is small-string RESP2 cache traffic at pipelines 1 and
16, without persistence or TLS. It does not establish many-core scaling, large
value/mixed-traffic performance, production tail-latency guarantees or full
Redis compatibility. Pipeline latency is whole-batch latency, not per-command
latency at pipeline sizes above one. Shared-runner variability and timer
quantization remain visible limits. Losing cohorts remain visible too: the earlier
[PR160 four-CPU run](https://github.com/macel94/rillcache/actions/runs/41405957883)
won only 8/10 Valkey cases (losing PING/P1 and NX-existing/P1). It remains failed
evidence, not a pass. Different source/hardware cohorts are not a causal
before/after experiment.

## Find results or measure your workload

- [Benchmark dashboard](../benchmarks/README.md): published reports and retained per-run history.
- [Methodology](benchmarks.md): parameters, validation, timing and native process ownership.
- [Runtime and hot paths](hot-paths.md): architecture, measured decisions and trade-offs.
- [Upstream evidence](upstream-ci.md): separate original-tool compatibility/performance execution.

From a checkout, optional native development comparisons are available with:

```sh
make bench
make bench-threads
make compare
make compare BENCH_ARGS='--requests 10000 --clients 4 --pipeline 1 --runs 3'
```

Competitor builds need the documented native C tools; they are not application
Rust dependencies. The reply-validating client writes throughput and batch-tail
data, seeds reads and conditional writes, and excludes setup/warmup. A contended
development run is not a protected-gate pass or a substitute for a matched study.
For instrumentation and controlled experiments, follow the
[Actions-only development workflow](development-cycle.md).

## Earlier investigations

These retain earlier transport versions, losses and resource trade-offs; they
are history, not interchangeable evidence for the current executable:

- [Profile-guided before/after study](../benchmarks/studies/d57cb2e/README.md).
- [Kernel-guided I/O scheduling](../benchmarks/studies/io-scheduling/README.md).
- [Single-CPU client handoffs](../benchmarks/studies/client-handoffs/README.md).
- [Architecture and rejected approaches](plans/performance-architecture.md).
- [Automated report generation and analysis](benchmark-reports.md).

Contributors must satisfy the unchanged [protected quality gates](pr-quality-gate.md):
all 40 comparisons, every required check, an up-to-date source and actual review
requirements. A historical snapshot, fastest sample or averaged score never
substitutes for fresh acceptance, and failed evidence is retained.

[Documentation home](README.md) · [Get started](getting-started.md)
