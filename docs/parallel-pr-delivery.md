# Independent PR delivery and hosted branch reconciliation

## Independent implementation ownership

Parallel implementation stays independent: one bounded task, one pi agent, one
worktree, one branch and one PR. A parent agent must not collect patches into a
shared integration/audit branch or make workers depend on unpublished sibling
work. The owner of a feature resolves semantic conflicts in its own branch. See
[AGENTS.md](../AGENTS.md) and the mandatory [PR gates](pr-quality-gate.md).

Derive bounded INVEST work and measurable EARS acceptance criteria from the
request, repository requirements and verified upstream behavior. Implement
complete wiring, tests and documentation without waiting for routine design
approval. Do not replace requested usable behavior with a plan or stub. A
finished PR is ready for review, not draft by default; CI and actual configured
reviews remain mandatory. Protected squash auto-merge is enabled on the exact
review-ready head when requested. Never bypass protection, self-approve, weaken
the Valkey gate or rerun a failed gate until green.

## Hosted updater status

`.github/workflows/pr-branch-updater.yml` and
`scripts/update-pr-branches.py` implement the GitHub-hosted reconciliation
workflow. This change is **not deployed until its protected PR merges to `main`**.
It is not a local controller, installed service, or continuously running host
process. No local state directory or operator session is required.

The workflow runs a read-only self-test job on `pull_request`. Its privileged
`reconcile` job can run only when the event is not `pull_request` and the ref is
`refs/heads/main`; it checks out the trusted `main` branch with checkout
credentials disabled. The privileged job is never given PR artifacts or PR code
and does not call merge or enable auto-merge. The workflow is triggered by main
push, hourly schedule, manual dispatch on main, and completion of the existing
`CI` / `PR Valkey quality gate` / `PR multi-CPU Valkey performance` workflows. The `workflow_run` event executes the
workflow definition from the default branch; the updater validates repository,
branch and PR metadata before mutation.

The reconciliation is bounded to at most 1,000 open PRs in a pass. Eligible
candidates are every open PR targeting this repository's `main` with a
same-repository head and a well-formed base/head identity. Draft state and a
missing auto-merge request are deliberately **not** eligibility gates: the user
asked for every open PR branch to stay current, and the serialized queue below
keeps them all in scope across successive passes. A fork, closed PR, non-main
target, malformed identity or ambiguous duplicate branch/head is reported and
never updated. The updater does not promote a draft, add or revoke auto-merge,
or alter reviews, labels, comments, branch protection, merge strategy,
required-check configuration, or main.

The queue is **serialized**: a pass inspects candidates in ascending PR-number
order and performs **at most one successful branch refresh**. A refreshed head is
a new candidate identity that must run all required checks again, so refreshing
every stale PR in one pass would stampede the gate. A conflict, timeout, identity
race, failed gate or API error on the current candidate is recorded as failed
evidence and the pass continues to the next candidate; the first successful
refresh (draft or ready) ends the pass. Every PR behind that point is
reported as `deferred` in the run summary and per-PR artifact - never silently
omitted - and keeps its stale branch until a later pass advances the queue (main
push, hourly schedule, required-workflow completion, or manual main dispatch).
Already-current and already-ahead PRs are still inspected and have their exact
required checks reconciled, but they do not consume the refresh slot.

Staleness is measured against **current main**, not the PR's last-synced base
snapshot: GitHub reports `pr.base.sha` as the base at the PR's last sync, which
stays behind after main advances, so comparing it with current main would wrongly
mark every stale PR as a base race and never update anything. The updater instead
compares current `main` with the head and treats `behind`/`diverged` as stale.
For a stale branch it freezes current main and head SHAs, re-reads both before
mutation, and calls GitHub's PR branch-update API with `expected_head_sha`. It
never pushes or force-pushes. It observes the asynchronous update for a bounded
interval and accepts a candidate only after observing the new head and confirming
it contains the frozen main. A conflict, timeout, identity race or API error is
surfaced in the run summary and per-PR artifact; no attempt is made to resolve
conflicts or overwrite a worker's newer head. Draft PRs get their branch
refreshed but defer the expensive required checks until they are ready to merge;
that refresh still uses the pass's single refresh slot because it creates a new
head.

