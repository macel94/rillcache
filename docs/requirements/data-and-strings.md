# Data and string requirements

Part of the [requirements and incremental delivery index](../requirements.md). Requirements are preserved verbatim from the former monolithic EARS sections; add new requirements only in the matching area file.

## Data

- **R20 (event-driven):** When plain `SET key value` is received within the payload
  budget, the server shall atomically replace the value and reply `+OK`.
- **R21 (event-driven):** When `GET key` is received, the server shall return the
  stored bytes or a null bulk for a missing key, distinguishing an empty value.
- **R22 (unwanted behavior):** If an admitted SET would exceed the key/value
  payload budget, then the server shall return OOM and preserve the previous
  data. Overwrite and deletion shall reclaim payload budget. This is not an absolute RSS limit.
- **R23 (event-driven):** When `DEL key [key ...]` is received, the server shall
  atomically remove existing keys and count each removed key only once.
- **R24 (event-driven):** When `EXISTS key [key ...]` is received, the server shall
  count each existing argument, including duplicates, as Redis does.
- **R25 (event-driven):** When `SET key value NX` is received, the server shall
  atomically create the key and return `+OK` if absent; otherwise it shall return
  a null bulk and preserve the existing value, including an empty value.
- **R26 (event-driven):** When `SET key value XX` is received, the server shall
  atomically replace the value and return `+OK` if present; otherwise it shall
  return a null bulk without creating the key.
- **R27 (unwanted behavior):** If SET options contain both NX and XX or an
  unsupported token, then the server shall return a syntax error before checking
  conditions or changing data. Complete ASCII NX/XX tokens shall be
  case-insensitive; repeated identical conditions shall be accepted.
- **R28 (event-driven):** When concurrent clients perform NX writes to one absent
  key with sufficient payload budget and no intervening deletion, the server
  shall accept exactly one write and preserve that winner's bytes.
- **R29 (state-driven):** While a SET condition is not satisfied, the server shall
  neither allocate stored payload nor consume its budget; admitted writes shall
  retain R22's atomic OOM behavior and budget reclamation.

## Multi-key strings

- **R101 (event-driven):** When `MGET key [key ...]` is received, the server shall
  return one RESP2 array in argument order, preserving duplicate positions,
  empty/binary values and nulls for missing/expired keys from one consistent store
  observation.
- **R102 (event-driven):** When valid `MSET key value [key value ...]` pairs are
  received, the server shall atomically apply the final value for each distinct
  key, clear TTLs on successful replacement, and return `+OK`; if aggregate final
  payload admission fails, no live value or TTL shall be partially changed.
- **R103 (event-driven):** When valid MSETNX pairs are received, the server shall
  return `:0` without writes if any distinct target is live, otherwise atomically
  apply all final values and return `:1`; duplicate absent keys count once and the
  last value wins.
- **R104 (unwanted behavior):** If a complete MGET RESP reply would exceed 2 MiB,
  the server shall return one bounded error response without emitting a partial
  array; it shall not modify any live stored value. Its reply-size check includes
  RESP framing and duplicate results.

## Explicit startup error wording

As an operator, I can choose the verified Redis or Valkey MSETEX error wording
so that existing clients do not have to accept contradictory vendor responses.
This is a small independent string-compatibility increment, not a vendor-mode
claim for every command. Evidence: `tests/error_dialect.rs`, CLI parsing tests
and the offline upstream harness tests.

- **When** `--error-dialect redis|valkey` or `Config.error_dialect` is selected,
  the server **shall** retain that setting for every connection and EXEC batch;
  the default **shall** preserve Valkey's existing error wording.
- **When** a valid positive MSETEX count exceeds the available key/value pairs,
  the server **shall** return Redis's key/value-pair error or Valkey's syntax
  error according to the selected dialect, including protected large counts.
- **If** the setting is malformed, startup **shall** fail; other command,
  option, arity and numeric errors **shall** remain unchanged, and rejected
  writes **shall** preserve values, TTLs and WATCH state in every worker mode.
- **When** the original upstream suites run, the runner **shall** explicitly
  record the candidate's public reference-matched dialect without modifying
  Tcl assertions, exclusions, failure classification or performance settings.

## Transactions and optimistic locking (S26)

- **R105 (event-driven):** When `MULTI` is received outside an active transaction,
  the server shall start queueing connection state and reply `+OK`; a queued
  supported command with valid public arity shall reply `+QUEUED`, and nested
  `MULTI`, `EXEC`/`DISCARD` without `MULTI`, and `WATCH` inside `MULTI` shall
  return documented errors without executing or partially clearing a queue.
- **R106 (event-driven):** When `EXEC` runs after a dirty queue, the server shall
  return `EXECABORT` and execute nothing; otherwise it shall append one RESP array
  containing each queued command's reply in order, with runtime errors as array
  elements and no rollback of already completed commands.
- **R107 (state-driven):** While `EXEC` executes, ordinary commands and background
  reclamation shall not observe or mutate the keyspace between queued commands;
  the queue shall be bounded, owned by the connection, preserve binary arguments,
  survive worker handoffs and be discarded by `DISCARD`/`QUIT`/close.
- **R108 (unwanted behavior):** If a watched key is written, deleted, expires,
  changes TTL, or is cleared after `WATCH`, then `EXEC` shall return a null array
  (RESP2 `*-1`, RESP3 `_`) without changing data; bounded version eviction and
  flushes shall fail closed rather than miss a conflict. `UNWATCH` shall clear all
  watches.

