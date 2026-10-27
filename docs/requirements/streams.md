# Stream requirements

Part of the [requirements and incremental delivery index](../requirements.md). Requirements are preserved verbatim from the former monolithic EARS sections; add new requirements only in the matching area file.

## Stream pending, range and multi-stream reads (S43)

- **R169 (event-driven):** When `XACK key group id [id ...]` succeeds, it shall
  remove each owned pending entry from the group and its consumer and return the
  acknowledged count; missing keys/groups and unknown IDs shall return zero without
  mutation, and wrong types shall return WRONGTYPE.
- **R170 (event-driven):** When `XPENDING key group` summary or
  `XPENDING key group [IDLE ms] start end count [consumer]` extended forms run,
  they shall return count/min/max/consumer counts or bounded
  `[id, consumer, idle, deliveries]` rows with inclusive bounds, consumer and idle
  filtering; missing key/group shall return NOGROUP and invalid arguments shall
  fail before mutation.
- **R171 (event-driven):** When `XRANGE`/`XREVRANGE` or non-blocking
  `XREAD [COUNT] STREAMS ...` run, they shall return bounded entries/streams in
  requested order with inclusive IDs, `-`/`+` and incomplete-ID expansion, `$`
  resolved at command time and `*-1` when no stream has new entries. Blocking
  XREAD shall return an explicit unsupported error.
- **R172 (event-driven):** `XGROUP DESTROY`, `CREATECONSUMER` and `DELCONSUMER`
  shall update group/consumer metadata atomically, report created/removed counts,
  delete a consumer's pending references, preserve TTL/accounting and touch the
  stream for WATCH/journal capture. All-mode tests shall cover ack/pending/range/
  read/group lifecycle, errors and multi-stream reads; docs shall retain missing
  blocking/claim/trim/XINFO subcommands, volatile pending across restart and
  dynamic-key ACL limitations.

## Stream claiming and consumer introspection (S46)

- **R181 (event-driven):** When `XCLAIM key group consumer min-idle-time id [id ...]`
  runs, it shall claim existing pending IDs only when their idle time meets the
  threshold, transfer ownership and update delivery time/count, prune pending IDs
  whose stream entry no longer exists, and return claimed entries in argument
  order. `FORCE` shall create pending state for existing entries; `JUSTID`,
  `IDLE`, `TIME`, `RETRYCOUNT` and `LASTID` shall apply as documented without
  changing entry payloads.
- **R182 (event-driven):** `XAUTOCLAIM key group consumer min-idle-time start
  [COUNT count] [JUSTID]` shall scan at most a bounded multiple of `COUNT`
  pending entries from `start`, claim eligible entries, prune message-less
  pending IDs, and return exactly `[next-start-id, claimed, deleted]` with
  `JUSTID` selecting ID-only claimed rows. `COUNT` shall be positive.
- **R183 (event-driven):** `XINFO GROUPS key` / `XINFO CONSUMERS key group` shall
  return bounded RESP2 maps for group name/consumer count/pending count/
  last-delivered-id/`entries-read` (null)/`lag` and consumer name/pending count/
  `idle`/`inactive`, with missing-key, missing-group and wrong-type errors.
- **R184 (unwanted behavior):** If a claim command has malformed minimum-idle/
  start/ID/option/COUNT input or a missing/wrong-type key/group, it shall return
  one bounded error without partial transfer. Successful claims shall touch the
  stream for WATCH/journal capture, publish `xclaim`/`xautoclaim` notifications,
  preserve payload accounting and TTL, keep pending metadata process-local, and
  be covered by unit and all-mode wire tests. Blocking and history reads shall
  remain documented as unsupported.

## Stream trimming (S48)

- **R189 (event-driven):** When `XTRIM key MAXLEN|MINID [~|=] threshold
  [LIMIT count]` runs, it shall parse the strategy, optional approximate marker
  and LIMIT before key access, reject negative MAXLEN/LIMIT, invalid IDs,
  conflicting strategies and LIMIT without `~` with the documented bounded
  errors.
- **R190 (event-driven):** Exact `MAXLEN` shall remove oldest entries until the
  retained length is at most the threshold; exact `MINID` shall remove entries
  with IDs below the threshold. Approximate mode shall remove at most the given
  LIMIT (default 10,000, LIMIT 0 unbounded) and return the removed count.
- **R191 (event-driven):** `XTRIM` shall return zero for a missing key and
  WRONGTYPE for a non-stream key, preserve TTL and payload accounting, retain
  pending/consumer references to removed IDs (as XDEL does), and touch the stream
  for WATCH/journal capture and an `xtrim` keyspace event when entries are
  removed.
