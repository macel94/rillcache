# Production acceptance runbook

Use this procedure on an isolated staging deployment with disposable data and a
separate restore destination. **Rillcache is not production ready.** This
runbook does not waive a gap, replace independent review, or authorize rollout.

## 1. Check the candidate and scope

From the exact candidate checkout, record `git rev-parse HEAD`, the artifact
SHA-256 or OCI digest, the profile revision, supported topology, and the host,
OS, storage and test-tool versions. Do not mix artifacts or results from different
candidates. Link the maintainer-approved profile and any reviewed exclusions.

```sh
python3 scripts/production_readiness.py --check
python3 scripts/production_readiness.py --json > readiness-preflight.json
```

The first command checks the canonical inventory and its README links; a zero
exit means valid documentation only. The second writes the blocker inventory,
including open P2 follow-ups; exit 1 means the status or an open P0/P1 item
blocks promotion, and exit 2 means missing or invalid documentation. A zero
exit does **not** authenticate release evidence, certify acceptance or authorize
deployment. Attach the output to the candidate's evidence record.

## 2. Execute and retain the acceptance evidence

Record a result (`pass`, `fail`, or `not run`), the command and environment,
expected vs. observed result, and a raw evidence URL for each applicable stage.
A suggested order:

| Stage | What to record before proceeding |
| --- | --- |
| Merge gate | All exact-head protected contexts, raw Valkey cohorts, and failed attempts; passing this merge gate does not substitute for any release-acceptance stage. |
| Scope | The approved profile, supported topology, explicit reviewed deployment exclusions (use an empty list when none apply), and the named scope reviewer’s candidate-specific decision. |
| Compatibility | Pinned Redis/Valkey and language-client versions; supported command/feature rows, exact replies and compatibility-specific exclusions. |
| Security | Independent review of the actual profile and artifact, disposition of findings, verified secret/ACL/TLS/audit controls, and named approval. |
| Load and soak | Controlled hardware, offered and completed workload, expected SLOs, measured request tails and resource bounds; retain overload, dropped work and failed samples. |
| Recovery | From a disposable staging journal, record acknowledged writes and timestamps before each fault. Restore into an empty isolated directory, verify data and TTLs before allowing traffic, and compare measured loss and recovery time with the profile's RPO/RTO. Follow the [offline backup procedure](../persistence.md#offline-backup-and-restore). |
| Staged rollout and rollback | Evaluate the exact artifact with representative traffic in isolation. Record the previous artifact digest, recoverable backup, rollback trigger, and an exercised rollback result. Do not point an older binary at the only copy of a newer journal. |
| Distribution and sign-off | Record license and notices, artifact verification, and named reviewers' candidate-specific decisions for the deployment profile, security, recovery and performance. |

### Check the evidence record

Store a version-2 JSON record alongside the raw artifacts. It has `schema: 2`
and a `results` object keyed by the table stages, in order: `merge`, `scope`,
`compatibility`, `security`, `load-soak`, `recovery`, `rollout-rollback`, and
`distribution-signoff`. Each row records `candidate` (40-character commit),
`artifact` (`sha256:` plus 64 hex characters), `profile` (40-character commit),
and `result` (`pass`, `fail`, or `not-run`). A passing row also requires
`method`, `environment`, `observed`, and `evidence_url` (HTTPS). The `scope`,
`security` and `distribution-signoff` rows also require a named `reviewer`. The
scope row must include non-empty `supported_topology` and an `exclusions` array;
`[]` explicitly means no deployment exclusions, while compatibility-specific
command/feature exclusions remain in compatibility evidence. Record the
reference, expected result and observed outcome in the method and observation,
and keep all earlier failed attempts in the linked evidence.

```sh
python3 scripts/release_evidence.py evidence.json \
  --candidate "$candidate_commit" --artifact "$artifact_digest" \
  --profile "$profile_commit"
```

Provide the expected identities from the candidate and approved profile, not
from the record being checked. The checker reads only that local record (at
most 256 KiB) and returns 1 for incomplete, failed or mismatched evidence, 2
for invalid inputs, and 0 only for a structurally complete record. It does not
fetch evidence or verify assertions, reviewer authority, link contents, or
release acceptance. The checker verifies only that the scope fields and named
reviewer are present; it does not establish reviewer independence, authority,
the truth of a topology/exclusion claim, or the linked decision's contents. Even
a complete record prints `release_evidence_verified: false` and
`promotion_authorized: false`. Run the canonical checklist preflight separately;
both results are for human review, not a deployment trigger.

Stop promotion on a failed, missing or stale result. Preserve its logs and mark
which criterion failed before testing a changed candidate. A documented scope
exclusion needs the responsible review and a decision in the
[canonical checklist](../production-readiness-gaps.md); this runbook is not an
exclusion mechanism. Record the decision to promote or stop with the evidence;
this runbook has not yet been executed for a production release candidate.
