# Runtime clock and object introspection

Rillcache exposes the server wall clock and the actual storage representation
selected for a key. It does not invent Redis-internal metrics that the store does
not maintain.

## TIME

```text
TIME
```

Returns a two-element array of bulk strings: Unix seconds and the microsecond
component. The value comes from the system wall clock and is not monotonic; clock
adjustments are visible to clients. TIME is read-only and does not touch the
keyspace.

## OBJECT

```text
OBJECT ENCODING key
OBJECT REFCOUNT key
OBJECT FREQ key
OBJECT IDLETIME key
OBJECT HELP
```

- `ENCODING` returns the actual representation: `raw` for strings, `listpack` or
  `quicklist` for lists, `intset`/`listpack`/`hashtable` for sets,
  `listpack`/`hashtable` for hashes, `listpack`/`skiplist` for sorted sets and
  `stream` for streams. Rillcache does not track Redis's `embstr`/integer string
  encodings, so all strings report `raw`.
- `REFCOUNT` returns `1`: values are uniquely owned rather than shared by Redis's
  integer-object cache.
- `FREQ` returns the same LFU-not-selected error as Redis because Rillcache has no
  LFU policy or access-frequency counter.
- `IDLETIME` returns an explicit unsupported error. Rillcache's eviction LRU is
  mutation recency, not a read-idle clock, so a numeric idle time would be
  fabricated.
- `HELP` returns the subcommand help list.
- Missing or expired keys return a null bulk reply for every key-taking
  subcommand. Unknown subcommands and wrong arities return the Redis-shaped
  `OBJECT HELP` error.

ACL discovery maps both commands to the generic/read categories; `OBJECT` checks
the key argument at position 2 for key-pattern rules.

## Limits

TIME is a wall-clock observation, not a latency/CPU measurement. OBJECT reports
Rillcache's own representation names and does not expose allocator, RSS, radix
tree or shared-reference statistics. LFU frequency, true per-key read-idle time
and Redis-internal encoding labels remain unsupported and are reported as errors
or documented differences rather than fabricated values.
