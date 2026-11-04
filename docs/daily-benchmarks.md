# Daily five-server comparison

The **Upstream generator performance** workflow contains a read-only daily job
for **Rillcache, Redis, Valkey, Dragonfly and Garnet**, scheduled at **05:17 UTC**.
This schedule becomes active only after its protected workflow PR merges to main.
GitHub schedules can be delayed; a missing run is not fresh evidence.

[Open the workflow and its run summaries](https://github.com/macel94/rillcache/actions/workflows/upstream-performance.yml).
The summary contains every server's throughput spread, CV, pipeline p99, CPU and
RSS, plus a reference ranking per workload. The artifact contains all raw files
and every pairwise comparison, not only Rillcache's wins. Artifacts last 30 days.
No binaries or credentials are published. No AI service or new secret is needed.
The older Rust-generator/report-publishing workflow remains manual, not a second
weekly scheduled comparison with a different server set.

## Run and inspect

```sh
gh workflow run upstream-performance.yml --ref BRANCH -f daily=true
gh run view RUN_ID --log
gh run download RUN_ID -D evidence
python3 scripts/upstream/daily.py --check evidence/daily
```

The existing manual three-server study remains available with `-f daily=false`;
`-f study=true` selects the separate isolated-host development study instead.
Native builds and comparative traffic are Actions-only. Offline harness tests:

```sh
python3 -m unittest discover -s scripts/upstream -p 'test_*.py' -v
```

## Matched scope

- One runner/job, one fresh owned server at a time; exactly four logical server
  CPUs. Generator CPUs are separate when available, otherwise explicitly shared.
- Same unmodified checksum-pinned official `valkey-benchmark` for all five.
- PING, SET, GET hit, rejected existing NX and successful existing XX; pipelines
  1/16, eight clients, two generator threads, 1024 keys and 64-byte values.
- Five repetitions with balanced rotated order: each server occupies every
  position once per condition; **250 ten-second measured samples**, each after
  8000 warmup commands. No fastest-sample selection or aggregate escape.
- Preload, full-keyspace validation and conditional-write probes surround timed
  load. The upstream client rejects error replies but does not check every
  non-error payload. This differs from the original Rust client's contract.
- Actual process affinity, source/build/executable/runtime hashes, argv, raw CSV,
  exit status and joined shutdown are retained and independently revalidated.
  Any incomplete/corrupt cohort is invalid and produces no ranking or winner.

Redis/Valkey/Rust pins are unchanged in `ci/upstream/pins.json`. Additional tools
are pinned in `ci/daily-pins.json`: Dragonfly **2.0.2**, Garnet **2.2.5** and private
.NET runtime **10.0.14**. Source/release checksums are verified before execution,
including SHA-512 for the .NET archive; there is no latest-tag runtime download.
Garnet's release is framework-dependent. Its log/page/index are explicitly sized
to 64/1/4 MiB instead of the default 16 GiB log. This is not equivalent to a Redis
maxmemory setting or an equal-RSS guarantee. Fixed warmup does not establish
steady-state JIT/GC performance. Persistence and TLS are disabled for every server.
Dragonfly is BSL-1.1, non-production benchmark use only; Garnet is MIT. License
notices are retained with tool-build evidence. None is an application dependency.

## Why not declare Dragonfly the only PR opponent?

The existing gate cohorts already show different leaders by workload. There is
no justified assumption that Dragonfly, Valkey, Redis or Garnet wins every row.
The daily report orders references for each measured condition and labels a
leader clear of its advisory noise band only above max(5%, twice either sample
CV). A single reference is named across the measured matrix only if it clears
that band in **every** row. This is descriptive, not statistical significance,
proof of future superiority, or permission to discard a losing case.

All [14 protected PR checks](pr-quality-gate.md), the 20 original and 10
four-CPU Valkey wins, and the current Dragonfly comparisons remain unchanged.
The daily job never edits protection, changes a reference pin, skips a PR check,
reuses a historical pass, or promotes itself into a gate. Replacing mandatory
comparisons requires a separately reviewed policy and verified live-ruleset
rollout; editing YAML alone cannot safely migrate required check identities.
The two generator contracts and CPU profiles must not be silently conflated.

This is a comprehensive **five-server view of the stated small-string scope**,
not realistic mixed traffic, production compatibility, many-core scaling or an
SLO. p99 at pipeline >1 is pipeline latency, not per-command latency. Shared-host
noise, client contention and residual timer quantization remain visible limits.
