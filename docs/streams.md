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
- `XGROUP CREATE key group id|$ [MKSTREAM]` and `XGROUP SETID key group id|$`.
- `XREADGROUP GROUP group consumer [COUNT n] [NOACK] STREAMS key >` for a
  single stream and new entries. COUNT is per stream. A normal delivery updates
  the consumer and group pending-entry state; NOACK advances the group without
  adding pending entries.
- `XINFO STREAM key` and `XINFO STREAM key FULL` return actual stream entries,
  group cursors, consumers and pending records in bounded RESP2 arrays.
- `COPY source destination [DB 0] [REPLACE]` creates an independent copy of
  string or stream state. Other database numbers/options are rejected.

## Limits

This is not Redis Streams feature completeness. Blocking reads, multiple streams,
history/pending reads, XACK, claiming, trimming, XSETID, other XGROUP/XINFO
subcommands, replication and persistence are unsupported. Unsupported stream
options return errors; no blocking behavior is faked. Stream IDs are unsigned
64-bit pairs. Generated IDs use the local Unix-millisecond clock and increment
sequence when the clock does not advance. Consumer delivery time uses the
store's configured clock. XINFO's FULL response is sufficient for state
inspection and independent COPY checks but does not include Redis internal radix
node statistics or every newer Redis-specific extended field. `entries-read` is
reported as unknown (null); this subset does not track Redis's historical read
counter or deletion-aware lag estimator. `lag` counts currently retained entries
after the group's cursor. `nacked-count` is zero because no unowned NACK operation
is supported; owned pending records are counted by `pel-count`. TCP regression
coverage asserts explicit cursor, pending IDs/owners/counts and NOACK behavior,
not just equality between source and copy.

The payload budget counts key bytes and retained stream field/value bytes.
It intentionally does not claim an RSS bound and does not count Rust container
capacity or group/consumer bookkeeping. Requests and replies retain the server's
standard frame and bulk limits. All mutations are serialized by the store lock;
no stream state is accessed during socket I/O.
