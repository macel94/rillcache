# Collection requirements

Part of the [requirements and incremental delivery index](../requirements.md). Requirements are preserved verbatim from the former monolithic EARS sections; add new requirements only in the matching area file.

## Blocking list pops (S35)

- **R137 (event-driven):** When `BLPOP`/`BRPOP key [key ...] timeout` finds a live
  nonempty list, it shall pop one left/right element in key order and reply a
  two-element `[key, value]` array; a wrong-type key shall return WRONGTYPE and a
  missing/empty set of keys shall return the null array at expiry.
- **R138 (state-driven):** While waiting, the connection shall hold no store lock,
  apply no ordinary idle disconnect, parse no further commands from its bounded
  input, and remain joined to its worker; polling shall be bounded (about 10 ms in
  default mode), not a push/wakeup or latency guarantee. Disconnect/shutdown shall
  drop the blocked state.
- **R139 (unwanted behavior):** If the timeout is negative, NaN, infinite or out of
  range, or arity is invalid, then the command shall return a bounded error before
  mutating or blocking. ACL key patterns shall be checked before blocking, blocked
  pops shall use the normal journal gate and be recoverable after restart, and
  blocking commands shall be rejected inside `MULTI` with dirty-transaction
  semantics (documented divergence).
- **R140 (ubiquitous):** All-mode wire tests shall cover waiting for a push,
  multi-key/left/right order, timeout nulls, RESP3 conversion, pipelined commands
  after unblock, idle-timeout immunity, wrong-type/float errors and shutdown.
  Documentation shall retain the absence of a condition-variable/push design,
  strict latency SLOs, fairness under many blocked clients and
  cluster/replication blocking semantics; blocking `BLMOVE`/`BRPOPLPUSH` list
  moves are specified separately by S68.

## Typed collection mutations (S37)

- **R145 (event-driven):** When hash mutation commands run, the server shall
  preserve binary field/value bytes and TTL; `HSETNX`/`HDEL`/`HEXISTS` shall use
  field presence, `HMGET` shall preserve argument order and nulls, `HKEYS`/`HVALS`
  shall return deterministic byte order, and `HINCRBY`/`HINCRBYFLOAT` shall reject
  malformed stored values, integer overflow or non-finite results without changing
  the hash. Removing the final field shall remove the key.
- **R146 (event-driven):** When list mutation commands run, the server shall
  preserve left/right order and negative indexes, insert relative to the first
  matching pivot, remove from the requested end, trim inclusively, and reject
  missing keys or out-of-range `LSET` indexes as documented. Empty results shall
  delete the key, and successful mutations shall preserve TTL.
- **R147 (event-driven):** When set mutation or algebra commands run, the server
  shall treat members as binary values, suppress duplicates, count removals once,
  and make `SMOVE` and the `SINTERSTORE`/`SUNIONSTORE`/`SDIFFSTORE` replacements
  atomic. Sources and existing destinations shall be unchanged on wrong type,
  invalid options or admission failure; an empty store result shall delete the
  destination and clear its TTL.
- **R148 (event-driven):** When sorted-set mutation or range commands run, the
  server shall preserve IEEE-754 score ordering (including `-inf`/`+inf`) with
  binary member tie-breaking, apply `NX`/`XX`/`GT`/`LT`/`CH`/`INCR`, exclusive
  score bounds, rank/score removal and bounded pops, and reject invalid or NaN
  scores without mutating state. Successful single-key mutations shall preserve
  TTL and delete the key when its last member is removed.

## HyperLogLog cardinality (S39)

- **R153 (event-driven):** When `PFADD key [element ...]` receives a missing key,
  it shall create an empty valid HLL value; subsequent distinct elements shall be
  added atomically, duplicate/no-op additions shall return `0` and a changed or
  created value shall return `1`. It shall preserve an existing TTL and reject
  non-string keys or malformed HLL payloads without mutation.
- **R154 (event-driven):** When `PFCOUNT key [key ...]` receives keys, it shall
  estimate the union cardinality, count missing keys as empty and return `0` for an
  empty union. Values up to 4,096 distinct hashes shall be counted exactly; larger
  promoted 2^14-register sketches shall use a bounded approximate estimator.
