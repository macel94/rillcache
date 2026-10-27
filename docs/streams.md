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

- `XADD key [NOMKSTREAM] [MAXLEN|MINID [~|=] threshold [LIMIT count]] id|*
  field value [field value ...]`: explicit ordered `ms-seq` IDs and generated
  IDs. IDs must be strictly greater than the stream's last ID. `NOMKSTREAM`
  returns null instead of creating a missing key. `MAXLEN`/`MINID` trim after the
  append using the same exact/approximate and LIMIT rules as XTRIM. The append's
  ID is returned even when the new entry is immediately trimmed; an admission
  failure leaves the previous stream intact. XADD trim publishes the `xadd`
  event only; use XTRIM when an `xtrim` event is required.
- `XDEL key id [id ...]`, `XLEN key`.
- `XGROUP CREATE key group id|$ [MKSTREAM]`, `SETID`, `DESTROY`, `CREATECONSUMER`
  and `DELCONSUMER`.
- `XREADGROUP GROUP group consumer [COUNT n] [NOACK] STREAMS key >` for a
  single stream and new entries. COUNT is per stream. A normal delivery updates
  the consumer and group pending-entry state; NOACK advances the group without
  adding pending entries. A non-`>` ID selects that consumer's pending history
  with IDs greater than the supplied ID in ascending order: present entries carry
  their fields, deleted pending IDs return a null fields array, and an empty range
  returns the stream key with no entries. History creates a missing consumer and
  increments delivery counts/times for returned pending records without advancing
  the group cursor; `NOACK` does not suppress those updates. `$` is rejected.
- `XACK key group id [id ...]` removes owned pending entries from the group and
  consumer and returns the acknowledged count.
- `XCLAIM key group consumer min-idle-time id [id ...] [IDLE ms] [TIME ms]
  [RETRYCOUNT count] [FORCE] [JUSTID] [LASTID id]` transfers existing pending
  entries to a consumer after the minimum idle time, optionally creating pending
  entries for existing stream messages with `FORCE`. `JUSTID` returns only IDs,
  `RETRYCOUNT` sets the delivery count, `IDLE`/`TIME` set the recorded delivery
  time (bogus values are clamped to now) and `LASTID` advances the group cursor.
  Pending entries whose message was deleted are pruned instead of returned.
- `XAUTOCLAIM key group consumer min-idle-time start [COUNT count] [JUSTID]`
  scans pending entries from `start` (default count 100, bounded attempts),
  claims up to the count and returns `[next-start-id, claimed, deleted]`.
  `COUNT` must be positive; deleted-message pending IDs are pruned and reported in
  the third array so a caller can advance or restart a recovery loop.
- `XSETID key last-id [ENTRIESADDED count] [MAXDELETEDID id]` aligns the stream
  cursor and retention counters, for example after restore. The key must exist
  and the last ID cannot be below the greatest retained entry or the effective
  max-deleted ID. `ENTRIESADDED` cannot be below the retained length. `XADD`
  increments entries-added even when the new entry is trimmed; `XDEL`, `XTRIM`
  and XADD trimming advance max-deleted-entry-id.
- `XINFO GROUPS key` and `XINFO CONSUMERS key group` expose group name, consumer
  count, pending count, last-delivered ID, unknown `entries-read` (null) and
  retained-entry `lag`, plus consumer name, pending count, `idle` and `inactive`.
- `XTRIM key MAXLEN|MINID [~|=] threshold [LIMIT count]` removes oldest entries
  until the retained length/ID threshold is met and returns the removed count.
  Exact mode ignores no entries; approximate (`~`) mode caps work at `LIMIT`
  (default 10,000; `LIMIT 0` means unbounded). Pending IDs whose messages are
  trimmed remain in the PEL, as with XDEL, until XACK/XCLAIM/XAUTOCLAIM prunes
  them.
- `XPENDING key group` summary and
  `XPENDING key group [IDLE ms] start end count [consumer]` extended forms with
  inclusive `-`/`+` bounds, consumer filtering and idle filtering.
- `XRANGE key start end [COUNT n]` / `XREVRANGE key end start [COUNT n]` with
  inclusive IDs, `-`/`+`, incomplete-ID expansion and bounded results.
- `XREAD [COUNT n] [BLOCK ms] STREAMS key [key ...] id [id ...]` reads over one
  or more streams. `$` resolves to the last generated ID at command time; with
  `BLOCK`, that resolution is captured once when blocking starts. A positive BLOCK
  timeout returns a null array on expiry, `0` waits indefinitely, and appends to
  any listed stream wake the read using the normal XREAD reply shape. The blocked
  connection uses the same bounded 10 ms polling model as list/zset blocking.
  Inside `MULTI`, BLOCK runs one non-blocking read at `EXEC`.
- `XINFO STREAM key` and `XINFO STREAM key FULL` return actual stream entries,
  group cursors, consumers and pending records in bounded RESP2 arrays, including
  `last-generated-id`, `entries-added` and `max-deleted-entry-id`.
- `COPY source destination [DB 0] [REPLACE]` creates an independent copy of
  string or stream state. Other database numbers/options are rejected.

## Limits

This is not Redis Streams feature completeness. Blocking `XREADGROUP`,
multi-stream history reads, replication and durable pending/consumer restoration
are unsupported. History reads cover one stream per command. `XSETID` metadata is
journaled, but pending owners/delivery counters remain volatile. XTRIM's
`~` mode removes an exact number of entries up to the LIMIT/default bound rather
than Redis's listpack-node granularity, so it never leaves more entries than the
bound but may remove fewer than a native approximate trim. `XREAD` declares no static key positions, so ACL key
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
is supported; owned pending records are counted by `pel-count`. Pending is process
memory: `XACK`, `XCLAIM` and `XAUTOCLAIM` mutate it for WATCH/journal capture, but
the append-only journal rebuilds only stream entries and group cursors, not
pending owners, delivery counts or consumer activity; treat pending as volatile
across restart. `XINFO CONSUMERS` reports `inactive` as the same process-clock
idle time as `idle` because this subset does not track Redis's separate active
time. TCP regression coverage asserts explicit cursor, pending IDs/owners/counts,
ACK, claim/autoclaim, range/read and NOACK behavior, not just equality between
source and copy.

The payload budget counts key bytes and retained stream field/value bytes.
It intentionally does not claim an RSS bound and does not count Rust container
capacity or group/consumer bookkeeping. Requests and replies retain the server's
standard frame and bulk limits. All mutations are serialized by the store lock;
no stream state is accessed during socket I/O.
