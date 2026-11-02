# String expiration and integer coverage

Open items are tracked in the canonical [production-readiness gaps](../production-readiness-gaps.md); this file records current behavior and boundaries, not a second checklist.

Part of the [production-readiness plan](../production-readiness.md). This file owns
the capability row so independent feature increments edit only their own area.

| Capability | Current boundary / evidence required before closing |
| --- | --- |
| String expiration and SET options | Implemented S12: injectable clock; EX/PX/EXAT/PXAT, GET/KEEPTTL, TTL/PTTL and expiration commands; option/error/overflow semantics, atomic conditional writes and bounded expiry metadata/reclamation, with deterministic and wire/reference tests. Redis/Valkey digest-based `IFEQ`/`IFNE`/`IFDEQ`/`IFDNE` conditions and `DIGEST`/`DELEX`/`DELIFEQ` are implemented and covered by conditional-write tests. |
| Integer/string command coverage | Implemented S13: INCR/DECR/INCRBY/DECRBY, canonical signed-64-bit parsing, overflow rejection, TTL retention and concurrent unit/socket tests. Implemented S25: MGET/MSET/MSETNX with atomic payload admission, duplicate-key semantics, expiry behavior and a 2 MiB bounded MGET reply. Remaining string/bit coverage gaps are part of canonical C5. |
