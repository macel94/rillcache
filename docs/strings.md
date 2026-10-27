# String, expiration and integer semantics

Supported commands use the same bounded binary-safe RESP2 transport in every
execution mode. Names/options are ASCII case-insensitive. Values/keys are bytes,
not UTF-8. Full Redis compatibility is still a [production-readiness goal](production-readiness.md).

## Binary string length, append and ranges

`APPEND key value` appends binary bytes (creating a missing key) and returns the
resulting byte length. `STRLEN key` returns zero for a missing key. `GETRANGE key
start end` uses inclusive signed 64-bit indices and negative endpoints relative to
the string length. `SETRANGE key offset value` overwrites from a nonnegative
offset, zero-fills gaps, creates a missing key for nonempty values, and preserves
an existing TTL; an empty replacement does not create a missing key. See [string
range acceptance and limits](string-ranges.md), including its 512 MiB per-string
ceiling and configured payload-budget behavior.

## SET and related commands

- `SET key value [NX | XX | IFEQ value | IFNE value | IFDEQ digest | IFDNE digest] [GET] [KEEPTTL | EX seconds | PX milliseconds | EXAT unix-seconds | PXAT unix-milliseconds]`
- `SETNX key value`, `SETEX key seconds value`, `PSETEX key milliseconds value`
- `GET key`, `GETSET key value`, `GETDEL key`
- `GETEX key [PERSIST | EX seconds | PX milliseconds | EXAT unix-seconds | PXAT unix-milliseconds]`
- `DIGEST key`, `DELEX key [IFEQ value | IFNE value | IFDEQ digest | IFDNE digest]`, `DELIFEQ key value`
- `MSETEX numkeys key value [key value ...] [NX | XX] [EX seconds | PX milliseconds | EXAT unix-seconds | PXAT unix-milliseconds | KEEPTTL]`

NX creates only absent keys; XX overwrites only existing keys. Valkey also supports
IFEQ; IFNE/IFDEQ/IFDNE and DIGEST/DELEX are Redis extensions. Redis extended
SET conditions are mutually exclusive and test the existing binary value or its
XXH3 digest. Expired keys are absent. A plain successful SET returns OK and clears
the old TTL. GET returns the
previous bytes (or null), **including an existing value when NX rejects the
write**. GETSET clears TTL. KEEPTTL retains it; counter operations also retain it.
No admitted write or TTL update is visible partially to other workers.

Syntax, conflicting options, numeric format and timestamp overflow are checked
before SET's condition/mutation. Repeated identical NX/XX/GET/KEEPTTL are accepted;
repeated identical expiry kinds use the final argument, as upstream does.
Different expiry kinds and KEEPTTL plus an expiry conflict. Positive SET/GETEX
TTLs use canonical signed-64-bit integer syntax. Nonpositive values and conversion/
relative-time overflow fail. Past absolute SET/GETEX deadlines immediately delete
the key after applying the condition and determining the old-value reply.

An admitted payload-budget failure returns **one OOM error**, preserving the live
value and TTL, even with GET. Conditional rejection does not consume payload.
GETEX validates option grammar before lookup, but (like the references) a missing
key returns null without interpreting the expiry value. Valkey 9.1.2 differs on
that invalid-TTL/missing-key edge: it returns an integer error. Rillcache follows
Redis 8.10.2; an explicit vendor-specific reference test records the divergence
rather than pretending both references agree. GETDEL removes value and
TTL atomically. IFEQ/IFNE compare binary values atomically; IFDEQ/IFDNE compare the
XXH3-64 digest emitted by Redis DIGEST (16 hexadecimal digits, case-insensitive).
Redis DELEX performs an atomic optional value/digest comparison and removal;
Valkey DELIFEQ is its value-equal deletion form. MSETEX validates its full argument block before applying
an atomic deduplicated batch with shared expiration, NX/XX, KEEPTTL and exact payload
admission. Past absolute batch deadlines leave the targets absent. Rejected
conditions return null without GET, or the previous value with GET. Digest
validation follows Redis: live strings require exactly 16 bytes (nonhex bytes
simply fail equality); absent keys bypass digest validation. String comparisons
and DIGEST reject existing non-string values; unconditional DELEX deletes any
supported type. Conditional DELEX uses Redis's specific type error, whereas
DELIFEQ returns WRONGTYPE.

