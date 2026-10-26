# Payload-budget eviction policies

Rillcache can evict keys when admitting a write would exceed `--max-data-bytes`.
The budget is **key and value bytes**, not allocator overhead, maps, threads or
RSS. Eviction changes a hard OOM budget into a bounded cache budget but does not
make the process memory-safe or provide an RSS limit.

## Policies

| `--maxmemory-policy` | Candidates |
| --- | --- |
| `noeviction` (default) | none; over-budget writes return `OOM` unchanged |
| `allkeys-random` | any key except the key being written |
| `allkeys-lru` | any key, approximate least-recently-mutated |
| `volatile-random` | keys with a TTL only |
| `volatile-lru` | TTL keys, approximate least-recently-mutated |
| `volatile-ttl` | TTL key with the nearest absolute deadline |

`CONFIG GET maxmemory-policy` reports the live policy and authenticated
`CONFIG SET maxmemory-policy <name>` changes it atomically with other supported
`CONFIG SET` pairs. `INFO memory` includes `maxmemory_policy`; `INFO stats`
reports `evicted_keys`, which `CONFIG RESETSTAT` zeroes.

## Admission sequence

For a write with a candidate final payload:

1. If it already fits, no eviction occurs.
2. Logically expired keys are reaped without counting as evictions.
3. If it still does not fit, `noeviction` returns OOM with no state change.
4. Otherwise the server proves the write can fit by evicting eligible candidates
   (excluding the target key, whose replacement bytes are already accounted). If
   even evicting every eligible candidate cannot fit, it returns OOM **before**
   deleting anything.
5. It evicts candidates one at a time until the final payload fits, then applies
   the write atomically. The target key is never selected, so an overwrite cannot
   evict itself.

A write that reaches admission therefore succeeds rather than leaving a partial
eviction. Multi-key writes evict only other keys and admit the entire final
payload. Eviction removes an entry completely, including its TTL index, and
records a WATCH conflict/version change; existing append-only journals capture the
deletion through the normal changed-key snapshot. There is no background eviction
or asynchronous deleter.

## Candidate selection limits

- `allkeys-random`/`volatile-random` rotate a counter over HashMap iteration order;
  this is bounded, reproducible per process sequence but not cryptographic
  randomness or uniformly sampled.
- `allkeys-lru`/`volatile-lru` use the entry's last **mutation** sequence, not read
  recency. Reads do not take the store write lock, so read-based LRU is not
  implemented. Renames/copies receive a fresh mutation rank.
- `volatile-ttl` compares absolute deadlines, so expired keys are removed by the
  prior reaping step rather than counted as evictions.
- Selection scans the eligible key set, so a near-full keyspace can make an
  over-budget write O(number of keys) before admission. There is no sampled-LRU
  approximation or LFU.

## Deliberate gaps

- No true RSS/allocator accounting, `maxmemory` bytes runtime setting, memory
  fragmentation control, active defragmentation, object-size sampling, LFU,
  eviction pools, replica-aware eviction or keyspace-notification events for
  evictions. The keyspace event is not published.
- Eviction is process-local; replicated/cluster policies and persistence of
  eviction policy are absent. The CLI policy must be supplied again after restart;
  `CONFIG SET` is runtime-only.
- Evictions are immediate and blocking under the store write lock; there is no
  background reclaimer, latency SLO or fairness guarantee under sustained pressure.

## Evidence

Unit tests cover noeviction atomic OOM, allkeys random success, volatile policies
refusing to remove persistent keys, nearest-TTL selection, mutation-LRU selection
and journal replay with the same survivors. All-mode wire tests cover policy
configuration, eviction counting, `INFO memory`/`INFO stats`, `CONFIG RESETSTAT`,
runtime switching to noeviction and persistent-key protection.
