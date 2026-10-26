# Binary string ranges (incremental compatibility)

## EARS acceptance criteria

- **When** APPEND receives a key and binary value, the server shall atomically append and return the resulting byte length; absent keys are created, including for an empty append.
- **When** STRLEN receives an absent key, it shall return integer zero; existing lengths count bytes, not Unicode characters.
- **When** GETRANGE receives signed inclusive endpoints, it shall normalize negative endpoints relative to the byte length, clamp endpoints as Redis does, and return an empty bulk for missing/out-of-range/reversed ranges. Both endpoints are parsed before lookup; signed 64-bit extrema must not overflow normalization.
- **When** SETRANGE receives a nonempty value at a valid nonnegative offset, it shall overwrite/extend atomically, zero-fill gaps, return the new byte length, and retain any existing expiry. Missing values start empty.
- **When** SETRANGE receives an empty value, it shall return the existing length (or zero) without creating a missing key; the offset still must parse and be nonnegative.
- **If** the offset is negative, then Redis's `offset is out of range` error is returned. If an integer is malformed/outside signed 64-bit range, then the standard integer error is returned. If the resulting string exceeds 512 MiB, then the upstream maximum-string error is returned; configured key/value budget failures return this project's OOM error.
- **If** admission or fallible temporary reservation fails, then prior data/TTL remain unchanged. Payload reservation precedes allocating the replacement buffer; mutation and accounting occur under one write lock.
- **When** commands arrive concurrently through managed per-client, one-worker or four-worker servers, updates remain atomic and other command dispatch is unaffected.

## Limits and status

This increment implements only APPEND, STRLEN, GETRANGE, and SETRANGE. SUBSTR is intentionally omitted. Strings are binary-safe. Redis's 512 MiB string ceiling is enforced independently of the configured payload budget (64 MiB by default). A stored value/full GET reply is bounded by the configured store payload budget; GETRANGE encodes only the selected slice. Wire tests store/read a 900 KiB value and verify an eight-byte range reply. Payload accounting is not an RSS cap. No attempt is made to allocate a huge replacement before budget admission. Expiration visibility follows the Store clock and mutation retains a live key's TTL. This is not a complete Redis string surface or production compatibility claim.

The normal RESP request-size limit can be lower than the string limit. Native upstream behavior was probed against the repository's Redis 8.10.2 reference for malformed integers, negative offsets, empty SETRANGE and maximum size; official Redis command references: [APPEND](https://redis.io/docs/latest/commands/append/), [STRLEN](https://redis.io/docs/latest/commands/strlen/), [GETRANGE](https://redis.io/docs/latest/commands/getrange/), [SETRANGE](https://redis.io/docs/latest/commands/setrange/).
