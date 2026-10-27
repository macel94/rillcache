# Typed collections (incremental compatibility)

The store supports binary-safe lists (`LPUSH`, `RPUSH`, `LPUSHX`, `RPUSHX`, `LPOP`,
`RPOP`, `LMOVE`, `RPOPLPUSH`, `LLEN`, `LRANGE`, `BLPOP`, `BRPOP`, `BLMOVE`,
`BRPOPLPUSH`), sets, hashes and sorted sets in one
key namespace with strings and streams. Collection values live in the same key
entry as strings and share the same expiration index, payload limit, keyspace
commands and exclusive store lock. There is no shadow namespace. TYPE reports the
actual type; DEL/UNLINK/EXISTS/TOUCH/DBSIZE/expiry/RENAME/COPY apply to all types; string,
integer and bitmap commands reject collections with WRONGTYPE. MGET returns null
for non-string values, while MSET replaces any existing type, consistent with
Redis behavior.

## Supported commands

- Lists: `LPUSH`, `RPUSH`, `LPUSHX`, `RPUSHX`, `LPOP`, `RPOP`, `LMOVE`,
  `RPOPLPUSH`, `LLEN`, `LRANGE`, `LINDEX`, `LINSERT`, `LREM`, `LSET`, `LTRIM`,
  `LPOS`, `LMPOP`, `BLPOP`, `BRPOP`, `BLMOVE`, `BRPOPLPUSH`, `BLMPOP`.
- Sets: `SADD`, `SREM`, `SMEMBERS`, `SCARD`, `SISMEMBER`, `SMISMEMBER`,
  `SMOVE`, `SINTER`, `SUNION`, `SDIFF`, `SINTERCARD`, `SINTERSTORE`,
  `SUNIONSTORE`, `SDIFFSTORE`, `SPOP`, `SRANDMEMBER`.
- Hashes: `HSET`, `HSETNX`, `HGET`, `HMGET`, `HGETALL`, `HDEL`, `HGETDEL`,
  `HEXISTS`, `HLEN`, `HKEYS`, `HVALS`, `HSTRLEN`, `HINCRBY`, `HINCRBYFLOAT`,
  `HRANDFIELD`, `HGETEX`, `HSETEX`; per-field `HEXPIRE`/`HPEXPIRE`,
  `HEXPIREAT`/`HPEXPIREAT`, `HTTL`/`HPTTL`, `HEXPIRETIME`/`HPEXPIRETIME`, `HPERSIST`.
- Sorted sets: `ZADD` with `NX`, `XX`, `GT`, `LT`, `CH` and `INCR`, `ZREM`,
  `ZINCRBY`, `ZSCORE`, `ZCARD`, `ZCOUNT`, `ZRANGE` (rank, score or lex with
  `BYSCORE`/`BYLEX`, `REV`, `LIMIT` and `WITHSCORES`), `ZREVRANGE`,
  `ZRANGEBYSCORE`, `ZREVRANGEBYSCORE`, `ZRANGESTORE`, `ZLEXCOUNT`,
  `ZRANDMEMBER`, `ZRANK`, `ZREVRANK`, `ZREMRANGEBYRANK`,
  `ZREMRANGEBYSCORE`, `ZREMRANGEBYLEX`, `ZPOPMIN`, `ZPOPMAX`, `ZUNION`,
  `ZUNIONSTORE`, `ZINTER`, `ZINTERSTORE`, `ZDIFF`, `ZDIFFSTORE`,
  `ZINTERCARD` and `ZMPOP`. Geospatial
`GEOADD`/`GEOSEARCH`/`GEOSEARCHSTORE` use the same sorted-set representation and
publish `zadd`/`geosearchstore` events; see [geospatial limits](geospatial.md).

