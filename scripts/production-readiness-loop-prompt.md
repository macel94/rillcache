# Persistent Rillcache production-readiness loop

Continue until independent evidence supports production acceptance. Rillcache is **NOT PRODUCTION READY**. Use only the existing pinned Pi/tmux session and ten-minute timer; never create another loop, interrupt active work, use OpenRouter, or change providers from the approved `openai-codex/gpt-6-luna` without the explicitly authorized authenticated fallback. Keep reporting private; use only repository PRs/issues and the canonical local checkpoint/ledger.

## Each wake

1. Read `/root/.local/state/rillcache-frame-lifecycle/production-readiness-checkpoint.md` and the ledger tail before acting.
2. Audit live main, open PRs/reviews/checks, readiness issue claims/assignees/comments, active Pi/tmux owners, worktrees/branches and uncommitted/unpushed state, filesystem capacity, and timers. Preserve all other work and failed evidence; never duplicate an owner.
3. Continue one active issue claim. If none exists, select the highest-priority unclaimed, independently actionable issue. Skip and document items requiring a product/legal/owner decision, independent sign-off, a real release, controlled staging/hardware, or unavailable external evidence. Do not invent a substitute or claim an issue whose work is already owned. Claim visibly before implementation using `<!-- rillcache-agent-claim:v1 issue=NUMBER owner=rillcache-pi-wake -->` and name the isolated branch/worktree and bounded deliverable.
4. Implement a complete small INVEST increment with measurable EARS acceptance, tests and required docs. Preserve all branches, dirty work, losses and raw evidence. Rust builds/tests/Clippy/benchmarks/native builds are hosted-only. No main/force push, bypass, self-approval, weakened gates or unchanged retries. Required gates remain all 14 live contexts, 20/20 standard Valkey wins and 10/10 four-CPU Valkey wins; Dragonfly is diagnostic. Use PR-only protected delivery and exact-current-head checks. A merged PR is not production acceptance; never close a readiness issue until its complete acceptance criteria have independent evidence.
5. If blocked after a genuine attempt, record concise evidence and the external blocker in the issue comment, checkpoint and append-only ledger; release the claim only when safe. Do not fabricate reviews, sign-offs, releases, SLOs, deployments or acceptance.
6. Keep the checkpoint concise and the ledger append-only. Preserve unrelated user changes. Do not delete worktrees, branches, logs or caches without verified ownership/reproducibility and authorization.

## Avoiding no-progress cycles

A full audit is warranted on the first wake, when the persistent prompt changes, or when verified external readiness state changes. If nothing is safely actionable because the cycle is waiting for independent review or other external evidence, record that state once and do not repeat the same audit, claim search, or unchanged progress report on each ten-minute timer tick. The wake controller suppresses further prompts while the GitHub readiness fingerprint is unchanged; it wakes on main/PR/issue changes or at least once every 24 hours. Do not disable or replace the timer. On a heartbeat, verify whether the blocker changed before doing any work; if not, retain the waiting state and return to idle without duplicating the audit record.

At the end of every completed turn, include exactly one machine-readable marker in the final response:

- `<!-- RILLCACHE_WAKE: WAITING -->` only when there is no active implementation to continue and progress is waiting on a meaningful external state change.
- `<!-- RILLCACHE_WAKE: CONTINUE -->` when an owned task is actively being implemented, a bounded next action remains, or the external wait is not yet safely verified.

If any implementation remains in progress, do not report completion or use the WAITING marker. Report actual progress and blockers plainly. Never call a repeated audit progress. Keep the production status **NOT PRODUCTION READY** unless the independent acceptance evidence is complete.
