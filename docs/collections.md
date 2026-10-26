# Typed collections (incremental compatibility)

The store supports binary-safe lists (`LPUSH`, `RPUSH`, `LPOP`, `RPOP`, `LLEN`,
`LRANGE`, `BLPOP`, `BRPOP`), sets (`SADD`, `SMEMBERS`, `SCARD`, `SISMEMBER`), hashes (`HSET`,
`HGET`, `HGETALL`, `HLEN`), and sorted sets (`ZADD`, `ZRANGE`, `ZSCORE`,
`ZCARD`). Collection values live in the same key entry as strings and share the
same expiration index, payload limit, keyspace commands and exclusive store lock.
There is no shadow namespace. TYPE reports the actual type; DEL/EXISTS/TOUCH/
DBSIZE/expiry/RENAME apply to all types; GET, integer operations and string range
commands reject collections with WRONGTYPE. MGET returns null for non-string
values, while MSET replaces any existing type, consistent with Redis behavior.

Collection mutations stage their result and check the configured key/value
payload budget before publishing; failed writes preserve the previous value and
TTL. The payload budget counts binary member bytes, hash field/value bytes and
sorted-set member bytes, plus key bytes. It excludes allocator, map, index and
numeric-score representation overhead and is not an RSS bound. Collection size
is additionally bounded to one million elements per key to prevent unbounded
zero-byte-element growth. Empty collections are not retained.

SMEMBERS and HGETALL use deterministic byte ordering for stable replies; Redis
leaves these iteration orders unspecified. ZRANGE sorts by ascending score then
binary member. Supported sorted-set scores are finite UTF-8 floating-point
numbers; ZADD options, reverse/lex/score ranges, rank/range removals and other
collection commands are not implemented. Deep COPY preserves the type, independent contents and key TTL of supported
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

Unit tests cover binary data, ordering, duplicate members/fields, score updates,
invalid score handling, WRONGTYPE, OOM atomicity, TTL preservation, keyspace
rename and string replacement. `tests/keyspace.rs` exercises list/set/hash/zset
commands through RESP in managed per-client, one-worker and four-worker modes.
