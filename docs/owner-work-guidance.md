# Owner occupancy of O(N) commands

**Scope: one development host, advisory measurement, not a latency contract.**
The profile has **no per-command deadline, cancellation or CPU quota**: a
whole-keyspace command occupies the single owner until it finishes. This page
measures how long each documented O(N) command actually holds the execution turn,
so operators can keep them below the sizes that matter for their own latency
targets instead of guessing.

```sh
python3 scripts/owner_work_profile.py --binary target/release/rillcache \
    --keys 20000 --collection 20000
```

Each row is one command on an otherwise idle server with an otherwise populated
keyspace (20 000 string keys, a sorted set with 20 000 members and a hash with
20 000 fields); the reported time is the client-observed wall time for that single
call, which equals the owner's occupancy because one command owns the turn.

## Measured

| Command | Keys | Collection | Owner occupancy ms |
|---|---:|---:|---:|
| KEYS * | 20 000 | 20 000 | 352.23 |
| SCAN full iteration (COUNT 1 000 000) | 20 000 | 20 000 | 164.94 |
| ZRANGEBYSCORE all | 20 000 | 20 000 | 332.24 |
| ZRANGE 0 -1 | 20 000 | 20 000 | 393.97 |
| HGETALL (20 000 fields) | 20 000 | 20 000 | 812.31 |
| SORT (single-element source) | 20 000 | 20 000 | 1.09 |

## Declared position

- These commands are **excluded from the profile's latency contract**: while one
  runs, no other client is served and no deadline interrupts it. The measured
  occupancies above (165–812 ms at 20 000 items) are two to three orders of
  magnitude above the ~1 ms per-command cost the workload evidence reports, so a
  deployment that needs bounded latency must not issue them against large
  collections.
- Guidance, derived from the table rather than asserted: keep whole-keyspace
  enumeration and full-collection reads at or below ~1 000 items when a
  single-digit-millisecond turn matters, and treat 20 000 items as a
  hundreds-of-milliseconds stall. `SORT` on a small source is cheap (1.09 ms);
  its cost follows the sorted and looked-up cardinality, not the keyspace size.
- There is **no** cancellation path today: `CLIENT KILL` closes the connection but
  the owner still finishes the command before serving anyone else, and no timeout
  aborts it. Any future deadline/quota must come with its own evidence.

## Retained limits

- One development host, one client, no concurrent load, no controlled hardware
  (`P2`). Sizes above 20 000 items, `SORT` with large sources, `GEOSEARCH` and
  full-state rewrites (`BGREWRITEAOF`) are **not** measured here; the rewrite
  pause is measured separately by the [durability profile](persistence-performance.md)
  (`D10`) and the storage-fault drills.
- No SLO is inferred. This page bounds what the operator must avoid, not what the
  server guarantees.
