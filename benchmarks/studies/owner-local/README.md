# Owner-local storage evidence

The [owning design](../../../docs/plans/owner-local-storage.md) describes the usable
one-owner increment, compatibility modes and unfinished target scope.

## 2026-10-30-v1: retained contention-affected advisory cohort

[All ten A/B rows](2026-10-30-v1/summary.md),
[source/binary identities](2026-10-30-v1/manifest.json),
[contention provenance](2026-10-30-v1/contention-provenance.json) and all 60 validated
records are retained. Eight candidate medians are lower. A sibling network
worktree's full CI/build overlapped timing; independent post-run process/load/
cgroup observations confirm active sibling build work on this shared six-CPU
host. Current ancestor/scope quotas were `max 100000`, not a private CPU budget.
These observations do not reconstruct load during every sample.

**This cohort is not controlled causal evidence for improvement or regression.**
There was no unchanged rerun to obtain a cleaner result, subtraction against
another runner, omitted loss or protected-gate substitution. The original
as-generated summary is retained separately from the provenance annotation.
Whole-batch p99 is not per-command latency; sampled CPU is tick-quantized.

The baseline release binary was frozen at clean `f0d7d186f01e74323cb7dfee19a99829de13b3af`.
The candidate binary corresponds to that base plus `candidate.patch` (including
untracked implementation files); the manifest records its combined patch hash
and individual untracked-file hashes. These patches describe the build input
**before** adding this evidence directory. Both binaries used the same frozen
client, compiler/release profile, server/client CPU placement and workload
parameters. The diagnostic driver is archived as executed, not a new gate or a
retry tool; it refers to the original retained local binaries under `target/`.

`ci-smoke/` retains the normal release smoke (default/one/four socket workers) and
public Store core results from the passing full `make ci`. They are correctness
smoke/microbenchmark output, not matched throughput controls. In particular,
external Store and multi-worker compatibility pay synchronous routing/copy costs;
poor four-worker smoke throughput is retained, not relabeled as local scaling.
No improvement claim is made from those short samples.

The whole-Data lease source patch is also retained as `rejected-lease.patch`
(SHA256 `2c94c7870b124c604498822f3281828f579ca2d0e1378df285ef4e99a91399fe`).
Its old diagnostic screen had invalid identical source labels and is not accepted
A/B evidence. The test-only head's failed official four-CPU
[run 41035897482](https://github.com/macel94/rillcache/actions/runs/41035897482)
remains failed evidence, not reused or retried as this implementation.

The final implementation still requires every protected current-head check,
including all 20 original and all 10 official four-CPU strict Valkey wins.
