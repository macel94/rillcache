# Binary string ranges (incremental compatibility)

## EARS acceptance criteria

- **When** APPEND receives a key and binary value, the server shall atomically append and return the resulting byte length; absent keys are created, including for an empty append.
- **When** STRLEN receives an absent key, it shall return integer zero; existing lengths count bytes, not Unicode characters.
- **When** GETRANGE receives signed inclusive endpoints, it shall normalize negative endpoints relative to the byte length, clamp endpoints as Redis does, and return an empty bulk for missing/out-of-range/reversed ranges. Both endpoints are parsed before lookup; signed 64-bit extrema must not overflow normalization.
- **When** SETRANGE receives a nonempty value at a valid nonnegative offset, it shall overwrite/extend atomically, zero-fill gaps, return the new byte length, and retain any existing expiry. Missing values start empty.
- **When** SETRANGE receives an empty value, it shall return the existing length (or zero) without creating a missing key; the offset still must parse and be nonnegative.
- **If** the offset is negative, then Redis's `offset is out of range` error is returned. If an integer is malformed/outside signed 64-bit range, then the standard integer error is returned. If the resulting string exceeds 512 MiB, then the upstream maximum-string error is returned; configured key/value budget failures return this project's OOM error.
- **If** admission or fallible buffer growth fails, then the target's prior data, TTL and mutation version remain unchanged. Payload admission precedes stored-buffer growth; mutation and accounting occur under one owner-local mutable borrow. Existing expiry/eviction side effects of admission remain unchanged.
- **When** a live string is appended or patched, the server shall mutate its stored buffer rather than reconstruct and copy its unchanged bytes. Growth shall be fallibly reserved before mutation, amortized while budget permits, with requested spare capacity capped by the remaining logical payload budget; adequate existing capacity shall be reused.
- **When** a mutation succeeds, payload accounting, mutation recency, WATCH invalidation and incremental journal/rewrite tracking shall reflect that write. Empty APPEND retains write semantics; empty SETRANGE remains a no-op.
- **When** commands arrive concurrently through managed per-client, one-worker or four-worker servers, updates remain atomic and other command dispatch is unaffected.

## Limits and status

This increment implements APPEND, STRLEN, GETRANGE, SETRANGE and the `SUBSTR`
alias of GETRANGE. Strings are binary-safe. Redis's 512 MiB string ceiling is
enforced independently of the configured payload budget (64 MiB by default). A stored value/full GET reply is bounded by the configured store payload budget; GETRANGE encodes only the selected slice. Wire tests store/read a 900 KiB value and verify an eight-byte range reply. Payload accounting is not an RSS cap. No attempt is made to allocate a huge replacement before budget admission. Expiration visibility follows the Store clock and mutation retains a live key's TTL. This is not a complete Redis string surface or production compatibility claim.

`APPEND` and `SETRANGE` preserve existing TTL/index metadata and mutate the stored
`Vec` in place. A patch within the current length needs no value-buffer allocation;
appends copy only their suffix except when amortized growth moves the buffer.
Near the payload ceiling, less spare capacity is requested, so growth can move
more frequently. `SET` retains its existing overwrite/shrink behavior. Unit tests
cover buffer reuse, bounded spare capacity, deterministic allocation failure,
eviction/accounting and journal/rewrite tracking; `tests/persistence.rs` verifies
WATCH, empty/rejected writes, TTL and incremental plus rewritten AOF restart.
These are mechanism/correctness checks, not native performance measurements.

The normal RESP request-size limit can be lower than the string limit. Native upstream behavior was probed against the repository's Redis 8.10.2 reference for malformed integers, negative offsets, empty SETRANGE and maximum size; official Redis command references: [APPEND](https://redis.io/docs/latest/commands/append/), [STRLEN](https://redis.io/docs/latest/commands/strlen/), [GETRANGE](https://redis.io/docs/latest/commands/getrange/), [SETRANGE](https://redis.io/docs/latest/commands/setrange/).
