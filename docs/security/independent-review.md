# Independent security review: scope, findings and sign-off record

**Status: scope published, reviewer not appointed.** This is the review charter
the project hands to an independent reviewer. Everything the readiness criterion
names exists here except the external signature itself: the scope and date, the
threat model it references, the penetration-test case list, the findings log with
closure and retest evidence, and a machine-checked sign-off registry. Project
tests and audits are listed as *existing evidence for the reviewer to
re-execute*, never as a substitute for review.

- Threat model: [threat-model.md](threat-model.md)
- Machine-readable record: [review-registry.json](review-registry.json)
- Validator: `python3 scripts/security_review.py --check` (runs in the required CI
  graph through `tests/documentation.rs`)
- Scope defined: **2026-11-27**
- Declared profile under review: `rillcache-standalone-0.1-draft`
  ([production-profile.md](../production-profile.md))
- Readiness item: [S1](../production-readiness-gaps.md#4-security-and-transport)

## Reviewer appointment (outstanding)

| Field | Value |
| --- | --- |
| Reviewer | *not appointed* — must be independent of the project (no authoring or merge rights) |
| Signature | *outstanding* — recorded as `SO-2` in the registry |
| Required output | completed case list below, findings with severity, retest verdict, signed scope |

The single external action is: an independent party runs the published scope
against a recorded candidate and replaces the `SO-2` record with
`status: "signed"`, a reviewer name, a date and a signature reference file. The
validator refuses a `signed` independent record without those fields, and refuses
either document claiming a completed independent review while the record is
outstanding.

## Candidate record

A sign-off applies to exactly one commit. Reusing a prior pass for a changed head
is prohibited; the reviewer fills one row per candidate.

| Candidate | Date | Reviewer | Verdict | Signature reference |
| --- | --- | --- | --- | --- |
| *none recorded yet* | — | — | — | — |

## Scope

**In scope.** Authentication and session admission (T1); multi-user
authorization (T2); TLS transport, probe trust and SNI selection (T3); RESP and
inline parsing with pipelining (T4); slow readers, blocked clients and queue
growth (T5); memory budget and OOM behavior (T6); journal, anchor and archive
integrity (T7); audit custody (T8); dependency and build supply chain (T9);
fail-closed behaviour of absent subsystems (T10).

**Out of scope.** Scripting, modules, cluster and replication are not
implemented; their security is excluded by the profile and covered only by T10
(fail closed). Operational hardening of the host, kernel tuning and cloud IAM are
operator responsibilities.

**Explicitly not claimed.** `maxmemory`-style payload budgets are not an RSS
enforcement mechanism (F-02), the default user has no password unless configured
(F-01), slow-client kill policy is absent (F-03) and certificate status checking
is absent (F-04). The reviewer must treat these as known-open, not as findings
the project disputes.

## Review method and penetration-test cases

1. **Document and configuration review.** Compare
   [threat-model.md](threat-model.md) against
   [security.md](../security.md), [tls.md](../tls.md),
   [persistence.md](../persistence.md) and the CLI surface; report every control
   that is claimed but not enforced, and every enforced control that is not
   documented.
2. **Black-box cases.** Execute each case below against a candidate built from
   the recorded commit; record the exact command, observed reply and verdict.

| Case | Threat | Action | Expected observable |
| --- | --- | --- | --- |
| P-01 | T1 | Connect with no credentials to a `--requirepass` process and send `GET k` | `-NOAUTH` before dispatch; no key disclosure |
| P-02 | T1 | Replay wrong passwords past the failure budget | Bounded failures, then rejection; no timing oracle in replies |
| P-03 | T1 | Open many unauthenticated sessions | Documented admission behaviour; record where the missing cap (F-01) bites |
| P-04 | T2 | `ACL SETUSER` user limited to `+get` on `app:*`, then attempt `SET other:*`, `KEYS *`, `SUBSCRIBE other` | `-NOPERM` on each; keyspace untouched |
| P-05 | T2 | Load an ACL file whose default user is `off` | Startup or reload rejects the file (`MissingDefault`), no permissive fallback |
| P-06 | T3 | TLS handshake with wrong hostname, wrong CA, TLS 1.2 | Handshake refused; no plaintext fallback |
| P-07 | T3 | Load invalid SNI certificate material | Whole generation rejected; previous good generation stays active |
| P-08 | T4 | Fragmented RESP, inline request at the bulk/frame limit, pipelined mix | No parser desynchronization; explicit limit errors; replies ordered |
| P-09 | T5 | Subscribe then stop reading while another client writes | Bounded queues, other clients served; record the missing stall policy (F-03) |
| P-10 | T6 | Fill the payload budget, then exceed process memory | Payload rejection observed; record whether the kernel OOM-kills (F-02) |
| P-11 | T7 | Flip bytes in journal frames with and without HMAC, truncate at a frame boundary, tamper with the anchor | Replay refuses or detects corruption; anchor mismatch detected |
| P-12 | T8 | Delete/alter an audit record, rotate the keyring | Verification failure is reported; rotation does not weaken new records |
| P-13 | T9 | Rebuild from vendored sources; enumerate the dependency set and unsafe exceptions | Build succeeds offline; dependency and unsafe inventory matches the reviewed exceptions |
| P-14 | T10 | Send scripting, module, cluster and replication commands | Explicit unsupported errors, no partial execution path |

3. **Retest.** Every finding the reviewer raises gets an owner, a fix reference
   and a retest with the same case id. Project retests are recorded as evidence
   for the reviewer; only the reviewer's verdict closes a finding for sign-off
   purposes.

## Findings log

Statuses mirror [review-registry.json](review-registry.json); `open` findings name
the readiness item that tracks them, `closed` findings carry closure and retest
evidence paths that the validator checks exist, and `risk-accepted` findings carry
a written risk note.

| Finding | Threat | Severity | Status | Tracking / evidence |
| --- | --- | --- | --- | --- |
| F-01 implicit no-password default with unbounded unauthenticated sessions | T1 | high | open | R5 |
| F-02 no enforced RSS/cgroup bound, OOM behavior untested | T6 | high | open | O1 |
| F-03 no configurable slow-client threshold or kill policy | T5 | medium | open | O6 / S9 |
| F-04 no certificate status validation or terminator anti-bypass attestation | T3 | medium | open | S5 |
| F-05 audit records HMAC-framed with keyring rotation | T8 | low | closed | `tests/audit_hmac.rs`, `tests/audit_keyring.rs`, `tests/audit_syslog.rs` |
| F-06 CRC-only detection without HMAC; archive encryption at rest excluded | T7 | medium | risk-accepted | [persistence.md](../persistence.md) |
| F-07 inline requests bounded by the same frame contract | T4 | low | closed | `tests/protocol_fuzz.rs`, `tests/boundaries.rs` |
| F-08 absent scripting/module/cluster surfaces fail closed | T10 | low | closed | `tests/error_dialect.rs`, [unsupported.md](../unsupported.md) |

## Retained failures and limits

- No external fuzzing campaign exists; T4 evidence is project-authored and
  seed-reproducible only (V4).
- Test agents are AI-generated. The project therefore does not treat any of the
  evidence above as independent validation, and the readiness claim stays open.
- The four CPU-cohort benchmark gate and the differential corpora are quality
  gates, not penetration evidence.

## Sign-off records

| Id | Kind | Status | Reviewer | Date |
| --- | --- | --- | --- | --- |
| SO-1 | project | signed | rillcache maintainers | 2026-11-27 |
| SO-2 | independent | **outstanding** | *not appointed* | — |

The outstanding line is the whole of what is missing: an independent reviewer's
signature over this scope for a recorded candidate.
