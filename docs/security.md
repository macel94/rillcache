# Password authentication and transport boundaries

Rillcache supports either one operator-configured password for the implicit Redis
`default` user or a Redis-style ACL file with multiple users, password hashes,
command categories/commands, key patterns and channel patterns. This is a real
access-control boundary for a private deployment, not a complete Redis security
design. It adds per-connection and per-peer-IP failed-`AUTH` budgets but not
global/cross-process rate limiting, delayed responses or username lockout, TLS
hardening (SNI/hot reload/OCSP), collector sender authentication/TLS/federation,
full-chain audit signing, key IDs or hot-rotation custody, or credential encryption at rest.

## What is implemented

- **Explicit configuration.** `--requirepass PASSWORD` sets the password for
  this process. `--requirepass-file PATH` reads it from a file and strips at most
  one trailing `\n` or `\r\n`; an empty or larger-than-1024-byte file is a startup
  error, not a silent no-password mode.
- **Startup default.** Without either option the server remains an unauthenticated
  loopback service. Containers pass `--bind 0.0.0.0:6379` by default, so operators
  must add authentication before exposing them beyond a trusted private network.
- **Required authentication.** When a password is configured, a new connection is
  unauthenticated. Only `AUTH`, `HELLO` (including `HELLO ... AUTH ...`) and
  `QUIT` execute; every other command, including `MULTI`, `INFO`, `CONFIG`,
  `COMMAND`, reads and writes, returns `NOAUTH Authentication required.` before
  touching the store.
- **Credential handling.** `AUTH [username] password` accepts only `default`
  (case-insensitive) and compares SHA-256 digests in constant time. The server
  stores only the digest. Wrong usernames, wrong passwords and disabled
  credentials return the generic `WRONGPASS` error. Calling `AUTH` when no
  password exists returns Redis's explicit no-password configuration error.
- **HELLO negotiation.** `HELLO <2|3> AUTH default <password> [SETNAME name]`
  validates every option and the credential before changing protocol, metadata or
  authentication state. A failed `HELLO` leaves all three unchanged.
- **Runtime rotation.** `CONFIG SET requirepass <value>` sets a new password;
  an empty value disables authentication. Changes are atomic with other
  `CONFIG SET` options: a rejected pair leaves both collection thresholds and the
  credential unchanged. `CONFIG GET requirepass` never returns the secret:
  unset reports an empty value, configured reports `<redacted>`.
- **No secret in diagnostics.** `Config`'s `Debug` representation, startup lines,
  errors and `CONFIG GET` redact the password. The password is never logged.

## Multi-user ACLs

`--aclfile PATH` loads a Redis-style text file instead of `--requirepass`
(the two options are mutually exclusive). Each non-comment line is
`user <name> <rules>`; an enabled `default` user is required or startup fails.
Rules are applied to a fresh disabled/no-command/no-key user per line, so a full
ACL file round-trips through `ACL SAVE`.

Supported rules:

- `on` / `off`
- `>password`, `<password`, `#<64-hex sha256>`, `!<64-hex sha256>`, `nopass`,
  `resetpass`
- `allcommands`/`+@all`, `nocommands`/`-@all`, `+@category`, `-@category`
- `+command`, `-command`, `+command|subcommand`, `-command|subcommand`
- `allkeys`, `resetkeys`, `~pattern`, `%R~pattern`, `%W~pattern`, `%RW~pattern`
- `allchannels`, `resetchannels`, `&pattern`
- `reset`

Rules are ordered: the last matching command/category/command rule decides.
`+command|subcommand` and `-command|subcommand` match the literal first argument
case-insensitively and participate in the same ordering, so a later
`-config|set` can carve a denial out of an earlier `+config`. A subcommand rule
does not grant the bare command, and the first argument has no wildcard form.
The `|` rule is supported in root rules and selector groups, is rendered by
`ACL LIST`/`ACL GETUSER`, round-trips through `ACL SAVE`/`LOAD`, and is evaluated
by `ACL DRYRUN` without executing the command.
Supported categories are `@all`, `@read`, `@write`, `@keyspace`, `@string`,
`@list`, `@set`, `@hash`, `@zset`, `@stream`, `@bitmap`, `@connection`,
`@transaction`, `@admin`, `@fast`, `@slow` and `@dangerous`. Category membership
is an approximation over the published command table, not a byte-for-byte copy of
a specific Redis release. Bounded subcommand overrides refine `CLIENT`, `MEMORY`,
`OBJECT`, `ACL`, `COMMAND`, `XGROUP` and `XINFO`: `@admin`/`@dangerous` match only
the administrative `CLIENT` and `ACL` subcommands, `MEMORY USAGE` is `@read` while
statistics are `@admin`, all `OBJECT` subcommands are `@keyspace` with data
inspection also `@read`, `COMMAND` subcommands are `@connection`, `XGROUP` write
subcommands exclude `HELP`, and `XINFO` read subcommands exclude `HELP`. `ACL CAT` and
`COMMAND LIST FILTERBY ACLCAT` enumerate these implemented subcommand entries;
commands outside the table keep parent entries.

