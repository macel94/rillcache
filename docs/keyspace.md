# Keyspace inspection and atomic rename

## Acceptance criteria (EARS)

- When `TYPE key` is received, the server shall return the actual string/list/set/
  hash/zset/stream type for a live key and `none` for a missing or expired key.
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
- When `COPY source destination [DB 0] [REPLACE]` is received, the server shall
  make an independent deep copy of any supported value and its expiry metadata,
  respect replacement and payload-budget rules atomically, and reject unsupported
  databases.
- When `KEYS pattern` is received, the server shall match binary key bytes using
  Redis-style `*`, `?`, escapes and character classes, while bounding pathological
  wildcard work.
- When `RANDOMKEY` is received, the server shall return a live key or a null bulk
  reply for an empty database; it shall not consistently select the first key.
- If any command has invalid arity, then it shall return the standard command
  arity error without mutation. Key bytes are binary-safe.

## Actual semantics and limits

Strings, lists, sets, hashes, sorted sets and streams share one key namespace. TYPE, rename, TOUCH, DBSIZE, COPY, KEYS and RANDOMKEY run under the store's
exclusive lock so expiration visibility and mutation are consistent with other
commands. See [typed collection limits](collections.md) for supported command
subsets. COPY supports only DB 0 in this single-database server; it clones typed
collection contents, stream entries/groups and TTL, and destination replacement plus payload admission
are atomic. RENAME replacement and payload admission are atomic; source TTL is
transferred to the destination. A rename to itself returns `+OK` for RENAME and
integer 0 for RENAMENX when the source exists. Missing-source errors use
`-ERR no such key`.

All operations are O(1) expected per key except TOUCH, which is O(N) in its
argument count; DBSIZE, KEYS and RANDOMKEY scan/reap the keyspace in O(K + E)
where K is live keys and E is expired entries. KEYS uses bounded iterative
backtracking and returns no match after its 4,000,000-step budget or for more
than 1,000 separated wildcard groups. COPY is O(payload size) to deep-clone the
value. RENAME is expected O(1) hash-map work, plus expiry-index and replaced-value
cleanup. Large destination reclamation can therefore take time. The payload limit accounts for key and value bytes only,
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
[TOUCH](https://redis.io/docs/latest/commands/touch/),
[DBSIZE](https://redis.io/docs/latest/commands/dbsize/),
[COPY](https://redis.io/docs/latest/commands/copy/),
[KEYS](https://redis.io/docs/latest/commands/keys/), and
[RANDOMKEY](https://redis.io/docs/latest/commands/randomkey/).
