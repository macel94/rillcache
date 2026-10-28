# String expiration and integer coverage

Part of the [production-readiness plan](../production-readiness.md). This file owns
the capability row so independent feature increments edit only their own area.

| Capability | Current boundary / evidence required before closing |
| --- | --- |
| String expiration and SET options | Implemented S12: injectable clock; EX/PX/EXAT/PXAT, GET/KEEPTTL, TTL/PTTL and expiration commands; option/error/overflow semantics, atomic conditional writes and bounded expiry metadata/reclamation, with deterministic and wire/reference tests. Newer digest-based SET conditions require a separate compatibility decision/test corpus. |
| Integer/string command coverage | Implemented S13: INCR/DECR/INCRBY/DECRBY, canonical signed-64-bit parsing, overflow rejection, TTL retention and concurrent unit/socket tests. Implemented S25: MGET/MSET/MSETNX with atomic payload admission, duplicate-key semantics, expiry behavior and a 2 MiB bounded MGET reply; other string/bit commands need their own wire corpus. |
