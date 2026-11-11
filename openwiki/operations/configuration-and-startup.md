---
type: operations
title: Configuration and startup
description: Rillcache's CLI and programmatic configuration, validated defaults, listener selection, persistence settings, password/ACL inputs and the absence of a configuration-file parser.
tags: [configuration, startup, cli, defaults, persistence, tls]
sources:
  - id: openwiki-source-bb1ebe868e35e9e500714501
    resource: repo://Dockerfile
  - id: openwiki-source-37d619b6bb7c649a39dcca8f
    resource: repo://docs/containers.md
  - id: openwiki-source-0117ac68a72ed7ce495ae997
    resource: repo://docs/persistence.md
  - id: openwiki-source-0541899300248196690b9ec6
    resource: repo://src/engine/administration.rs
  - id: openwiki-source-350cb0ce838ab0c30fe785a9
    resource: repo://src/engine/journal.rs
  - id: openwiki-source-a49be758573b96f47b54fa8f
    resource: repo://src/healthcheck.rs
  - id: openwiki-source-b55a21a31ede1b56cd31a6a6
    resource: repo://src/main.rs
  - id: openwiki-source-dfa02c19f5d8e4215d36f9c0
    resource: repo://src/server.rs
generated: { by: "opencode", at: "2026-11-11T08:27:05.780Z" }
verified:
  - by: openwiki/0.9.0
    at: 2026-11-11T08:27:05.780Z
---

# Configuration and startup

Rillcache has no `redis.conf` parser. A binary start is a finite command line, and
an embedded start constructs `Config` directly. Both paths are validated before a
listener is bound or a journal is opened.

## Basic start

```sh
# Native binary
cargo run --locked --offline --release --bin rillcache

# Container
docker run --rm --init -p 127.0.0.1:6379:6379 \
  ghcr.io/macel94/rillcache:latest
```

The standalone binary and library defaults bind `127.0.0.1:6379`. The container
entrypoint overrides this to `0.0.0.0:6379` inside the container, while the
published host port in the examples stays loopback-only. Binding to a wider
interface is a deployment decision, not a default.

`--help` prints the complete option list. `--version` prints the crate version.
Invalid options, missing values, malformed addresses and out-of-range numbers exit
with status 2 before any network activity.

## Common configurations

```sh
# Password from a file (preferred over a visible process argument)
rillcache --requirepass-file /run/secrets/rillcache

# Multi-user ACL file (mutually exclusive with requirepass)
rillcache --aclfile /etc/rillcache/users.acl

# Append-only journal with fsync on every acknowledged write
rillcache --appendonly yes --dir /data --appendfsync always

# Authenticated journal (the key requires --appendonly yes)
rillcache --appendonly yes --dir /data \
  --aof-hmac-key-file /run/secrets/rillcache-aof.key

# Offline backup and restore of a stopped server's journal
rillcache-backup create --dir /data --output /backups/rillcache.rcb
rillcache-backup verify --input /backups/rillcache.rcb
rillcache-backup restore --input /backups/rillcache.rcb --dir /restore

# Opt-in async plaintext transport
rillcache --io-mode tokio

# Opt-in multi-owner data execution
rillcache --storage-owners 4

# Native TLS 1.3 on a distinct listener
rillcache \
  --tls-bind 127.0.0.1:6380 \
  --tls-cert-file /etc/rillcache/server.pem \
  --tls-key-file /etc/rillcache/server.key
```

The plaintext listener remains active when TLS is enabled. Keep it on loopback or
private space. `--tls-client-ca-file` additionally requires client certificates.
`--tls-bind` must differ from `--bind`.

## Defaults

| Setting | Default | Notes |
| --- | --- | --- |
| Bind | `127.0.0.1:6379` | Loopback only |
| Max clients | 128 | Admission bound, not a file-descriptor limit |
| Idle timeout | 30 s | Socket inactivity |
| Max data bytes | 64 MiB | Key/value payload, not RSS |
| Max request frame | 2 MiB | Configurable up to 512 MiB |
| Max bulk argument | 1 MiB | Must fit inside the frame |
| Max arguments | 1024 | Per request |
| Worker mode | Mio readiness | Tokio is explicit |
| Socket workers | 1 reactor | 1–16 when set; forced to 1 for journal/ACL/audit/TLS |
| Storage owners | 1 local owner | 2–64 opt-in |
| Append-only | Off | `everysec` when enabled |
| Journal basename | `appendonly.aof` | Basename only, inside `--dir` |
| Journal authentication | Off | HMAC key requires append-only mode |
| Maxmemory policy | `noeviction` | Eviction is opt-in per policy |
| Notifications | Off | Empty flag string |
| Error dialect | `valkey` | Selects verified MSETEX wording |
| TLS | Off | TLS 1.3 only when configured |
| Auth | Off | Password or ACL file enables it |

