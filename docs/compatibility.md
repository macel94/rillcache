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

- **One logical database and standalone operation.** Connections start in DB 0;
  `SELECT 0` and `COPY ... DB 0` use that same shared keyspace. Nonzero `SELECT`
  and `COPY ... DB n` are rejected without changing data; `MOVE` and `SWAPDB`
  are unsupported. `FLUSHDB` clears the only keyspace. Clients that require a
  nonzero logical database or DB-based isolation are incompatible: a client
  configured with a nonzero database fails during connection startup, and
  `DBSIZE`/`INFO keyspace` report only `db0`. Choose databases at the
  application layer instead of with `SELECT`; there is no per-database
  observability, `FLUSHDB`/`FLUSHALL` database argument or isolation boundary.
  Replication,
  failover and Redis Cluster routing are not supported. `ROLE` reports a
  standalone master with no replicas or replication stream; it does not imply
  replica or failover support. Cluster clients that need `CLUSTER SHARDS`,
  `SLOTS`, `NODES`, `KEYSLOT`, `MOVED`/`ASK` redirects, `CROSSSLOT` semantics,
  hash tags or topology refresh cannot connect as a cluster: every `CLUSTER`
  subcommand except `HELP` returns `ERR This instance has cluster support
  disabled`, and multi-key commands over different hash tags run locally. Sharded-channel pub/sub is a
  messaging namespace, not a distributed cluster.
  See [replication status](replication.md) and [unsupported capabilities](unsupported.md).
- **Command discovery is not a compatibility matrix.** Hosted RESP2 tests
  check that `COMMAND COUNT`, `COMMAND LIST` and `COMMAND INFO` agree for the
  complete advertised registry, including targeted case-insensitive lookups.
  A metadata entry does not mean every option or subcommand is implemented.
  The exact name-level inventory is the generated
  [command matrix](production-readiness/command-matrix.md): it records every
  pinned-upstream name this server advertises, and the exact wire reply for every
  name it does not. Option-level limits still live in the area documents.
