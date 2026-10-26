# Repository delivery policy

- Never commit on or push directly to `main`, including generated docs/reports.
- Work on a branch and open a PR. Merge only through GitHub's protected PR path.
- Required checks: `Tests and offline CI` and `Valkey performance gate`, on an
  up-to-date branch/current head. All 20 scoped medians must strictly beat Valkey.
- Never use administrator bypass, disable/relax the gate, omit losing cases, reuse
  historical passes or rerun until green to land a change. Preserve failed evidence.
- CI/policy files require the configured code-owner review. Do not self-approve.
- Read `docs/pr-quality-gate.md`, `CONTRIBUTING.md` and `docs/requirements.md`.
- Preserve safe Rust, zero external Rust dependencies, honest benchmark scopes,
  expiry/accounting/atomicity invariants, bounded I/O and joined shutdown.
