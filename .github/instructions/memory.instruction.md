---
applyTo: '**'
---

User delivery preference: every future `main` commit must come from a pull request.
Never commit or push directly to main. Tests and the Valkey performance gate are
mandatory for every PR, including docs/generated reports; merge only when all 20
scoped benchmark medians strictly outperform Valkey and required checks pass on
an up-to-date head. No admin bypass or threshold relaxation to land a change.
See `docs/pr-quality-gate.md` and the active main ruleset.
