---
type: architecture
title: Runtime architecture overview
description: A map of Rillcache's binary, transport, session, engine, storage and persistence layers, including which thread owns each kind of state.
tags: [architecture, runtime, threading, storage, persistence, overview]
verified:
  - by: openwiki/0.7.3
    at: 2026-11-02T16:04:12.575Z
sources:
  - id: openwiki-source-651d1fb6c9e49916a916ab51
    resource: repo://Cargo.toml
  - id: openwiki-source-0117ac68a72ed7ce495ae997
    resource: repo://docs/persistence.md
  - id: openwiki-source-ccfe8cc4504f25c304787a46
    resource: repo://src/engine.rs
  - id: openwiki-source-39b0609b5d3153459fb6d632
    resource: repo://src/engine/bridge.rs
  - id: openwiki-source-b156f0c5004fc1ca4c6d7464
    resource: repo://src/engine/data.rs
  - id: openwiki-source-ad8899f1c7d3f26ff3767f9f
    resource: repo://src/engine/dispatch/mod.rs
  - id: openwiki-source-350cb0ce838ab0c30fe785a9
    resource: repo://src/engine/journal.rs
  - id: openwiki-source-ed8bf05e307c6278442542c2
    resource: repo://src/lib.rs
  - id: openwiki-source-b55a21a31ede1b56cd31a6a6
    resource: repo://src/main.rs
  - id: openwiki-source-dfa02c19f5d8e4215d36f9c0
    resource: repo://src/server.rs
generated: { by: "opencode", at: "2026-11-02T15:32:39.597Z" }
---

# Runtime architecture overview

Rillcache is a single-process Redis-compatible cache server. The crate is split so
that protocol framing, connection policy, data execution and persistence can be
tested without starting a process, while the binary owns only argument parsing,
startup and shutdown.

## Layers

| Layer | Main files | Responsibility |
| --- | --- | --- |
| Binary | `src/main.rs`, `src/healthcheck.rs` | Parse CLI options, validate them, bind, report the listener, run or probe |
| Transport | `src/server.rs`, `src/server/*` | Accept sockets, choose a scheduling policy, move connections, flush replies, join shutdown |
| Protocol | `src/protocol.rs` | Bounded RESP2 and inline framing; binary-safe argument borrowing |
| Session | `src/session/*` | Per-connection protocol, auth, ACL, transactions, blocking, pub/sub, client metadata |
| Engine | `src/engine.rs`, `src/engine/dispatch/*` | Command-family routing, owner turns, observation, effects |
| Data | `src/engine/data.rs`, `keyspace.rs`, `collections.rs`, `streams.rs`, `geo.rs`, `hll.rs` | Entries, collections, expiry, versions, accounting, eviction |
| Effects | `src/engine/journal.rs`, `notify.rs`, `slowlog.rs`, `latency.rs` | Journal capture, keyspace events, observation collectors |
| Concurrency | `src/engine/shard.rs`, `partition.rs`, `mailbox.rs`, `owner_actor.rs`, `router.rs` | Routing, bounded mailboxes, reservations, multi-owner coordination |
| Security | `src/engine/auth.rs`, `audit*.rs`, `src/server/tls.rs` | Passwords, ACLs, denial audit, TLS 1.3 |

The library exposes `engine`, `protocol`, `server` and `session` modules plus
`Config`, `Server` and `ServerHandle`. The package forbids unsafe code at every
crate root and in `Cargo.toml`; only rustls, Mio, Tokio and `core_affinity` are
approved dependency exceptions, with their required transitives vendored.

## Startup

`main` parses a deliberately small option set. Values are validated before bind:
ports must be usable, limits must be positive and bounded, `--threads` is 1–64,
`--storage-owners` is 1–64, `--socket-workers` is 1–16, TLS requires a distinct
listener with a certificate and key, and `requirepass` and `aclfile` are mutually
exclusive. A password file is read, stripped of at most one line ending and bounded
to 1024 bytes.

`Server::bind` then:

1. validates the complete `Config`;
2. loads authentication and audit HMAC material;
3. opens the journal or creates the in-memory store;
4. attaches audit sinks and publishes runtime settings into `ServerServices`;
5. binds the plaintext listener and, optionally, the TLS listener;
6. chooses an immutable `SchedulingPolicy` from CPU capacity and the configured
   concurrency options.

`Server::run` blocks until shutdown. `Server::spawn` performs the same work on a
managed thread and returns a `ServerHandle` whose drop shuts down and joins the
accept thread. Tests use ephemeral ports and the managed handle rather than fixed
pauses.