Key patterns carry access modes. `~pattern` and `%RW~pattern` grant reads and
writes, `%R~pattern` grants reads only and `%W~pattern` writes only. Commands
require the access their Redis key specs perform: `GET` reads, `SET` without
`GET`, `MSET` and `DEL` write, and read-modify-write operations require both.
Mixed commands are checked per key (`BITOP` destination versus sources, `COPY`
source versus destination, `RENAME` source versus destination, and `SORT`,
`GEORADIUS*` or `*STORE` sources versus destinations). `SET ... GET` and
mutating `BITFIELD` operations require read and write. `PFCOUNT` is read-write
because Redis may promote a compact representation even though the command is
otherwise read-only. `XREAD`/`XREADGROUP` keys are extracted from the `STREAMS`
key/ID block; malformed/unbalanced blocks do not produce partial key lists. The
last `STORE`/`STOREDIST` destination is checked for last-wins commands.
A read-write key needs both a read and a write grant, which may come from
different patterns. Patterns are cumulative and `resetkeys` clears all modes;
`%RW~x` is listed canonically as `~x`, and adding read and write grants for the
same pattern merges them into `~x`.

`ACL SETUSER` is atomic: a bad rule leaves the previous user untouched. A
parenthesized rule group, such as `(+set %W~app:*)`, defines a Redis 7 selector.
Selectors are alternatives to the root rules: a command is allowed when the root
rules or any selector permits it, and its keys/channels must be permitted by that
same set. `clearselectors` removes them and `reset` clears them with the rest of
the user.
`ACL DELUSER`, `ACL WHOAMI`, `ACL USERS`, `ACL LIST`, `ACL GETUSER`, `ACL CAT`,
`ACL SAVE`, `ACL LOAD`, `ACL GENPASS`, `ACL DRYRUN` and `ACL LOG` are implemented.
`ACL DRYRUN user command [arg ...]` evaluates the named user's current rules
without executing the command or recording a denial. `ACL GENPASS [bits]` returns
hex from `/dev/urandom` (1..=1024 bits, default 256) and fails visibly if the OS
source is unavailable rather than falling back to weak randomness.

`ACL LOG [count]` returns the most recent denial entries (default 10, bounded to
128) as `[count, reason, context, object, username, age-seconds]` arrays;
identical denial tuples aggregate by count. Reasons distinguish `noauth`,
`command`, `key` and `channel`; context is `toplevel` or `multi`. `ACL LOG RESET`
clears the buffer. Entries contain command/key/channel names and usernames and are
therefore administrative data. The in-memory buffer is process-local, not
replicated, and is not an exhaustive security audit trail. Unauthenticated
denials are attributed to `default`.

`--audit-log PATH` adds an opt-in append-only denial trail. Each denial writes
one bounded line of `unix-ms reason context username-hex object-hex`; binary,
empty and newline-containing subjects are hex-encoded and truncated to fixed
bounds, so the file never contains plaintext passwords or key values. The file is
flushed per denial and reserves one generation for rotation at
`--audit-log-max-bytes` (1 KiB..=1 GiB, default 8 MiB), renaming the previous file
to `PATH.1`. `ACL LOG RESET` clears only the in-memory aggregation; the file is
append-only and appends across restart. Runtime write failures leave the command
reply unchanged but increment `audit_log_write_failures` in `INFO stats`. This is
a local audit aid, not fsync-per-line durability, full-chain signing/key
rotation, authenticated/centralized shipping or a substitute for rate limiting
and independent review.

`--audit-syslog IP:PORT` additionally sends each denial line as one best-effort
UDP datagram. It works with or without the local file, validates a numeric
address with a nonzero port at startup, and shares the same fixed, redacted line
format. UDP can lose, duplicate, reorder or expose datagrams to the path; there
is no collector authentication, TLS, retry, backpressure, aggregation or delivery
guarantee. Runtime send failures do not change the reply but increment
`audit_syslog_write_failures` in `INFO stats`. The local file, when configured,
remains the durable evidence path.

