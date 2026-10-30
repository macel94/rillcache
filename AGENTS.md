# Repository delivery policy

- Never commit on or push directly to `main`, including generated docs/reports.
- Work on a branch and open a PR. Merge only through GitHub's protected PR path.
- Required checks: `Tests and offline CI`, `Valkey performance gate`, and
  `Valkey 4-CPU performance gate`, on an up-to-date branch/current head. All 20
  original medians and all 10 official-benchmark four-CPU medians must strictly
  beat Valkey. All 14 contexts in `.github/pr-workflow-gates.json`, including
  native container builds, every Tcl matrix member and PR policy tests, are
  live-required. Run `python3 scripts/pr_gate_policy.py --live` to detect drift.
- Never use administrator bypass, disable/relax the gate, omit losing cases, reuse
  historical passes or rerun until green to land a change. Preserve failed evidence.
- CI/policy files require the configured code-owner review. Do not self-approve.
- Read `docs/pr-quality-gate.md`, `CONTRIBUTING.md` and `docs/requirements.md`.
- Preserve unsafe-free application Rust, only the authorized Tokio/core_affinity/
  Mio/rustls dependency exceptions and their required vendored transitives, honest benchmark scopes,
  expiry/accounting/atomicity invariants, bounded I/O and joined shutdown.

## Efficient development loop

- During editing, run relevant filtered tests or `make dev-check`; run `make test`
  after a complete logical change and full `make ci` before review. The editing
  targets never replace required checks, release/mode validation or merge gates.
- Before a performance experiment, record one mechanism, measurable prediction,
  matched baseline and rejection condition. Two unsuccessful experiments or 30
  minutes without new evidence trigger a diagnosis review, not more knob changes.
- Reuse this task's existing worktree/cache for serial changes; isolate parallel
  work. Retain all earlier branches, worktrees, raw measurements and failures.
- Keep one short canonical checkpoint and one append-only experiment ledger.
  Load the performance-investigation skill when available and follow
  `docs/development-cycle.md`. Do not push tooling-only commits as a way to retry
  an unchanged failed runtime performance candidate.

## User preference: independent parallel agents

- When parallel work is requested, launch independent pi agents in tmux, each
  owning a different, independently deliverable feature or task in its own Git
  worktree and non-main branch. Use `openai-codex/gpt-6-luna` with high reasoning
  effort for these agents unless the user specifies otherwise.
- Do not act as a central implementation orchestrator: no shared integration
  branch, combined audit build, cross-agent scheduling, staged waves, or parent-
  owned final wiring. Do not collect agents' patches into one combined change.
- Each agent owns its complete task, including necessary shared-file wiring,
  tests, documentation and validation within its own worktree. Isolated branches
  may change the same filenames; do not make agents wait for parent integration.
- When PR delivery is requested, each agent independently commits its own work,
  pushes only its own branch and opens its own separate PR against current main.
  Keep PRs self-contained; never silently turn missing implementation into a
  completed plan, stub or dependency on another agent's unpublished work.
- A status/check request means inspect and report progress, PRs, checks and
  blockers; it is not permission to redirect agents, combine branches, restart
  work, merge PRs, or retry a performance gate until green. Do not silently stop
  or repurpose another agent. Respect all delivery gates above.
- A documentation, status or cleanup-assessment request alone does not authorize
  staging, committing, pushing, merging, or destructive cleanup.
- Before cleanup, check live processes, worktree changes, unpushed commits and
  open PRs. Retain active agents and open-PR worktrees, resumable sessions, logs,
  and failed evidence. Report reclaimable build output and abandoned diagnostic
  work separately; get approval before deleting work or stopping agents. Never
  delete branches or discard uncommitted/unpushed work as automatic cleanup.

## User preference: autonomous INVEST/EARS completion

- The user wants complete routine implementation autonomy, not proposals to
  approve. Derive a bounded INVEST story and measurable EARS acceptance criteria
  from the request, repository requirements and verified upstream behavior;
  make ordinary design decisions, implement, test and deliver without waiting
  for the user to approve a plan. Keep research in working notes unless the user
  explicitly requests a documentation/research deliverable.
- Feature work must deliver usable behavior, complete wiring and passing tests,
  not a plan-only PR, stub, unimplemented foundation or sibling-dependent patch.
  Split oversized work into independently valuable, estimable, small and testable
  increments; do not redefine an unfinished feature as completed documentation.
