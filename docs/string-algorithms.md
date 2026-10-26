# Bitmap, LCS, and floating-point string operations

This increment implements `SETBIT`, `GETBIT`, `LCS` (`LEN`, `IDX`,
`WITHMATCHLEN`, and `MINMATCHLEN`), and `INCRBYFLOAT`. `SUBSTR` is supported as
the legacy alias of `GETRANGE`.

- Bitmap bits are numbered most-significant-bit first within each byte. Reads of
  missing/out-of-range bytes return zero; writes atomically zero-fill gaps, retain
  the existing expiry, and count key/value payload against the configured budget.
  Offsets are checked against the 32-bit Redis bit-offset range and the 512 MiB
  string semantic limit before allocation.
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
exact payload accounting; no new Rust dependencies are introduced. Tests cover
binary bit operations, bounded writes, malformed arguments, finite-result
rejection, LCS LEN/IDX framing, and the SUBSTR alias. This is not full Redis
compatibility: allocator exhaustion behavior and cross-platform long-double
formatting differ from native implementations, and the explicit LCS budget is
stricter than Redis's configured transient-memory bound.

Reference research: the pinned Redis 8.10.2 and Valkey 9.1.2 Tcl cases
(`tests/unit/type/string.tcl`, `incr.tcl`) and native implementations
(`src/bitops.c`, `src/t_string.c`) were inspected. Official command references:
[SETBIT](https://redis.io/docs/latest/commands/setbit/),
[GETBIT](https://redis.io/docs/latest/commands/getbit/),
[LCS](https://redis.io/docs/latest/commands/lcs/),
[INCRBYFLOAT](https://redis.io/docs/latest/commands/incrbyfloat/).