- **R155 (event-driven):** When `PFMERGE destination [source ...]` runs, it shall
  merge the existing destination value (if any) and all sources into the
  destination, preserve the destination TTL, create an empty destination for
  destination-only/missing inputs and return `+OK`. Wrong types, malformed HLL
  payloads or payload-budget failure shall leave the destination unchanged.
- **R156 (ubiquitous):** HLL values shall remain ordinary binary string keys for
  `TYPE`, expiry, keyspace, ACL key positions and append-only journal handling,
  with a private exact/dense payload rather than a Redis HLL encoding claim.
  Tests shall cover exact and dense promotion, cardinality tolerance, binary/empty
  elements, TTL, merge, malformed payloads, wrong types, ACL category metadata and
  transaction queueing in all execution modes.

## Geospatial sorted sets (S45)

- **R177 (event-driven):** When `GEOADD key [NX|XX] [CH] longitude latitude member
  ...` receives triples, it shall validate every longitude/latitude and option
  before any key access, reject unparseable/NaN coordinates with the float error
  and out-of-range/non-finite pairs with the bounded pair error, encode each point into a 52-bit interleaved geohash score,
  and apply `ZADD` NX/XX/CH semantics while preserving TTL and exact payload
  accounting.
- **R178 (event-driven):** When `GEOPOS`, `GEOHASH` or `GEODIST` reads members, it
  shall decode geohash scores and return 17-fractional coordinate text, an
  11-character standard geohash, or a four-decimal distance in M/KM/FT/MI
  respectively. Missing members shall return the command's null shape; wrong-type
  keys shall return WRONGTYPE; invalid units or coordinates shall fail before
  producing a result.
- **R179 (event-driven):** When `GEOSEARCH` runs, it shall require exactly one of
  FROMMEMBER/FROMLONLAT and exactly one of BYRADIUS/BYBOX, support ASC/DESC,
  COUNT with optional ANY validation and WITHCOORD/WITHDIST/WITHHASH, resolve
  FROMMEMBER against the live source, scan the bounded sorted set with the
  documented haversine/rectangle checks, and return at most COUNT deterministic
  results. A missing source shall return an empty array.
- **R180 (unwanted behavior):** When `GEOSEARCHSTORE` stores matches, it shall
  perform one atomic destination replacement, clear the destination TTL, support
  STOREDIST, delete the destination for an empty result and never partially write
  on malformed options, wrong source type or payload OOM. `GEOADD` and
  `GEOSEARCHSTORE` shall publish `zadd`/`geosearchstore`/`del` events, expose
  truthful `@geo` metadata and two ordered keys for the store command, and be
  covered by unit and all-mode wire tests. Legacy `GEORADIUS*` aliases are
  delivered in S55; native neighbour indexes and RDB interchange shall remain
  documented as unsupported.

## Sorted-set range and selection completion (S50)

- **R197 (event-driven):** `ZRANGE key start stop [BYSCORE|BYLEX] [REV]
  [LIMIT offset count] [WITHSCORES]` shall preserve rank-mode behavior and add
  score/lex selection with optional reversal. `LIMIT` shall be valid only with
  `BYSCORE`/`BYLEX`; `WITHSCORES` shall be rejected with `BYLEX`; score and lex
  bounds shall use inclusive/exclusive markers and `-`/`+` terminal bounds.
- **R198 (event-driven):** `ZRANGESTORE destination source start stop
  [BYSCORE|BYLEX] [REV] [LIMIT offset count]` shall apply the same selection,
  atomically replace the destination sorted set, clear its TTL, delete it for an
  empty selection and preserve the destination on wrong-type source or payload
  OOM.
- **R199 (event-driven):** `ZLEXCOUNT key min max` and
  `ZREMRANGEBYLEX key min max` shall count/remove members in the lex range,
  deleting the key when the set becomes empty, preserving TTL and touching the
  stream for WATCH/journal capture. `ZRANDMEMBER key [count [WITHSCORES]]` shall
  return one random member, up to `count` distinct members for positive counts,
  `abs(count)` samples with replacement for negative counts (bounded by the
  collection limit), and flat member/score pairs when `WITHSCORES` is requested.
