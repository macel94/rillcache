# Bitmap, LCS, and floating-point string operations

This increment implements `SETBIT`, `GETBIT`, `BITCOUNT`, `BITPOS`, `BITOP`
(`AND`/`OR`/`XOR`/`NOT`), `BITFIELD`/`BITFIELD_RO`, `LCS` (`LEN`, `IDX`,
`WITHMATCHLEN`, and `MINMATCHLEN`), and `INCRBYFLOAT`. `SUBSTR` is supported as
the legacy alias of `GETRANGE`.

- Bitmap bits are numbered most-significant-bit first within each byte. Reads of
  missing/out-of-range bytes return zero; writes atomically zero-fill gaps, retain
  the existing expiry, and count key/value payload against the configured budget.
  Offsets are checked against the 32-bit Redis bit-offset range and the 512 MiB
  string semantic limit before allocation.
- `BITCOUNT key [start end [BYTE|BIT]]` counts set bits over an inclusive range.
  Negative indexes are relative to the byte or bit length; explicitly selected
  `BYTE`/`BIT` units use that length for normalization and clamping. Missing keys
  return zero. `BITPOS key bit [start [end [BYTE|BIT]]]` returns the first matching
  MSB-first position, or `-1`. A clear-bit search without an explicit end treats
  the right side of a nonempty existing string as zero-padded; a missing key returns
  `0` for bit `0` and `-1` for bit `1`. Argument grammar, integer parsing and
  `BYTE`/`BIT` validation happen before key access.
- `BITFIELD key [GET type offset] [SET type offset value] [INCRBY type offset
  increment] [OVERFLOW WRAP|SAT|FAIL]` executes signed `i1`..`i64` and unsigned
  `u1`..`u63` fields left-to-right. Offsets are bit positions or `#n` field
  multiples; `SET` returns the previous field value and `INCRBY` the new value.
  `OVERFLOW` applies to following writes: `WRAP` is the default, `SAT` clamps and
  `FAIL` returns a null element without writing. Missing bytes read as zero and
  writes zero-fill gaps. `BITFIELD_RO` accepts only `GET` (and `OVERFLOW`) and
  returns the documented error for writes. All subcommands are parsed before key
  access; the final value is payload-admitted and published once.
- `BITOP AND|OR|XOR destination source [source ...]` combines the longest source
  with missing/short positions treated as zero; `NOT` requires exactly one source.
  An empty result deletes the destination. The destination is replaced as a string,
  clearing its TTL; source aliases are read from their pre-command values, and any
  wrong-type source leaves the destination unchanged. Result admission follows the
  configured payload budget and updates journal/WATCH/expiry accounting atomically.
- `LCS` uses a full byte-oriented dynamic-programming table to preserve upstream
  tie-breaking and IDX range ordering. The table is rejected above 4,194,304
  cells (16 MiB) rather than allocating unbounded quadratic memory/work. Missing
  keys are empty strings. `LEN` returns an integer; IDX returns the RESP2 array
  representation of the Redis map (`matches`, then `len`).
- `INCRBYFLOAT` parses finite UTF-8 floating-point values, rejects invalid input
  and non-finite results without mutation, normalizes negative zero to `0`,
  preserves TTL, and uses Rust's shortest-round-trip decimal representation.
  Binary/non-UTF-8 strings and whitespace-padded values are invalid.

The implementation uses the existing store write lock for atomic updates and
exact payload accounting; no new Rust dependencies are introduced. `BITCOUNT`
uses byte popcounts with edge masks, while `BITPOS` scans only the requested
bit positions without allocating a full bit expansion. Tests cover binary bit
operations, BYTE/BIT and negative ranges, padding/empty/missing behavior, alias
and empty-result `BITOP`, TTL clearing, wrong types, bounded writes, malformed
arguments, finite-result rejection, LCS LEN/IDX framing, and the SUBSTR alias in
per-client, one-worker and four-worker modes. This is not full Redis compatibility:
allocator exhaustion behavior and cross-platform long-double formatting differ from
native implementations, and the explicit LCS
budget is stricter than Redis's configured transient-memory bound.

Reference research: the pinned Redis 8.10.2 and Valkey 9.1.2 Tcl cases
(`tests/unit/type/string.tcl`, `incr.tcl`) and native implementations
(`src/bitops.c`, `src/t_string.c`) were inspected. Official command references:
[SETBIT](https://redis.io/docs/latest/commands/setbit/),
[GETBIT](https://redis.io/docs/latest/commands/getbit/),
[BITCOUNT](https://redis.io/docs/latest/commands/bitcount/),
[BITPOS](https://redis.io/docs/latest/commands/bitpos/),
[BITOP](https://redis.io/docs/latest/commands/bitop/),
[BITFIELD](https://redis.io/docs/latest/commands/bitfield/),
[BITFIELD_RO](https://redis.io/docs/latest/commands/bitfield_ro/),
[LCS](https://redis.io/docs/latest/commands/lcs/),
[INCRBYFLOAT](https://redis.io/docs/latest/commands/incrbyfloat/).
