# Production acceptance sign-offs

This is the V3 artifact: a machine-checked registry of who accepted which
production claim, for which candidate, on what evidence. A merged pull request is
not a sign-off; a comment on an issue is not a sign-off. A row in
`docs/acceptance/signoffs.tsv` whose evidence link resolves and whose candidate
matches is a sign-off.

## Required scopes

Every release candidate needs a signed acceptance row for each scope below before
`scripts/cut-release.sh` may publish a stable tag:

| Scope | What is being accepted | Who may sign |
| --- | --- | --- |
| `profile` | the published compatibility profile and its exclusions | maintainer or independent reviewer |
| `security` | the threat model, review scope, findings closure and retest evidence | **independent reviewer only** |
| `performance` | benchmark evidence, its scope and the stated SLOs | maintainer or independent reviewer |
| `recovery` | backup, restore, restart and failure-drill evidence | maintainer or independent reviewer |
| `release` | the release notes, migration/rollback position and support window | release owner |

`security` is the one scope a maintainer cannot self-sign: the S1 criterion exists
because a project-authored test cannot substitute for an independent review. The
registry therefore records the honest state: the artifact set can be complete while
that one signature is still `unassigned`.

## Registry format

`docs/acceptance/signoffs.tsv` is tab-separated with a header row:

```
candidate	scope	reviewer	reviewer_kind	evidence	review_scope	findings	signed_at	state
```

- `candidate` — full 40-character commit SHA the acceptance applies to.
- `scope` — one of the scopes above.
- `reviewer` — the named person who accepted it.
- `reviewer_kind` — `maintainer` or `independent`.
- `evidence` — repository path or URL that contains the evidence accepted.
- `signed_at` — ISO-8601 date (`YYYY-MM-DD`).
- `review_scope` — the reviewer's own statement of what was reviewed at this
  candidate (required for a `signed` row; `-` is not accepted).
- `findings` — comma-separated finding ids from the findings log, or `-`. A
  signed row may not reference a still-open finding.
- `state` — `signed`, `unassigned` or `withheld`.

## Findings log

`docs/acceptance/findings.tsv` records each review finding with the scope, a
summary, severity (`low`/`medium`/`high`/`critical`), disposition, retest
evidence and status (`open`/`closed`). A `closed` finding must name both a
disposition and retest evidence that resolves in the repository. Signed rows
reference closed findings by id, so a scope cannot be accepted while a finding
for it is still open.

The findings log is the place where an independent reviewer records what the
review found and how each item was retested; it is the machine-checked link
between "the review happened" and "the accepted evidence is the retested
evidence".

An `unassigned` or `withheld` row is a legitimate registry entry: it records that
the scope is not accepted yet and keeps the gap visible. It never counts as
acceptance.

## Rules the checker enforces

`scripts/check_acceptance.py` fails when:

1. the header or a row is malformed, or a scope is missing entirely;
2. a `signed` row names no reviewer, no evidence, or a non-ISO date;
3. a `signed` row uses a candidate that is not a full 40-character SHA;
4. a `security` row is `signed` by a `maintainer` (self-signing a security review);
5. a `signed` row's evidence path does not exist in the repository;
6. a scope required by the release is `unassigned`/`withheld` and
   `--require-all` is passed (this is what a release gate uses);
7. a `signed` row has no `review_scope` statement;
8. a `signed` row references a finding that is not in the findings log or is
   still `open`;
9. the findings log is malformed, has a duplicate id, or closes a finding
   without a disposition and resolvable retest evidence.

`scripts/check_acceptance.py --outstanding` prints one line per scope whose
acceptance is still missing - the scope, its state and the exact reviewer kind
that must sign it - and exits 0. That report is the record of the residual
external dependency; it does not claim acceptance.

Run it directly, or through `make acceptance-check` when a release is prepared.

## Current state

The registry records the acceptance that exists today and the acceptance that does
not. `security` is `unassigned` until a named independent reviewer signs; nothing
in this repository can honestly produce that signature, and no PR merge approval,
CI status or maintainer comment is recorded as a substitute. Everything else is
either signed with evidence or explicitly withheld with a reason.

## Evidence

- `docs/acceptance/signoffs.tsv` — the registry.
- `scripts/check_acceptance.py` + `scripts/test_check_acceptance.py` — the checker
  and its negative controls.
- `tests/documentation.rs` — asserts the registry parses and the checker rejects a
  self-signed security row.

## Current outstanding signatures

At candidate `b27da344`, the artifact set above is complete and the registry
records the honest state:

```
security	unassigned	needs independent reviewer only signature
performance	withheld	needs maintainer or independent reviewer signature
recovery	withheld	needs maintainer or independent reviewer signature
```

The security signature is the only one no in-repo artifact can produce: it is an
independent human review recorded through the findings log. The performance and
recovery signatures stay withheld until the P-series measurement and the
O-series soak/recovery evidence close.
