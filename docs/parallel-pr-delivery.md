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
`CI` / `PR Valkey quality gate` workflows. The `workflow_run` event executes the
workflow definition from the default branch; the updater validates repository,
branch and PR metadata before mutation.

The reconciliation is bounded to at most 1,000 open PRs in a pass. Eligible
candidates are open, non-draft PRs targeting this repository's `main`, with a
same-repository head and an existing explicit protected auto-merge request. A
fork, draft, closed PR, non-main target, missing opt-in, malformed identity or
ambiguous duplicate branch/head is reported and never updated. The updater does
not add auto-merge requests or alter reviews, labels, comments, branch
protection, merge strategy, required-check configuration, or main.

For a stale eligible branch it freezes current main and head SHAs, re-reads both
before mutation, and calls GitHub's PR branch-update API with
`expected_head_sha`. It never pushes or force-pushes. It observes the asynchronous
update for a bounded interval and accepts a candidate only after observing the
new head and confirming it contains the frozen base. A conflict, timeout,
identity race or API error is surfaced in the run summary and per-PR artifact;
no attempt is made to resolve conflicts or overwrite a worker's newer head.

For the exact candidate, the updater discovers runs by workflow path, event,
head/base association and SHA. It accepts only the required named check from the
GitHub Actions app and matching check suite. If a required run is already active,
it waits; if the latest exact candidate run failed, it reports the failure and
does not dispatch a retry. If no matching run is observable, it explicitly
dispatches only `ci.yml` and `pr-quality.yml` on the validated branch (the latter
with that PR's number). Those existing workflows retain read-only permissions
and their own head/base validation. This is necessary because GitHub suppresses
most events caused by `GITHUB_TOKEN`; token-created PR synchronization runs may
be approval-held, while `workflow_dispatch` is an explicit allowed trigger.

The privileged job has only the contents, pull-request and Actions permissions
needed for the GitHub branch-update and workflow-dispatch APIs. One repository-
wide Actions concurrency group has no active-run cancellation and allows up to
100 pending runs. Correctness does not depend on event ordering: each run lists
current PRs and reconciles observed GitHub state. Every run writes a summary and
retains machine-readable per-PR outcomes as an Actions artifact for 90 days.
Actions workflow runs and exact check identities are the durable evidence used
for recovery; no runner-local file is authoritative.

The updater does not merge PRs. GitHub's existing strict protection, real
required checks, required CODEOWNERS review, and protected squash auto-merge stay
authoritative. Failed benchmark evidence is retained by GitHub; an unchanged
failed head is not silently refreshed to obtain a new check identity. A genuine
worker source fix creates a new head and is evaluated normally. Manual dispatch
is restricted to main, and failures/races remain visible rather than being
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
- Only the two existing required workflows can be explicitly dispatched; their
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
reconcile again from current GitHub state. A persistent conflict, failed gate,
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
queued predecessors, but both required workflows must genuinely support
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
