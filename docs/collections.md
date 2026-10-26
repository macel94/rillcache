# Typed collections (incremental compatibility)

The store supports binary-safe lists (`LPUSH`, `RPUSH`, `LPUSHX`, `RPUSHX`, `LPOP`,
`RPOP`, `LLEN`, `LRANGE`, `BLPOP`, `BRPOP`), sets, hashes and sorted sets in one
key namespace with strings and streams. Collection values live in the same key
entry as strings and share the same expiration index, payload limit, keyspace
commands and exclusive store lock. There is no shadow namespace. TYPE reports the
actual type; DEL/EXISTS/TOUCH/DBSIZE/expiry/RENAME/COPY apply to all types; string,
integer and bitmap commands reject collections with WRONGTYPE. MGET returns null
for non-string values, while MSET replaces any existing type, consistent with
Redis behavior.

## Supported commands

- Lists: `LPUSH`, `RPUSH`, `LPUSHX`, `RPUSHX`, `LPOP`, `RPOP`, `LLEN`,
  `LRANGE`, `LINDEX`, `LINSERT`, `LREM`, `LSET`, `LTRIM`, `BLPOP`, `BRPOP`.
- Sets: `SADD`, `SREM`, `SMEMBERS`, `SCARD`, `SISMEMBER`, `SMISMEMBER`,
  `SMOVE`, `SINTER`, `SUNION`, `SDIFF`, `SINTERCARD`, `SINTERSTORE`,
  `SUNIONSTORE`, `SDIFFSTORE`, `SPOP`, `SRANDMEMBER`.
- Hashes: `HSET`, `HSETNX`, `HGET`, `HMGET`, `HGETALL`, `HDEL`, `HEXISTS`,
  `HLEN`, `HKEYS`, `HVALS`, `HSTRLEN`, `HINCRBY`, `HINCRBYFLOAT`.
- Sorted sets: `ZADD` with `NX`, `XX`, `GT`, `LT`, `CH` and `INCR`, `ZREM`,
  `ZINCRBY`, `ZSCORE`, `ZCARD`, `ZCOUNT`, `ZRANGE`, `ZREVRANGE`,
  `ZRANGEBYSCORE`, `ZREVRANGEBYSCORE`, `ZRANK`, `ZREVRANK`,
  `ZREMRANGEBYRANK`, `ZREMRANGEBYSCORE`, `ZPOPMIN`, `ZPOPMAX`.

Mutation commands stage their complete replacement value and check the configured
key/value payload budget before publishing; failed writes preserve the previous
value and TTL. The payload budget counts key bytes plus binary member bytes, hash
field/value bytes and sorted-set member bytes. It excludes allocator, map, index
and numeric-score representation overhead and is not an RSS bound. Collection
size is additionally bounded to one million elements per key to prevent
unbounded zero-byte-element growth. Empty collections are not retained.

SMEMBERS, HGETALL, HKEYS, HVALS and set algebra replies use deterministic byte
ordering for stable output; Redis leaves iteration order unspecified. ZRANGE,
ZREVRANGE and score ranges sort by score then binary member. Supported sorted-set
scores are IEEE-754 floating-point values, including `-inf` and `+inf`, but NaN
is rejected. `ZADD INCR` and `ZINCRBY` reject a NaN resulting score. `SPOP`,
`SRANDMEMBER` and `ZPOPMIN`/`ZPOPMAX` use bounded local randomness; Redis does not
promise a particular random member.

Deep COPY preserves the type, independent contents and key TTL of supported
collections. KEYS and RANDOMKEY operate over this same keyspace; glob evaluation
is byte-oriented and bounded against pathological patterns. CONFIG GET/SET exposes
mutable listpack/intset thresholds and the legacy list/hash/zset ziplist aliases.
These settings choose actual storage representations: packed versus chunked lists,
integer/packed/tree sets, packed versus map-backed hashes, and packed versus
map-backed sorted sets. A subsequent collection write re-evaluates its encoding;
changing CONFIG does not rewrite existing values until they are mutated. The
accounting scope excludes representation overhead. This is not full Redis
compatibility.

### Blocking list pops

`BLPOP key [key ...] timeout` and `BRPOP ...` check keys in argument order and pop
from the first live nonempty list, replying `[key, value]`; a wrong-type key returns
WRONGTYPE. A nonempty list completes immediately. Otherwise the connection is
marked blocked and no reply is written until data appears or the float-seconds
timeout expires (the null array, or RESP3 null). Timeout `0` waits indefinitely;
negative, NaN, infinite or out-of-range values return an error, as do missing keys
or an invalid arity.

A blocked connection is polled about every 10 ms by its owning worker/lane. It does
not hold the store lock while waiting, does not apply the ordinary socket
inactivity timeout, and does not parse further pipelined commands until it
unblocks; those bytes remain in the bounded input buffer and execute afterwards in
order. This is bounded active polling, not a condition-variable wakeup, push
notification or strict latency guarantee. A disconnect or shutdown closes the
connection and drops its blocked state. `BLPOP`/`BRPOP` take the normal
transaction/journal gate only while attempting the pop, so an append-only journal
records the removal and ACL key patterns are checked before blocking. Blocking
commands are rejected inside `MULTI` rather than executing a non-blocking variant;
this is a documented divergence from Redis's immediate-null EXEC behavior.

Unit tests cover journal replay after a blocking pop. All-mode wire tests cover
waiting for a push, multiple-key order, left/right order, timeout nulls,
WRONGTYPE/float errors, pipelined commands after unblock, RESP3 null conversion,
idle-timeout immunity and MULTI rejection.

## Limits and atomicity

- Index and count parsing follows canonical signed 64-bit integer syntax. Negative
  list/range indexes count from the end; out-of-range reads return null or an
  empty array, while `LSET` reports `ERR index out of range`.
- Removing the final element or field deletes the key and its expiry metadata.
  `SINTERSTORE`/`SUNIONSTORE`/`SDIFFSTORE` replace the destination and clear its
  TTL; an empty result deletes the destination.
- Multi-key set algebra and `SMOVE` validate every source type and the complete
  resulting payload before changing either key, so a wrong type or OOM leaves all
  inputs and the destination unchanged.
- `HINCRBY`, `HINCRBYFLOAT` and `ZINCRBY` preserve an existing TTL and reject
  integer overflow or non-finite results without changing the stored value.
- `ZADD NX` and `XX` filters and `GT`/`LT` score filters are applied per member;
  `CH` counts added-or-changed members. `INCR` accepts one score/member pair and
  returns a null bulk when the condition rejects the update.

Unit tests cover binary data, ordering, duplicates, score updates, invalid
numbers, WRONGTYPE, OOM atomicity, TTL preservation, keyspace rename and string
replacement. `tests/collections.rs` exercises representative hash/list/set/zset
mutations, errors and a transaction through RESP in managed per-client,
one-worker and four-worker modes.