By default MSETEX follows Valkey's generic `ERR syntax error` for an argument
list shorter than `2 + 2*numkeys`, including the overflow-protection counts.
Operators can select `--error-dialect redis` to return Redis's
`ERR wrong number of key-value pairs` for that short counted pair block, or
explicitly retain the default with `--error-dialect valkey`. Library users set
`Config.error_dialect` or build a `Store` with `with_error_dialect` and the public
`engine::ErrorDialect` enum. The setting is immutable after startup; malformed
names fail startup rather than selecting an automatic mode. Malformed or
conflicting options remain `ERR syntax error` in both dialects. Only a genuinely invalid numkeys value
(`0`, negative, non-canonical integer, or greater than `i32::MAX`) returns
`ERR invalid numkeys value or out of range`, and fewer than four arguments
return the command arity error. Redis 8.10.2 instead answers
`ERR wrong number of key-value pairs` for a short counted pair block;
the explicit startup setting lets applications choose either verified wording
without changing numeric validation, successful commands, TTLs, atomicity,
transaction behavior or other command errors. It is not a full Redis/Valkey
compatibility mode: for example the GETEX missing-key divergence above is
unchanged. Every rejected form is rejected without writes. GET and value/digest
conditions are SET-only and rejected by MSETEX. MSETEX notifications are described
in [keyspace notifications](notifications.md); replication remains unsupported.

## Expiration

- `TTL key`, `PTTL key`, `EXPIRETIME key`, `PEXPIRETIME key`
- `EXPIRE key seconds [NX | XX | GT | LT]`
- `PEXPIRE key milliseconds [NX | XX | GT | LT]`
- `EXPIREAT key unix-seconds [NX | XX | GT | LT]`
- `PEXPIREAT key unix-milliseconds [NX | XX | GT | LT]`
- `PERSIST key` removes expiry, not the value.

TTL queries return -2 when missing and -1 when persistent. Seconds are rounded
with Redis's `(milliseconds + 500) / 1000` rule, without overflowing the addition.
NX requires no previous expiry; XX requires one; GT/LT compare deadlines, treating
persistent keys as infinitely lived. XX can combine with GT or LT; NX cannot.
Nonpositive relative durations/past absolute deadlines delete admitted keys.
Invalid options and numeric/overflow errors preserve the previous live state.
Unsupported option error text intentionally does not echo arbitrary input bytes.

The injectable `Clock` supplies Unix milliseconds. The default follows system
wall time (not a monotonic duration), as Redis does. Existing-key visibility uses
`now > deadline`; a zero TTL can be observable for one millisecond. Explicit past
expiration commands use `deadline <= now`. Once a read observes expiry, it is
reclaimed so clock rollback cannot resurrect it. Socket inactivity uses Instant
and is independent of wall time.

One ordered expiry index entry exists per expiring key; replacement, PERSIST,
deletion and expiration remove old metadata. Both server modes attempt bounded
128-key reclamation every 100 ms. Lookups enforce expiry immediately regardless
of that housekeeping. Payload admission reclaims all due entries when necessary,
so logically expired bytes cannot cause false OOM. This budget excludes expiry
index/map/allocator/thread/socket overhead and is **not an RSS bound or eviction
policy**. Large reclamation bursts and wall-clock adjustments need operational
measurement before production use.

## Atomic signed-64-bit counters

`INCR key`, `DECR key`, `INCRBY key amount`, `DECRBY key amount` use zero for a
missing/expired key. Canonical base-10 signed 64-bit integers only: no leading
zeros, plus sign, `-0`, whitespace, floating point, trailing bytes or overflow.
DECRBY rejects the minimum i64 operand before negation, as upstream does.
Parsing, arithmetic and payload errors retain the old live value and TTL. An
existing expiry is retained on success; newly created counters are persistent.

Offline injected-clock tests cover boundaries, rollback, rejection, budget and
metadata invariants without sleeps. Many-writer unit/socket tests cover shared
counters in all worker modes. Optional pinned Redis/Valkey wire conformance runs
before native comparisons; it uses future absolute deadlines rather than
sleep-dependent TTL assertions.

Sources: [SET](https://redis.io/docs/latest/commands/set/), [DIGEST](https://redis.io/docs/latest/commands/digest/), [DELEX](https://redis.io/docs/latest/commands/delex/), [MSETEX](https://redis.io/docs/latest/commands/msetex/),
[EXPIRE](https://redis.io/docs/latest/commands/expire/),
[TTL](https://redis.io/docs/latest/commands/ttl/),
[INCR](https://redis.io/docs/latest/commands/incr/),
[Valkey SET](https://valkey.io/commands/set/).