- **R200 (unwanted behavior):** Invalid range/mode/LIMIT/COUNT arguments,
  malformed score or lex bounds, wrong types and missing keys shall return the
  documented empty, null or bounded errors without partial writes. Tests shall
  cover rank/score/lex ranges, `REV`, `LIMIT`, `WITHSCORES` combinations,
  `ZRANGESTORE` self/empty/TTL/OOM cases, lex count/removal, random counts,
  WATCH and all worker modes. Blocking `BZMPOP` is delivered separately in S53.

## Sorted-set algebra (S51)

- **R201 (event-driven):** `ZUNION`, `ZINTER` and `ZDIFF` shall parse a strictly
  positive `numkeys`, exactly that many source keys, and the documented trailing
  options. Union and intersection shall accept `WEIGHTS weight [weight ...]`
  (one per key) and `AGGREGATE SUM|MIN|MAX`; all three shall accept `WITHSCORES`
  for non-store variants. `ZDIFF` shall reject weights and aggregation. Missing
  source keys shall count as empty sets, and any wrong-type source shall fail
  without a result write. Replies shall be ordered by score then binary member.
- **R202 (event-driven):** Weighted scores shall multiply each input score and
  aggregate with the selected operation; a NaN intermediate or aggregate score
  shall become zero as in Redis, while infinities remain. `ZDIFF` shall return
  first-set members absent from every later set with their original scores.
  `ZINTERCARD numkeys key [key ...] [LIMIT limit]` shall count members present in
  every set, cap the reply at a positive limit, use the full count for zero, and
  reject negative or non-integer limits.
- **R203 (event-driven):** `ZUNIONSTORE`, `ZINTERSTORE` and `ZDIFFSTORE` shall
  compute the same result and atomically replace the destination, clear its TTL,
  delete it for an empty result, support a destination that is also a source,
  enforce payload and element limits before replacing the destination, and
  publish a `zunionstore`/`zinterstore`/`zdiffstore` event for the destination
  only.
- **R204 (unwanted behavior):** Invalid `numkeys`, insufficient keys, malformed
  weights, unknown aggregation or options, non-store `WITHSCORES` misuse and
  wrong types shall return the documented bounded errors without partial writes.
  ACL key patterns shall cover the counted source list and the store destination.
  Tests shall cover union/intersection/difference with weights and every
  aggregation, missing keys, destination replacement/TTL/self/empty/OOM cases,
  NaN handling, cardinality limits, errors and all worker modes.

## Sorted-set multi-pop (S52)

- **R205 (event-driven):** `ZMPOP numkeys key [key ...] MIN|MAX [COUNT count]`
  shall require a positive counted key list, choose the first live non-empty
  sorted-set key in argument order, reject a wrong-type key encountered before a
  candidate, and parse `MIN`/`MAX` plus at most one positive `COUNT` before key
  access.
- **R206 (event-driven):** On success it shall return
  `[key, [[member, score], ...]]` with up to COUNT score-then-member ordered
  members, delete the key when emptied, preserve remaining TTL and payload
  accounting, and touch the key for WATCH/journal capture with a `zpopmin` or
  `zpopmax` notification matching the direction.
- **R207 (event-driven):** If no listed key has a non-empty sorted set, it shall
  return a null array without creating keys. Missing keys in the list shall be
  skipped, and a key appearing again after being emptied shall not be recreated.
- **R208 (unwanted behavior):** Invalid numkeys/COUNT/direction/extra options,
  wrong types and negative or zero counts shall return bounded errors without
  partial pops. Tests shall cover first-key selection, MIN/MAX, COUNT, missing
  keys, emptied keys, TTL, MULTI, WATCH, notifications, errors and all worker
  modes. Blocking `BZMPOP` is delivered separately in S53.

## Blocking sorted-set multi-pop (S53)

