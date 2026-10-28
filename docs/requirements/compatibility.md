# Compatibility boundary (v0.1)

Part of the [requirements and incremental delivery index](../requirements.md). Requirements are preserved verbatim from the former monolithic EARS sections; add new requirements only in the matching area file.

Supported: RESP2 nonempty arrays of non-null bulk strings and Redis-style inline
commands with whitespace, quotes and escapes, case-insensitive ASCII command names,
binary keys/values (including escaped inline `\xHH` bytes), multiple connections
and ordered mixed-protocol pipelines; `PING`, `ECHO`, `SET` with NX/XX/GET/KEEPTTL/
EX/PX/EXAT/PXAT and IFEQ (plus Redis IFNE/IFDEQ/IFDNE); `GET`, `DEL`, `EXISTS`,
`QUIT`; SETEX/PSETEX/SETNX/GETSET/GETDEL/GETEX; TTL/PTTL/EXPIRETIME/
PEXPIRETIME, EXPIRE/PEXPIRE/EXPIREAT/PEXPIREAT with NX/XX/GT/LT, PERSIST;
INCR/DECR/INCRBY/DECRBY/INCRBYFLOAT; MGET/MSET/MSETNX/MSETEX;
MULTI/EXEC/DISCARD/WATCH/UNWATCH with a bounded queue, atomic batch execution
and optimistic key invalidation; multi-user ACL users/rules, read/write key
patterns, selectors and atomic ACL-file save/load; opt-in RESP-command append-only persistence with
fsync policies, restart recovery and synchronous `BGREWRITEAOF` compaction;
payload-budget eviction policies (`noeviction`, allkeys/volatile random/LRU and
volatile TTL); keyspace/keyevent notifications for supported mutation classes;
ACL GENPASS/DRYRUN and a bounded denial log; Redis DIGEST/DELEX
and Valkey DELIFEQ; APPEND/STRLEN/GETRANGE/SETRANGE/SUBSTR; SETBIT/GETBIT/
BITCOUNT/BITPOS/BITOP/BITFIELD/BITFIELD_RO; LCS (LEN/IDX/WITHMATCHLEN/
MINMATCHLEN); lists, sets, hashes and sorted sets with
creation, mutation, removal, bounded random selection, set algebra/store
commands, LPOS/LMPOP list position/multi-pop, score/rank/lex ranges, range storage,
random members, sorted-set
algebra/cardinality/multi-pop, blocking multi-pop, pops, bounded
`BLPOP`/`BRPOP`, stream `XSETID` metadata control and consumer history reads
documented
in [typed collections](../collections.md); geospatial sorted sets (`GEOADD`,
`GEOPOS`, `GEODIST`, `GEOHASH`, `GEOSEARCH`, `GEOSEARCHSTORE`) documented in
[geospatial limits](../geospatial.md); SCAN/HSCAN/SSCAN/ZSCAN bounded cursor
iteration with MATCH/COUNT/TYPE/NOVALUES; PFADD/PFCOUNT/PFMERGE approximate
cardinality; PUBLISH/SPUBLISH, SUBSCRIBE/PSUBSCRIBE/SSUBSCRIBE,
UNSUBSCRIBE/PUNSUBSCRIBE/SUNSUBSCRIBE and PUBSUB introspection.
See [string semantics](../strings.md), [string ranges](../string-ranges.md),
[bitmap/LCS/float limits](../string-algorithms.md), [multi-key string limits](../multikey-strings.md),
[typed collections](../collections.md), [HyperLogLog](../hyperloglog.md),
[transactions](../transactions.md), [security](../security.md),
[persistence](../persistence.md) and [pub/sub](../pubsub.md)
for grammar,
clocks, rounding, integer limits, resource bounds, queue isolation and atomic
error behavior. Conditions are atomic and
case-insensitive. Option tokens must match completely; NULs/prefixes are rejected.
A bounded stream subset supports XADD with NOMKSTREAM/MAXLEN/MINID retention,
XDEL, XLEN, XGROUP CREATE/SETID/DESTROY/
CREATECONSUMER/DELCONSUMER, XREADGROUP GROUP with COUNT/NOACK for new entries on
one or more streams with new-entry and history reads, XACK, XPENDING summary/extended,
XRANGE/XREVRANGE, non-blocking and blocking XREAD over multiple streams,
XCLAIM/XAUTOCLAIM, XTRIM, XSETID, blocking group reads, and XINFO STREAM (with
FULL)/GROUPS/CONSUMERS. See [stream limits](../streams.md). All types share expiry, payload
accounting, TYPE, RENAME and deep same-database COPY; byte-oriented KEYS and
RANDOMKEY inspect that same keyspace. See [keyspace limits](../keyspace.md).
CONFIG GET/SET collection thresholds select actual collection representations;
other runtime settings remain unsupported. FLUSHDB/FLUSHALL clear all types,
CONFIG RESETSTAT retains its empty-statistics behavior, and MEMORY USAGE reports
a deterministic logical estimate, not allocator/RSS measurements.