- Open finished worker PRs ready for review, never draft by default. A draft is
  allowed only when the user explicitly requests a draft/WIP. Pending CI, optional
  out-of-scope validation or a self-imposed human-review checkpoint is not a
  reason to publish or leave a finished PR in draft.
- The completion contract is a tested, self-contained PR with `isDraft: false`
  and verified protected auto-merge enabled (or a normal merged state), plus a
  concise evidence/limits report. Fix failures and conflicts in the owning
  worktree; do not hand routine implementation decisions back to the user.
- Autonomy never waives actual required reviews, security constraints or failed
  gates. Report genuine external/permission blockers accurately; do not claim
  completion, invent authorization, self-approve or use a draft as a hidden
  blocker queue. Read `docs/parallel-pr-delivery.md` for INVEST/EARS examples,
  the observed draft root cause and the verifiable completion checklist.

## User preference: auto-merge and stale-PR delivery

- When delivering requested parallel-worker PRs, the user gives standing
  authorization for each worker to enable protected squash auto-merge on its own
  review-ready PR; do not leave that toggle for the user. Use
  `gh pr merge <PR> --auto --squash --match-head-commit <HEAD_SHA>` and verify the
  auto-merge request or normal merged state. This may merge immediately when all
  rules already pass. No administrator bypass, direct main push or branch deletion.
- Create the PR ready for review and enable auto-merge while required CI runs;
  do not wait for green checks before flipping from draft. Required reviews and
  real failures remain visible merge blockers, not extra user-approval steps
  invented by agents. Never hide failures or override revoked merge intent.
- Auto-merge does not refresh stale branches. The remembered solution is a
  delivery-only serialized auto-update lane: refresh only the next eligible PR
  against current main, run all required checks on its exact new candidate,
  let protected auto-merge land it, then advance. Development and PR ownership
  stay independent; no shared implementation/integration branch.
- Prefer native GitHub merge queues if supported, with all required workflows
  genuinely validating merge-group candidates. This personal-account repository
  currently lacks that feature; do not claim native queue support. The user
  permits a free organization if it is genuinely the best queue solution, with
  `macel94` retaining the organization Owner role and primary control; never
  purchase a plan or migrate without preserving access, gates and delivery.
  Preserve strict up-to-date checks, all 20 original wins and all 10 four-CPU wins.
  Conflicts return to the owning worker; failures pause without retries
  until green, historical-pass reuse or speculative conflict resolution.
- Read `docs/parallel-pr-delivery.md` for hosted PR branch reconciliation,
  token/dispatch constraints, exact check identity and failure behavior. The
  trusted-main `.github/workflows/pr-branch-updater.yml` keeps every open
  same-repository PR branch current with `main` (drafts included) and must not
  treat draft state or a missing auto-merge request as a reason to skip branch
  freshness. It never promotes a draft, enables/revokes auto-merge, approves or
  merges, and never executes PR-authored workflow code with write access: before
  any privileged `workflow_dispatch` it verifies the branch's required workflow
  file still matches protected `main`, and it measures staleness against current
  `main` (not the PR's last-synced base snapshot). Preserve protected gates and
  failed evidence. It is deployed only after its protected PR merges to `main`;
  do not describe an unmerged workflow as active.

## Web search

- Search with DuckDuckGo's HTML endpoint rather than the built-in search tool:
  fetch `https://html.duckduckgo.com/html/?q=<SEARCH_QUERY>` with the
  webpage-fetch tool and `format: markdown`. It needs no API key and no JS.
- URL-encode the query, spaces as `+` (`?q=tailscale+web+ssh+tmux+utf8`).
  Optional parameters: `&kl=us-en` for region, `&df=d|w|m|y` for a time filter.
- Result links are DuckDuckGo redirects of the form
  `//duckduckgo.com/l/?uddg=<percent-encoded-url>&rut=...`; decode `uddg` once
  to recover the real URL, then fetch that URL if the page itself is needed.
- Each result block already carries title, domain and snippet; prefer it over
  opening links.
- On an empty response, consent page or anti-bot challenge: retry once, then
  fall back to the built-in search tool. Do not use `lite.duckduckgo.com`.
