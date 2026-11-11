---
type: storage
title: Data model, expiration and memory accounting
description: Rillcache's per-key Entry, absolute key and field expiry, lazy and maintenance reaping, payload accounting, WATCH versions and bounded eviction policies.
tags: [storage, expiry, ttl, accounting, eviction, watch]
verified:
  - by: openwiki/0.9.0
    at: 2026-11-11T08:27:05.780Z
sources:
  - id: openwiki-source-25016b93f9cf5f97ffcc38b0
    resource: repo://docs/collections.md
  - id: openwiki-source-b1fffd486916f9c015825d3d
    resource: repo://docs/eviction.md
  - id: openwiki-source-14edd632e05711377b527b45
    resource: repo://docs/keyspace.md
  - id: openwiki-source-ccfe8cc4504f25c304787a46
    resource: repo://src/engine.rs
  - id: openwiki-source-b156f0c5004fc1ca4c6d7464
    resource: repo://src/engine/data.rs
  - id: openwiki-source-bd650e00cd7b5e0604f48bac
    resource: repo://src/engine/eviction.rs
  - id: openwiki-source-8c8ffef1dc5c6d8b8898d899
    resource: repo://src/engine/expiry_tests.rs
generated: { by: "opencode", at: "2026-11-02T15:32:39.597Z" }
---

# Data model, expiration and memory accounting

Every Rillcache key is one `Entry` in one `Data` map. The entry may hold a string,
a collection or a stream, and always carries the metadata needed for expiry,
accounting, access ranking and optimistic locking. There is no separate keyspace
for a data type.

## The Entry

```text
Entry
  value: Vec<u8>              raw string bytes
  collection: Option<Collection>
  stream: Option<Stream>
  payload_len: usize
  expires: Option<i64>        absolute Unix milliseconds
  field_ttl: BTreeMap<field, deadline>
  last_access: u64            mutation rank, not a read clock
```

At most one of `collection` and `stream` is set. A plain string has neither.
`payload_len` counts the value bytes or collection/stream field bytes, excluding
the key length and container overhead. The key's own length is added when `Data`
computes the total payload.

`Data` also holds:

- `entries`: the key map;
- `payload_bytes` and `peak_payload_bytes`;
- `expirations`: a sorted index of absolute key deadlines;
- `versions` and `version_seq` for `WATCH`;
- `journal_events`, a bounded mutation log;
- `field_ttls`: the number of field deadlines for fast skipping;
- `expired_keys`, `expired_subkeys` and `evicted_keys` counters;
- bounded expired/evicted lifecycle queues;
- the active eviction policy and sequence counters.

## Absolute time

A key's `expires` is an absolute Unix-millisecond deadline. A relative `EX`, `PX`
or `EXPIRE` is converted at command time; `EXAT`/`PXAT`/`PEXPIREAT` stores the
supplied deadline. No duration is persisted or recomputed on replay.

An entry is alive while `now <= deadline`. The boundary matters: at exactly the
deadline, `TTL` can report zero and the key is still readable; one millisecond
later it is expired. Tests use an injected clock and future absolute timestamps
rather than sleeping.

Field TTLs work the same way per hash field. `HEXPIRE`/`HPEXPIRE` and their `AT`
forms store an absolute deadline in `field_ttl`. Field expiry is lazy: it runs
when the hash key is accessed. Removing the last field removes the key. A field
deadline at or before now is applied immediately on the next relevant command.

## Lazy and active expiry

There are two reclamation paths:

- **Lazy**: a command that touches a key first checks its deadline and removes an
  expired entry before reading or writing.
- **Active**: the maintenance loop calls `purge_expired(128)` about every 100 ms,
  removing at most 128 keys from the front of the sorted expiry index.

The maintenance limit is bounded work, not a deadline for every expired key. A
key that has crossed its deadline but has not been touched or reaped still
occupies memory and can appear in payload accounting, though logical commands
treat it as absent. `DBSIZE`, `KEYS`, `RANDOMKEY` and scans reap due keys before
producing their result.

Key expiry emits a bounded `expired` lifecycle event when the matching
notification class is enabled. Field expiry increments `expired_subkeys` but
does not emit Redis's full `hexpired` event family.

A wall-clock rollback can resurrect an unobserved expired key because the
deadline is compared with the current wall clock. The persistence layer stores
absolute deadlines, but it has no clock high-water mark; this is a recorded
durability gap, not a monotonic guarantee.

## Mutation, versions and journal capture

Every successful mutation calls `Data::touch`:

1. updates the peak payload high-water mark;
2. increments the global version sequence;
3. updates the key's `WATCH` version;
4. records the key in the bounded journal-event queue if effects are enabled.