Payload limits are deliberately smaller than Redis defaults. They bound key and
value bytes only; they do not include maps, allocator slack, expiry indexes,
threads, sockets or journal buffers. The process can use substantially more memory
than `--max-data-bytes`.

## Validation before bind

`Config::validate` rejects:

- `--threads` outside 1–64 and `--socket-workers` outside 1–16;
- zero clients, zero payload budget or a zero timeout;
- a request frame below 8 bytes or above 512 MiB;
- a bulk limit larger than the frame limit;
- an empty or over-1024-byte password;
- both `requirepass` and `aclfile`;
- an empty audit path or an audit rotation bound outside 1 KiB–1 GiB;
- an audit syslog port of zero;
- both audit HMAC key file and keyring file;
- an empty journal HMAC key path, `--aof-hmac-migrate` without a key, or journal
  authentication without append-only mode;
- `--storage-owners` outside 1–64;
- TLS options without a TLS listener, a TLS listener without certificate and key,
  or identical TLS and plaintext addresses;
- a journal basename containing `/` or `\`, or an empty `--dir`.

The error is returned before binding. The binary prints it to stderr and exits 1.
The library caller receives an `io::Error` from `Server::bind`.

`Config`'s `Debug` implementation redacts the password. It still prints the ACL
file and audit paths, so do not put secrets in paths.

## Authentication inputs

`--requirepass` is accepted for local experiments, but it is visible in process
arguments and shell history. Prefer `--requirepass-file`; the file must be
nonempty, at most 1024 bytes, and have at most one trailing line ending stripped.

`--aclfile` loads Redis-style `user ...` lines. The password and ACL-file options
are mutually exclusive to avoid ambiguous default-user state. ACL `SAVE` requires
a writable directory; a read-only secret mount can load users but cannot persist
runtime edits.

See [security, TLS and audit](security-and-audit.md) for the authorization model.

## Persistence settings

```sh
rillcache --appendonly yes --dir /data --appendfilename appendonly.aof \
          --appendfsync everysec
```

`--dir` must exist and be writable by the process. The journal file is created
with mode `0600` on Unix. `--appendfilename` is a basename, not a path; `.` and
`..` are rejected. Startup claims `<journal>.lock` with the owner PID and kernel
start time, so a second process on the same journal fails rather than interleaving
appends; the lock is advisory and local-filesystem only.

`--aof-hmac-key-file` adds a keyed HMAC-SHA256 chain over every journal
transaction. `--aof-hmac-migrate` is the one-time seal for an already checksummed
journal; omit it on later starts. The offline `rillcache-backup` binary accepts the
same key for create, verify and restore.

At runtime, `CONFIG GET` can inspect `appendonly`, `appendfilename` and
`appendfsync`. Only `appendfsync` is mutable with `CONFIG SET`; `appendonly` and
`appendfilename` are startup-only. There is no configuration reload and no
`CONFIG REWRITE`.

## Runtime configuration

A subset of settings is mutable through `CONFIG SET`, including:

- `requirepass`;
- `maxclients`;
- `maxmemory` and `maxmemory-policy`;
- collection listpack/intset thresholds and legacy aliases;
- `notify-keyspace-events`;
- slowlog threshold/length, latency threshold and commandstats;
- `appendfsync` and automatic rewrite thresholds.

`CONFIG GET` never returns the password in plaintext. Unsupported settings or
options return an explicit error rather than silently succeeding. There is no
general configuration-file parser, no include mechanism, no precedence chain and
no atomic multi-setting transaction beyond the documented paired options.

## Healthcheck and startup failure

```sh
rillcache --healthcheck 127.0.0.1:6379
```

The probe sends RESP2 `PING`, expects `PONG`, and is quiet on success. It uses one
one-second socket deadline and exits 0 or 1. It checks request service, not the
journal, persistence or replication. A server at its client limit can make the
probe fail; leave headroom if a supervisor depends on it.

If the bind address is occupied, the process exits 1 with the OS error. If the
journal cannot be opened or replayed, startup fails before listening. If an ACL
file is malformed, startup fails rather than silently disabling authentication.

## What is missing

Rillcache does not implement:

- `redis.conf` parsing, includes or file-based reload;
- `CONFIG REWRITE`;
- hot certificate reload, SNI or OCSP;
- a portable SIGTERM/SIGINT drain in the standalone binary;
- shared-filesystem journal coordination; ownership is a local advisory lock, not
  a distributed lease;
- a general TLS cipher/version policy beyond TLS 1.3 and the rustls provider.

The container sets `STOPSIGNAL SIGTERM`, but the safe-std server has no portable
signal handler. A supervisor that sends SIGTERM is relying on process termination,
not the in-process joined-shutdown path. Protocol `SHUTDOWN` or dropping a
`ServerHandle` uses the managed path. This is a production gap, not a hidden
feature.

The remaining operational blockers are recorded in
[compatibility and production gaps](../boundaries/compatibility-and-production-gaps.md).
