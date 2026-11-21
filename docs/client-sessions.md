# Client sessions and protocol negotiation

Rillcache starts each socket in RESP2 mode. `HELLO` supports no-argument
capability discovery and `HELLO 2` / `HELLO 3`; a valid negotiation returns the
usual key/value capability fields as a RESP2 array or RESP3 map, respectively.
Identity is reported truthfully as `server=rillcache` and the Cargo package
version. `proto` and `id` use RESP integer values, matching the HELLO contract;
`server`, `version`, and `mode` are bulk strings. The response intentionally
omits claims about modules, replication, Redis version, or features not
represented by the server.

A session retains its protocol, monotonically assigned connection ID, optional
name, and optional client-library name/version. State follows the connection
when a fixed worker owns it or when the managed per-client socket lease is
handed to another worker. Metadata is limited to 256 bytes for names and 128
bytes per library field; values must be printable non-space ASCII. Empty names
clear the name. `CLIENT INFO` exposes the retained ID, database zero, set
metadata, the connection file descriptor, `age`, the `b`/`P`/`N` flags, separate
`sub` (channel), `psub` (pattern) and `ssub` (shard-channel) counts, the live
MULTI queue length and the WATCH count. It remains a bounded subset of Redis's
connection statistics: no idle time, last command, input/output buffer,
allocator, network-byte or cross-process fields are fabricated.

Supported connection commands are `CLIENT ID`, `CLIENT SETNAME`, `CLIENT
GETNAME`, `CLIENT SETINFO LIB-NAME|LIB-VER`, `CLIENT INFO`, `CLIENT LIST`,
`CLIENT KILL`, `CLIENT PAUSE`, `CLIENT UNPAUSE`, `CLIENT UNBLOCK`,
`SELECT 0` and `RESET`.
Other database indices are rejected and never alias database zero. `AUTH` and
`HELLO ... AUTH ...` authenticate either the `default` user or an ACL user when
credentials are configured. `ACL WHOAMI`, `ACL GETUSER` and the other supported
ACL subcommands expose and enforce the current policy. See
[security](security.md) for the actual boundary and remaining gaps.

`CLIENT LIST [TYPE normal|pubsub|master|replica|slave] [ID id ...]` returns one newline-delimited
key=value record per registered connection with `id`, peer `addr`, local
`laddr`, `fd`, `name`, `age`, `flags` (N/P plus `b` while blocked), `db=0`, live
`sub` (channel), `psub` (pattern) and `ssub` (shard-channel) counts, the MULTI
queue length, the WATCH count, negotiated `resp`, `user`, `lib-name` and
`lib-ver`. Shard-only subscribers have the
`pubsub` type and `P` flag. `fd` is captured once when the connection is
registered; `multi` and `watch` are refreshed by the owning session on the rare
commands that change them, so no per-command registry write is added. It
intentionally omits fields this server does not measure (idle, last command,
buffers, memory, redirection, network totals) rather than inventing values.
`CLIENT KILL` uses intersecting filters: `ID <id>`, `ADDR <ip:port>`,
`LADDR <ip:port>`, `TYPE normal|pubsub|master|replica|slave`, `USER <name>` and
`MAXAGE <milliseconds>` (matching connections at least that old), with
`SKIPME yes` (default) or `SKIPME no`. Standalone servers have no master/replica
clients, so those types match nothing. `SKIPME yes` always skips the caller;
`SKIPME no` may kill the caller, in which case its socket closes and the reply
can race with the close. The legacy single-address form returns `+OK`/`No such
client`, while filtered forms return the killed count. A killed socket is shut
down immediately. `INFO clients` reports `connected_clients`,
`cluster_connections:0`, configured `maxclients`, and `blocked_clients` (the
current number of connections waiting in a blocking command).

`CONFIG GET maxclients` reports the live connection limit and `CONFIG SET
maxclients <positive-integer>` changes it at runtime in both thread-per-client
and fixed-worker modes. Lowering the limit below the current admitted count does
not close existing clients; new sockets receive `ERR max number of clients
reached` and increment `rejected_connections` until capacity is available.
Raising the limit admits new connections without a restart. Zero, negative,
empty and non-integer values are rejected without changing the limit. The value
is a connection count, not a file-descriptor, memory or cross-process capacity
guarantee.

`CLIENT PAUSE timeout [WRITE|ALL]` temporarily defers new client commands for
`timeout` milliseconds; the default mode is `ALL`. Under `WRITE`, read-only
commands continue while commands carrying the write flag (and `EXEC`) wait.
`timeout` must be a non-negative integer; `0` is a no-op. A later deadline wins
and an active `ALL` pause is not downgraded by a subsequent `WRITE` pause.
`CLIENT UNPAUSE` ends the pause immediately. Both management commands are exempt
from the pause so an operator can always inspect or end it; the command is
otherwise subject to the normal `CLIENT` permission.

