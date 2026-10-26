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
GETNAME`, `CLIENT SETINFO LIB-NAME|LIB-VER`, `CLIENT INFO`, and `SELECT 0`.
Other database indices are rejected and never alias database zero. `AUTH` and
`HELLO ... AUTH ...` are explicitly rejected: the server has no authentication,
ACL, or authorization implementation. No security boundary is implied.

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
