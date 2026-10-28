# Actions merge-gate audit — 2026-10-28

## Confirmed merge gap and immediate correction

PR [#131](https://github.com/macel94/rillcache/pull/131) merged as
`9328300f5f707630823ed72525d07e1838c1d06d` at **09:46:40 UTC**, even though
its four-CPU job on head `b1f463f` had failed at **09:40:27 UTC** in
[run 40662798308](https://github.com/macel94/rillcache/actions/runs/40662798308).
That latest job failed before measurements at its exact installation-bootstrap
SHA guard after main advanced; prior completed cohorts had genuine numeric losses.
Neither a setup error nor a throughput loss was a live merge requirement then.
GitHub recorded squash auto-merge enabled again at 09:40:17 UTC. The recorded
account identity does not identify which client initiated that request.

The live ruleset still required **only** `Tests and offline CI` and `Valkey
performance gate`. Both passed, so GitHub was allowed to merge. The third context
in tracked JSON was not deployed to GitHub settings. A workflow named “gate” and
a checked-in ruleset file do not themselves enforce a merge requirement.

The operator audit first added the missing four-CPU context from the already
merged policy, then strengthened live protection to all **14 existing PR checks**
below. Effective `rules/branches/main` and the configured ruleset were independently
re-read. Every old check and all other settings were preserved: active ruleset,
strict current-main checks, GitHub Actions app pin **15368**, empty bypass actors,
code-owner/thread-resolution requirements, squash-only merges, deletion/force-push
protection. No failed check was rerun or bypassed to make this correction.

An observed open PR (#133) had 13 successful required checks and a failed four-CPU
check, with auto-merge requested. After the correction GitHub reported it
**BLOCKED** and listed the failed four-CPU job among its required checks. No merge
attempt or modification of that PR/worker was needed for this read-only validation.

Earlier four-CPU failures remain evidence, not historical passes: count-based
run `40658513184` failed 9/10, and corrected duration-based run `40659849789`
failed 5/10 (all pipeline-1 cases). Fixing enforcement does not fix those runtime
performance losses or establish that current main passes the new gate.

## Every repository workflow is classified

Authoritative machine-readable coverage: [workflow inventory](../.github/pr-workflow-gates.json).
Tracked protection: [main ruleset](../.github/main-ruleset.json).

| Workflow | Required on every main PR | Excluded jobs/scope |
| --- | --- | --- |
| `ci.yml` | `Tests and offline CI` | Main-only archive steps are not separate PR requirements |
| `pr-quality.yml` | `Valkey performance gate` — all original 20 medians strictly win | No path/report exemption |
| `pr-multicpu-quality.yml` | `Valkey 4-CPU performance gate` — all 10 official-generator medians strictly win | No tie/partial-cohort pass |
| `container.yml` | `verify`, `build (amd64, ubuntu-24.04)`, `build (arm64, ubuntu-24.04-arm)` | `publish` and its `public-pull` dependents cannot run on PRs |
| `upstream-compatibility.yml` | `Build pinned TCP compatibility tools`; `Tcl redis / string`, `/ incr`, `/ keyspace`; `Tcl valkey / string`, `/ incr`, `/ keyspace` | Native controls and candidate failures both remain real failures |
| `pr-branch-updater.yml` | `read-only-self-tests` | Write-capable `reconcile` is trusted-main-only, never PR code |
| `benchmarks.yml` | Not PR-triggered | Explicit manual/weekly comparisons, optional inference and trusted report publisher; generated report PRs still require all 14 checks |
| `hotpaths.yml` | Not PR-triggered | Manual trusted-main profiling/A-B experiment |
| `copilot-auth-check.yml` | Not PR-triggered | Manual trusted-main inference-auth probe; no PR credential path |
| `upstream-performance.yml` | Not PR-triggered | Manual three-server comparison, not a substitute for either required Valkey gate |

GitHub's platform-generated Dependency Graph workflow is not a repository YAML
workflow or an every-PR gate. GitGuardian is a separate external integration,
not a GitHub Actions producer; this policy does not pretend that either is one
of the 14 repository-owned checks.

## Fail-closed regression and drift checks

`python3 scripts/pr_gate_policy.py` checks all workflow files and PR jobs against
that explicit inventory and the tracked ruleset. New workflows, unclassified PR
jobs, missing architecture/Tcl matrix members, colliding required check names,
path exemptions, skipped required jobs and `continue-on-error` cannot silently
become advisory. Publication/writer exclusions must retain their non-PR guards.
The small static scanner intentionally supports this repository's canonical YAML
layout, not arbitrary YAML; unsupported layout changes fail instead of guessing.

`python3 scripts/pr_gate_policy.py --live` additionally performs bounded **GET-only**
queries to verify actual ruleset enforcement, main's effective rules, app-pinned
contexts, strict freshness, no bypass and active required workflows. API failure,
drift or missing protection is a failure, not a warning or a successful no-op.
There is no settings-write token, stored secret, privileged PR execution or
administrator bypass in this checker.

GitHub redacts `bypass_actors` unless the caller can **write the ruleset** (ordinary
`contents: write` is insufficient). The first Actions attempt correctly failed
rather than inventing an empty list: run `40674262028`, head `665d05c`. Full
operator `--live` mode still requires that field to be visible and empty. Normal
`GITHUB_TOKEN` automation explicitly selects `--live --read-only-view`: all
required/effective checks, freshness, reviews and active workflows remain strict,
visible bypass actors still fail, and a redacted list is labeled **UNVERIFIED**
in logs/summary. This mode cannot detect a hidden later bypass-list change; it
must not be described as full no-bypass proof. No admin secret or PR write scope
is introduced to overcome this documented API boundary.

The audit branch wires that live check into the required read-only policy job on
every PR and into trusted reconciliation before any branch/action mutation.
Those **source changes are deployed only after their protected PR merges**;
the live 14-check configuration correction is already applied independently.
A live check cannot undo past merges, defeat an administrator deliberately
removing protections, or claim a configuration is immutable after observation.

The report publisher also audits live protection before producing its dispatch/
auto-merge plan, so workflow successes cannot substitute for missing live rules.
Branch/report automation consumes the same six-workflow inventory. It reconciles
all matrix check identities by exact source SHA, suite, check name and Actions
app, rejects skipped/missing/ambiguous checks, retains any exact-head failure,
and verifies protected workflow blobs before privileged dispatch. Policy tests
can run on branch `workflow_dispatch` with read-only permissions; the writer
still requires `refs/heads/main`. No new PR-triggered write privilege is introduced.

Offline regression commands:

```sh
python3 -m unittest scripts.test_pr_gate_policy scripts.test_pr_branch_updater -v
python3 scripts/test-pr-policy.py
python3 scripts/pr_gate_policy.py
```

Read-only live verification:

```sh
python3 scripts/pr_gate_policy.py --live
gh api repos/macel94/rillcache/rules/branches/main
gh pr checks NUMBER --required
```

Settings must be verified after a policy change; merging tracked JSON is never
reported as live enforcement. Required review and failed performance remain
real delivery blockers for the audit PR itself.

Official references: [available rules](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/available-rules-for-rulesets),
[rules REST API](https://docs.github.com/en/rest/repos/rules),
[required-check troubleshooting and skipped jobs](https://docs.github.com/en/pull-requests/how-tos/merge-and-close-pull-requests/troubleshooting-required-status-checks).
