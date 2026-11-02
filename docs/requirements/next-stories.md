# Next independently deliverable stories

Part of the [requirements and incremental delivery index](../requirements.md).
Requirements are preserved verbatim from the former monolithic EARS sections; add
new requirements only in the matching area file.

The authoritative list of open work is
[production readiness gaps](../production-readiness-gaps.md). This file no longer
maintains a second list of missing features; select the next INVEST increments
from the P0/P1 items there.

Sequencing constraints that still apply:

- Prefer a small, independently testable increment with EARS acceptance criteria,
  unit and wire tests, all three execution modes and the relevant pinned
  Redis/Valkey comparison.
- Do not treat a plan, a stub, a truthful unsupported error, a no-op or a single
  local benchmark as completion of a production gate.
- Keep the [definition of done](../requirements.md#definition-of-done) and the
  [PR quality gate](../pr-quality-gate.md) unchanged. A passing merge gate is not a
  compatibility, durability, security or SLO proof.
