---
type: operations
title: Security, TLS and audit
description: Rillcache's password and ACL authorization, per-source auth throttling, TLS 1.3 listener, denial logging, HMAC-signed audit transport and the security boundaries still missing.
tags: [security, acl, tls, audit, authentication, authorization]
sources:
  - id: openwiki-source-7501e61db97f4591cd581692
    resource: repo://docs/security.md
  - id: openwiki-source-ac7cbaacb6b4f9b0c71806fd
    resource: repo://docs/tls.md
  - id: openwiki-source-72728abb32f3fcdc6deb8320
    resource: repo://src/bin/rillcache-audit-collector.rs
  - id: openwiki-source-422879bcd26a517422fa714b
    resource: repo://src/engine/audit_syslog.rs
  - id: openwiki-source-eb3011597a053fe1616e8f54
    resource: repo://src/engine/audit.rs
  - id: openwiki-source-8979011e932b93615564a18e
    resource: repo://src/engine/auth_guard.rs
  - id: openwiki-source-68d897f0c4cc8d3306a79b58
    resource: repo://src/engine/auth.rs
  - id: openwiki-source-0f296229415aa5168299027b
    resource: repo://src/server/tls.rs
  - id: openwiki-source-4b50bebca60a7d5b99fbfd7d
    resource: repo://src/session/auth.rs
  - id: openwiki-source-7961d265231bc3dc15731437
    resource: repo://src/session/transaction.rs
generated: { by: "opencode", at: "2026-11-02T15:32:39.597Z" }
verified:
  - by: openwiki/0.7.3
    at: 2026-11-02T16:04:12.575Z
---

# Security, TLS and audit

Rillcache has a real authentication and authorization boundary for private
deployments. It is not a complete Redis security design, and it does not create
tenant isolation. All users share one process, one keyspace and one payload
budget.

## Authentication modes

A process uses one of two exclusive modes:

- **Default password.** `--requirepass` or `--requirepass-file` sets a password
  for the implicit `default` user. The server stores only its SHA-256 digest.
- **ACL file.** `--aclfile` loads Redis-style `user ...` lines with multiple
  users, password hashes, command/category rules, key patterns and channel
  patterns.

Without either option the server is unauthenticated. The standalone binary binds
loopback by default, but the container entrypoint binds the container interface;
operators must add authentication before exposing it beyond a trusted network.

When authentication is required, a new connection is unauthenticated. Only
`AUTH`, `HELLO` (including `HELLO ... AUTH ...`) and `QUIT` execute. Everything
else, including `MULTI`, `INFO`, `CONFIG`, `COMMAND`, reads and writes, returns
`NOAUTH Authentication required.` before touching the store.

`AUTH [username] password` accepts `default` case-insensitively or a configured
ACL user. Stored digests are compared with a constant-time equality function.
Wrong username, wrong password and disabled users all return the same
`WRONGPASS` reply. Calling `AUTH` when no password exists returns Redis's explicit
no-password configuration error.

`CONFIG SET requirepass <value>` rotates the default password; an empty value
disables it. `CONFIG GET requirepass` reports an empty value or `<redacted>`, never
the secret. The password is redacted from `Config` debug output, startup lines,
errors, slowlog and MONITOR.

## ACL model

An ACL file must define an enabled `default` user. Each `user <name> <rules>` line
starts from a disabled, no-command, no-key user, so the line is the complete user
definition. The file is bounded to 4 MiB, 10,000 users, 1,024 rules per user and
1,024 key/channel patterns per user.

Supported rule families include:

- `on`/`off`, `nopass`, `resetpass`, `>password`, `<password`,
  `#<hex sha256>` and `!<hex sha256>`;
- `allcommands`/`+@all`, `nocommands`/`-@all`, `+@category`, `-@category`;
- `+command`, `-command`, `+command|subcommand`, `-command|subcommand`;
- `allkeys`, `resetkeys`, `~pattern`, `%R~pattern`, `%W~pattern`,
  `%RW~pattern`;