`CLIENT UNBLOCK <client-id> [TIMEOUT|ERROR]` wakes only a currently blocked
connection and returns `:1`, or `:0` for an unknown/non-blocked ID. TIMEOUT (the
default) completes the blocked command as a null; ERROR returns
`UNBLOCKED client unblocked via CLIENT UNBLOCK`. The request is process-local,
consumed once, discarded on completion/disconnect and subject to the normal
`CLIENT` permission; RESP3 timeout wake-ups use the protocol null frame.

A paused connection executes at most one deferred command, stops parsing
pipelined commands until the pause ends, and polls with the same bounded model as
blocking commands without holding the store lock. Replies preserve wire order,
and a disconnect or shutdown drops the deferred command and the connection.
Commands that started before the pause, already-blocked commands, background
expiry and eviction, and journal syncing are not paused. This is deliberately a
client-command gate, not Redis's full pause surface (which also pauses expiry,
eviction and replica traffic); replication does not exist here.

`CLIENT HELP` returns the supported subcommand list and argument hints.

`CLIENT LIST`/`KILL` expose addresses, names, users and live connections; they
require the normal `CLIENT` command permission and are only appropriate for trusted
administrators. The registry is process-local, bounded by `max_clients`, and does
not survive restart.

`RESET` implements a disconnect-equivalent server-side reset: it discards an
active `MULTI`, clears `WATCH`, unsubscribes channel/pattern subscriptions, returns
the protocol to RESP2, selects database zero (the only database), and clears the
authenticated user so a password-protected deployment requires `AUTH` again. The
client name and library metadata are retained; there is no `MONITOR`, tracking,
`NO-TOUCH` state or multiple-database state to clear. `RESET` is allowed
without authentication and from RESP2 subscriber mode, replies `+RESET`, and
restores `CLIENT REPLY` to `ON`.

`CLIENT REPLY ON|OFF|SKIP` controls reply suppression. `OFF` suppresses every
subsequent command reply until `ON` (whose `+OK` is delivered); `SKIP`
suppresses exactly the next reply. The arming command itself never replies for
`OFF`/`SKIP`. Suppression covers immediate, paused and blocked-completion
replies, and disabling replies also disables multi-owner deferral for that
session so a suppressed reply cannot be written from the owner path. Pub/Sub
push frames and `MONITOR` feeds are separate from command replies.

`CLIENT NO-EVICT ON|OFF` is accepted and replies `+OK`. Established clients are
never evicted—`max_clients` rejects new connections—so both modes have the same
enforced behavior. The flag is not separately reported because no eviction path
exists to suppress.

Client-side caching is not implemented, so its introspection reports the disabled
state: `CLIENT GETREDIR` returns `-1`, `CLIENT TRACKINGINFO` reports flags
`["off"]`, redirect `-1` and no prefixes, and `CLIENT CACHING YES|NO` returns
Redis's "only in tracking mode with OPTIN or OPTOUT" error. `CLIENT TRACKING
ON` remains an explicit unsupported-capability error.

RESP3 conversion is per reply and framing-aware. Current data commands produce
simple strings, errors, integers, and bulk strings; RESP2 null bulk replies
become the genuine RESP3 null (`_\r\n`), including RESP2 null arrays
(`*-1\r\n`) and null children. Conversion is by frame boundaries. Binary bulk payload bytes—including bytes that resemble a
RESP null frame—are preserved. RESP2 behavior and `Store::execute` remain
unchanged.

## Session module layout

`src/session/mod.rs` owns the `Session` value, protocol mode, connection ID and
the `execute` entry point. Command routing and authorization ordering live in
`dispatch.rs`; `transaction.rs` owns MULTI/EXEC/WATCH queue state; `auth.rs`
owns AUTH/ACL; `pubsub.rs` owns channel, pattern, shard and `MONITOR`
subscriptions plus delivery; `blocking.rs` owns blocked-command state and
wakeup polling; `pause.rs` owns `CLIENT PAUSE` deferral; `client.rs` owns
`HELLO`, `CLIENT` metadata/list/kill and metadata sync; `reply.rs` owns
RESP2-to-RESP3 conversion; and `command.rs` holds shared command
classification and arity replies. This is an internal split only: the public
`Session` API and wire behavior are unchanged.

This is not full RESP3 or Redis compatibility. Pushes, tracking, pub/sub,
streaming types, RESP3 booleans/maps/sets for data commands, AUTH/ACL, TLS, and
multiple logical databases are unsupported. Redis-style inline request lines are
supported in addition to RESP arrays; see [requirements](requirements.md) for the
bounded quoting, escape and line semantics. The server does
not emit asynchronous push frames. A pinned `redis-cli` RESP2/RESP3, AUTH and
client-metadata interop test is implemented (S103); Python/Node/Go client-library
handshake/interoperability validation remains a separate gate. Passing local wire
tests is not evidence of production readiness.
