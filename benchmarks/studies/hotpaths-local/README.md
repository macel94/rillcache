# Exploratory local hot-path measurements

These are **dirty-tree diagnostics, not release/native Redis comparisons**.
Baseline source: `3dca0c7fb4beb9923bd05f0d891602a5cd9b3790`.
[`manifest.json`](manifest.json) records compiler, executable hashes and the
candidate application's exact source-content hashes. Ten raw JSONL files are
preserved beside it. All earlier glob-selected stale-binary microbenchmarks are
excluded; Cargo's exact compiler-artifact executable was used here.

Five alternating baseline/candidate processes, each reporting the median of seven
50,000-operation samples after 1000 warmups. Both are pinned to CPU 2 on a shared
six-CPU AMD EPYC VM with competing load. Numbers below are the median and range
of those **five process medians**, in ns/op—not confidence intervals or CPU time.
The candidate adds three diagnostic cases; shared cases retain their operations,
arguments, reply checks and sampling parameters.

| Shared case | Before median [min–max] ns/op | After median [min–max] ns/op |
| --- | ---: | ---: |
| Owned RESP2 SET parser | 92.71 [82.28–253.87] | 76.52 [67.22–84.86] |
| GET, 64-byte value | 162.01 [154.64–189.48] | 76.45 [74.41–91.43] |
| SET, 64-byte overwrite | 300.59 [290.71–346.10] | 151.42 [131.17–202.81] |
| SET NX, existing | 132.75 [118.06–157.53] | 96.46 [93.33–104.95] |
| SET XX, existing | 313.55 [295.05–348.65] | 147.22 [138.40–162.86] |
| SET XX, missing | 118.83 [94.95–169.10] | 91.36 [81.10–126.49] |

Candidate-only diagnostics: borrowing dispatch 75.38 [65.27–99.78], store PING
11.73 [10.13–12.39], bulk encoding 26.88 [26.41–31.89] ns/op. There is no matched
standalone baseline for those three extra cases, so they are not speedup ratios.

Store medians fell roughly 23–53% in this small cohort. Severe spread, especially
in the baseline parser, prevents a precise/general performance conclusion. An
intermediate parser wrapper regressed and was changed before this capture; that
result was not concealed by renaming the benchmark. Lower store time does not
establish lower TCP latency, better CPU efficiency or a win against Valkey.

See [investigation, changes and reproduction](../../../docs/hot-paths.md) and the
separate manual [native before/after workflow](../../../.github/workflows/hotpaths.yml).
Only the [comparative dashboard](../../README.md) contains the independently
validated current Redis/Valkey cohort and attributed Copilot assessment.
