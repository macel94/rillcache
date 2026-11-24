# Optional native baseline conformance

`tests/reference_baseline.rs` is an independent, dependency-free wire corpus for
commands supported by the current baseline. Its normal local-mode test runs
without Redis, Valkey, network access, or external Rust crates. The native test is
ignored by default and requires `RILLCACHE_REFERENCE_ADDR` plus
`RILLCACHE_REFERENCE_KIND=redis|valkey` pointing to a disposable server.

## Acceptance criteria

- **When** a client sends binary values containing NUL, CRLF, `\xff`, or bytes
  resembling a RESP null frame, **then** GET shall return those exact bytes.
- **When** the stored string is empty or the key is absent, **then** GET shall
  distinguish an empty bulk from a null bulk.
- **When** EXISTS or DEL receives repeated key arguments, **then** EXISTS shall
  count each existing argument and DEL shall count/remove each key only once.
- **When** SET NX rejects an existing key with GET, or SET rejects conflicting
  options / an invalid expiry, **then** the old value and absolute expiry shall
  remain unchanged; failures shall not partially mutate the key.
- **When** `MSETNX` encounters an existing key, **then** it shall not insert an
  earlier absent key or change the existing binary value or absolute deadline.
  When all keys are absent, it shall write all values, including empty strings.
- **When** `HSETNX` encounters an existing binary field or a string instead of a
  hash, **then** it shall preserve the value and absolute deadline, returning
  zero or `WRONGTYPE` respectively. On a missing hash, it shall create the field.
  An existing field with an empty value still rejects replacement; the rejection
  checks the field without cloning the hash.
- **When** counter updates succeed, **then** the integer result and existing TTL
  shall be preserved; **if** signed-64-bit arithmetic overflows or PEXPIRE receives
  an invalid integer, **then** the error shall leave the value and TTL unchanged.
- **When** absolute expiry is conditional on NX, XX, GT or LT, **then** the
  reply, stored value and deadline shall match the pinned references, including
  equal deadlines and persistent keys treated as having infinite TTL.
- **When** PERSIST or GETEX PERSIST clears a timeout, SET KEEPTTL or APPEND
  preserves it, COPY duplicates it, or RENAME overwrites another key, **then**
  PEXPIRETIME/EXPIRETIME shall report the exact resulting deadline. Clearing the
  source's expiry shall not change the copy's deadline.
- **When** GETEX PXAT uses a past timestamp, **then** it shall return the previous
  binary value and remove the key; PEXPIRE with zero shall also remove it. Missing
  and persistent keys shall report the stable -2 and -1 TTL/PTTL sentinels.
- **When** commands are queued in MULTI, **then** another connection shall not
  observe their writes until EXEC; EXEC shall return ordered results, including
  runtime errors, and continue executing later commands without rollback.
- **When** DISCARD clears a queue or a queue-time arity error causes EXECABORT,
  **then** no queued writes shall become visible to another connection.
- **When** another connection changes a watched key or creates a watched missing
  key, **then** EXEC shall return a null array and leave the other connection's
  value intact. The aborted EXEC shall clear WATCH for the next transaction.
- **When** a RESP request arrives byte-by-byte or multiple commands are pipelined,
  **then** framing and ordered one-reply-per-command behavior shall be preserved.
- **When** QUIT precedes another pipelined write, **then** the server shall flush
  `+OK`, close, and not execute the write.
- **When** the corpus runs against managed per-client, one-worker, or four-worker
  Rillcache, **then** each mode shall satisfy the same stable RESP2 assertions.
- **When** the optional reference uses Redis, a missing GETEX key with an invalid
  expiry shall return null; **when** it uses Valkey, the native error is expected.
  Rillcache follows Redis for this documented divergence.

- **When** several clients subscribe to the same binary pattern, or one client
  repeats that subscription, **then** `PUBSUB NUMPAT` and `INFO STATS` shall
  count the pattern once and keep counting it until its last subscriber leaves.
  Per-client subscription acknowledgements shall still report per-client counts.

## Reader and scope

The response reader recognizes RESP2 simple strings, errors, integers, bulk
strings/nulls, and arrays. It bounds each reply to 2 MiB, collections to 4096
items, nesting to 16 levels, and socket operations to three seconds. The corpus
uses future absolute timestamps for deterministic TTL checks rather than sleeps.
It compares expected stable command semantics, not server identities, version
text, implementation metadata, or all command behavior. It is not a full Redis
compatibility suite, exhaustive transaction/isolation test, client-library
integration suite, cluster test, or performance benchmark.

The optional test runs the same current-supported corpus against Rillcache in all
three managed modes and the selected disposable native reference. Its key namespace
includes process/time identity and it removes its fixture keys before use. The
reference database is still shared external state: run only against a disposable
instance. The ordinary unignored test runs entirely against ephemeral local
Rillcache servers.

## Verification and version notes

The command contracts were checked against official Redis SET, GETEX, INCR,
EXPIRE and RESP protocol documentation. The optional reference corpus has been
exercised against the repository's pinned Redis 8.10.2 and Valkey 9.1.2 binaries. In
particular, GETEX for a missing key with an invalid EX value differs: Redis returns
null, while Valkey returns `ERR value is not an integer or out of range`. This
corpus preserves that explicit distinction instead of declaring the vendors
identical. The repository's `docs/strings.md` documents the current supported
command and expiry limits.

Run offline coverage:

```sh
cargo test --offline --locked --test reference_baseline
```

Run the native comparison against a disposable reference:

```sh
RILLCACHE_REFERENCE_ADDR=127.0.0.1:6379 \
RILLCACHE_REFERENCE_KIND=redis \
cargo test --offline --locked --test reference_baseline -- --ignored
```

The native test writes only keys under `rillcache:baseline:*` and deletes its
fixtures at startup. It does not require or install Python, Node, Go, redis-cli,
or any Rust dependency.

Sources: [Redis SET](https://redis.io/docs/latest/commands/set/),
[Redis GETEX](https://redis.io/docs/latest/commands/getex/),
[Redis INCR](https://redis.io/docs/latest/commands/incr/),
[Redis EXPIRE](https://redis.io/docs/latest/commands/expire/),
[Redis transactions](https://redis.io/docs/latest/develop/using-commands/transactions/),
[RESP specification](https://redis.io/docs/latest/develop/reference/protocol-spec/).