`--audit-hmac-key-file PATH` authenticates each denial line with a local
16..=1024-byte key loaded before accept; one trailing newline is stripped. The
server appends one space and a 64-character lowercase hex HMAC-SHA256 over the
exact line body, using the dependency-free SHA-256 implementation. The same
signed line is written to the file and UDP sink. Without a key, the five-field
format is unchanged. The key is never included in diagnostics or replies.
`--audit-hmac-keyring-file PATH` accepts a text keyring of hex keys (one per line,
`#` comments allowed, at most eight unique 16..=1024-byte keys, at most 64 KiB of
text including comments; repeated keys are ignored); the first key
signs, and the collector accepts any key in its own keyring. An operator can
therefore restart the signer with the new key first while the collector retains
the old key until old lines age out. This is per-line authentication, not a hash
chain: it does not detect deleted or reordered lines, provide replay protection,
key IDs, hot reload, HSM/KMS custody, authenticated collector delivery or
tamper-proof storage. Key files/keyrings and their permissions are operator
duties.

`rillcache-audit-collector` is a separate standard-library-only UDP sink for
off-host aggregation. `--bind ADDRESS` and `--output PATH` are required
operationally (defaults are loopback and `audit.log`); each nonempty datagram up
to `--max-datagram` is appended as one newline-terminated line, flushed, and
rotated to `PATH.1` at `--max-bytes` (1 KiB..=1 GiB). Empty/oversized datagrams
are dropped and counted. `--max-datagrams N` exits after N accepted datagrams for
bounded tests or one-shot collection. `--hmac-key-file PATH` loads a shared
16..=1024-byte key and retains only lines with a valid per-line HMAC, while
`--hmac-keyring-file PATH` verifies against any listed rotation key; malformed,
tampered and wrong-key datagrams are dropped and counted, and a collector without
a key stores lines unverified. Verification does not authenticate sender
identity, encrypt traffic, prevent replay, retry, reorder, deduplicate or federate
multiple collectors; UDP loss and spoofing remain possible. The output is created
mode-`0600` on Unix, but directory permissions and key/log custody are operator
duties; the key is never written to the output or diagnostics.

`ACL SAVE` writes a mode-`0600` temporary file in the same directory, fsyncs it,
renames it over the target and fsyncs the directory (where supported); hashes are
saved as `#...`, never plaintext passwords. ACL files are bounded to 4 MiB,
10,000 users, 1,024 rules per user/operation and 1,024 key/channel patterns per
user; exceeding a bound is a visible error, not truncation.
`ACL LOAD` parses a replacement map before publishing it. `ACL SAVE/LOAD` require
a configured ACL file. The `default` user cannot be deleted.

Authorization is evaluated before command dispatch and store access for known
commands with declared keys. `EXEC` re-checks every queued command against the
current ACL, so revoking or disabling a user between `MULTI` and `EXEC` cannot
commit the queued writes. Recovery commands (`AUTH`, `HELLO`, `QUIT`) remain
available to unauthenticated connections. Denied commands return `NOPERM`; denied
keys return the key-specific `NOPERM`; disabled/deleted users and `NOAUTH` are
re-evaluated on every command, so `ACL SETUSER`/`DELUSER`/`LOAD` changes apply to
live connections immediately. `ACL WHOAMI` is available to any authenticated user.

## Deliberate limits

- **Not full Redis ACL.** Remaining subcommand-specific categories beyond the
  bounded `CLIENT`/`MEMORY`/`OBJECT`/`ACL`/`COMMAND`/`XGROUP`/`XINFO`/`CONFIG`/
  `LATENCY`/`SLOWLOG`/`PUBSUB` overrides,
  nested selectors, dynamic key-sandbox rules, collector sender
  authentication/TLS/federation, full-chain signing/key-ID/hot rotation,
  global/cross-process rate limiting and TLS hardening (SNI/hot reload/OCSP) are
  absent.
  Selectors, first-argument command rules, `ACL LOG`, `ACL DRYRUN`, `ACL GENPASS`
  and the optional local denial audit are implemented for the published command table;
  commands not present in that table remain unsupported.
  Channel patterns gate PUBLISH/SUBSCRIBE/PSUBSCRIBE/UNSUBSCRIBE/PUNSUBSCRIBE, but
  sharded pub/sub and keyspace notifications do not exist. `PUBSUB` introspection
  is category-governed rather than channel-pattern-filtered, so it can reveal channel
  names to a user allowed to run `@pubsub`. `CLIENT LIST`/`KILL` reveal connection
  addresses/names/users and terminate sessions; grant the CLIENT command only to
  trusted administrators. Key patterns apply only to commands with declared key
  positions:
  `KEYS`, `RANDOMKEY`, `DBSIZE`, `FLUSHDB/FLUSHALL` and similar commands are not
  a per-key sandbox. `XREAD` and `XREADGROUP` key lists are extracted from their
  `STREAMS` block, but dynamic `SORT BY/GET` lookups remain all-key-only. Do not grant these commands to tenants who must not
  see or clear the whole keyspace.
