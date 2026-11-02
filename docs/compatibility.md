# Compatibility at a glance

Rillcache speaks the Redis wire protocol and implements a tested, growing command
surface. It is early-stage software for development and evaluation, not yet a
production-ready drop-in replacement. Start with the commands your application
actually uses and check their reference pages below.

## Supported command families

| Area | Available behavior and reference |
| --- | --- |
| Strings and keys | Binary-safe `SET`/`GET`, conditional writes, expiration, counters and key management: [strings](strings.md), [multi-key operations](multikey-strings.md), [ranges](string-ranges.md), [keyspace](keyspace.md) |
| Bitmaps and string algorithms | Bit access/operations, bitfields, floating-point increments and LCS: [reference](string-algorithms.md) |
| Collections | Lists, sets, hashes, sorted sets, blocking operations, field expiration and algebra: [collections](collections.md); [SORT](sort.md) |
| Geographic and cardinality data | [Geospatial commands](geospatial.md) and [HyperLogLog](hyperloglog.md) |
| Streams | Entries, reads, trimming, consumer groups, pending delivery and claims: [streams](streams.md) |
| Transactions | Supported data commands with `MULTI`/`EXEC`, `WATCH` and expiry-aware invalidation: [transactions](transactions.md) |
| Messaging | Channels, patterns and sharded-channel namespaces: [pub/sub](pubsub.md); [keyspace notifications](notifications.md) |
| Memory and inspection | [Eviction policies](eviction.md), [observability](observability.md), [runtime introspection](introspection.md) |
| Clients and discovery | Connection metadata/control, `HELLO`, database zero and discovery: [sessions](client-sessions.md), [discovery](discovery.md) |
| Access control | Passwords, users, ACL rules and audit tools: [security](security.md); native TLS 1.3: [TLS](tls.md) |
| Restart recovery | Optional append-only journal, fsync policies and synchronous compaction: [persistence](persistence.md) |

Each reference describes command options, error behavior, atomicity and bounds.
A listed family does not imply every command or every Redis behavior in that family.

## Clients and protocol

Use `redis-cli -2` for the first connection. RESP2 and inline requests are
supported, along with the documented `HELLO`/RESP3 subset. Pipelined and fragmented
requests, binary data, authentication and common client metadata are tested.

Client libraries can issue discovery commands or scripts before your first
`GET`. Arbitrary client-library interoperability is not yet established. Check
[client sessions](client-sessions.md) and [discovery](discovery.md), then exercise
your application's connection setup and commands. Existing upstream
[conformance evidence](reference-conformance.md) and [original Tcl suites](upstream-ci.md)
show what has actually been checked, including exclusions and failures.

## Important boundaries

- **One logical database and standalone operation.** `SELECT 0` is supported;
  replication, failover and Redis Cluster routing are not. Sharded-channel
  pub/sub is a messaging namespace, not a distributed cluster.
  See [replication status](replication.md) and [unsupported capabilities](unsupported.md).
- **No scripts or functions.** Applications depending on Lua, server-side
  functions or loadable modules need functionality not currently available.
- **Rebuildable data, not a system of record.** The journal supports replay and
  synchronous compaction, but there is no Redis RDB interoperability,
  background/incremental rewrite or validated crash/backup/restore guarantee.
  Stream consumer pending state does not survive restart.
- **Explicit resource bounds.** Payload budgets are not RSS limits; large atomic
  replies and process overhead have separate costs. See [startup defaults](getting-started.md#configure-only-what-you-need)
  and each command's limits. `MGET` replies are capped to prevent duplicate-key amplification.
- **Security features are not a production certification.** Authentication and
  TLS must be configured; independent hardening and operational validation are
  still needed. See [security](security.md) and the [readiness plan](production-readiness.md).

These boundaries are documented rather than emulated with invented topology,
storage or ownership information. The [requirements](requirements.md) and
[production-readiness plan](production-readiness.md) track the remaining work.

[Documentation home](README.md) · [Get started](getting-started.md)