`HRANDFIELD key [count [WITHVALUES]]` returns a random field or, with count,
distinct fields for positive counts and duplicate-capable fields for negative
counts, adding values when requested. `HGETDEL key FIELDS numfields field
[field ...]` returns requested values (null for missing fields), atomically
deletes present fields, deletes an emptied hash key, and preserves remaining TTL
and payload accounting. Per-field TTL controls use `HEXPIRE`/`HPEXPIRE`,
`HEXPIREAT`/`HPEXPIREAT`, `HTTL`/`HPTTL`, `HEXPIRETIME`/`HPEXPIRETIME` and
`HPERSIST`; field deadlines are lazy and purge on hash access. `HSETEX key
[FNX|FXX] [EX|PX|EXAT|PXAT value|KEEPTTL] FIELDS numfields field value ...` sets
field/value pairs with one optional condition and expiry clause, while
`HGETEX key [EX|PX|EXAT|PXAT value|PERSIST] FIELDS numfields field ...` returns
values and optionally sets or clears their TTLs. Options may precede or follow
the counted FIELDS block. FNX requires every target field to be absent; FXX
requires every target to exist, checked before any pair is changed. Duplicate
HSETEX fields use the final value. Default HSETEX clears written field TTLs;
KEEPTTL preserves them. Default HGETEX leaves TTLs alone and PERSIST clears them.
Both preserve the hash key's own TTL and reject invalid/conflicting clauses
without partial live writes. Counts are positive signed-32-bit integers.

Nonnegative EX/PX/EXAT/PXAT values resolve to absolute deadlines no larger than
`(2^48 - 1) >> 2` milliseconds, matching the verified Redis field-expiry ceiling.
A deadline at or before now takes effect immediately: HSETEX deletes its target
fields without retaining supplied values; HGETEX returns each field's value then
deletes it, so a later duplicate returns null. Untouched fields and TTLs remain;
removing the last field removes the key and reclaims its payload.

