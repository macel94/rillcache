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
clear the name. `CLIENT INFO` exposes the retained ID, database zero, and set
metadata; it is intentionally a small subset of Redis's connection statistics.

Supported connection commands are `CLIENT ID`, `CLIENT SETNAME`, `CLIENT
GETNAME`, `CLIENT SETINFO LIB-NAME|LIB-VER`, `CLIENT INFO`, `CLIENT LIST`,
`CLIENT KILL`, `CLIENT PAUSE`, `CLIENT UNPAUSE`, `SELECT 0` and `RESET`.
Other database indices are rejected and never alias database zero. `AUTH` and
`HELLO ... AUTH ...` authenticate either the `default` user or an ACL user when
credentials are configured. `ACL WHOAMI`, `ACL GETUSER` and the other supported
ACL subcommands expose and enforce the current policy. See
[security](security.md) for the actual boundary and remaining gaps.

`CLIENT LIST [TYPE normal|pubsub] [ID id ...]` returns one newline-delimited
key=value record per registered connection with `id`, `addr`, `name`, `age`,
`flags` (N/P), `db=0`, live `sub`/`psub` counts, negotiated `resp`, `user`,
`lib-name` and `lib-ver`. It intentionally omits fields this server does not
measure (fd, idle, command, buffers, memory, redirection, network totals) rather
than inventing values. `CLIENT KILL` uses filters intersecting with each other:
`ID <id>`, `ADDR <ip:port>`, `TYPE normal|pubsub`, `USER <name>` and
`MAXAGE <milliseconds>` (matching connections at least that old), with
`SKIPME yes` only (default); `SKIPME no` returns an explicit unsupported error.
The legacy single-address form returns `+OK`/`No such client`, while filtered
forms return the killed count. The calling connection is always skipped, and a
killed socket is shut down immediately. `INFO clients` reports
`connected_clients`, `cluster_connections:0` and the configured `maxclients`.

`CLIENT PAUSE timeout [WRITE|ALL]` temporarily defers new client commands for
`timeout` milliseconds; the default mode is `ALL`. Under `WRITE`, read-only
commands continue while commands carrying the write flag (and `EXEC`) wait.
`timeout` must be a non-negative integer; `0` is a no-op. A later deadline wins
and an active `ALL` pause is not downgraded by a subsequent `WRITE` pause.
`CLIENT UNPAUSE` ends the pause immediately. Both management commands are exempt
from the pause so an operator can always inspect or end it; the command is
otherwise subject to the normal `CLIENT` permission.

A paused connection executes at most one deferred command, stops parsing
pipelined commands until the pause ends, and polls with the same bounded model as
blocking commands without holding the store lock. Replies preserve wire order,
and a disconnect or shutdown drops the deferred command and the connection.
Commands that started before the pause, already-blocked commands, background
expiry and eviction, and journal syncing are not paused. This is deliberately a
client-command gate, not Redis's full pause surface (which also pauses expiry,
eviction and replica traffic); replication does not exist here.

`CLIENT LIST`/`KILL` expose addresses, names, users and live connections; they
require the normal `CLIENT` command permission and are only appropriate for trusted
administrators. The registry is process-local, bounded by `max_clients`, and does
not survive restart.

`RESET` implements a disconnect-equivalent server-side reset: it discards an
active `MULTI`, clears `WATCH`, unsubscribes channel/pattern subscriptions, returns
the protocol to RESP2, selects database zero (the only database), and clears the
authenticated user so a password-protected deployment requires `AUTH` again. The
client name and library metadata are retained; there is no `MONITOR`, tracking,
`NO-EVICT`/`NO-TOUCH` or multiple-database state to clear. `RESET` is allowed
without authentication and from RESP2 subscriber mode, and replies `+RESET`.

RESP3 conversion is per reply and framing-aware. Current data commands produce
simple strings, errors, integers, and bulk strings; RESP2 null bulk replies
become the genuine RESP3 null (`_\r\n`), including RESP2 null arrays
(`*-1\r\n`) and null children. Conversion is by frame boundaries. Binary bulk payload bytes—including bytes that resemble a
RESP null frame—are preserved. RESP2 behavior and `Store::execute` remain
unchanged.

This is not full RESP3 or Redis compatibility. Pushes, tracking, pub/sub,
streaming types, RESP3 booleans/maps/sets for data commands, AUTH/ACL, TLS, and
multiple logical databases are unsupported. Redis-style inline request lines are
supported in addition to RESP arrays; see [requirements](requirements.md) for the
bounded quoting, escape and line semantics. The server does
not emit asynchronous push frames. Real client-library handshake/interoperability
validation remains a separate gate; passing local wire tests is not evidence of
production readiness.