- **Hashes are unsalted SHA-256.** This mirrors Redis ACL hash compatibility but
  is fast to brute-force offline; use long random passwords and protect the ACL
  file. File permissions, backups and secret distribution are operator duties.
- **No tenant isolation at the process/network level.** All users share one
  in-memory keyspace and one process. ACL permissions do not create separate
  databases, memory budgets, CPU budgets or network identities.
- **Native TLS via rustls.** The project has one explicit dependency exception:
  rustls and the transitive crates required to build it, vendored for offline
  builds. Native TLS 1.3 is terminated in-process; there is no OpenSSL, cgo or
  sidecar. TLS 1.2 and older are rejected, and HTTP/2 or HTTP/3 are not offered
  because Redis clients speak RESP. The plaintext listener still exists and must
  be bound to loopback/private space or firewalled. See [native TLS](tls.md).
- **Per-connection failed-auth budget.** `--auth-failure-limit N` (default 10,
  `0` disables) closes a connection after `N` consecutive failed `AUTH` or
  `HELLO ... AUTH` attempts; a successful authentication or `RESET` clears the
  counter. The standard `WRONGPASS` reply is sent before the close. A client can
  reconnect, so this is not a per-source or global lockout.
- **Per-peer-IP failed-auth throttle.** `--auth-source-limit N` (default `0`,
  disabled) counts failures by peer IP within `--auth-source-window-ms W` (default
  60000). Once a source reaches `N`, further `AUTH`/`HELLO AUTH` credential
  attempts receive `ERR too many authentication attempts, try later` until the
  window expires; a successful authentication resets that source. The table is
  process-local and bounded, so shared NAT addresses can throttle each other and
  an attacker can exhaust the table; there is no delay, username lockout,
  cross-process or global rate limit. Use network policy/firewalling and strong,
  unique passwords; do not expose the port directly to untrusted clients.
- **No live deauthentication.** Rotating or disabling the password affects new
  authentication decisions; a connection that already authenticated remains
  authenticated, as in Redis. Close/reconnect administrative clients when rotating.
  Existing unauthenticated connections begin receiving `NOAUTH` immediately after
  a password is set.
- **Process arguments are visible.** `--requirepass VALUE` can appear in process
  listings and shell history. Prefer `--requirepass-file` or `--aclfile` with a
  mode-`0400` secret mount. Files are read at startup and by explicit `ACL LOAD`;
  later password rotation uses authenticated `CONFIG SET`, and user changes use
  `ACL SETUSER` plus `ACL SAVE`.
- **No persistence.** The password is not written to disk and does not survive a
  restart unless the deployment supplies the flag/file again. The server also has
  no data persistence or replication.
- **Single-process scope.** This is process-local admission control. It does not
  provide distributed identity, failover, fencing tokens or protection against a
  host/container administrator.

## Deployment guidance

1. Keep the default loopback bind for local development.
2. Inside Docker/Kubernetes, mount a secret file read-only and append
   `--requirepass-file /run/secrets/rillcache` or `--aclfile /run/secrets/users.acl`;
   keep the root filesystem read-only. `ACL SAVE` needs a writable file/directory,
   so mount a dedicated writable volume only when runtime ACL mutation+save is
   required.
3. Restrict ingress with a network policy or firewall. Use the native
   [TLS 1.3 listener](tls.md) or another audited terminator for anything crossing
   an untrusted network, and keep the plaintext listener private.
4. Use a long random password from a secret manager, not a short human password;
   SHA-256 comparison does not make weak passwords hard to guess offline.
5. Rotate with authenticated `CONFIG SET requirepass`, then restart clients, and
   verify a new connection needs the new credential.

## Evidence

Unit tests cover known SHA-256 vectors, constant-time digest equality, disabled and
rotated credentials, ordered category/command overrides, key-pattern denials,
atomic failed `SETUSER`, ACL-file hash/rule round-trips, exact Redis-compatible
errors, `HELLO` atomicity, `NOAUTH` before data/admin access, unauthenticated
`QUIT`, and `CONFIG` redaction/rotation. Wire tests run in thread-per-client,
one-worker and four-worker modes, including ACL-file startup failures, restricted
users, `ACL SAVE/LOAD`, deletion/deactivation of live users, pipelined
rejection/authentication/order and a full runtime rotation cycle.

References: [Redis AUTH](https://redis.io/docs/latest/commands/auth/),
[Redis security](https://redis.io/docs/latest/operate/oss_and_stack/management/security/),
[Redis TLS](https://redis.io/docs/latest/operate/oss_and_stack/management/security/encryption/).
