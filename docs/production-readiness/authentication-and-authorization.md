# Authentication and authorization

Part of the [production-readiness plan](../production-readiness.md). This file owns
the capability row so independent feature increments edit only their own area.

| Capability | Current boundary / evidence required before closing |
| --- | --- |
| Authentication and authorization | Partial implementation (S27–S28): single-password mode plus Redis-style multi-user ACL files, SHA-256 password hashes, ordered command/category/command rules, read/write/read-write key patterns, Redis 7 selectors, live `NOPERM`/`NOAUTH` denial, `ACL SETUSER/DELUSER/WHOAMI/USERS/LIST/GETUSER/CAT/SAVE/LOAD`, atomic fsynced ACL persistence and non-secret diagnostics. All-mode wire tests cover restrictions, revocation, file round-trips and startup failures. Still required: subcommand-specific categories, persistent/centralized audit and rate limiting, independent security review. Use the TLS 1.3 sidecar for untrusted networks; ACL rules over plaintext TCP are not a complete security design. |