- **R209 (event-driven):** `BZMPOP timeout numkeys key [key ...] MIN|MAX
  [COUNT count]` shall parse the float-seconds timeout before key access, reject
  NaN/negative/out-of-range timeouts with the documented errors, require a
  positive counted key list and exactly one direction, and accept at most one
  positive `COUNT`. Timeout zero shall wait indefinitely.
- **R210 (event-driven):** On success it shall pop from the first live non-empty
  sorted-set key in argument order, return `[key, [[member, score], ...]]` with up
  to `COUNT` score-then-member ordered members, delete the key when emptied,
  preserve remaining TTL and payload accounting, commit journal state, and publish
  a `zpopmin`/`zpopmax` event matching the direction. A wrong-type key before a
  candidate shall fail without popping; missing keys shall be skipped and later
  keys re-checked when blocked.
- **R211 (state-driven):** While no listed key is ready, the connection shall wait
  using the existing bounded polling model without holding the store lock. A
  positive timeout shall produce a null array when it expires; disconnect,
  shutdown or RESET shall clear the blocked state. Inside `MULTI`, `BZMPOP` shall
  execute as its non-blocking multi-pop equivalent at `EXEC` while still
  validating the timeout, and `WATCH` conflicts shall abort `EXEC` without pops.
- **R212 (unwanted behavior):** Invalid timeout/numkeys/direction/COUNT/options,
  wrong types and empty results shall return documented bounded errors or a null
  array without partial pops. ACL key patterns shall cover the counted key list
  before blocking. Tests shall cover immediate pops, MIN/MAX, COUNT, key order,
  timeout nulls, wake-on-write, TTL, delete-on-empty, MULTI/WATCH, notifications,
  ACL, errors and all worker modes.

## Legacy geospatial radius aliases (S55)

- **R217 (event-driven):** `GEORADIUS key longitude latitude radius unit` and
  `GEORADIUSBYMEMBER key member radius unit` shall parse WGS-84 coordinates or a
  live member origin, a non-negative radius and a supported M/KM/FT/MI unit, then
  apply the existing bounded radius search. They shall accept
  `WITHCOORD`/`WITHDIST`/`WITHHASH`, `COUNT count [ANY]`, `ASC`/`DESC`, `STORE key`
  and `STOREDIST key` in any order, with the same distance/coordinate/hash reply
  text as `GEOSEARCH`. Missing sources shall return an empty array, a missing
  member in an existing source shall return the member-decode error, and
  wrong-type sources shall return WRONGTYPE.
- **R218 (event-driven):** `GEORADIUS_RO` and `GEORADIUSBYMEMBER_RO` shall return
  the same reads but reject `STORE`/`STOREDIST` with a syntax error. For the
  writable variants, the last `STORE`/`STOREDIST` wins, the destination stores
  geohash scores or distances in the requested unit respectively, and combining a
  store option with `WITHCOORD`/`WITHDIST`/`WITHHASH` shall fail with the legacy
  GEORADIUS incompatibility error.
- **R219 (event-driven):** Legacy store variants shall atomically replace the
  destination, clear its TTL, delete it for an empty result or missing source and
  return the stored count, without partial writes on wrong type, malformed options
  or payload OOM. They shall commit journal state while publishing no separate
  destination notification, matching standalone Redis. Source and last-wins store
  destination shall both be covered by ACL key patterns.
- **R220 (unwanted behavior):** Invalid floats/coordinates, negative radius,
  unsupported units, non-positive or non-integer COUNT, ANY without COUNT, unknown
  options, invalid arity and read-only store attempts shall return bounded errors
  before any destination write. Tests shall cover both origins, every option
  combination, RO variants, store/storedist and last-wins behavior, TTL, missing
  keys/members, wrong types, ACL, discovery metadata and all worker modes.

## Bounded collection sorting (S56)

- **R221 (event-driven):** `SORT key [ASC|DESC] [ALPHA] [LIMIT offset count]
  [STORE destination]` and `SORT_RO ...` shall accept list, set and sorted-set
  sources, reject other types, parse every option before key access, and return
  an empty array for a missing source. Numeric mode shall reject non-double/NaN
  elements with the documented error; `ALPHA` shall compare raw bytes.
