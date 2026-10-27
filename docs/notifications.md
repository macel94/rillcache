# Keyspace notifications

Rillcache can publish Redis-shaped keyspace/keyevent notifications to normal
pub/sub channels when supported commands mutate keys. Notifications are
at-most-once, process-local, not persisted and not replicated.

## Configuration

```sh
rillcache --notify-keyspace-events KEA
```

The flag string is also runtime-configurable:

```text
CONFIG SET notify-keyspace-events KEg$lshzt
CONFIG GET notify-keyspace-events
```

| Flag | Meaning |
| --- | --- |
| `K` | publish `__keyspace@0__:<key>` messages with the event name |
| `E` | publish `__keyevent@0__:<event>` messages with the key |
| `g` | generic events: del/expire/persist/rename/copy |
| `$` | string events |
| `l` | list events |
| `s` | set events |
| `h` | hash events |
| `z` | sorted-set events |
| `t` | stream events |
| `A` | alias for `g$lshzt` |

`A` deliberately differs from Redis's alias by excluding expired/evicted,
key-miss, new-key and module classes. Those classes (`x`, `e`, `m`, `n`, `d`) and
any unknown character are rejected by startup/config validation rather than
silently accepted. Empty flags disable publication (the default).

## Delivery

For each changed key, the server publishes to:

```text
__keyspace@0__:<key>   payload: <event>
__keyevent@0__:<event> payload: <key>
```

The two classes obey their `K`/`E` flags and the event category flag. Channels and
keys are binary-safe; subscribers can use exact `SUBSCRIBE` or `PSUBSCRIBE`
patterns. Message/pattern-message frames follow the connection's RESP2/RESP3
pub/sub mode and normal ACL channel patterns must allow the notification channel
at subscription time. There is no separate notification ACL bypass.

Events are emitted only for keys the command actually changed. A failed or
non-mutating command (for example rejected `SET NX`, overflow, wrong type) emits
nothing. `MULTI` emits each queued command's events in execution order; blocking
pop success emits `lpop`/`rpop`. `FLUSHDB`/`FLUSHALL`, expiry/eviction/reaping and
journal-overflow full snapshots deliberately do not emit events, so consumers must
not rely on notifications as an exhaustive audit or cache-coherence guarantee.
No event is queued for offline subscribers or replayed after reconnect.

Supported event names mirror the command family: `set`, `del`, `expire`,
`persist`, `rename_from`, `rename_to`, `copy_to`, `append`, `setrange`, `setbit`,
`incrby`, `decrby`, list push/pop/insert/trim/rem, set add/remove/pop/move/store,
hash set/delete/counter, sorted-set add/remove/counter/pop/range-removal, stream
add/delete/group/read/ack/claim and `pfadd` for HLL changes. `GEOADD` maps to
`zadd`; `GEOSEARCHSTORE` emits `geosearchstore` when it writes the destination and
`del` when an empty result removes it. Unsupported or
unknown mutating commands emit nothing. Event names are command-family labels,
not a complete replay log.

When notifications are enabled, mutating commands take the store's exclusive
transaction gate so change capture and publication keep command order. This is a
throughput cost; default-off mode is unchanged. Notifications are published after
the changed keys are captured and before the client reply is written; ordering
relative to other connections is therefore not a global happens-before guarantee.

## Limits

- No expired/evicted/keymiss/new-key/module events; no `FLUSH*` keyless event.
- No retry, acknowledgment, durable queue, replay, replication, cluster routing or
  ordering across independent publishers. Delivery is only to currently connected
  subscribers and can be lost under queue pressure/disconnect.
- No per-event filtering beyond the flag classes, no keyspace-notification
  counters in `INFO`, and no guarantee that an event is observed before the
  mutating client receives its reply.
- Event names/category mapping are approximate for compound commands and pending
  stream metadata; use the affected key state as the source of truth.

## Evidence

Unit tests cover flag parsing/round-trip and rejection of unsupported classes.
All-mode wire tests cover keyspace/keyevent frames, category filtering, runtime
enable/disable, `MSET`/`MULTI`, hash/list/set/zset/stream/HLL events, expire,
rename, blocking-pop events, binary channels and `CONFIG`/`INFO` exposure.