For the exact candidate, the updater discovers runs by workflow path, event,
branch and the workflow run's own immutable `head_sha`. GitHub's nested
`pull_requests[].head` and `.base` objects can change when the associated PR is
updated: they are not snapshots of the source/base tested by that old run. They
must never relabel an old success or failure as evidence for a newer head.
Current-main ancestry is checked against the frozen actual main/head refs.
The updater accepts only every required named check (all matrix members included)
from the GitHub Actions app and matching check suite for the actual run SHA. If **any** exact-head run is
active, it waits; if **any** exact-head run failed, it reports that failure even
when a later run succeeded and never dispatches a retry. Exact source identity
also prevents old failures from blocking a genuinely fixed new source head.
Failed evidence survives a main advance and cannot be erased by a base refresh.
`cancelled` and `stale` runs are not verdicts: concurrency cancellation and a
branch refresh routinely supersede an older run for the same head. They are
ignored for the failed-evidence decision, while a genuine `failure`, `timed_out`
or `startup_failure` run for that head remains sticky and blocks a retry. A
cancelled run therefore cannot mask a real failure and cannot by itself block a
valid head. If no
matching run is observable, it first verifies that the branch's
all six allowlisted workflow blobs (CI, both Valkey gates, container verification,
upstream Tcl compatibility and read-only PR policy tests) still
match protected `main` (a branch that rewrote a required workflow is reported as
`untrusted_workflow` and never dispatched, because `workflow_dispatch` would
otherwise run that rewritten workflow with the updater's write token). Only then
does it explicitly dispatch those six workflows from the shared inventory on the
validated branch (both benchmark workflows with that PR's number). Those existing workflows retain read-only
permissions and their own head/base validation. This is necessary because GitHub
suppresses most events caused by `GITHUB_TOKEN`; token-created PR synchronization
runs may be approval-held, while `workflow_dispatch` is an explicit allowed trigger.

The privileged job has only the contents, pull-request and Actions permissions
needed for the GitHub branch-update and workflow-dispatch APIs. One repository-
wide Actions concurrency group has no active-run cancellation and allows up to
100 pending runs. Correctness does not depend on event ordering: each run lists
current PRs and reconciles observed GitHub state, and each run advances the
serialized queue by at most one branch refresh, so the single concurrency group
also guarantees at most one branch-changing pass at a time. Every run writes a
summary and retains machine-readable per-PR outcomes (schema 2, including
`refreshed` and `deferred` markers) as an Actions artifact for 90 days.
Actions workflow runs and exact check identities are the durable evidence used
for recovery; no runner-local file is authoritative.

The updater does not merge PRs. GitHub's existing strict protection, real
required checks, required CODEOWNERS review, and protected squash auto-merge stay
authoritative. Failed benchmark evidence is retained by GitHub; an unchanged
failed head is not silently refreshed to obtain a new check identity. A genuine
worker source fix creates a new head and is evaluated normally. Privileged reconciliation dispatch
is restricted to main; non-main dispatch runs only read-only policy tests.
Failures/races remain visible rather than being
retried until green.

### Security and event boundary

- No `pull_request_target` trigger is used. The PR-event job has only
  `contents: read` and runs offline mocked policy/state-machine tests.
- The write-capable job excludes `pull_request`, checks out only trusted `main`,
  persists no checkout credentials, and executes only repository-owned helper
  code. It does not consume artifacts from the triggering `workflow_run`.
- API data and branch names are passed through the Python standard-library HTTP
  client, not interpolated into shell commands. API error bodies and credentials
  are not printed.
- Only the six inventory-allowlisted required workflows can be explicitly dispatched; their
  exact workflow paths, run event, candidate identity, check suite, check name
  and GitHub Actions app are validated before a pass is reported.
- `contents:write` is present only on the trusted reconciliation job because
  the PR branch-update endpoint may require repository-content permission for
  app tokens. The helper uses that permission only through the guarded branch
  update endpoint; it contains no push or merge operation.

### Limits and operator response

This is a self-contained workflow, not an independently deployed service. It
becomes active only after merge to the default branch; before then the PR's
read-only self-test job is the only execution. The workflow has a 30-minute job
bound, 1,000-PR pagination bound, 90-second asynchronous update observation,
and 90-day artifact retention. GitHub's workflow-run/check history is the source
of truth for previously submitted required checks. If the API accepts an
asynchronous update or dispatch but its result is not yet visible, the run
reports a pending/uncertain outcome; the schedule and later workflow events
reconcile again from current GitHub state. Each pass performs at most one
successful branch refresh, so a large stale queue is drained one PR at a time
instead of rebasing and re-gating every open PR at once; deferred PRs stay
visible in the summary/artifact. A persistent conflict, failed gate,
missing exact check identity or race requires inspection by the PR owner; the
automation does not retry failed gates, resolve conflicts, or bypass protection.

Check the `PR branch updater` Actions run's summary and
`pr-branch-updater-<run-id>` artifact for per-PR outcomes. A failed candidate
must receive a genuine owner fix or explicit investigation; do not re-run its
Valkey gate until the failure is resolved. Workflow-file changes are owned by
`@macel94`; this PR needs the configured code-owner review before protected
merge. Do not claim the workflow is deployed before it actually merges.

## Native merge queues

GitHub's native merge queue can validate candidates against the latest base and
queued predecessors, but all required workflows must genuinely support
`merge_group` and preserve candidate/base identities. Simply enabling a queue
would not adapt the current PR-only Valkey validation. The present personal-
account
repository is not eligible for native merge queues. Recheck availability before
any future migration; an organization transfer must preserve visibility, access,
protection, GHCR delivery and required gates. No paid plan, migration or weaker
rule is authorized by this document.

## Official references

- [Events that trigger workflows (`workflow_run`)](https://docs.github.com/en/actions/writing-workflows/choosing-when-your-workflow-runs/events-that-trigger-workflows#workflow_run)
- [Triggering workflows and `GITHUB_TOKEN` event behavior](https://docs.github.com/en/actions/how-tos/writing-workflows/choosing-when-your-workflow-runs/triggering-a-workflow)
- [Update a pull request branch (`expected_head_sha`)](https://docs.github.com/en/rest/pulls/pulls#update-a-pull-request-branch)
- [Create a workflow dispatch event](https://docs.github.com/en/rest/actions/workflows#create-a-workflow-dispatch-event)
- [GitHub Actions concurrency](https://docs.github.com/en/actions/using-jobs/using-concurrency)
- [Automatic token authentication and least-privilege permissions](https://docs.github.com/en/actions/security-for-github-actions/security-guides/automatic-token-authentication)
- [Secure use reference: untrusted code checkout](https://docs.github.com/en/enterprise-cloud@latest/actions/reference/security/secure-use)
- [Protected auto-merge](https://docs.github.com/en/pull-requests/collaborating-with-pull-requests/incorporating-changes-from-a-pull-request/automatically-merging-a-pull-request)
