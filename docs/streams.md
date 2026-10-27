# Bounded stream compatibility

Rillcache stores stream values inside the same key record as strings, so TYPE,
EXISTS, DEL, TTL, expiry, RENAME and COPY apply to a single key identity.
XADD field/value bytes are charged to the configured key/value payload budget;
XDEL reclaims those bytes, and rejected appends leave the previous stream intact.
As in pinned Redis, XDEL retains group/consumer pending references to deleted IDs;
message removal is not acknowledgment. Stream groups, consumers and pending-delivery
references are deep-cloned by COPY after admission. RENAME instead transfers the
owned value/tree after checking NX rejection and final payload admission.

## Supported commands

- `XADD key id|* field value [field value ...]`: explicit ordered `ms-seq` IDs
  and generated IDs. IDs must be strictly greater than the stream's last ID.
- `XDEL key id [id ...]`, `XLEN key`.
- `XGROUP CREATE key group id|$ [MKSTREAM]`, `SETID`, `DESTROY`, `CREATECONSUMER`
  and `DELCONSUMER`.
- `XREADGROUP GROUP group consumer [COUNT n] [NOACK] STREAMS key >` for a
  single stream and new entries. COUNT is per stream. A normal delivery updates
  the consumer and group pending-entry state; NOACK advances the group without
  adding pending entries.
- `XACK key group id [id ...]` removes owned pending entries from the group and
  consumer and returns the acknowledged count.
- `XPENDING key group` summary and
  `XPENDING key group [IDLE ms] start end count [consumer]` extended forms with
  inclusive `-`/`+` bounds, consumer filtering and idle filtering.
- `XRANGE key start end [COUNT n]` / `XREVRANGE key end start [COUNT n]` with
  inclusive IDs, `-`/`+`, incomplete-ID expansion and bounded results.
- `XREAD [COUNT n] STREAMS key [key ...] id [id ...]` non-blocking reads over one
  or more streams; `$` resolves to the last generated ID at command time. Blocking
  reads are rejected explicitly.
- `XINFO STREAM key` and `XINFO STREAM key FULL` return actual stream entries,
  group cursors, consumers and pending records in bounded RESP2 arrays.
- `COPY source destination [DB 0] [REPLACE]` creates an independent copy of
  string or stream state. Other database numbers/options are rejected.

## Limits

This is not Redis Streams feature completeness. Blocking `XREAD`/`XREADGROUP`,
history reads from a consumer's pending list, `XCLAIM`/`XAUTOCLAIM`, trimming,
`XSETID`, `XINFO GROUPS`/`CONSUMERS`, replication and durable pending/consumer
restoration are unsupported. `XREAD` declares no static key positions, so ACL key
patterns do not constrain its dynamic stream list; restrict the command itself.
Unsupported stream options return errors; no blocking behavior is faked. Stream IDs are unsigned
64-bit pairs. Generated IDs use the local Unix-millisecond clock and increment
sequence when the clock does not advance. Consumer delivery time uses the
store's configured clock. XINFO's FULL response is sufficient for state
inspection and independent COPY checks but does not include Redis internal radix
node statistics or every newer Redis-specific extended field. `entries-read` is
reported as unknown (null); this subset does not track Redis's historical read
counter or deletion-aware lag estimator. `lag` counts currently retained entries
after the group's cursor. `nacked-count` is zero because no unowned NACK operation
is supported; owned pending records are counted by `pel-count`. Pending is process memory:
`XACK` mutates it for WATCH/journal capture, but the append-only journal rebuilds
only stream entries and group cursors, not pending owners/delivery counts; treat
pending as volatile across restart. TCP regression coverage asserts explicit
cursor, pending IDs/owners/counts, ACK, range/read and NOACK behavior, not just
equality between source and copy.

The payload budget counts key bytes and retained stream field/value bytes.
It intentionally does not claim an RSS bound and does not count Rust container
capacity or group/consumer bookkeeping. Requests and replies retain the server's
standard frame and bulk limits. All mutations are serialized by the store lock;
no stream state is accessed during socket I/O.
