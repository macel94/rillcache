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
CONFIG SET notify-keyspace-events KEg$lshztxe
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
| `x` | expired-key events |
| `e` | evicted-key events |
| `m` | key-miss events for supported reads |
| `n` | new-key events for first creation |
| `A` | alias for `g$lshztxe` (deliberately excludes `m` and `n`) |

`m` reports a lookup that found no live key: `GET` and each missing `MGET`
occurrence emit `keymiss` on the normal keyspace/keyevent channels. Hits and
existing non-string keys do not. Other read commands are not covered yet.

`n` reports a key that did not exist and was created by the command: `SET` on a
missing key, a new hash/list/set/zset/stream key, and a `COPY`/`RENAME`
destination that was absent. Overwrites, additional stream entries and read
misses do not emit `new`.

Module classes (`d`) and any unknown character are rejected by startup/config
validation rather than silently accepted. Empty flags disable publication (the
default).

While none of the lifecycle classes (expired, evicted, hash, generic, key-miss,
new-key) is enabled, command dispatch skips the lifecycle-delivery drain entirely instead
of taking a store write lock to discard keys. Pending lifecycle keys are bounded and are cleared whenever the
notification flags change, so enabling a class never replays events observed
before that point.

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
pop success emits `lpop`/`rpop`. `FLUSHDB`/`FLUSHALL` and journal-overflow full
snapshots deliberately do not emit keyless or per-key events, so consumers must
not rely on notifications as an exhaustive audit or cache-coherence guarantee.
No event is queued for offline subscribers or replayed after reconnect.

For successful `MSETEX` writes, notifications follow the supplied key/value pair
order, including repeated keys: each pair emits `set` (`$` class), immediately
followed by `expire` (`g` class) when EX/PX/EXAT/PXAT assigns a deadline.
KEEPTTL and writes without an expiry emit only `set`. Class and K/E flags filter
these events independently. Rejected NX/XX, malformed arguments and payload OOM
emit no write events, even when that command reaps unrelated expired keys;
those removals can still emit their separate `expired` lifecycle events.
This does not change the existing past-deadline storage/removal boundary or make
notification delivery durable.

Expiration and eviction use the separate `expired` and `evicted` event names
(payload/key pair shaped like every other event) and are enabled by `x` and `e`
respectively. `expired` is emitted when a key is logically removed on lookup or
by bounded background reclamation, not when a TTL is merely set. `evicted` is
emitted only when a configured eviction policy actually removes a victim; a
`noeviction` rejection emits no `evicted` event. Expiry/eviction detection records
keys in bounded per-kind queues (1024 entries each) that are drained after the
triggering command or maintenance pass. Lifecycle events are published before the
triggering command's own mutation events, so a key recreated by that command
reports `expired` before its `set`. Under sustained pressure older lifecycle
records can be dropped, and events are never duplicated or retried.
Command events for a key that was itself chosen as an eviction victim in the same
command are suppressed so an eviction does not appear as a successful mutation.

Hash field expiry uses its own lifecycle pair. A lazy pass that removes one or
more expired fields publishes a single `hexpired` event for that key (`h` class),
and, when the last field disappeared and the key itself was removed, a generic
`del` event (`g` class) afterwards. The events are published even when the
command that observed the expiry fails or writes nothing, because the removal
really happened; an unrelated rejected write still emits no hash command event.
Field expiry is detected only when the hash key is touched by a command, so the
event can be delayed until the next access.

An explicit hash delete (`HDEL` or `HGETDEL`) publishes `hdel` when it removed at
least one field, followed by `del` (`g` class) when that removal emptied the
hash. A no-op delete, or a command that only observed lazy field expiry, emits
neither `hdel` nor `del`; the expiry path owns those events and reports them
once.

