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
- When RENAME/RENAMENX is rejected, the server shall not clone the source value;
  after admission it shall transfer the owned string buffer or collection/stream
  tree without copying it or changing its retained capacities.
- When COPY is admitted, the server shall create an independent value after
  checking destination rejection and final payload admission, retaining source TTL.
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

Strings, lists, sets, hashes, sorted sets and streams share one key namespace.
TYPE, rename, TOUCH, DBSIZE, COPY, KEYS and RANDOMKEY run under the store's
exclusive lock so expiration visibility and mutation are consistent with other
commands. See [typed collection limits](collections.md) and the
[bounded stream subset](streams.md) for supported commands.

COPY supports only DB 0 in this single-database server; it clones collection
contents, stream entries/groups and TTL only after admission, without an
intermediate second string copy. Destination replacement and payload admission
are atomic. RENAME moves the original owned entry and its TTL. Replaced
destination state and its expiry index entry are removed atomically. A rename
to itself returns `+OK` for RENAME and integer 0 for RENAMENX when the source
exists. Missing-source errors use `-ERR no such key`.

TOUCH is O(N) in its argument count. DBSIZE, KEYS and RANDOMKEY scan/reap the
keyspace, with ordered expiry-index maintenance; KEYS additionally sorts matched
keys and performs glob work per key. Glob backtracking and character-class scans
share a 4,000,000-step budget per match; more than 1,000 separated wildcard groups
or exhausting that budget returns no match. COPY is O(payload size) to deep-clone
the value, plus due-key reclamation when needed for admission. RENAME is expected
O(1) hash-map work, plus expiry-index and replaced-value cleanup. Large destination
reclamation can therefore take time. The payload limit accounts for key and value
bytes only, not allocator/map capacity, thread buffers, or process RSS. DBSIZE is
one in-memory database only; only SELECT 0 is supported. No last-access metadata
or eviction policy is implemented, so TOUCH reports existence accurately without
updating access metadata.

## Evidence

`tests/keyspace.rs` exercises binary keys, duplicates, replacement, typed COPY,
CONFIG, mixed-type MGET, string guards, FLUSHALL and stream RESP boundaries in
all managed execution modes. Unit tests in `src/engine/keyspace.rs` cover
injected-clock expiry, accounting, OOM atomicity, self-renames, missing/occupied
keys, long/pathological glob inputs and untouched output for unknown commands.
Additional tests verify retained string/nested-stream capacities across ownership
transfer, exact-fit admission, NX/OOM rejection, expiry-index replacement and
independent COPY mutation. Official Redis command references:
[TYPE](https://redis.io/docs/latest/commands/type/),
[RENAME](https://redis.io/docs/latest/commands/rename/),
[RENAMENX](https://redis.io/docs/latest/commands/renamenx/),
[TOUCH](https://redis.io/docs/latest/commands/touch/),
[DBSIZE](https://redis.io/docs/latest/commands/dbsize/),
[COPY](https://redis.io/docs/latest/commands/copy/),
[KEYS](https://redis.io/docs/latest/commands/keys/), and
[RANDOMKEY](https://redis.io/docs/latest/commands/randomkey/).
