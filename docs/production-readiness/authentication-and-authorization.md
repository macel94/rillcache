# Authentication and authorization

Open items are tracked in the canonical [production-readiness gaps](../production-readiness-gaps.md); this file records current behavior and boundaries, not a second checklist.

Part of the [production-readiness plan](../production-readiness.md). This file owns
the capability row so independent feature increments edit only their own area.

| Capability | Current boundary / evidence required before closing |
| --- | --- |
| Authentication and authorization | Partial implementation (S27–S28): single-password mode plus Redis-style multi-user ACL files, SHA-256 password hashes, ordered command/category/command rules, read/write/read-write key patterns, Redis 7 selectors, live `NOPERM`/`NOAUTH` denial, `ACL SETUSER/DELUSER/WHOAMI/USERS/LIST/GETUSER/CAT/SAVE/LOAD`, atomic fsynced ACL persistence and non-secret diagnostics. All-mode wire tests cover restrictions, revocation, file round-trips and startup failures. A bounded local ACL denial audit (S104), a per-connection failed-auth budget (S105), a per-peer-IP throttle (S106), best-effort UDP off-host audit shipping (S107), per-line HMAC signing (S109), a bounded UDP collector (S110), shared-key collector verification (S111) and manual audit keyring rotation (S115) are implemented. Open work is tracked in the canonical gaps file (S1–S4, S6). Use the native TLS 1.3 listener for untrusted networks; ACL rules over plaintext TCP are not a complete security design. |