- **R222 (event-driven):** Sorting shall apply ASC/DESC, deterministic
  member-byte tie-breaking and post-sort LIMIT clamping (negative offset to zero,
  negative count as unbounded). `SORT_RO` shall reject `STORE`; `BY` and `GET`
  patterns shall be rejected explicitly rather than ignored.
- **R223 (event-driven):** `STORE destination` shall write the selected values as
  a list, replace any destination type, clear its TTL, return the count, delete
  the destination for an empty result and support source==destination from the
  pre-command source snapshot. A wrong-type source or payload OOM shall leave the
  destination unchanged and publish `sortstore`/`del` notifications only on
  successful mutation.
- **R224 (ubiquitous):** ACL key extraction shall cover the source and STORE
  destination, discovery shall expose truthful `SORT`/`SORT_RO` metadata and the
  documented categories, and tests shall cover numeric/alpha/ASC/DESC/LIMIT,
  list/set/zset sources, missing/empty/self/overwrite/TTL/OOM/wrong-type cases,
  SORT_RO, BY/GET rejection, MULTI, WATCH, notifications and all worker modes.
  Dynamic BY/GET patterns remain documented as unsupported.

## List position and multi-key pop (S60)

- **R237 (event-driven):** `LPOS key element [RANK rank] [COUNT count]
  [MAXLEN maxlen]` shall search lists forward for positive ranks and backward for
  negative ranks, reject rank zero and negative COUNT/MAXLEN, and limit examined
  elements when MAXLEN is positive. Without COUNT it shall return the zero-based
  index or a null bulk; with COUNT it shall return an index array, with COUNT 0
  meaning all matches.
- **R238 (event-driven):** `LMPOP numkeys key [key ...] LEFT|RIGHT [COUNT count]`
  shall require a positive counted key list and COUNT, choose the first live
  non-empty list in argument order, reject a wrong-type key encountered before a
  candidate, and return `[key, [element, ...]]` in pop order (head-to-tail for
  LEFT, tail-to-head for RIGHT).
- **R239 (event-driven):** LMPOP shall cap pops at the list length and COUNT,
  delete emptied lists, preserve remaining TTL and payload accounting, touch keys
  for WATCH/journal capture and publish an `lmpop` event on mutation. A missing
  or exhausted key set shall return a null array.
- **R240 (unwanted behavior):** Invalid rank/COUNT/MAXLEN/numkeys/direction/extra
  options, wrong types and missing keys shall return bounded errors, nulls or
  empty arrays without partial pops. Tests shall cover rank/count/maxlen
  combinations, binary elements, LEFT/RIGHT order, COUNT bounds, TTL, MULTI,
  WATCH, notifications and all worker modes. Blocking `BLMPOP` remains
  documented as unsupported.

## Blocking list multi-pop (S63)

- **R249 (event-driven):** `BLMPOP timeout numkeys key [key ...] LEFT|RIGHT
  [COUNT count]` shall parse a non-negative finite float-seconds timeout, a
  positive counted key list, exactly one direction and at most one positive
  COUNT before key access. Zero timeout shall wait indefinitely; NaN/negative/
  out-of-range timeouts, malformed keys/counts/directions/options shall return
  bounded errors.
- **R250 (event-driven):** If a listed list already has elements, BLMPOP shall
  immediately perform LMPOP selection: pop from the first live non-empty list in
  argument order, return `[key, [element, ...]]` in head-to-tail LEFT or
  tail-to-head RIGHT order, delete emptied lists and preserve remaining TTL and
  payload accounting. A wrong-type key before a candidate shall fail without a
  pop. Missing keys shall be skipped.
- **R251 (state-driven):** Otherwise the connection shall wait using the bounded
  polling model without holding the store lock. A positive timeout shall produce
  a null array when it expires; append wakeups, disconnect, shutdown and RESET
  shall clear blocked state. Inside `MULTI`, BLMPOP shall validate the timeout but
  execute the non-blocking LMPOP form once at `EXEC`; WATCH conflicts shall abort
  without pops.
