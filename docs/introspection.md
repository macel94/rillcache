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

## MEMORY

```text
MEMORY USAGE key
MEMORY STATS
MEMORY DOCTOR
MEMORY HELP
```

`MEMORY USAGE key` returns the deterministic logical entry-size estimate used by
the store: a fixed header, the key length and the payload length. It is not RSS
or allocator data.

`MEMORY STATS` returns a flat RESP2 map with these measured fields:

- `peak.allocated`: process-lifetime high-water payload bytes, retained across
  deletes, `FLUSHALL` and `CONFIG RESETSTAT`;
- `total.allocated` and `dataset.bytes`: current key-plus-value payload bytes;
- `keys.count`: live keys (logically expired keys are excluded);
- `dataset.percentage`: current payload as a percentage of the configured
  payload limit;
- `rillcache.payload.limit_bytes`: the live `maxmemory` byte budget;
- `rillcache.payload.scope`: `key_and_value_bytes_not_RSS`.

Byte fields are payload currently held and can still include an expired entry
awaiting lazy reclamation, so `dataset.bytes` and `keys.count` measure different
things. There are no allocator active/resident, fragmentation, RSS, Lua or
replication-buffer fields; they are omitted rather than reported as zero.

`MEMORY DOCTOR` returns a bounded plain-text report derived from the same
snapshot. It distinguishes an empty payload, high pressure at or above 90% with
`noeviction`, pressure at or above 75%, and a healthy state, and always states
that the report is payload accounting rather than RSS or allocator health.
`MEMORY HELP` lists the supported subcommands.

## Limits

TIME is a wall-clock observation, not a latency/CPU measurement. OBJECT reports
Rillcache's own representation names and does not expose allocator, RSS, radix
tree or shared-reference statistics. LFU frequency, true per-key read-idle time
and Redis-internal encoding labels remain unsupported and are reported as errors
or documented differences rather than fabricated values. MEMORY reports payload
accounting only; it is not an RSS, allocator-fragmentation, background-eviction or
production memory-admission claim.