- **R192 (unwanted behavior):** Invalid or partial trimming arguments shall not
  mutate the stream. Tests shall cover exact/approximate MAXLEN/MINID, `=` and
  `~`, LIMIT bounds, empty and missing streams, TTL, pending retention, WATCH and
  wrong types in unit and all-mode wire tests. Blocking reads and history reads
  shall remain documented as unsupported.

## Stream append retention and NOMKSTREAM (S54)

- **R213 (event-driven):** `XADD key [NOMKSTREAM]
  [MAXLEN|MINID [~|=] threshold [LIMIT count]] id|* field value ...` shall parse
  the options before key access, reject conflicting/negative/invalid trim or ID
  arguments, and require at least one field/value pair after the ID.
- **R214 (event-driven):** `NOMKSTREAM` shall return a null bulk reply when the
  key does not exist and shall not create a stream. Existing streams shall append
  normally.
- **R215 (event-driven):** After appending, `MAXLEN`/`MINID` shall remove entries
  with the same exact or bounded approximate rules as XTRIM, including `LIMIT`
  only with `~`, return the newly generated/explicit ID, preserve TTL and exact
  payload accounting, and leave the previous stream unchanged on parse,
  wrong-type or payload-admission failure. An emptied retained stream may remain
  as an existing zero-length stream key.
- **R216 (unwanted behavior):** Tests shall cover exact/approximate MAXLEN and
  MINID, `=`/`~`, LIMIT bounds, NOMKSTREAM on missing/existing keys, TTL,
  wrong types, malformed options/IDs, MULTI, WATCH, OOM and all worker modes.
  XADD trim shall publish the `xadd` event only.

## Stream ID and metadata control (S57)

- **R225 (event-driven):** `XSETID key last-id [ENTRIESADDED count]
  [MAXDELETEDID id]` shall parse the stream ID and optional metadata pairs,
  reject malformed IDs/options, non-integer or negative `ENTRIESADDED`, and
  require an existing stream key. It shall reject a last ID below the greatest
  retained entry, below the effective maximum-deleted ID, or an entries-added
  value below the retained length with the documented Redis errors and no partial
  update.
- **R226 (state-driven):** Streams shall track entries-added across accepted
  XADD operations (including immediately trimmed entries) and the maximum deleted
  entry ID across XDEL, XTRIM and XADD trimming. `XSETID` shall update
  last-generated-id, entries-added and max-deleted-entry-id, and `XINFO STREAM`
  (with and without FULL) shall expose `entries-added` and
  `max-deleted-entry-id` truthfully.
- **R227 (event-driven):** `XSETID` shall touch the key for WATCH/journal capture
  and publish no separate keyspace event, matching standalone Redis. Journal
  snapshots shall emit an `XSETID` record preserving last ID, entries-added and
  max-deleted-entry-id so restart reproduces the metadata and enforces subsequent
  XADD ID ordering.
- **R228 (unwanted behavior):** Missing/wrong-type keys, invalid IDs, bad option
  ordering/arity, negative counters and inconsistent metadata shall return bounded
  errors without modifying the stream. Tests shall cover metadata changes, ID
  ordering, XDEL/trim counter updates, MULTI, WATCH conflicts, restart replay and
  all worker modes.

## Stream consumer history reads (S58)

- **R229 (event-driven):** `XREADGROUP GROUP group consumer [COUNT n] [NOACK]
  STREAMS key id` shall treat `>` as a new-entry read but any other ID as a
  history read of that consumer's pending entries. `$` shall return the documented
  "meaningless" error, malformed IDs shall fail before key access, and `COUNT`
  shall cap the returned pending range while `NOACK` shall not suppress history
  metadata updates.
- **R230 (event-driven):** A history read shall return `[key, entries]` for the
  single supported stream, selecting pending entries owned by that consumer with
  IDs greater than the requested ID in ascending order. Present entries shall
  include their fields, deleted pending IDs shall return a null fields array, and
  an empty result shall still return the key with an empty entries array. The
  consumer shall be created when absent, and each returned pending record shall
  increment its delivery count and advance its delivery time while leaving the
  group's last-delivered ID unchanged.
- **R231 (unwanted behavior):** Missing keys/groups shall return bounded NOGROUP
  errors naming the requested key/group, and wrong-type keys shall return
  WRONGTYPE. History reads shall touch the stream for WATCH/journal capture and
  follow the existing `xreadgroup` notification behavior without claiming durable
  pending state. Multiple history streams are specified in S66.
- **R232 (ubiquitous):** Tests shall cover full/partial/deleted pending history,
  ordering, COUNT, no-pending and absent-consumer behavior, delivery-count/time
  updates, unchanged group cursors, NOACK, `$`/malformed-ID/NOGROUP/WRONGTYPE
  errors, WATCH conflicts and all worker modes. Durable pending restoration shall
  remain documented as unsupported.