- **R252 (unwanted behavior):** Invalid timeout/numkeys/count/direction/options,
  wrong types and empty results shall return bounded errors or a null array
  without partial pops. ACL key patterns shall cover the counted key list before
  blocking. Tests shall cover immediate/wake/timeout paths, MIN/MAX-equivalent
  LEFT/RIGHT order, COUNT, TTL, delete-on-empty, MULTI/WATCH, notification,
  disconnect behavior and all worker modes.

## SORT dynamic weights and projections (S64)

- **R253 (event-driven):** `SORT`/`SORT_RO` shall accept `BY pattern` and one or
  more `GET pattern` options before or after other supported options, validate
  their arguments before key access, and reject STORE on SORT_RO.
- **R254 (event-driven):** `BY` with `*` shall substitute the first occurrence per
  source element, support a non-empty `->field` hash dereference, and use the
  resulting value as numeric or ALPHA weight. Missing/wrong-type weight keys
  shall contribute zero/empty, while present non-numeric numeric weights shall
  return the conversion error. A `BY` pattern without `*` shall disable sorting
  and use deterministic native list/zset order, reversed by DESC.
- **R255 (event-driven):** `GET` shall substitute each selected element, support
  `GET #`, and return null for missing/wrong-type references in reply mode or an
  empty string when storing. Multiple GET patterns shall multiply output length.
  Because BY/GET patterns derive key names, restricted ACL users without all-key
  access shall be denied before any derived lookup; constant BY patterns remain
  allowed with source-key access.
- **R256 (unwanted behavior):** Invalid/missing pattern arguments, malformed
  numeric weights, wrong types, missing keys and OOM shall return bounded errors
  or documented null/empty values without partial destination writes. Tests shall
  cover string/hash weights, missing and invalid weights, constant BY, ALPHA,
  multiple GETs, store projections, ACL denial/allkeys, TTL/OOM, MULTI/WATCH and
  all worker modes. Cluster routing and durable indexes remain unsupported.

## Hash random fields and atomic get-delete (S65)

- **R257 (event-driven):** `HRANDFIELD key [count [WITHVALUES]]` shall return one
  random field without count, distinct fields for positive counts, up to
  `abs(count)` duplicate-capable fields for negative counts, and flat field/value
  pairs with WITHVALUES. Missing keys and zero counts shall return empty/null
  shapes as documented; wrong types and malformed counts/options shall fail.
- **R258 (event-driven):** `HGETDEL key FIELDS numfields field [field ...]` shall
  require the FIELDS token, a positive numfields exactly matching the supplied
  fields, and return one value-or-null per field while atomically deleting
  present fields.
- **R259 (event-driven):** HGETDEL shall preserve remaining TTL and payload
  accounting, delete an emptied hash key, leave missing keys null-filled, and
  touch the key for WATCH/journal capture. Successful deletions shall publish
  `hdel`, or `del` when the hash becomes empty.
- **R260 (unwanted behavior):** Invalid/missing FIELDS, non-positive/mismatched
  numfields, wrong types, malformed random counts and options shall return
  bounded errors without partial mutation. Tests shall cover random single/count/
  negative/WITHVALUES shapes, missing keys, TTL/empty deletion, WATCH/MULTI,
  notifications and all worker modes. Hash-field expiration remains unsupported.

## List element moves (S68)

- **R269 (event-driven):** `LMOVE source destination LEFT|RIGHT LEFT|RIGHT` and
  `RPOPLPUSH source destination` shall atomically pop the requested source side
  and push onto the requested destination side, replying the moved element, null
  for an absent or empty source, and WRONGTYPE without mutation for a wrong-type
  source or destination.
- **R270 (state-driven):** While `source` and `destination` are the same key, the
  command shall rotate that list in place exactly once, without duplicating or
  losing elements. Moving the last source element shall delete the source key; an
  existing destination shall keep its TTL; a newly created destination shall have
  no TTL; and an over-budget move shall reply OOM without changing either key.