The binary prints the version and addresses to stderr. It does not daemonise, write
a PID file, parse `redis.conf` or install an OS signal handler. A protocol
`SHUTDOWN` or a dropped `ServerHandle` is the managed shutdown path; a container
supervisor that sends `SIGTERM` is not the same thing as a graceful drain, which is
one of the recorded production gaps.

## State ownership

There are two broad kinds of state:

- **Thread-confined data**: entries, expiry indexes, mutation versions, eviction
  state and payload accounting live in `Data`, inside a `ShardExecutor`.
- **Shared control plane**: authentication, ACL state, client registry, pub/sub,
  monitors, counters, observation collectors and configuration live in
  `ServerServices` behind atomics or `RwLock`/`Mutex`.

The default one-owner path installs `ShardExecutor` directly on the socket reactor
or single fixed worker. Ordinary commands do not cross a channel. Journal, ACL-file,
audit and mixed-TLS configurations instead use the bounded `Store` compatibility
bridge: one owner thread constructs `Data`, executes complete owned requests and
returns complete owned replies. `Data` never leaves that thread.

`Data` stores a `Entry` per key. An entry may hold a string value, a collection, a
stream, field TTLs, an absolute expiry deadline, a payload length and an access
counter. The keyspace carries a sorted expiry index, bounded `WATCH` versions and a
bounded mutation-event log. Expiry is absolute wall-clock milliseconds; an entry is
alive while `now <= deadline`.

`ServerServices` is deliberately not one lock. It uses an atomic `security_word`
and `observer_word` for hot checks, sharded command counters to avoid a single
cache line, an epoch for caches that must invalidate together, and separate locks
for colder maps. The design removes the shared data lock from the default path; it
does not claim the whole control plane is lock-free.

## Thread roles

| Configuration | Threads involved |
| --- | --- |
| Default Unix plaintext | One or more Mio reactors; the first owns `Data`, workers migrate data connections to it |
| `--threads N` | Accept thread plus N fixed I/O lanes; one lane owns `Data`, others use the compatibility path |
| `--io-mode tokio`, eligible plaintext | Runtime zero accepts and serves with local `Data`, plus one joined cold-service thread |
| Tokio with TLS | Several current-thread network runtimes and one external compatibility owner; TLS has its own accept/client threads |
| `--storage-owners N` | Router reactor plus N owner threads, each with a current-thread runtime and `LocalSet` |
| Persistence, ACL file, audit or TLS | Shared external owner outlives all frontends and is joined after their workers |

Every path has bounded admission and joined shutdown. The transport page explains
the handoff details; the ownership page explains data routing and reservations.

## Command execution

The engine keeps only `PING` as a top-level fast path. `GET`, `SET` and `ECHO` call
a small inlined strings entry, while every other command routes through a family
module: strings, keyspace, collections, streams, pub/sub and administration. Each
family returns an `Action`; a new command belongs in its family, not in a large
central match.

A command turn can produce more than a reply. It may queue journal events, keyspace
notifications, expiry or eviction lifecycle events, slowlog entries, latency
samples and command/error statistics. Journal and notification capture share the
same bounded mutation-event log. If journaling is enabled, the write record is
appended before the turn finishes; append failure replaces the success reply with
an error.

`MULTI`/`EXEC`, blocking commands and pub/sub are session concerns, but `EXEC`
replays queued store commands through the same family dispatcher. That keeps
transaction semantics aligned with ordinary commands without putting transaction
state into the engine.

## Persistence

The optional journal is a private sequence of RESP command arrays. There is no RDB
writer, no multipart AOF compatibility, no checksum and no background or forked
rewrite. Replay uses the ordinary command implementation and fails visibly on a
damaged mid-file record; a partial tail is truncated under the documented recovery
rules. `BGREWRITEAOF` is synchronous and pauses command service while it writes and
renames a compact snapshot.

The fsync policy controls the acknowledgement window: `always` syncs before the
reply, `everysec` writes immediately and syncs on maintenance, and `no` leaves
durability to the OS and orderly shutdown. This is a real recovery path, not a
production durability contract; crash, ENOSPC, backup and point-in-time recovery
evidence is still missing.

## Design boundaries

- No replication, failover, Sentinel, cluster bus, modules, Lua or multiple logical
  databases.
- No RDB import/export, no interoperable AOF and no cross-process journal lock.
- RESP3 is a bounded subset of the types this command set emits.
- The default is one data owner. Multi-owner execution is opt-in and does not
  provide cluster semantics.
- Performance claims are scoped to the recorded benchmarks and gates; the
  repository is explicitly not production ready.

These boundaries are not omissions in this overview; they are part of the
architecture. The checked list lives in
[compatibility and production gaps](../boundaries/compatibility-and-production-gaps.md).
