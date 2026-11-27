# Security threat model (draft profile authority)

**Status: SCOPED, NOT INDEPENDENTLY REVIEWED.** This document is the project's
authoritative threat model for the deployment shape declared in
[production-profile.md](../production-profile.md)
(`rillcache-standalone-0.1-draft`). It is an input to readiness item
[S1](../production-readiness-gaps.md#4-security-and-transport), not a security
sign-off: the project cannot review itself, and no external reviewer has signed
this scope yet. The machine-readable record of every threat, finding and sign-off
lives in [review-registry.json](review-registry.json) and is validated by
`python3 scripts/security_review.py --check` in the required CI graph.

## Assets

| Asset | Why it matters |
| --- | --- |
| Keyspace contents and expiries | Customer data; disclosure or silent mutation is unrecoverable without a journal. |
| Credentials (`requirepass`, ACL file, audit keyring, TLS private keys) | Compromise converts any network position into full data access. |
| Journal, rollback anchor and offline archive | The only durable record; unauthenticated edits become state after replay. |
| Audit records | Custody evidence for operator actions and authentication outcomes. |
| Availability of the single owner-local data path | One stalled or oversized client can starve every other client. |

## Trust boundaries

1. **Network client → listener.** Untrusted; authenticated (password or ACL
   user) before non-authentication commands, optionally TLS 1.3.
2. **Operator → process configuration.** Trusted but error-prone: CLI flags,
   ACL file, audit keyring, TLS material, journal directory.
3. **Process → host filesystem.** Journal, anchor, ACL/audit files; write access
   to these paths is equivalent to data access.
4. **Process → host resources.** RSS, cgroup limits, kernel OOM killer.
5. **Build → dependency supply chain.** Vendored crates, pinned upstream
   archives, container base image.
6. **Absent subsystems.** Anything not implemented must fail closed, never
   half-exist as a trusted sandbox or topology.

## T1 — Authentication and session admission

**Surface.** Client listener; `AUTH`, ACL default user, connection admission.
**Attack.** Unauthenticated peer reaches dispatch, guesses credentials, or opens
unbounded unauthenticated sessions and starves legitimate clients.
**Controls.** `--requirepass`/`--requirepass-file` (regular file, no group/other
bits; inline secrets below 16 bytes refused); authentication required before any
non-authentication command once configured; failure budget and per-source
limiting; explicit no-password configuration error instead of silent permissive
mode.
**Evidence.** [security.md](../security.md), `tests/auth.rs`,
`tests/auth_failure_budget.rs`, `tests/auth_source_limit.rs`.
**Residual risk.** The implicit no-password default user persists when nothing is
configured, and unauthenticated session capacity is not capped (F-01, tracked by
R5).

## T2 — Authorization (multi-user ACL)

**Surface.** `ACL SETUSER`, command/key/channel grants, default-user handling.
**Attack.** A user performs a command, key or channel operation outside its
grant; an operator loads a file that silently broadens access.
**Controls.** Versioned allow/deny matrix executed against both pinned references
and the candidate; a file whose default user is `off` is rejected rather than
treated as permissive; keyspace-safety and category-selector audits.
**Evidence.** [security.md](../security.md), `ci/upstream/acl-v1.json`,
`scripts/upstream/acl_matrix.py`, `tests/acl.rs`,
`tests/acl_keyspace_safety.rs`, `tests/acl_category_selector_audit.rs`.
**Residual risk.** Not full Redis ACL semantics (S2) and no per-user command or
memory quota (R5).

## T3 — Transport (TLS listener, probe trust, SNI)

**Surface.** TLS 1.3 listener, probe client, SNI certificate selection.
**Attack.** Downgrade, interception, endpoint impersonation, or certificate
selection that trusts the wrong material.
**Controls.** TLS 1.3 only through the reviewed rustls stack with no plaintext or
insecure fallback; probes require an explicit CA bundle and verify chain plus
DNS/IP identity before `AUTH`; one invalid SNI key rejects the whole generation.
**Evidence.** [tls.md](../tls.md), `tests/native_tls.rs`.
**Residual risk.** Certificate status (OCSP/CRL) and a terminator anti-bypass
attestation are absent (F-04, tracked by S5).

## T4 — Protocol parsing (RESP frames, inline requests, pipelining)

**Surface.** RESP encoder/decoder, inline requests, pipelined batches.
**Attack.** A malformed frame desynchronizes the parser, executes unintended
commands, or allocates unbounded buffers.
**Controls.** Finite bulk (8 MiB) and frame (16 MiB) bounds with explicit
errors; seeded fragmentation, random-byte and pipeline-consumption properties;
boundary and error-dialect tests over the same limits.
**Evidence.** `tests/protocol_fuzz.rs`, `tests/boundaries.rs`,
[unsupported.md](../unsupported.md).
**Residual risk.** No external fuzzing campaign and no sustained adversarial
corpus in CI (V4).

## T5 — Denial of service (slow readers, blocked clients, queue growth)

**Surface.** Output queues, blocked-client bookkeeping, `CLIENT PAUSE`.
**Attack.** A client stops reading, or a fan-out workload grows queues until the
owner starves.
**Controls.** Bounded frames and a finite client count; blocked-client accounting
and pause interaction; output backpressure instead of unbounded buffering.
**Evidence.** `tests/clients.rs`, `tests/blocked_clients.rs`,
`tests/client_pause.rs`.
**Residual risk.** No configurable stall threshold, kill policy or fair share for
large replies (F-03, tracked by O6/S9).

## T6 — Host resources (memory bound and OOM)

**Surface.** Keyspace allocation, indexes, buffers, cgroup/RSS accounting.
**Attack.** Memory growth beyond the host budget so the kernel OOM-kills the
process, or a budget that is claimed but not enforced.
**Controls.** Logical payload budget with rejection and capacity reporting; RSS
and cgroup v1/v2 current/limit observation with an explicit scope; persistent
`WATCH`-history byte cap.
**Evidence.** [eviction.md](../eviction.md), `tests/eviction.rs`,
`tests/memory_stats.rs`.
**Residual risk.** Observation is not enforcement: there is no RSS/cgroup
bound, headroom contract or tested OOM behavior (F-02, tracked by O1).

## T7 — Persistence integrity and custody

**Surface.** Journal frames, rollback anchor, offline archive.
**Attack.** Alter, truncate or forge journal/anchor/archive bytes so recovery
loads unauthenticated or semantically different state.
**Controls.** Optional CRC32/HMAC framing validated on replay; rollback anchor
recorded outside the journal; offline `rillcache-backup` archive with
verification.
**Evidence.** [persistence.md](../persistence.md), `tests/durability_faults.rs`,
`tests/aof_backup.rs`, `tests/aof_anchor.rs`.
**Residual risk.** HMAC is optional and archive encryption at rest is an explicit
exclusion (F-06, risk accepted for the draft profile).

## T8 — Audit custody

**Surface.** Audit log framing, keyring rotation, syslog delivery.
**Attack.** Forge or edit audit records, or leak credentials through diagnostic
output.
**Controls.** HMAC-framed records with keyring rotation; bounded redaction of
credential-bearing diagnostics; syslog delivery tests.
**Evidence.** `tests/audit_hmac.rs`, `tests/audit_keyring.rs`,
`tests/audit_syslog.rs`, [security.md](../security.md).
**Residual risk.** Custody depends on the operator's external syslog store; a
host-level attacker can delete it.

## T9 — Supply chain

**Surface.** Vendored crates, pinned upstream archives, container image.
**Attack.** A compromised or unexpected dependency or toolchain injects code into
a release build.
**Controls.** Vendored locked dependencies; pinned upstream archives re-validated
on every use; no application-level unsafe Rust beyond the reviewed
rustls/Tokio/CPU-affinity exceptions and required transitives; pinned reference
versions for every differential and benchmark.
**Evidence.** `Cargo.lock`, [requirements.md](../requirements.md),
`ci/upstream/pins.json`.
**Residual risk.** No SBOM, image signature, provenance attestation or
reproducible-build comparison yet (L3/L8).

## T10 — Absent subsystems must fail closed

**Surface.** Scripting, modules, cluster, replication and other excluded
features.
**Attack.** A partially present sandbox or topology surface is trusted with
credentials or data.
**Controls.** Absent commands return explicit unsupported errors from a
documented list; the declared profile lists the exclusions; adding any such
feature requires its own threat model before it is claimed secure.
**Evidence.** [unsupported.md](../unsupported.md),
[production-profile.md](../production-profile.md), `tests/error_dialect.rs`.
**Residual risk.** Any future addition must not inherit this model (S10).

## How findings and sign-off are tracked

Every threat above has a machine-readable entry in
[review-registry.json](review-registry.json); closed findings must carry both
closure and retest evidence paths that exist in the tree, open findings must name
the checklist item that tracks them, and an independent sign-off can only be
recorded as `signed` with a named reviewer, a date and a signature reference.
Project-authored tests are listed as control evidence, never as independent
validation.
