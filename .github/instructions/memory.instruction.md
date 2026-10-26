---
applyTo: '**'
---

User delivery preference: every future `main` commit must come from a pull request.
Never commit or push directly to main. Tests and the Valkey performance gate are
mandatory for every PR, including docs/generated reports; merge only when all 20
scoped benchmark medians strictly outperform Valkey and required checks pass on
an up-to-date head. No admin bypass or threshold relaxation to land a change.
See `docs/pr-quality-gate.md` and the active main ruleset.

User parallel-work preference: independent pi agents in tmux, each with a distinct
self-contained task, Git worktree/branch and separately owned PR. No parent-owned
combined implementation/audit branch, patch collection or cross-agent scheduling.
Use openai-codex/gpt-6-luna with high reasoning unless the user overrides it.

User auto-merge preference: workers delivering requested parallel PRs must enable
protected squash auto-merge themselves once review-ready and verify it is enabled
(or the intended PR already merged). This is standing authorization; do not make
the user toggle every PR. Open the completed PR ready for review while CI runs;
drafts/WIP require an explicit user request. Pending CI, optional out-of-scope
validation and self-imposed human-review checkpoints are not draft reasons.
Never bypass actual required reviews, override explicit revocation, or retry
performance gates until green.

User autonomy preference: independently derive and implement bounded INVEST
stories with measurable EARS acceptance cases, complete wiring/tests/docs and
honest evidence. Resolve routine design decisions without requesting proposal
approval. Keep research in working notes; no plan-only PRs for implementation
requests, no stubs and no sibling-dependent patches as finished features.
A worker is done only with a ready PR (`isDraft: false`) and verified protected
auto-merge (or a normal merged state), not merely a PR URL. Genuine external or
permission blockers must be reported honestly without bypassing safety or actual
code-owner review. Explicitly requested documentation is a valid separate task.

Remembered stale-PR solution: a delivery-only serialized auto-update lane refreshes
one eligible PR against current main, obtains fresh exact-candidate tests and all
20 Valkey wins, then leaves merging to protected auto-merge before advancing.
Conflicts belong to the original worker; failed candidates pause with evidence.
Native merge queues are preferred when supported, but the present personal-account
repository is not eligible. The fallback is a design, not installed automation.
Read project-root `AGENTS.md` and `docs/parallel-pr-delivery.md` before rollout;
GitHub-token updates need reliable explicit CI dispatch, identity validation,
idempotency and an independently reviewed automation PR, not weaker protection.
The user permits a free organization if genuinely best for this queue solution,
with macel94 as organization Owner and retaining primary control. No paid plan,
unnecessary transfer or loss of existing access/protection/delivery is authorized.
Creating an empty organization alone does not enable a queue for a personal repo.