- **R271 (event-driven):** `BLMOVE ... timeout` and `BRPOPLPUSH ... timeout` shall
  perform the move immediately when a live source element exists, otherwise wait
  without holding the store lock and reply null on expiry. `0` shall wait
  indefinitely and negative, non-float or out-of-range timeouts shall fail before
  mutation. Inside `MULTI` both commands shall execute once, non-blockingly, at
  `EXEC`; a conflicting `WATCH` shall abort without moving.
- **R272 (ubiquitous):** Successful moves shall be journaled and shall emit
  `lpush`/`rpush` on the destination, `lpop`/`rpop` on the source, and generic
  `del` when the move empties the source. Tests shall cover all sides, same-key
  rotation, wrong types, TTL, OOM, blocking wake/timeout, MULTI/WATCH,
  notifications, journal replay and every worker mode.

## UNLINK keyspace deletion (S69)

- **R273 (event-driven):** `UNLINK key [key ...]` shall remove every supplied
  existing key, treat logically expired keys as absent, count duplicate keys once
  and return the number of removed keys. Missing arguments shall return the
  standard `unlink` arity error.
- **R274 (state-driven):** While this server has no background reclamation
  thread, UNLINK shall free the removed value synchronously and shall otherwise
  behave exactly like DEL: atomic under the store gate, journaled as a replay-safe
  DEL plus final state, ACL-checked as a declared-key command, and notified with
  one generic `del` event per removed key. Synchronous reclamation is documented
  as a semantic difference from Redis's background free, not as a blocking
  guarantee.
- **R275 (ubiquitous):** Tests shall cover strings, lists, hashes and sorted sets,
  duplicate and missing keys, expired keys, MULTI execution, COMMAND GETKEYS,
  notification delivery, arity errors and every worker mode.

## Hash field expiry controls (S71)

- **R280 (event-driven):** `HEXPIRE`/`HPEXPIRE`, `HEXPIREAT`/`HPEXPIREAT`,
  `HTTL`/`HPTTL`, `HEXPIRETIME`/`HPEXPIRETIME` and `HPERSIST` shall parse the
  documented FIELDS count and field list, reject malformed/mismatched counts and
  negative relative times, and operate on individual hash fields. Expire forms
  shall accept one NX/XX/GT/LT condition and return per-field `1`/`0`/`-2`;
  introspection shall return remaining/absolute time, `-1` for no TTL and `-2`
  for a missing field/key; persist shall return `1`/`-1`/`-2`.
- **R281 (state-driven):** Field deadlines shall be stored separately from hash
  values, survive key-level TTL operations, COPY/RENAME and journal snapshots,
  and be cleared when a field is overwritten by HSET, deleted, or removed by
  HGETDEL. HINCRBY shall preserve a field's TTL. Lazy access shall purge fields
  whose deadline has passed, reclaim their payload bytes, delete the key when the
  last field expires, and touch the key for WATCH/journal capture.
- **R282 (unwanted behavior):** Wrong-type keys, missing keys/fields, malformed
  FIELDS/count/options, negative times and unsupported conditions shall return
  bounded Redis-shaped errors or per-field `-2`/`-1` values without partial
  writes. Field expiry is lazy rather than actively scanned, publishes no
  per-field notification. Combined HGETEX/HSETEX value/TTL operations are
  specified separately by S-HC; this is not a full vendor notification claim.
- **R283 (ubiquitous):** Tests shall cover conditional expire/GT/LT/NX/XX,
  remaining/absolute TTL forms, past-deadline lazy removal, key deletion, HSET/
  HINCRBY/HDEL/HGETDEL TTL interactions, missing/wrong-type/error cases, journal
  restart and all worker modes. Field-TTL bookkeeping remains excluded from the
  key/value payload accounting claim and carries no active-expiry or RSS claim.

## Legacy hash and sorted-set commands (S90)

- **R341 (event-driven):** `HMSET key field value [field value ...]` shall write
  fields exactly like `HSET`, including TTL clearing, payload admission, journal
  and notification behavior, but return `+OK` instead of the added-field count.
  `ZMSCORE key member [member ...]` shall return one score bulk string per
  requested member, a null bulk string for a missing member or key, and preserve
  argument order; both shall reject wrong arity and wrong value types.