- `allchannels`, `resetchannels`, `&pattern`;
- `reset` and parenthesised selector groups such as `(+set %W~app:*)`.

Rules are ordered; the last matching command/category rule decides. A
`command|subcommand` rule matches the literal first argument
case-insensitively. A subcommand rule does not grant the bare command. Selectors
are alternatives to root rules: a command is allowed when the root rules or a
selector permit it, and keys/channels must be permitted by that same set.

Key access is split into read and write modes. A read-write operation needs
grants for both, potentially from different patterns. Mixed commands such as
`BITOP`, `COPY`, `RENAME`, `SORT` and geo store variants are checked per source
and destination. Dynamic `SORT BY`/`GET` patterns require all-key access because
the looked-up key is not statically declared.

Supported categories are an approximation over the published command table:
`@all`, `@read`, `@write`, `@keyspace`, `@string`, `@list`, `@set`, `@hash`,
`@zset`, `@stream`, `@bitmap`, `@connection`, `@transaction`, `@admin`, `@fast`,
`@slow` and `@dangerous`. Bounded subcommand overrides refine `CLIENT`, `MEMORY`,
`OBJECT`, `ACL`, `COMMAND`, `XGROUP`, `XINFO`, `CONFIG`, `LATENCY`, `SLOWLOG` and
`PUBSUB`. This is not a byte-for-byte copy of one Redis release.

`ACL SETUSER` is atomic: a rejected rule leaves the previous user untouched.
`ACL SAVE` writes a mode-0600 temporary file, fsyncs it, renames it over the target
and fsyncs the directory where supported. Hashes are saved as `#...`, never
plaintext. `ACL LOAD` parses a replacement map before publishing it.

`ACL DELUSER`, `WHOAMI`, `USERS`, `LIST`, `GETUSER`, `CAT`, `SAVE`, `LOAD`,
`GENPASS`, `DRYRUN` and `LOG` are implemented. `GENPASS` reads `/dev/urandom` and
fails visibly if the source is unavailable. `DRYRUN` evaluates the named user
without executing the command or recording a denial.

## Live authorization

Authorization runs before store dispatch for known commands with declared keys.
Denied commands return `NOPERM`; denied keys return the key-specific `NOPERM`.
`EXEC` re-checks every queued command against the current ACL, so revoking or
disabling a user between `MULTI` and `EXEC` cannot commit queued writes. Disabled
or deleted users are re-evaluated on every command, so ACL changes apply to live
connections immediately.

`ACL LOG` keeps a bounded, process-local aggregate of recent denials with reason,
context, object, username and age. It is administrative data, not an exhaustive
audit trail. `ACL LOG RESET` clears only the in-memory buffer.

Key patterns are not a whole-keyspace sandbox. `KEYS`, `RANDOMKEY`, `DBSIZE`,
`FLUSHDB`/`FLUSHALL` and similar commands do not have per-key positions to check.
Do not grant them to tenants who must not see or clear the whole keyspace.
`PUBSUB` introspection is category-governed and can reveal channel names.
`CLIENT LIST`/`KILL` reveal addresses and terminate sessions; grant them only to
trusted administrators.

## Failed-auth budgets

Two independent budgets reduce online guessing:

- `--auth-failure-limit N` (default 10, zero disables) closes a connection after
  N consecutive failed `AUTH`/`HELLO AUTH` attempts. A success or `RESET` clears
  the counter.
- `--auth-source-limit N` (default 0, disabled) counts failures per peer IP
  within `--auth-source-window-ms` (default 60,000 ms). Once the limit is
  reached, further credential attempts from that source receive
  `ERR too many authentication attempts, try later` until the window expires.

The source table is process-local and bounded at 4,096 entries. A successful
authentication resets that source. Shared NAT addresses can throttle each other,
and an attacker can fill the table. There is no global, cross-process, per-user
or delayed-response rate limiter. Use network policy and strong random passwords.