- **No scripts or functions.** Clients that require Lua-backed operations
  (including script registration and `EVALSHA` fallback), server-side functions,
  or loadable modules are incompatible. `SCRIPT FLUSH` and `FUNCTION FLUSH`
  acknowledge an empty registry; neither indicates execution support.
  `COMMAND INFO` returns no metadata for the execution commands. The exclusion is
  explicit for every named entry point: `EVAL_RO`/`EVALSHA_RO` are absent,
  `SCRIPT DEBUG|KILL|LOAD|EXISTS` and `FUNCTION
  DELETE|STATS|DUMP|RESTORE|KEYS|LOAD|LIST` return a deliberate
  unsupported-subcommand error, and an invalid `FLUSH` mode is rejected rather
  than acknowledged. See
  [scripting exclusion](production-profile.md#explicit-exclusions).
- **Pub/Sub is at-most-once; stream groups are the redelivery alternative.**
  Pub/Sub delivery is process-local, bounded per client and lost on disconnect,
  queue pressure, restart or crash, with no replay, acknowledgment or
  replication. For acknowledged redelivery use a stream consumer group
  (`XREADGROUP`/`XPENDING`/`XCLAIM`/`XAUTOCLAIM`/`XACK`); pending entries survive
  a client disconnect and are recorded in the journal, but consumer pending
  state does not survive a server restart. Recovery beyond the process or host
  needs an external durable transport. Neither path is replicated.
  See [Pub/Sub](pubsub.md) and [streams](streams.md#reliable-delivery-across-client-disconnects).
- **No RDB/DUMP interchange; migrate offline.** `SAVE`/`BGSAVE` are explicit RDB
  errors, `DUMP`/`RESTORE`/`MIGRATE` return one capability error, and `LASTSAVE`
  is `0`. Use the bounded offline DB 0 string import, backup create/verify and a
  controlled first start described in
  [migration and interchange](production-profile.md#migration-and-interchange)
  before allowing traffic. Live cutover and native format conversion are not
  supported.
- **Rebuildable data, not a system of record.** The journal supports replay and
  synchronous compaction, but there is no Redis RDB interoperability,
  background/incremental rewrite or validated crash/backup/restore guarantee.
  Stream consumer pending state does not survive restart.
- **Explicit resource bounds.** Payload budgets are not RSS limits; large atomic
  replies and process overhead have separate costs. See [startup defaults](getting-started.md#configure-only-what-you-need)
  and each command's limits. `MGET` replies are capped to prevent duplicate-key amplification.
- **Rate limits are per process, not deployment-wide.** `--auth-failure-limit`
  bounds consecutive failures per connection; `--auth-source-limit` and
  `--auth-global-limit` add opt-in per-IP and process-wide lockouts over a fixed
  window. No counter is shared between server processes, so replicas and
  shared-NAT clients are throttled independently. A deployment with more than
  one process must front it with an authoritative, fail-closed rate limiter
  that runs before `AUTH`; the server budgets stay as defence in depth.
  See [rate limiting](production-profile.md#deployment-wide-rate-limiting).
- **Slow clients are bounded, not fair-shared.** Pub/Sub subscribers have a
  1,024-frame queue; a full queue drops the subscriber and increments
  `pubsub_queue_full_drops`, and other clients keep working. `MGET` replies are
  capped at 2 MiB with an explicit error. There are no configurable slow-client
  thresholds, kill policy, fair-share guarantees or per-tenant quotas. See
  [queue policy](production-profile.md#slow-client-and-queue-policy).
- **Eviction is LRU-approximate; LFU and idle stats are excluded.** `maxmemory`
  applies to logical key/value bytes with `noeviction`, `allkeys/volatile-lru`,
  `allkeys/volatile-random` or `volatile-ttl`. "LRU" tracks mutation order, not
  reads; `OBJECT FREQ`/`IDLETIME` and `allkeys-lfu`/`volatile-lfu` are explicit
  errors, and the policy is visible in `CONFIG GET` and `INFO memory`. See
  [eviction policy](production-profile.md#eviction-and-access-statistics-policy).
- **One process is one tenant.** Multi-tenant deployment is excluded: ACL users
  share the keyspace, payload budget, connection/CPU budget, Pub/Sub namespace,
  auth budgets and process lifetime. Scoped users cannot read other tenants'
  keys or run keyspace-wide commands without an all-key grant, but a user with
  `@pubsub` can enumerate channel names and any tenant can consume shared
  capacity. Run one process per tenant with separate network and resource
  limits. See
  [tenant isolation](production-profile.md#tenant-isolation-boundary).
- **HyperLogLog uses a private payload.** `PFADD`/`PFCOUNT`/`PFMERGE` work with an
  exact set up to 4,096 members and then a 2^14-register sketch (~0.81 % standard
  error). Redis/Valkey payloads are not interchangeable: `DUMP`/`RESTORE`/
  `MIGRATE` and RDB are unsupported, and the offline import accepts only `SET`
  records. `PFDEBUG`/`PFSELFTEST` are private diagnostics. See
  [HyperLogLog](production-profile.md#hyperloglog-representation-and-interchange).
- **RESP3 is a bounded, tested subset.** `HELLO 3` returns a map and Pub/Sub
  uses push frames; supported data commands keep RESP2-compatible types (for
  example `HGETALL` stays a flat array). Double/boolean/big-number/verbatim/set
  types, attributes, streamed strings, client tracking and multiple databases
  are not emitted or not supported, and `HELLO` rejects other protocol versions
  with `NOPROTO`. See
  [RESP3 subset](production-profile.md#resp3-type-and-session-subset-used-by-the-supported-clients).
- **One data owner and a declared admission envelope.** The default data path is
  a single owner thread, so command execution is one CPU; `--storage-owners` is
  opt-in experimental and not a scaling claim. Declared capacity is the enforced
  envelope (`max_clients` 128, 64 MiB logical payload, 1 MiB/2 MiB request
  bounds), not a throughput SLO. See
  [single-owner capacity](production-profile.md#single-owner-capacity-envelope).
- **Runtime changes are atomic and limited to a declared subset.** `CONFIG SET`
  validates every pair before publishing, rejects startup-only names such as
  `appendonly`/`appendfilename` and unknown names with an explicit error, and
  leaves the previous values in effect when any pair fails. `CONFIG REWRITE` is
  not supported. See
  [runtime configuration](production-profile.md#runtime-configuration-changes).
- **Audit records are best-effort and need compensating controls.** Denials go to
  an optional file and UDP syslog sink without sender authentication, encryption,
  delivery guarantee, retry or aggregation; the audit path never gates commands
  and a failed collector exits nonzero with a redacted count. Use an
  independently administered collector with durable retention, HMAC key custody
  and rotation, and alerting, and treat these records as secondary evidence. See
  [audit custody](production-profile.md#audit-custody-boundary-and-compensating-controls).
- **Security features are not a production certification.** Authentication and
  TLS must be configured; independent hardening and operational validation are
  still needed. See [security](security.md) and the
  [canonical gap checklist](production-readiness-gaps.md).

These boundaries are documented rather than emulated with invented topology,
storage or ownership information. The [requirements](requirements.md) and the
[canonical production-readiness gap checklist](production-readiness-gaps.md)
track the remaining work.

[Documentation home](README.md) · [Get started](getting-started.md)
