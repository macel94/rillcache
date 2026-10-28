# Authentication and authorization

Part of the [production-readiness plan](../production-readiness.md). This file owns
the capability row so independent feature increments edit only their own area.

| Capability | Current boundary / evidence required before closing |
| --- | --- |
| Authentication and authorization | Partial implementation (S27–S28): single-password mode plus Redis-style multi-user ACL files, SHA-256 password hashes, ordered command/category/command rules, read/write/read-write key patterns, Redis 7 selectors, live `NOPERM`/`NOAUTH` denial, `ACL SETUSER/DELUSER/WHOAMI/USERS/LIST/GETUSER/CAT/SAVE/LOAD`, atomic fsynced ACL persistence and non-secret diagnostics. All-mode wire tests cover restrictions, revocation, file round-trips and startup failures. A bounded local ACL denial audit (S104), a per-connection failed-auth budget (S105), a per-peer-IP throttle (S106) and best-effort UDP off-host audit shipping (S107) are implemented; still required: subcommand-specific categories, authenticated/centralized audit aggregation, tamper-evident signing, global/cross-process rate limiting and independent security review. Use the native TLS 1.3 listener for untrusted networks; ACL rules over plaintext TCP are not a complete security design. |
