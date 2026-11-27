# Overload and slow-client shedding contract

This document is the operator contract behind readiness item S9: which limits
are enforced, what a client sees when it crosses one, and which limits are
delegated to the operating system. It describes behavior that exists in the
server; it is not a capacity or SLO claim. Measured capacity evidence remains
tracked separately in [production-readiness-gaps.md](production-readiness-gaps.md)
(P1–P7).

## Enforced limits

| Limit | Default | Scope | Exceeding client sees |
| --- | --- | --- | --- |
| `max-frame-bytes` / `max-bulk-bytes` / argument cap | 2 MiB / 1 MiB / derived | one request frame | `-ERR Protocol error: request frame limit exceeded` (or the frame/bulk wording) and the connection closes |
| `max-clients` | 128 | accepted sockets | `-ERR max number of clients reached`, the socket closes, `rejected_connections` increments |
| `idle-timeout-ms` | 30000 | one connection | silent close after no read/write progress for the timeout |
| `frame-assembly-timeout-ms` | 60000 | one incomplete plaintext frame | silent close; complete buffered requests still run (0 disables) |
| `write-stall-timeout-ms` | 30000 | one connection's pending reply bytes | silent close after no **write progress** for the timeout while a reply is pending (0 disables) |
| `global-command-budget` | 0 (off) | whole process | `-ERR overload: global command budget exhausted`; the command is not executed |
| `user-command-budget` | 0 (off) | one authenticated user | `-ERR overload: user command budget exhausted`; the command is not executed |
| `auth-failure-limit` / `auth-source-limit` / `auth-global-limit` | 10 / off / off | connection / peer IP / process | `-ERR` authentication failures and a bounded lockout window |

`CONFIG SET maxclients` changes only the connection admission limit. Request
limits, the frame and write-stall deadlines and the command budgets are
startup-only and validated by `--check-config` before the listener binds.

## Slow readers

The write-stall deadline is independent of the read-idle timeout:

- A connection that stops draining a pending reply is closed after
  `write-stall-timeout-ms` without needing a socket event. This prevents a
  non-reading client from holding a `max-clients` slot and buffered reply bytes
  indefinitely.
- A slow reader that keeps making progress is **not** shed, regardless of how
  long the complete reply takes. Progress, not total transfer time, refreshes
  the deadline.
- The effective socket write deadline is the shorter of `idle-timeout-ms` and
  `write-stall-timeout-ms`, so an existing long idle timeout keeps its current
  behavior and a shorter explicit stall deadline sheds faster.
- The policy applies to the default readiness reactor, the shared reactors,
  fixed I/O lanes, the managed blocking/TLS fallback and the opt-in Tokio
  transport. TLS sockets use the same write deadline: replies are written in
  bounded chunks and the rustls send buffer is drained after each chunk, so a
  stalled TLS reader is closed like a plaintext one instead of letting rustls
  buffer an unbounded reply in memory. Tokio applies the deadline per write-
  progress step, not to the total reply duration.
- Shedding is silent at the protocol level: the server does not inject an error
  frame it cannot deliver. Dropping the connection discards the unsent backlog,
  and the freed admission slot serves new clients. `INFO clients
  connected_clients` reflects the release.

## Command budgets

Budgets are fixed one-second windows computed from the server wall clock. They
bound dispatch, not queueing or payload size:

- Limits are `0..=1000000` commands per second; `0` disables that dimension and
  removes the accounting entirely from the hot path.
- `--global-command-budget N` bounds all admitted commands in the process;
  `--user-command-budget N` bounds each authenticated user name separately.
  Unauthenticated traffic is accounted to the `default` identity. At most 1024
  distinct identities are tracked per window; beyond that cap the per-user
  dimension stops admitting new names and only the global budget applies.
- `AUTH`, `HELLO` and `QUIT` are exempt so an exhausted budget can never lock a
  client out of its credential handshake or disconnect path.
- A rejected command produces the error reply above and is not executed, not
  recorded in `total_commands_processed`, not observed by `SLOWLOG`, `LATENCY`,
  `MONITOR` or `COMMANDSTATS`, and not journaled. Rejections increment
  `command_budget_rejections` in `INFO stats`; `CONFIG RESETSTAT` clears it.
- The connection stays open after a rejection so a well-behaved client can retry
  in the next window. `CLIENT REPLY OFF/SKIP` suppression is honored, and a
  rejection never closes a connection or aborts a pipelined batch of already
  buffered commands beyond the rejected command itself.
- The fixed window may admit up to two windows' worth of commands around a
  boundary. That bounded burst is intentional; the budgets are overload
  shedding thresholds, not billing meters.
- A disabled budget adds no separate load to the command hot path: its presence
  is encoded in the security/observer words the dispatcher already reads, and
  the per-user map is allocated only while a budget is configured.

## CPU

There is no in-process CPU quota and none is claimed. CPU is bounded in three
layers:

1. the per-connection command quantum (bounded commands per reactor pass),
2. the cooperative run budget that yields between passes when several clients
   are admitted, and
3. the optional global command budget, which bounds total dispatch rate.

Hard CPU quotas are delegated to the operating system/container CPU controller
(cgroup `cpu.max`, Kubernetes `limits.cpu`, or equivalent). Operators who need a
strict CPU ceiling must configure it there; `INFO` does not promise an
in-process accounting of it. This is the recorded S9 scope decision.

## Interaction with persistence, transactions and pub/sub

- Pipelined commands already parsed before a budget rejection still produce
  their replies in order; the rejected command is the only one skipped.
- Commands inside `MULTI` are charged when queued (and not again when `EXEC`
  runs them); `EXEC` itself is charged once. A rejected queued command is not
  queued, so `EXEC` runs only the accepted commands.
- Blocked commands (`BLPOP`, `WAIT`, `XREAD`, …) are charged when the blocking
  command is dispatched; the later wake-up does not consume another budget slot.
- Pub/sub delivery, expiry/eviction maintenance and the journal's own flush work
  are server work, not client commands, and are not charged to a budget.
- Persistence latency is not a client admission limit. A slow `appendfsync`
  delays replies instead of dropping clients; the write-stall deadline is the
  bound that prevents those delayed replies from pinning a stalled socket.

## Evidence

- `tests/overload_policy.rs` exercises the readiness reactor, the Tokio
  transport and fixed lanes 1/4: global and per-user budget shedding and
  recovery, "rejected command is not executed", a non-reading client shed after
  the write-stall deadline with its `max-clients` slot reclaimed, a slow but
  progressing reader that survives, and a disabled deadline that keeps the
  backlogged connection.
- `src/engine/admission_tests.rs` covers the fixed-window accounting,
  per-user isolation, global/user precedence and `CONFIG RESETSTAT`.
- `src/server/pool.rs` unit tests cover the stall clock (armed by a blocked
  write, cleared only by write progress) and `poll` shedding.
- `tests/tls_write_stall.rs` covers a non-reading TLS client shed after the
  deadline and a slow but progressing TLS reader that survives.
- `tests/protocol_fuzz.rs` retains the seeded framing/command property corpus.

Failures are retained in the PR that lands a change; a green merge gate is not
capacity evidence and does not replace the P-series performance work.
