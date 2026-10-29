# Authentication and authorization

Part of the [production-readiness plan](../production-readiness.md). This file owns
the capability row so independent feature increments edit only their own area.

| Capability | Current boundary / evidence required before closing |
| --- | --- |
| Authentication and authorization | Partial implementation (S27–S28): single-password mode plus Redis-style multi-user ACL files, SHA-256 password hashes, ordered command/category/command rules, read/write/read-write key patterns, Redis 7 selectors, live `NOPERM`/`NOAUTH` denial, `ACL SETUSER/DELUSER/WHOAMI/USERS/LIST/GETUSER/CAT/SAVE/LOAD`, atomic fsynced ACL persistence and non-secret diagnostics. All-mode wire tests cover restrictions, revocation, file round-trips and startup failures. A bounded local ACL denial audit (S104), a per-connection failed-auth budget (S105), a per-peer-IP throttle (S106), best-effort UDP off-host audit shipping (S107), per-line HMAC signing (S109), a bounded UDP collector (S110), shared-key collector verification (S111) and manual audit keyring rotation (S115) are implemented; still required: remaining subcommand-specific categories beyond CLIENT/MEMORY/OBJECT/ACL/COMMAND/XGROUP/XINFO/CONFIG/LATENCY/SLOWLOG/PUBSUB, collector sender authentication/TLS/federation, full-chain signing/key-ID/hot rotation, global/cross-process rate limiting and independent security review. Use the native TLS 1.3 listener for untrusted networks; ACL rules over plaintext TCP are not a complete security design. |
