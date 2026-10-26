# Atomic multi-key strings: MGET, MSET, MSETNX and MSETEX

## Acceptance (EARS)

- **When** `MGET key [key ...]` receives one or more keys, **the server shall** return one RESP2 array in argument order, with a bulk value for each live key and a null bulk for each missing or expired key. Duplicate keys remain duplicated in the result; empty and arbitrary binary keys/values remain distinguishable and unchanged.
- **When** `MSET key value [key value ...]` receives one or more complete pairs, **the server shall** replace all named values atomically, clear prior TTLs, return `+OK`, and let the final pair win for a repeated key.
- **When** `MSETNX` receives valid pairs, **the server shall** set the complete final key set and return `:1` only if none of its distinct target keys is live; otherwise it shall return `:0` without changing any target. Repeated absent keys are one target and the last value wins.
- **When** `MSETEX numkeys` receives exactly `numkeys` key/value pairs and valid NX/XX/expiry options, **the server shall** atomically set the deduplicated final pairs, apply one shared deadline or KEEPTTL, and return `:1`; failed NX/XX returns `:0` without writes.
- **If** MGET has no keys, or MSET/MSETNX have no pair or an incomplete pair, **then** the server shall return the command arity error without changing data.
- **If** an admitted MSET would exceed the configured key/value payload budget, **then** the server shall return the OOM error and preserve every live value and expiry. Expired payload is reclaimed before admission; rejected MSETNX performs no writes.
- **When** clients issue multi-key commands concurrently, **the server shall** serialize each command against one store lock so no client observes a partial multi-key update.
- **If** the complete RESP2 MGET reply would exceed 2 MiB, **then** the server shall return `-ERR MGET reply limit exceeded` without emitting a partial array. The limit counts array/bulk framing and repeated values.

## Verified behavior

The implementation and standalone wire tests cover binary/empty values, misses, duplicate MGET positions, final-value duplicate replacement, MSETNX all-or-none behavior, MSETEX conditions/expiry/TTL retention and malformed pairs, arity, OOM preservation, and concurrent pair atomicity. Wire tests run per-client, one-worker and four-worker modes. Module tests cover injected-clock expiration, exact payload accounting, TTL clearing and concurrent MSETNX winners.

The official [MGET](https://redis.io/docs/latest/commands/mget/), [MSET](https://redis.io/docs/latest/commands/mset/), [MSETNX](https://redis.io/docs/latest/commands/msetnx/) and [MSETEX](https://redis.io/docs/latest/commands/msetex/) references describe the value-array read, pairwise set, conditional set and shared-expiry batch semantics. Local probes of pinned Redis 8.10.2 and Valkey 9.1.2 observed that MSETNX with duplicate absent keys succeeds, retains the last value, and returns repeated values for duplicate MGET arguments. Rillcache matches these observations.

## Limits and compatibility boundary

These commands operate only on the in-memory string store. RESP2 and the configured bounded request frame/argument/bulk limits still apply. MGET additionally caps its complete encoded reply at 2 MiB to bound duplicate-key response amplification; an over-limit request receives one error and no partial array, a deliberate compatibility limit. The multi-key commands do not add transactions, persistence, eviction, authentication, or support for other data types. The configured payload budget counts unique live key and value bytes, not map/allocator/connection memory or total process RSS. Expiry uses the existing store millisecond visibility rule (a deadline remains live at `now == deadline`). No Redis/Valkey version is reported or emulated.

Wire/reference and deterministic module tests establish the behavior above; they do not establish full Redis compatibility or production readiness.