## Expiration and integer compatibility

- **R65 (event-driven):** When SET receives EX/PX/EXAT/PXAT, GET or KEEPTTL with
  valid nonconflicting options, it shall apply the selected condition/value/TTL
  atomically and return Redis-compatible old-value replies. Identical expiry kinds
  may repeat with the final value winning; all syntax and final numeric/overflow
  validation shall precede condition evaluation and mutation.
- **R66 (state-driven):** While an entry has a deadline, reads, conditions and
  multi-key counts shall use Redis's millisecond visibility boundary; observed
  expiration shall not resurrect after a clock rollback. Expiration shall reclaim
  payload, maintain at most one indexed expiry per key and never cause false OOM.
  Injected-clock tests shall not depend on wall-clock sleeps.
- **R67 (event-driven):** When TTL/PTTL/EXPIRETIME/PEXPIRETIME, EXPIRE/PEXPIRE/
  EXPIREAT/PEXPIREAT with NX/XX/GT/LT, or PERSIST is used, the store shall preserve
  missing/persistent sentinels, rounding, option compatibility and failure atomicity.
- **R68 (event-driven):** When SETEX/PSETEX/SETNX/GETSET/GETDEL/GETEX is used, the
  store shall implement value/expiry and reply semantics, including past absolute
  deadlines, GETEX PERSIST and rejected/OOM writes retaining old state.
- **R69 (event-driven):** When INCR/DECR/INCRBY/DECRBY receives a canonical signed
  64-bit integer string (missing means zero), it shall atomically calculate and
  return the new integer and retain an existing TTL. Concurrent updates shall not
  lose or duplicate values.
- **R70 (unwanted behavior):** If integer parsing, arithmetic, DECRBY negation or
  payload admission fails, then the command shall return the corresponding error
  without changing the live value or TTL, in debug and release builds.
- **R71 (event-driven):** When optional native conformance runs, the new command
  common corpus shall match pinned Redis and Valkey wire replies in per-client,
  one-worker and four-worker Rillcache modes before comparative timing begins.
  Deliberate vendor divergences shall have explicit per-vendor expectations;
  Rillcache shall follow the documented Redis behavior.

## Bitmap algorithms (S42)

- **R165 (event-driven):** When `BITCOUNT key [start end [BYTE|BIT]]` receives a
  live string, it shall count set bits in the inclusive normalized range, using
  byte or bit length for negative indexes and clamping, and return `0` for missing
  keys or empty/reversed ranges. Argument counts, integers and unit tokens shall be
  validated before key access.
- **R166 (event-driven):** When `BITPOS key bit [start [end [BYTE|BIT]]]` receives
  bit `0`/`1`, it shall return the first matching MSB-first position or `-1`.
  Without an explicit end, a clear-bit search over a nonempty existing string shall
  treat the right side as zero-padded; a missing key shall return `0` for bit `0`
  and `-1` for bit `1`. Explicit ranges, negative indexes and unit boundaries shall
  be normalized without overflow.
- **R167 (event-driven):** When `BITOP AND|OR|XOR destination source [source ...]`
  or `BITOP NOT destination source` runs, it shall combine the longest source with
  missing/short bytes as zero, require one source for `NOT`, read aliased sources
  from their pre-command values, delete the destination for an empty result, clear
  destination TTL on replacement and return the result byte length.
- **R168 (unwanted behavior):** If a bitmap command has malformed arguments, an
  invalid bit/unit, a wrong-type source or destination admission failure, then it
  shall return one bounded error without partially changing any key. Successful
  mutations shall use exact payload accounting, journal/WATCH versions and atomic
  publication; tests shall cover BYTE/BIT and negative ranges, padding, empty keys,
  aliases, destination TTL, wrong type and all execution modes.

## Arbitrary-width bitfields (S44)

- **R173 (event-driven):** When `BITFIELD key` receives `GET type offset`,
  `SET type offset value` and `INCRBY type offset increment` subcommands, it shall
  execute them left-to-right and return one RESP array element per operation:
  `GET` returns the field, `SET` returns the previous field, and `INCRBY` returns
  the new field. Types shall be signed `i1`..`i64` or unsigned `u1`..`u63`; offsets
  shall be numeric bit positions or `#n` field-width multiples within the bounded
  32-bit offset range. Missing bytes shall read as zero.
- **R174 (event-driven):** While `OVERFLOW WRAP|SAT|FAIL` applies to following
  `SET`/`INCRBY` operations, the server shall wrap (default), saturate or return a
  null array element without writing when the value cannot fit. Successful writes
  shall zero-fill gaps, preserve TTL and publish the complete new string only after
  all parse, offset and payload-admission checks succeed.
- **R175 (event-driven):** `BITFIELD_RO` shall permit only `GET` (and `OVERFLOW`)
  and return `ERR BITFIELD_RO only supports the GET subcommand` for writes. A
  wrong-type key, invalid type/offset/value/overflow token or payload-budget
  failure shall return one bounded error without exposing partial writes.
- **R176 (ubiquitous):** The commands shall publish truthful command metadata and
  the `@bitmap` ACL category, never require external dependencies or unsafe code,
  and be covered by unit and all-mode wire tests for signed/unsigned fields,
  `#` offsets, overflow policies, zero-fill, TTL, RO/error paths, transactions and
  OOM atomicity. `u64` remains unsupported and no Redis-internal encoding guarantee
  is claimed.