Supported event names mirror the command family: `set`, `del`, `expire`,
`persist`, `rename_from`, `rename_to`, `copy_to`, `append`, `setrange`, `setbit`,
`incrby`, `decrby`, list push/pop/insert/trim/rem, set add/remove/pop/move/store,
hash set/delete/get-delete/counter/expire/persist, sorted-set
add/remove/counter/pop/range-removal/store algebra/multi-pop, stream
add/delete/group/read/ack/claim/trim and `pfadd` for HLL changes. `GEOADD` maps to
`zadd`; `GEOSEARCHSTORE` emits `geosearchstore` when it writes the destination and
`del` when an empty result removes it. Unsupported or
unknown mutating commands emit nothing. Event names are command-family labels,
not a complete replay log.

`HEXPIRE`/`HPEXPIRE`/`HEXPIREAT`/`HPEXPIREAT` publish `hexpire` (`h` class) only
when at least one supplied field's deadline actually changes, and `HPERSIST`
publishes `hpersist` only when at least one field TTL is removed. Conditions that
leave every field unchanged (an `NX`/`XX`/`GT`/`LT` conflict or a missing field)
emit nothing and do not advance the key's `WATCH` version.

`HSETEX` reports the effective write rather than a generic success. A normal
write publishes `hset`, followed by `hexpire` only when EX/PX/EXAT/PXAT assigned a
future deadline; `KEEPTTL` and no-TTL writes publish `hset` alone. A deadline in
the past publishes `hdel` for the removed fields, plus `del` when that emptied the
hash. A condition miss or a past-deadline write that removed nothing emits no
event and does not advance the key's `WATCH` version.

`HGETEX` never emits a write event. `PERSIST` publishes `hpersist` only when it
actually removed a field TTL, and `EX`/`PX`/`EXAT`/`PXAT` with a future deadline
publishes `hexpire` when at least one returned field was updated. Missing fields
emit nothing, and no synthetic `hset` is emitted. `HGETEX` past-deadline
deletion events remain out of scope in this increment.

When notifications are enabled, mutating commands take the store's exclusive
transaction gate so change capture and publication keep command order. This is a
throughput cost; default-off mode is unchanged. Notifications are published after
the changed keys are captured and before the client reply is written; ordering
relative to other connections is therefore not a global happens-before guarantee.

## Limits

- No module events; no `FLUSH*` keyless event. Key-miss covers only
  `GET`/`MGET`; new-key covers supported creation paths.
- Expired/evicted records use bounded in-memory queues, so lifecycle events may be
  lost under queue pressure and are not a complete historical audit.
- No retry, acknowledgment, durable queue, replay, replication, cluster routing or
  ordering across independent publishers. Delivery is only to currently connected
  subscribers and can be lost under queue pressure/disconnect.
- No per-event filtering beyond the flag classes, no keyspace-notification
  counters in `INFO`, and no guarantee that an event is observed before the
  mutating client receives its reply.
- Event names/category mapping are approximate for compound commands and pending
  stream metadata; use the affected key state as the source of truth.

## Evidence

Unit tests cover flag parsing/round-trip, `A` expansion and rejection of
unsupported classes, plus injected-clock lazy/reaped expiry and eviction
publication. Hash-field tests cover the rejected-write case and the
`hexpired`-then-`del` order when the final field expires, plus `hdel`-then-`del`
for an emptied hash, the no-op delete case, the `HSETEX` write/`hexpire` versus
delete/`del` pairs, and `HGETEX` `hexpire`/`hpersist` without a write event.
All-mode wire tests cover
keyspace/keyevent frames, category filtering, runtime enable/disable,
`MSET`/`MULTI`, ordered MSETEX set/expire pairs, duplicate/binary keys, KEEPTTL
and rejected writes, hash/list/set/zset/stream/HLL events, hash field
`hexpire`/`hpersist` with no-op condition checks, `HDEL`/`HGETDEL` deletion
pairs, `GET`/`MGET` `keymiss` with hit/non-string exclusions, first-creation
`new` events including overwrite and second-entry exclusions, expire, rename,
eviction/`expired` classes, blocking-pop events, binary channels and
`CONFIG`/`INFO` exposure.
