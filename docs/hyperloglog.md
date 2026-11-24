# HyperLogLog cardinality estimates

Rillcache implements `PFADD`, `PFCOUNT` and `PFMERGE` as a bounded approximate
cardinality type. HyperLogLog values are stored as ordinary Redis-style string
payloads, so `TYPE` reports `string`, and expiry, `DEL`, `RENAME`, `COPY`, the
append-only journal and string payload accounting apply through the shared
keyspace lifecycle.

## Commands

- `PFADD key [element ...]` creates a missing value, adds distinct elements and
  returns `1` when the value was created or its sketch changed, otherwise `0`.
  Calling it with no elements creates an empty value.
- `PFCOUNT key [key ...]` estimates the cardinality of the union of the named
  values. Missing keys contribute nothing.
- `PFMERGE destination [source ...]` merges the destination's existing value and
  every named source into the destination. It preserves the destination TTL,
  creates an empty destination for missing inputs, and returns `+OK`. A
  destination-only call is valid.

`PFCOUNT` is read-only in this implementation. `PFADD` and `PFMERGE` preserve an
existing destination value's TTL and fail atomically on wrong type, malformed HLL
payload or payload-budget admission failure. String values that do not carry the
private HLL header return Redis's
`WRONGTYPE Key is not a valid HyperLogLog string value.` error; non-string keys
use the ordinary `WRONGTYPE` error.

## Diagnostics

`PFSELFTEST` runs a deterministic self-test of the private representation — the
register rank bound, the zero-occupancy estimator, monotonicity after adds,
exact/dense payload round-trips and rejection of truncated or corrupted
payloads — and replies `+OK` (or a named `ERR HyperLogLog self-test failed:
<step>`). It reads no stored key and says nothing about Redis-format payloads.

`PFDEBUG` supports two read-only subcommands over a key:

- `PFDEBUG ENCODING <key>` returns `exact` or `dense`, this project's private
  encoding names, not Redis's `sparse`/`dense` payloads.
- `PFDEBUG GETREG <key>` returns the 16,384 register values as an integer array.
  An exact value is projected into a zeroed array without mutating the stored
  value or its WATCH version; a dense value returns its stored registers. A
  missing or expired key reports the empty exact value (all-zero registers,
  `exact`). The diagnostic itself does not create or reap keys.

Redis's `PFDEBUG DECODE`/`TODENSE` payload conversions return an explicit
`ERR PFDEBUG supports only ENCODING and GETREG for this private representation`
error because there is no Redis sparse encoding to convert. Both diagnostics are
`@admin`, `@dangerous` and `@read` commands, so they need the normal ACL grants
for those categories or an all-key/admin user.

## Representation and limits

The private, versioned payload is deliberately **not** Redis's sparse/dense HLL
byte encoding:

- Values with at most 4,096 distinct 64-bit element hashes use an exact sorted
  hash set. This keeps small-cardinality results exact and payload size proportional
  to distinct elements.
- Larger values promote to a 2^14-register dense sketch (16,384 one-byte
  registers, 16 KiB payload). Cardinality then uses linear counting for low
  occupancy and the standard HyperLogLog estimator otherwise, with the usual
  approximately 0.81% standard error at large cardinalities.
- A promotion or merge is staged and payload-admitted before the value changes.
  Dense registers never exceed the representable rank, and malformed/truncated
  payloads are rejected rather than guessed.
- Hash collisions can make an exact small set slightly undercount; the dense
  estimator can over- or under-count. Results are estimates, not exact set
  membership or durable counts.

This increment does not claim interoperability with Redis/Valkey HLL payloads,
Redis's `PFDEBUG` payload encodings, sparse-encoding thresholds, or `CONFIG SET
hll-sparse-max-bytes`. It also does not add replication, cluster/key-slot or
cross-version persistence guarantees. The append-only journal reconstructs the
stored string payload directly.

## Verification

Unit tests cover exact creation, duplicate adds, empty elements, exact and dense
promotion, malformed payload rejection, wrong types, destination merging, TTL,
the deterministic self-test, the exact-to-register projection, the missing-key
diagnostic, TTL preservation and payload bounds. `tests/hyperloglog.rs` exercises
the commands, diagnostics, errors, dense cardinality tolerance and transaction
queueing through RESP in managed per-client, one-worker and four-worker modes. The
same-binary journal regression verifies exact and dense payload bytes,
cardinality and absolute expiry after compaction and restart with both CRC and
authenticated journals. It does not test upstream byte interchange or
compatibility across releases.
