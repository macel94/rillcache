# Independent workers, automatic merging and stale-PR delivery

## Status and user preference

This document records the user's requested delivery behavior and the recommended
solution to stale parallel PRs. **The branch-update queue described below is a
design, not installed automation.** Auto-merge is enabled at repository level,
but each eligible PR must also have its own auto-merge request.

Development stays independent: one task, tmux/pi agent, worktree, branch and PR.
The parent must not collect patches, create a shared integration/audit branch,
or sequence implementation through itself. Serializing protected delivery is a
separate concern; it does not change who owns implementation or conflict fixes.
See [AGENTS.md](../AGENTS.md) and the [mandatory gates](pr-quality-gate.md).

## Why the earlier agents left finished PRs in draft

The earlier worker instructions explicitly allowed a draft when CI or a human
review remained, forbade merging, and assigned three documentation/research-only
lanes instead of implementation stories. There was no verified ready/auto-merge
completion contract. This was a delegation-policy error, not a GitHub requirement.

The observed PRs/handoffs make the difference concrete:

- [PR #9](https://github.com/macel94/rillcache/pull/9) had successful required
  checks but remained draft for self-imposed human review and optional real-client
  validation. That validation limits broader compatibility claims; it is not an
  automatic draft reason for a tested, explicitly scoped protocol increment.
- [PR #2](https://github.com/macel94/rillcache/pull/2) and
  [PR #5](https://github.com/macel94/rillcache/pull/5) were plan-only assignments
  and waited for human decisions despite green required checks. Their documents
  implement no security, richer types or topology. Marking a plan ready must not
  be presented as completing those features.
- Other workers made their PRs ready after CI, showing inconsistent delivery
  rules. Waiting in draft also caused additional `ready_for_review` benchmark
  runs on unchanged heads. Create finished PRs ready initially instead.

This diagnosis is evidence from the handoffs, not a statement that these PR
states will never change. Do not keep opening speculative proposals and expect
the user to read them before agents can implement routine work.

## Autonomous INVEST/EARS contract

Agents shall derive acceptance criteria from the request, existing requirements,
verified upstream semantics and repository constraints, then implement without
waiting for proposal approval. Research/design notes are working material, not
substitutes for usable code. Documentation-only PRs are valid when the user asks
for documentation (including this delivery-policy change), not as an unsolicited
replacement for feature implementation.

Apply all six INVEST criteria before choosing the independently deliverable slice:

| Criterion | Required agent behavior |
| --- | --- |
| Independent | Own complete wiring/tests/docs on its branch; do not depend on sibling unpublished changes or parent integration. |
| Negotiable | Choose a sound implementation within existing constraints; record rationale, do not request routine design approval. |
| Valuable | Deliver an observable user outcome, not merely a plan, scaffolding or promise. |
| Estimable | Inspect the affected code and identify concrete behavior and verification before editing. |
| Small | Split an oversized epic into a bounded useful increment instead of an unfinished giant PR. |
| Testable | State expected wire/state behavior and prove it with deterministic unit/integration evidence. |

Use EARS requirements that can fail a test, for example for an atomic cache-write
increment (adapt these to the actual story rather than blindly copying them):

- **Ubiquitous:** The store shall maintain exact configured key/value payload
  accounting without unsafe project code or external Rust dependencies.
- **When** a complete valid supported write is received, the server **shall**
  apply the admitted mutation once and return one correctly framed response.
- **If** arguments are invalid or admission exceeds the payload budget, **then**
  the store **shall** return the specified error and preserve live values and TTLs.
- **While** a request is incomplete, the connection **shall** retain bounded bytes
  without executing it; concurrent clients shall not observe partial mutation.
- **Where** a worker mode is selected, the server **shall** preserve the same
  command semantics, buffer limits and joined shutdown in that mode.

Cover the story's actual binary/empty/duplicate data, malformed/overflow arguments,
expiry, failed-write atomicity, concurrency, resource bounds and execution modes.
A compatibility limit is not permission to omit a required acceptance case.
Optional unrelated capabilities must remain visibly unsupported, not become
invented user-approval checkpoints for the completed scoped story.

### Definition of done

1. The requested bounded behavior, complete dispatch/session wiring and regression
   tests are implemented; original failures and limits remain honestly recorded.
2. Normal local `make ci` and applicable release/reference tests pass. The PR
   body maps concise EARS outcomes to real evidence and describes actual limits.
3. The agent commits only its own changes, pushes its own branch and creates its
   separate PR **ready for review**, not with `--draft`. A draft/WIP is permitted
   only when explicitly requested by the user. No default "draft until CI passes"
   or "awaiting human design review" handoff for routine implementation.
4. The agent enables protected auto-merge while remote checks run, requests only
   reviews actually required by repository rules, and verifies
   `isDraft: false` plus `autoMergeRequest` or the normal merged state. Do not
   merely output a PR URL and abandon the ready/auto-merge requirement.
5. Report the PR, head, implemented acceptance cases, test evidence and actual
   merge/check state. Pending CI is not a failure or a reason for a draft. If a
   real gate fails, the owner investigates/fixes it without retrying until green.

If an already-published draft contains finished behavior, its owner verifies the
scope/evidence and marks it ready with `gh pr ready <PR>`, then enables auto-merge.
If it contains only a plan for a requested feature, the owner must finish an
independently useful implementation increment; changing draft status alone is
not a fix. Do not claim that documentation promotes itself into implemented code.
Do not mass-promote unrelated existing PRs from a documentation-only task.

Autonomy is not authority to bypass actual code-owner review, spend money, invent
credentials, weaken safeguards or claim impossible compatibility. Resolve routine
design choices independently; report only genuine external/permission constraints
with the exact required action. Do not conceal incomplete work as a draft or
claim completion when a real blocker prevents delivery. Preserve explicit user
revocation and explicitly requested WIP status.

## Workers must enable auto-merge themselves

The user gives standing authorization for workers delivering requested parallel
PRs to enable **protected squash auto-merge**, without asking the user to click
it for each PR. This does not authorize administrator bypass, direct main pushes,
weaker checks, self-approval, or claiming unfinished work is complete. Routine
implementation choices are the worker's responsibility, not a user approval gate.

Each worker shall:

1. Complete its own wiring, tests and documentation on its own branch and open
   its separate PR to current main. Request any required code-owner review.
2. Once its implementation is review-ready, mark its PR ready if necessary and
   enable auto-merge with `gh pr merge <PR> --auto --squash --match-head-commit <HEAD_SHA>`.
   Create a finished PR ready initially, while protected checks run. A draft/WIP
   requires explicit user direction. Actual required review or a failed gate
   remains a merge blocker; neither grants permission to bypass a rule or claim
   incomplete behavior is finished.
3. Verify `autoMergeRequest` is set, or that the exact intended PR was already
   merged normally. **If all requirements already pass, this command can merge
   immediately**; it is not merely a harmless future preference setting.
4. Report PR URL, head SHA, auto-merge/merged state and remaining blockers. A
   permissions/API failure must be visible, not described as enabled auto-merge.
   Do not add `--admin` or `--delete-branch`.

Auto-merge is not an automatic branch updater. A new main commit makes other
heads stale under the existing strict rule, even when their old checks passed.
Fresh checks after a genuine base/head change remain necessary; automate the
mechanical update and waiting, not the safety requirement.

## Native merge queue: preferred when available

GitHub's native merge queue validates candidates against the latest base and
queued predecessors, avoiding manual update/retest/merge cycles. It requires
both protected checks to run on `merge_group` candidates, with correct frozen
base/candidate identities; simply adding an event trigger to the present PR-only
Valkey validation is insufficient. Use squash merging, no bypass and no exemption
for individually failing PRs.

As verified for this decision, `macel94/rillcache` is a public repository owned
by a personal account (`User`). GitHub documents native merge queues for
organization-owned public repositories and eligible organization-owned private
repositories, **not personal repositories**. Therefore this is not an available
switch for the current repository. Recheck support before any future rollout.

The user permits creating a **free organization** if it is genuinely the best
queue solution, with `macel94` retaining the organization **Owner** role and
primary control. Creating an empty organization does not give the personal
repository a queue: the repository must also be transferred and CI must validate
merge-group candidates. A future migration must preserve the current public
visibility, rules/no bypass actors, access, open PRs, source links, GHCR delivery
and credentials; verify everything before claiming success. No paid plan or
weaker protection is authorized. The chosen current-repository fallback avoids
that migration; this documentation does not create an organization or transfer
anything. GitHub documents organization creation through account settings, and
names the administrative role Owner, not a separate "primary owner" role.

## Recommended current-repository solution: serialized auto-update lane

Implement a small trusted-default-branch delivery workflow. It updates only the
next eligible stale PR; it never combines multiple PRs or edits their features.

1. Reconcile open, non-draft, same-repository PRs targeting main with an explicit
   auto-merge request. Exclude revoked requests, unresolved reviews/design holds,
   and previously failed delivery attempts awaiting owner action. Pick eligible
   PRs deterministically, e.g. oldest first; do not depend on Actions concurrency
   providing FIFO ordering. Already-current green PRs may merge immediately via
   GitHub auto-merge: this fallback is not native merge-queue ordering semantics.
2. Allow only one active branch-refresh/check cycle. Wake on main advancement,
   relevant check completion and explicit dispatch, with scheduled reconciliation
   for lost/suppressed events and bounded waiting. Re-list durable GitHub state
   rather than treating workflow event order as a reliable queue.
3. Freeze the main SHA and selected PR head. If stale and conflict-free, use the
   PR branch-update API with `expected_head_sha`. Merge current main into that
   PR branch; do not force-push, rewrite other feature branches, or push main.
   Re-read actual identities after the asynchronous update. A head race requires
   fresh inspection, not writing over a worker's newer commit.
4. Ensure fresh **Tests and offline CI** and **Valkey performance gate** runs for
   the updated candidate, with the protected-base frozen harness and all 20
   strict median comparisons unchanged. Existing `ci.yml` and `pr-quality.yml`
   have `workflow_dispatch`; the latter accepts the PR number and validates its
   dispatch branch/head and current base. Match actual run SHAs and Actions check
   identities, not just check names or an older successful conclusion.
5. When using `GITHUB_TOKEN` for updates, explicitly dispatch the required checks
   on the validated updated branch. GitHub documents that token-generated PR
   events can require approval, while other token-generated events are generally
   suppressed; `workflow_dispatch` is an exception. Do not assume an API update
   automatically produces unattended CI. A GitHub App installation token is an
   alternative, but requires a separately approved credential/setup decision.
6. Let GitHub's ordinary protected squash auto-merge land the PR only after all
   rules pass. Reconcile the new main revision, then refresh the next stale PR.
   Never use the next PR's old checks as evidence for its updated candidate.

The privileged control job must use only trusted main code and validated API
metadata. It must not check out/execute PR code or consume untrusted artifacts
with write credentials. Keep test/benchmark jobs read-only, checkout credentials
unpersisted, and grants minimal. Validate dispatch workflow trust/permissions;
CI/policy changes still need eligible code-owner review. No `pull_request_target`
path executing PR code, PAT in worker prompts, or standing administrator token.
Account for token-suppressed wakeups instead of relying solely on main push events.

### Failures, conflicts and duplicate events

- A merge conflict returns to that PR's owning worker for semantic resolution,
  tests and a normal branch push. The delivery workflow must not invent conflict
  resolutions, cherry-pick siblings, or discard either feature to keep moving.
- A failed test, losing/tied median, invalid/incomplete evidence, missing review
  or timeout pauses that PR visibly. Preserve logs/artifacts and continue only
  with other independently eligible PRs; never silently retry the failed one.
- Record delivery attempts by `(PR, main SHA, head SHA)`. Duplicate webhooks,
  polling, process restarts and ready-for-review events must not launch repeated
  performance runs for an already attempted candidate. Resume observation of
  existing runs; a scheduled reconciliation is not permission to rerun them.
- A real source/base change requires fresh checks, but must not be manufactured
  to escape a failed gate. Failed PRs need owner investigation and a legitimate
  fix/explicit resolution; no automatic re-enqueue until green.
- Re-check user opt-in, reviews, base and head at each transition. If main advances
  during checking, fail closed for that stale candidate; strict branch protection
  remains the final guard. Never restore auto-merge after an explicit user revoke.
- Do not re-enable auto-merge just to force another benchmark on an unchanged
  candidate, or recreate PRs to reuse historical passes or bypass failure state.

## Rollout boundary

Installing this lane requires its **own reviewed automation PR**, with offline
mocked tests for stale/current branches, expected-head races, conflicts, revoked
opt-ins, duplicate events, restart recovery, wrong workflow/check identities,
failed gates and safe permission/checkout boundaries. Verify live rules and exact
head checks before calling it operational. Preserve required checks and owner
review; this documentation does not install a workflow or change GitHub settings.

Until installed, workers still enable their own auto-merge and own their branch
updates/conflict fixes. Do not claim that auto-merge alone drains a stale queue or
that an interactive worker remaining alive is a durable repository service.

## Sources

- [GitHub merge-queue availability, semantics and merge-group CI](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/configuring-pull-request-merges/managing-a-merge-queue)
- [Adding a PR to a native merge queue](https://docs.github.com/en/pull-requests/collaborating-with-pull-requests/incorporating-changes-from-a-pull-request/merging-a-pull-request-with-a-merge-queue)
- [Auto-merge behavior and availability](https://docs.github.com/en/pull-requests/collaborating-with-pull-requests/incorporating-changes-from-a-pull-request/automatically-merging-a-pull-request)
- [Workflow-token event behavior and explicit dispatch](https://docs.github.com/en/actions/how-tos/writing-workflows/choosing-when-your-workflow-runs/triggering-a-workflow)
- [PR branch-update API and expected head](https://docs.github.com/en/rest/pulls/pulls#update-a-pull-request-branch)
- [GitHub CLI merge options](https://cli.github.com/manual/gh_pr_merge)
- [INVEST story criteria](https://agilealliance.org/glossary/invest/)
- [EARS syntax and requirement patterns](https://alistairmavin.com/ears/)
- [Ready versus draft PR creation](https://docs.github.com/en/pull-requests/collaborating-with-pull-requests/proposing-changes-to-your-work-with-pull-requests/creating-a-pull-request)
- [Creating an organization](https://docs.github.com/en/organizations/collaborating-with-groups-in-organizations/creating-a-new-organization-from-scratch)
- [Organization Owner role](https://docs.github.com/en/organizations/managing-peoples-access-to-your-organization-with-roles/roles-in-an-organization)
