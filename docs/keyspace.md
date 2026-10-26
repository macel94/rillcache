# Keyspace inspection and atomic rename

## Acceptance criteria (EARS)

- When `TYPE key` is received, the server shall return the actual string/list/set/
  hash/zset type for a live key and `none` for a missing or expired key.
- When `RENAME source destination` is received, the server shall atomically move a
  live source over the destination, retaining the source expiry. A missing source
  shall return `ERR no such key` without changing another key.
- When `RENAMENX source destination` is received, the server shall move only if
  the destination is absent, return integer 0 if occupied and 1 if moved, and
  report a missing source as an error.
- If a rename cannot fit the configured key/value payload budget, then it shall
  return OOM and preserve both source and destination and their accounting.
- When `TOUCH key [key ...]` is received, the server shall count each existing
  argument (including repeated names) and shall not claim LRU behavior.
- When `DBSIZE` is received, the server shall remove all expired entries and
  return the live key count.
- If any command has invalid arity, then it shall return the standard command
  arity error without mutation. Key bytes are binary-safe.

## Actual semantics and limits

Strings, lists, sets, hashes and sorted sets share one key namespace. TYPE,
rename, TOUCH and DBSIZE run under the store's exclusive lock so expiration
visibility and mutation are consistent with other commands. See
[typed collection limits](collections.md) for supported command subsets. RENAME replacement and payload admission are atomic; source TTL is
transferred to the destination. A rename to itself returns `+OK` for RENAME and
integer 0 for RENAMENX when the source exists. Missing-source errors use
`-ERR no such key`.

All operations are O(1) expected per key except TOUCH, which is O(N) in its
argument count, and DBSIZE, which is O(K + E) here: it reaps E expired entries
and then counts K remaining live entries. RENAME is expected O(1) hash-map work,
plus expiry-index and replaced-value cleanup. Large destination reclamation can
therefore take time. The payload limit accounts for key and value bytes only,
not allocator/map capacity, thread buffers, or process RSS. DBSIZE is one
in-memory database only; nonzero SELECT indices are not implemented. No last-access metadata or
eviction policy is implemented, so TOUCH is a no-op on access metadata while
reporting existence accurately.

## Evidence

`tests/keyspace.rs` exercises RESP replies with binary keys, duplicates, overwrite
and all managed execution modes. Unit tests in `src/engine/keyspace.rs` cover
injected-clock expiry, accounting, OOM atomicity, self-renames, missing/occupied
keys and untouched output for unknown commands. Official Redis command references:
[TYPE](https://redis.io/docs/latest/commands/type/),
[RENAME](https://redis.io/docs/latest/commands/rename/),
[RENAMENX](https://redis.io/docs/latest/commands/renamenx/),
[TOUCH](https://redis.io/docs/latest/commands/touch/), and
[DBSIZE](https://redis.io/docs/latest/commands/dbsize/).