## Blocking stream reads (S59)

- **R233 (event-driven):** `XREAD [COUNT n] [BLOCK ms] STREAMS key [key ...]
  id [id ...]` shall accept an integer millisecond BLOCK value, use zero for an
  indefinite wait, reject negative/non-integer/out-of-range values and syntax
  errors before stream access, and let a later duplicate option win. Existing
  COUNT, `$`, explicit-ID, multi-key and unbalanced-list validation shall remain.
- **R234 (event-driven):** A blocking read shall return immediately when any
  listed stream already has entries above its start. Otherwise the connection
  shall wait using the existing bounded polling model; `$` shall be resolved once
  at block time to each live last-generated ID, and appends to any listed stream
  shall wake the read with the same RESP array shape as non-blocking XREAD.
  Positive timeouts shall return a null array; expiry, missing keys, wrong types
  and disconnect/shutdown shall not leak blocked state.
- **R235 (state-driven):** Inside `MULTI`, `BLOCK` shall execute the non-blocking
  read once at `EXEC` without waiting. Blocking reads shall not mutate data or
  journal state, shall not hold the store lock while waiting, and shall remain
  at-most-once observations of live entries rather than durable subscriptions.
- **R236 (ubiquitous):** Tests shall cover immediate reads, indefinite and finite
  waits, wake-on-append, `$` capture, multi-key wake selection, COUNT, timeout
  nulls, MULTI non-blocking behavior, wrong types, malformed BLOCK/IDs/STREAMS and
  all worker modes. Multi-stream history shall remain documented as unsupported.

## Blocking consumer-group reads (S62)

- **R245 (event-driven):** `XREADGROUP GROUP group consumer [COUNT n] [BLOCK ms]
  [NOACK] STREAMS key id` shall parse the same integer-millisecond BLOCK values as
  XREAD, reject negative/non-integer/out-of-range options and `$` with the
  documented errors, and apply waiting only to a `>` new-entry read. A history ID
  with BLOCK shall return history immediately without waiting.
- **R246 (event-driven):** When a `>` read has no new entries, a positive BLOCK
  shall wait using the bounded polling model and return null on expiry; zero shall
  wait indefinitely. Appends shall wake the read, deliver the normal response
  shape, advance the group cursor, add pending entries unless NOACK is set, update
  consumer activity, and publish the existing `xreadgroup` notification after
  journal capture. Missing keys/groups and wrong types shall fail immediately
  without blocking.
- **R247 (state-driven):** Inside `MULTI`, BLOCK shall execute one non-blocking
  `>` read at `EXEC` and never wait. Blocked connections shall not hold the store
  lock, shall clear state on disconnect/shutdown, and shall remain bounded polling
  observations rather than durable subscriptions.
- **R248 (ubiquitous):** Tests shall cover immediate group reads, finite and
  indefinite waits, wake-on-append, NOACK, cursor/pending updates, history with
  BLOCK, MULTI non-blocking behavior, missing/wrong-type errors, malformed
  BLOCK/IDs/STREAMS, notification direction and all worker modes. Multi-stream
  history and replication/durable pending remain unsupported.

## Multi-stream consumer-group reads (S67)

- **R265 (event-driven):** `XREADGROUP GROUP group consumer [COUNT n] [BLOCK ms]
  [NOACK] STREAMS key [key ...] id [id ...]` shall require an even, non-empty
  key/ID list and parse each ID independently as `>` or a history ID. `$`,
  malformed IDs and unbalanced lists shall fail before key access; COUNT shall
  apply per stream.
- **R266 (event-driven):** New `>` streams shall deliver entries above their own
  group cursor, advance cursors and add pending records unless NOACK is set.
  History streams shall return that consumer's owned pending IDs above the given
  ID, including the four stream shape and null fields for deleted entries, even
  when empty. Delivery counts/times are updated for returned history records.
  Results shall follow key argument order.
- **R267 (state-driven):** The whole command shall validate every key type and
  group before mutating any stream; a missing key/group or wrong type on a later
  stream shall fail the command without earlier cursor/pending changes. Mixed
  new/history and all-history commands shall return immediately. BLOCK shall wait
  only when every ID is `>` and shall wake on appends to any listed stream;
  finite timeouts return null, and `MULTI` runs one non-blocking attempt.
- **R268 (ubiquitous):** Tests shall cover multi-stream new/new, history/history,
  mixed reads, per-stream COUNT, NOACK, empty/deleted history, argument-order
  replies, validation atomicity, BLOCK all-new wake/timeout, mixed BLOCK
  immediacy, MULTI, `$`/unbalanced/missing-group/wrong-type errors, notifications
  and all worker modes. Durable pending restoration and replication remain
  unsupported.
