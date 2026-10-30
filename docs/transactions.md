# Client transactions and optimistic locking

Rillcache implements Redis-style `MULTI`, `EXEC`, `DISCARD`, `WATCH` and
`UNWATCH` for its supported command set. A transaction is a way to group
commands, not a claim of Redis scripting or full isolation beyond the limits
listed below.

## Queueing

- `MULTI` starts queueing and returns `+OK`. Nesting returns
  `ERR MULTI calls can not be nested`.
- While queueing, a supported command with command-table arity returns
  `+QUEUED` and is stored by value in connection state. Queue length is bounded
  at 10,000 commands.
- An unknown command, a public command-table arity error, or a connection-local
  command (`HELLO`, `CLIENT`, `SELECT`, `AUTH`, subscription and `PUBSUB`
  introspection commands) is reported immediately and marks the transaction
  dirty. `EXEC` then returns
  `EXECABORT Transaction discarded because of previous errors.` without running
  anything. Blocking pop commands are also rejected at queue time.
- `PUBLISH` and `SPUBLISH` are regular queued commands. They do not deliver
  anything while queueing; `EXEC` publishes each message once, in queue order,
  through the normal channel or sharded-channel fan-out and returns the delivered
  subscriber count for that command.
- Commands are validated against the same table published by `COMMAND`; command
  implementations can still reject values at execution time (for example
  `INCR` on a non-integer value).
- `QUIT` executes immediately even while queueing and discards the transaction.

## Execution

`EXEC` runs as one synchronous owner turn: it validates all watched keys,
then executes the queue in order without interleaving another data operation. Every queued command's reply
is appended as one RESP array, using the connection's negotiated RESP2/RESP3
framing. A command error is an array element; later commands still run - Redis
does not roll back a partially executed transaction.

Ordinary commands run on the same owner, so another client cannot observe or
mutate the keyspace between two commands of one `EXEC`. The external Store
bridge submits a complete EXEC batch as one request, not independent commands. The queue is
owned by the connection, survives worker handoffs with the rest of `Session`, and
is discarded when the connection closes.

An empty transaction returns `*0` in RESP2 and an empty array in RESP3.

## Optimistic locking

`WATCH key [key ...]` records the current mutation version and logical presence
of each key. WATCH calls accumulate. `UNWATCH` clears them; `DISCARD`, `EXEC`,
`QUIT` and connection close clear them as well.

`EXEC` returns a null array (RESP2 `*-1`, RESP3 `_`) and executes nothing if any
watched key was written, deleted, expired, renamed, had its TTL changed or was
affected by `FLUSHDB`/`FLUSHALL` after it was watched. Expiration is evaluated
against the store clock at `EXEC`; no polling or sleep is involved.

Mutation tracking is a bounded per-key version table (65,536 entries). When the
table must evict, or after a flush, a store epoch is advanced so outstanding
watches fail closed: they may abort unnecessarily, but cannot silently miss a
mutation. Version metadata is not persisted by a future snapshot and is never
part of the payload budget.

## Deliberate boundaries

- `WATCH` inside `MULTI` returns an error, as in Redis, and does not itself mark
  the transaction dirty.
- Connection-local commands and blocking pops are not queued. Redis executes some
  of them at EXEC time; this server rejects them at queue time and documents the
  divergence. `PUBLISH`/`SPUBLISH` are supported because they are ordinary
  store-visible commands with no connection-local wait.
- Subscription commands, `PUBSUB` introspection, scripts, functions and modules
  remain outside queued execution. Messages are at-most-once as elsewhere; no
  durable delivery is implied by publishing from `EXEC`.
- The single owner provides mutual exclusion with other data operations in this
  process. Multi-owner execution is not implemented by this increment (see the
  [ownership design](plans/owner-local-storage.md)). It is not replication,
  durable atomic commit, distributed locking or crash
  recovery. `SET key token NX PX ...` plus conditional release still has the
  failover and fencing limits described in [requirements](requirements.md).