- **R342 (event-driven):** `ZRANGEBYLEX key min max [LIMIT offset count]` and
  `ZREVRANGEBYLEX key max min [LIMIT offset count]` shall accept Redis lex bounds
  and return member arrays in ascending/reverse lexical order. They shall reject
  `WITHSCORES`, invalid bounds and negative arity exactly like the equivalent
  `ZRANGE ... BYLEX [REV]` forms they reuse.
- **R343 (ubiquitous):** All-mode tests shall cover `HMSET` replies and field/TTL
  effects, `ZMSCORE` order/nulls/missing keys, both lex aliases with bounds and
  `LIMIT`, wrong types and arity errors, and every worker mode. Documentation
  shall retain unsupported scripting, RDB/RESTORE, multiple databases and
  replication.

## Blocking sorted-set pops (S91)

- **R344 (event-driven):** `BZPOPMIN key [key ...] timeout` and
  `BZPOPMAX key [key ...] timeout` shall inspect keys in argument order, pop one
  member from the first live sorted set and reply `[key, member, score]`. They
  shall preserve the key TTL, reject wrong value types, arity, negative,
  non-numeric and out-of-range timeouts, and never treat the trailing timeout as
  a key for ACL or discovery purposes. A zero timeout shall block without a
  deadline; an expired wait shall return a null array.
- **R345 (state-driven):** While a client is blocked in `BZPOPMIN`/`BZPOPMAX`,
  server-side writes (including `ZADD` from other connections) shall wake it with
  the popped member, bounded by the existing polling cadence. Popped keys shall
  emit the canonical `zpopmin`/`zpopmax` journal event and notification. Inside
  `MULTI`, these commands shall execute without blocking and return the null
  array when no key is ready; `WATCH` conflicts shall still abort the
  transaction.
- **R346 (ubiquitous):** All-mode tests shall cover immediate ordered pops,
  minimum/maximum selection, TTL preservation, timeout and wake behavior,
  wrong-type and invalid-timeout errors, non-blocking transaction execution,
  watch aborts and every worker mode. Documentation shall retain unsupported
  scripting, RDB/RESTORE, multiple databases, replication and cluster routing.

## Combined hash field values and expiry (S-HC)

- **R-HC1 (event-driven):** When HSETEX receives a valid counted field/value block,
  it shall atomically apply all pairs (last duplicate wins), return one on success
  or zero when FNX/FXX fails against the original live fields, preserve the key
  TTL, clear written field TTLs by default, and support KEEPTTL or one explicit
  EX/PX/EXAT/PXAT deadline. A due deadline shall remove targeted fields without
  reserving discarded values; removing the final field shall delete the key.
- **R-HC2 (event-driven):** When HGETEX receives a valid counted field block, it
  shall return binary values/nulls in argument order, preserve TTLs by default,
  and apply PERSIST or one explicit deadline to existing fields only. A due
  deadline shall remove each field after its first returned value, so a later
  duplicate shall return null. Missing keys shall not be created.
- **R-HC3 (unwanted behavior):** If options/counts/times/types are invalid, final
  payload admission fails, or a complete HGETEX RESP reply exceeds 2 MiB including
  duplicates and framing, it shall return one bounded error with no partial reply
  or live value/TTL mutation. Counts shall be positive signed-32-bit values;
  deadlines shall not exceed Redis's verified `(2^48 - 1) >> 2` milliseconds.
  Options may precede or follow the counted FIELDS block; duplicate clauses and
  unsupported options shall be rejected without reflecting arbitrary bytes.
- **R-HC4 (ubiquitous):** Tests shall cover due/zero/equal deadlines with injected
  clocks, repeated/binary fields, FNX races, OOM and reply-size atomicity, errors,
  key/field TTL interactions, discovery/ACL, WATCH/MULTI, persistence and all worker
  modes. No active field-expiry scan, exhaustive vendor event parity, RSS ceiling,
  eviction rollback, replication or full hash-command compatibility is implied.