HGETEX stages at most a **2 MiB complete RESP reply**, including framing and
repeated values, before changing live TTLs. Larger results return one
`ERR HGETEX reply exceeds configured limit` error with no partial array or TTL
updates. This is an intentional bounded-I/O compatibility limit, not an RSS
ceiling. Arbitrary unknown option bytes are omitted from errors. All-mode wire
tests cover WATCH/MULTI, hash ACL categories and journal restart; injected-clock
regressions cover exact deadlines, OOM, FNX races and reply amplification.
The semantic reference is pinned Redis
[`t_hash.c`](https://github.com/redis/redis/blob/498ecd0d6d007db11ddb3aea9428552598a78622/src/t_hash.c)
and [`ebuckets.h`](https://github.com/redis/redis/blob/498ecd0d6d007db11ddb3aea9428552598a78622/src/ebuckets.h).
This does not claim the entire original upstream hash suite or vendor event parity.

Mutation commands stage their complete replacement value and check the configured
key/value payload budget before publishing; failed writes preserve the previous
value and TTL. The payload budget counts key bytes plus binary member bytes, hash
field/value bytes and sorted-set member bytes. It excludes allocator, map, index
and numeric-score representation overhead and is not an RSS bound. Collection
size is additionally bounded to one million elements per key to prevent
unbounded zero-byte-element growth. Empty collections are not retained.

SMEMBERS, HGETALL, HKEYS, HVALS and set algebra replies use deterministic byte
ordering for stable output; Redis leaves iteration order unspecified. ZRANGE,
ZREVRANGE, score and lex ranges sort by score then binary member. `BYLEX` is
intended for all-equal scores (as in Redis); mixed scores retain score-first
ordering. `ZRANGESTORE` selects with the same rules, replaces the destination
atomically, clears its TTL and deletes it for an empty selection. Supported sorted-set
scores are IEEE-754 floating-point values, including `-inf` and `+inf`, but NaN
is rejected. `ZADD INCR` and `ZINCRBY` reject a NaN resulting score. Sorted-set
algebra accepts per-key weights and `SUM`/`MIN`/`MAX` aggregation for union and
intersection; a NaN intermediate or aggregate score becomes zero, matching Redis.
`ZDIFF` preserves first-set scores, `ZINTERCARD` caps with `LIMIT`, and store
variants replace the destination atomically, clear its TTL and delete it when the
result is empty. `ZMPOP numkeys key [key ...] MIN|MAX [COUNT count]` pops from the
first non-empty key, returns `[key, [[member, score], ...]]`, deletes emptied keys
and preserves remaining TTL; blocking `BZMPOP` is unsupported. `SPOP`,
`SRANDMEMBER`, `ZRANDMEMBER` and `ZPOPMIN`/`ZPOPMAX` use
bounded local randomness;
Redis does not promise a particular random member. Negative `ZRANDMEMBER` counts
request duplicates and are capped by the collection element limit.

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
WRONGTYPE. `BLMPOP timeout numkeys key [key ...] LEFT|RIGHT [COUNT count]`
immediately performs the LMPOP selection when a list is ready, otherwise waits
with the same bounded polling model and returns `[key, [element, ...]]` on
wakeup or a null array on timeout. Inside `MULTI`, BLMPOP executes the
non-blocking LMPOP once at `EXEC` after validating its timeout. A nonempty list completes immediately. Otherwise the connection is
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
records the removal and ACL key patterns are checked before blocking. `BLPOP` and
`BRPOP` are rejected inside `MULTI` rather than executing a non-blocking variant;
this is a documented divergence from Redis's immediate-null EXEC behavior.

### List element moves

`LMOVE source destination LEFT|RIGHT LEFT|RIGHT` pops one element from the named
side of `source` and pushes it onto the named side of `destination`, replying the
moved value. `RPOPLPUSH source destination` is the `RIGHT` to `LEFT` shorthand.
An absent or empty source replies null without creating the destination. A
wrong-type source or destination replies WRONGTYPE before changing either key.
`source` and `destination` may be the same key, in which case the list is rotated
in place rather than duplicated. Moving the last element deletes the source key;
a moved-into key is created without a TTL, while an existing destination keeps its
TTL and a non-emptied source keeps its own. Payload admission is checked across
both keys before the single atomic commit, so an over-budget move replies OOM and
leaves both keys unchanged.

`BLMOVE source destination LEFT|RIGHT LEFT|RIGHT timeout` and
`BRPOPLPUSH source destination timeout` apply the same move once a live source
element exists. Otherwise the connection blocks with the bounded polling model
described below and replies null on timeout; inside `MULTI` they execute once,
non-blockingly, at `EXEC` after timeout validation. Timeout `0` waits
indefinitely; a negative timeout, a non-float value or an out-of-range duration
returns an error.

Moves use the normal transaction/journal gate. Non-blocking moves are replayed
from the final key state, and blocking moves commit the same journal records and
emit Redis-compatible keyspace notifications: `lpush`/`rpush` on the destination,
`lpop`/`rpop` on the source, and a generic `del` when the source was emptied.
They do not emit a synthetic `lmove` event.

### Blocking sorted-set pops

`BZMPOP timeout numkeys key [key ...] MIN|MAX [COUNT count]` applies the same
`ZMPOP` selection and reply shape but waits for the first live non-empty listed
sorted set. Positive timeouts return a null array; `0` waits indefinitely.
Timeout parsing distinguishes non-float/NaN, negative and out-of-range values.
The blocked connection uses the same bounded polling model as list pops. Unlike
`BLPOP`/`BRPOP`, `BZMPOP` is accepted inside `MULTI` and executes its non-blocking
multi-pop equivalent at `EXEC` after timeout validation; `WATCH` conflicts abort
`EXEC` without popping. Successful blocking and immediate pops share the journal
gate and emit `zpopmin`/`zpopmax`.

Persistence tests cover journal replay after blocking list moves, blocking list
pops and blocking sorted-set pops.
All-mode wire tests cover waiting for a push, multiple-key order, left/right order, timeout nulls,
WRONGTYPE/float errors, pipelined commands after unblock, RESP3 null conversion,
idle-timeout immunity and MULTI rejection. `BZMPOP` wire tests cover immediate
pops, blocking wakeups, timeout nulls, TTL/delete-on-empty, notification
directions, counted-key ACL checks, timeout errors and non-blocking `EXEC`.

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