The default wire mode is RESP2. `HELLO` can inspect the current mode or
negotiate RESP2/RESP3, and the connection retains its negotiated mode and common
`CLIENT` metadata across worker handoffs. Supported session commands are `CLIENT
ID`, `SETNAME`, `GETNAME`, `SETINFO LIB-NAME|LIB-VER`, `INFO`, `SELECT 0` and
`RESET`.
The RESP3 reply subset covers the current commands' simple strings, errors,
integers, bulk strings/arrays, and nulls; null bulk and null array frames are
converted by frame boundaries, preserving binary payloads. See
[client sessions](../client-sessions.md) for limits and response details. Real
redis-cli/library handshake interoperability is not yet verified.

Deliberately not supported: empty/null request arrays, other RESP3 types and
pushes, commands beyond the documented collection/stream subsets, nonzero SELECT
indices, TLS hardening (SNI/hot reload/OCSP), RDB/AOF-rewrite compatibility, replication, clustering,
sharded pub/sub, keyspace notifications and scripts. Cluster mode,
multi-database, and arbitrary Redis AOF/RDB files are not accepted.
Single-password and multi-user ACL-file authentication are supported with
`AUTH`, `HELLO ... AUTH`, `ACL` and `CONFIG SET requirepass`; ACL selectors,
read/write key modes, first-argument command rules, `ACL LOG`/`DRYRUN`/`GENPASS`
and pub/sub channel enforcement are implemented. Subcommand-specific categories,
collector sender authentication/TLS/federation and global/cross-process rate limiting remain absent. See
[security](../security.md).
Connection-local commands are rejected rather than queued inside MULTI. Unsupported SET options return
`ERR syntax error` and do not write. Redis normally ignores empty request arrays;
this strict initial subset rejects them. Unknown-command and unsupported-expire-
option errors omit arbitrary input bytes rather than reflecting them into RESP
error frames. Conservative defaults remain smaller than Redis limits.

Default limits: 128 clients, 30-second socket inactivity timeout, 1 MiB bulk,
1..=1024-byte password (via flag or file; ACL files add hashed users),
2 MiB complete request frame, 1024 arguments, 64 MiB key/value payload and a 2 MiB
maximum MGET or HGETEX RESP reply. CLI `--max-bulk-bytes` and `--max-frame-bytes` may raise
request bounds up to 512 MiB (bulk must fit within frame); larger requests remain
finite and grow buffers only as bytes arrive. Compatibility-only upstream runs
match native and candidate at 8 MiB bulk and 16 MiB frame; MGET remains capped at
2 MiB. Over-limit replies return `ERR MGET reply limit exceeded`
without a partial array. Payload accounting excludes map capacity, allocator and
connection/thread buffers.
The CLI uses loopback by default. Do not expose it to untrusted clients.