There is no live deauthentication: rotating the password affects new
authentication decisions, while an already authenticated connection remains
authenticated. Close/reconnect administrative clients after rotation.

## Denial audit

`--audit-log PATH` appends one bounded line per denial:

```text
unix-ms reason context username-hex object-hex [hmac-sha256-hex]
```

The reason/context are short tokens, username and object are hex-encoded and
truncated, and the file never contains plaintext passwords or key values. The
file is created mode-0600, flushed per denial, and rotates once to `PATH.1` at
`--audit-log-max-bytes` (1 KiB–1 GiB, default 8 MiB). A runtime write failure does
not change the command reply but increments `audit_log_write_failures`.

`--audit-hmac-key-file` appends an HMAC-SHA256 over the line body. The key is
16–1,024 bytes and is stripped of one trailing newline. `--audit-hmac-keyring-file`
accepts up to eight unique hex keys in a 64 KiB text file; the first key signs and
a collector may verify against all. This is per-line authentication, not a hash
chain: it does not detect deleted or reordered lines, provide replay protection,
key IDs or hot rotation.

`--audit-syslog IP:PORT` sends the same line as one best-effort UDP datagram. A
100 ms write timeout prevents an indefinitely blocked sender, but UDP can lose,
duplicate, reorder or expose datagrams. There is no collector authentication,
encryption, retry or delivery guarantee. The local file remains the durable
evidence path.

`rillcache-audit-collector` is a separate UDP sink:

```sh
rillcache-audit-collector \
  --bind 127.0.0.1:5514 \
  --output /var/log/rillcache/audit.log \
  --hmac-keyring-file /etc/rillcache/audit-keyring
```

It bounds datagram size, output bytes and optional accepted-datagram count,
counts empty/oversized and HMAC-rejected datagrams, writes mode-0600 output and
rotates one generation. It verifies a shared key or any keyring key. It does not
authenticate sender identity, encrypt traffic or federate collectors.

## Native TLS 1.3

`--tls-bind` starts a second listener; plaintext remains on `--bind` and the
addresses must differ. The acceptor uses rustls with its `ring` provider and TLS
1.3 only. TLS 1.0, 1.1 and 1.2 are rejected. HTTP, HTTP/2 and HTTP/3 ALPN are not
offered; RESP is carried directly.

```sh
rillcache \
  --bind 127.0.0.1:6379 \
  --tls-bind 0.0.0.0:6380 \
  --tls-cert-file /etc/rillcache/server-chain.pem \
  --tls-key-file /etc/rillcache/server-key.pem
```

`--tls-client-ca-file` enables required client certificates. A successful mTLS
handshake is not Rillcache authentication: continue to use `AUTH`/ACL rules.
Certificate identity is not mapped to an ACL user.

There is no SNI certificate selection, certificate hot reload, OCSP/CRL
integration, PROXY protocol or ALPN. Rotation requires a controlled restart. TLS
connections use per-client threads and are still bounded by `max_clients`. TLS
adds handshake and encryption cost; it is transport security, not durability or
availability.

## Security boundaries

- Hashes are unsalted SHA-256 for Redis ACL compatibility; weak passwords are
  brute-forceable offline. Use long random secrets and protect ACL files.
- No process/network tenant isolation, per-user memory/CPU quotas or separate
  keyspaces.
- No independent security review, full TLS hardening or global rate limiting.
- Audit lines are per-line authenticated but not chained or replay-protected.
- No credential encryption at rest; password files and ACL files are operator
  custody.
- No modules, scripts or functions, so there is no plugin sandbox to bypass.
- The plaintext listener still exists when TLS is enabled; bind it privately.

The project is not production ready. The remaining security and operational
blockers are recorded in
[compatibility and production gaps](../boundaries/compatibility-and-production-gaps.md).
