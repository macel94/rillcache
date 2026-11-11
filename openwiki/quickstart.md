---
type: index
title: Rillcache wiki quickstart
description: A task-oriented entry point to Rillcache's architecture, protocol, storage, operations, data types, testing and production boundaries.
tags: [quickstart, navigation, overview]
sources:
  - id: openwiki-source-bb1ebe868e35e9e500714501
    resource: repo://Dockerfile
  - id: openwiki-source-b043a13fb5b305282e0ccf86
    resource: repo://docs/client-sessions.md
  - id: openwiki-source-491734283c0612f21b1858f0
    resource: repo://docs/compatibility.md
  - id: openwiki-source-37d619b6bb7c649a39dcca8f
    resource: repo://docs/containers.md
  - id: openwiki-source-0117ac68a72ed7ce495ae997
    resource: repo://docs/persistence.md
  - id: openwiki-source-e600869a83d04cf90374b713
    resource: repo://docs/pr-quality-gate.md
  - id: openwiki-source-c23acd0ff2503357d054416d
    resource: repo://docs/production-readiness-gaps.md
  - id: openwiki-source-dfa02c19f5d8e4215d36f9c0
    resource: repo://src/server.rs
generated: { by: "opencode", at: "2026-11-11T08:27:05.780Z" }
verified:
  - by: openwiki/0.9.0
    at: 2026-11-11T08:27:05.780Z
---

# Rillcache wiki quickstart

Rillcache is a Redis-compatible cache server written in safe Rust. This wiki is
for coding agents and maintainers who need to change it without rediscovering the
same control flow. Read the narrow page for your task, then verify consequential
details against source and tests.

## Choose a task

| I need to… | Read |
| --- | --- |
| Understand the layers, threads and state ownership | [Runtime architecture overview](architecture/runtime-overview.md) |
| Trace one command from bytes to reply | [Request lifecycle](architecture/request-lifecycle.md) |
| Change socket handling, workers, Tokio or TLS | [Network frontends and client handoff](architecture/network-frontends.md) |
| Change owner-local execution, mailboxes or sharding | [Ownership, mailboxes and sharding](architecture/ownership-and-sharding.md) |
| Change expiry, payload accounting or eviction | [Data model, expiration and memory accounting](storage/data-model-and-expiry.md) |
| Change the append-only journal or replay | [Journal persistence and recovery](storage/persistence-and-recovery.md) |
| Change RESP framing, HELLO, CLIENT or discovery | [Client protocol, sessions and discovery](protocol/client-sessions-and-discovery.md) |
| Change MULTI/EXEC, blocking waits or pub/sub | [Transactions, blocking and pub/sub](features/transactions-blocking-and-pubsub.md) |
| Change a command or data type | [Data types and command families](features/data-types-and-commands.md) |
| Change startup options, TLS, auth or audit | [Configuration and startup](operations/configuration-and-startup.md) and [Security, TLS and audit](operations/security-and-audit.md) |
| Change INFO, metrics, slowlog or health | [Observability and health](operations/observability-and-health.md) |
| Run tests or deliver a PR | [Testing and delivery](development/testing-and-delivery.md) |
| Check whether a Redis feature exists | [Compatibility and production gaps](boundaries/compatibility-and-production-gaps.md) |

## The short version

- **Transport:** default Unix plain TCP uses a Mio readiness reactor; Tokio is
  opt-in with `--io-mode tokio`.
- **Data ownership:** one local `ShardExecutor` owns `Data` by default. Extra
  socket workers accept and migrate connections; they do not partition the
  keyspace. Multi-owner execution is a separate `--storage-owners` opt-in.
- **Execution:** the session layer applies auth, ACL, subscriber, transaction and
  blocking rules, then the engine executes one command turn. Journal and
  notification effects commit before the turn returns.
- **Persistence:** an optional private RESP journal with `always`/`everysec`/`no`
  fsync, CRC32 checksums, optional HMAC-SHA256 authentication, a local ownership
  lock, cooperative rewrite and offline backup/restore. Replay verifies complete
  groups, truncates only a torn tail and fails on committed corruption. Stream
  consumers and PEL owners/counts/timestamps survive restart; consumer activity
  time does not.
- **Protocol:** RESP2 and a bounded RESP3 subset, plus Redis inline commands.
  Only database 0 exists.
- **Security:** password or ACL-file auth, per-connection and optional per-source
  failure budgets, native TLS 1.3, and optional denial audit with per-line HMAC.
- **Delivery:** protected PR-only main, 14 required contexts, all 20 original and
  all 10 official four-CPU Valkey medians strictly winning, plus 10 Dragonfly
  medians in the same four-CPU job.

## Local start

```sh
cargo run --locked --offline --release --bin rillcache
redis-cli -2 PING
```

The standalone default binds `127.0.0.1:6379`. The container entrypoint binds the
container interface, but published host ports in the examples stay loopback-only.
For persistence:

```sh
rillcache --appendonly yes --dir /data --appendfsync always

# Optional journal authentication and an offline backup of a stopped server
rillcache --appendonly yes --dir /data \
  --aof-hmac-key-file /run/secrets/rillcache-aof.key
rillcache-backup create --dir /data --output /backups/rillcache.rcb
```

For TLS, pass a distinct listener and PEM files:

```sh
rillcache \
  --tls-bind 127.0.0.1:6380 \
  --tls-cert-file server.pem \
  --tls-key-file server.key
```

## Before changing code

1. Identify the owning module and family registry entry.
2. Read the relevant architecture or feature page and the nearest tests.
3. Preserve binary safety, atomic failure, expiry/accounting invariants, bounded
   I/O and joined shutdown.
4. Run focused tests during editing, `make test` after a logical change and
   `make ci` before review.
5. Do not weaken a benchmark gate, omit a losing row, reuse an old pass or claim
   production readiness from a subset.

## Important boundary

Rillcache is **not production ready** and is **not a drop-in Redis replacement**.
It has no RDB, replication, failover, cluster, modules, Lua/Functions, full RESP3,
multiple logical databases, real client-library CI matrix, independent security
review or complete durability validation. The command surface is useful and
tested, but the missing production evidence is explicit.

Start with [compatibility and production gaps](boundaries/compatibility-and-production-gaps.md)
when a task depends on Redis behavior outside the documented subset.