If the version table reaches 65,536 entries, it is cleared and the store epoch is
advanced. Outstanding watches then fail closed rather than silently missing a
mutation. If the journal-event queue reaches 65,536 entries, it clears and sets an
overflow flag; the next journal commit emits a full `FLUSHALL` snapshot instead of
incremental keys.

`remove` updates payload and expiry indexes, touches the version and journal
state, and returns the owned entry. `rename` transfers the owned entry after
admission; `copy` deep-clones it. Both preserve the source TTL according to the
documented command rules.

## Payload accounting

The configured `max_data_bytes` is a key-and-value byte budget:

```text
key bytes + string bytes
key bytes + member/field bytes for collections
key bytes + field/value bytes for streams
```

It excludes:

- `HashMap`/`BTreeMap` capacity and node overhead;
- `Vec` spare capacity and allocator slack;
- expiry and version indexes;
- thread stacks, sockets and network buffers;
- journal buffers and metadata.

Consequently, process RSS can be much larger than the configured payload. The
`INFO memory` and `MEMORY STATS` fields say this explicitly and do not report a
fabricated `used_memory` or RSS value.

Admission is atomic. A command computes its candidate final payload and calls
`Data::reserve`. If the write fits, it proceeds. Otherwise the store first reaps
logically expired keys, then applies eviction if enabled. If even a complete
eligible eviction pass cannot fit, the command returns `OOM` before deleting
anything. Multi-key writes validate the whole final payload before changing any
key.

Collection and stream writes replace the entire value rather than mutating live
bytes in place. This keeps a failed admission from leaving a partial collection.
An equal-size string overwrite reuses the existing allocation; a disproportionate
shrink releases the old buffer instead of retaining an arbitrarily large one.

## Eviction

`--maxmemory-policy` selects:

| Policy | Eligible victims |
| --- | --- |
| `noeviction` | none |
| `allkeys-random` | any key except the target |
| `allkeys-lru` | any key, approximate mutation recency |
| `volatile-random` | keys with a TTL only |
| `volatile-lru` | TTL keys, approximate mutation recency |
| `volatile-ttl` | TTL key with the nearest deadline |

Eviction runs only when an admitted write would otherwise exceed the budget. It
reaps expired keys first and never treats those removals as evictions. A volatile
policy refuses to remove persistent keys, so it can still return OOM while above
budget. The target key is excluded from victim selection.

LRU is **mutation recency**, not read recency: reads do not take the write lock,
so a read-idle clock would be fabricated. Random selection rotates a counter over
hash-map iteration order; it is reproducible per process but not uniformly
sampled. Candidate selection scans eligible keys, so a near-full store can make an
over-budget write O(number of keys).

Eviction is immediate and runs under the store lock. There is no background
evictor, sampled-LRU approximation, LFU, active defragmentation or RSS limit. An
evicted key is removed with its TTL and version state and emits a bounded
`evicted` event when that class is enabled.

## Keyspace inspection

`TYPE` reads the live representation. `DEL`/`UNLINK` remove it. `EXISTS` and
`TOUCH` count repeated arguments. `DBSIZE` reaps and counts live keys. `KEYS`
matches binary patterns with a bounded glob budget. `RANDOMKEY` selects a live
key rather than always returning the first.

`SCAN` uses a decimal cursor as an index into a sorted snapshot taken under the
store lock. It is a traversal hint, not a stable identity: a stable keyspace
returns each element once, while concurrent insertion/deletion can skip or
duplicate. `COUNT` is a work hint, capped at 10,000. `HSCAN`, `SSCAN` and `ZSCAN`
sort fields/members by raw bytes; `ZSCAN` returns the current score.

`COPY source destination [DB 0] [REPLACE]` deep-clones the value and TTL after
admission. `RENAME` moves the owned entry. Neither creates a second keyspace or a
second database.

## Invariants

- A live key has no expired state; a logically expired key is absent to commands.
- Removing the last list element, set member, hash field or stream entry removes
  the key.
- `SET` replaces any existing type; string-only reads reject non-strings with
  `WRONGTYPE`; `MGET` returns null for a non-string.
- A rejected write leaves the old value, TTL, payload and expiry index unchanged.
- Payload accounting is exact for counted bytes but not an RSS bound.
- `WATCH` may abort early after version-table eviction; it cannot miss a change.
- Eviction and expiry are process-local, and neither is replicated.

The persistence contract is in
[persistence and recovery](persistence-and-recovery.md). The observation fields
that expose this state are in
[observability and health](../operations/observability-and-health.md).
