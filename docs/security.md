# Password authentication and transport boundaries

For reporting repository or release vulnerabilities, use the private process in
[`SECURITY.md`](../SECURITY.md); do not disclose suspected vulnerabilities in a
public issue.

Rillcache supports either one operator-configured password for the implicit Redis
`default` user or a Redis-style ACL file with multiple users, password hashes,
command categories/commands, key patterns and channel patterns. This is a real
access-control boundary for a private deployment, not a complete Redis security
design. It adds per-connection and per-peer-IP failed-`AUTH` budgets plus an
optional process-wide aggregate budget, but not cross-process rate limiting,
delayed responses or username lockout, TLS
server-certificate OCSP validation, collector sender authentication/TLS/federation,
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
  unset reports an empty value, configured reports `<redacted>`. Rotation affects
  new authentication decisions only; close existing sessions explicitly with
  `CLIENT KILL USER <name>` (or `CLIENT KILL ID/ADDR/MAXAGE`) from an
  administrative connection, then reconnect with the new credential.
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
Supported category rules are `@all`, `@read`, `@write`, `@keyspace`, `@string`,
`@list`, `@set`, `@hash`, `@sortedset` (and the compatible `@zset` alias),
`@hyperloglog`, `@stream`, `@bitmap`, `@geo`, `@pubsub`, `@connection`,
`@transaction`, `@blocking`, `@admin`, `@fast`, `@slow` and `@dangerous`. The
`@generic` grouping and `@all` listing are Rillcache profile extensions;
`@scripting` is not accepted because scripting is unsupported. `ACL CAT` exposes
all accepted names, including the local `@zset`/`@generic` aliases. Category
membership is an approximation over the published command table, not a
byte-for-byte copy of a specific Redis release. Bounded subcommand overrides refine `CLIENT`, `MEMORY`,
`OBJECT`, `ACL`, `COMMAND`, `XGROUP` and `XINFO`: `@admin`/`@dangerous` match only
the administrative `CLIENT` and `ACL` subcommands, `MEMORY USAGE` is `@read`
while other implemented memory subcommands are `@slow` (not `@admin`), all
`OBJECT` subcommands are `@keyspace` with data
inspection also `@read`, keyspace and collection families are listed by `ACL CAT`,
`COMMAND` subcommands are `@connection`, `XGROUP` write
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
key/ID block; malformed/unbalanced blocks do not produce partial key lists.
Counted list-pop `LMPOP` and `BLMPOP` specifications authorize only the declared
`numkeys` keys, excluding direction and optional `COUNT` arguments. Rillcache's
`MSETEX` extension likewise checks only the keys in its counted key/value pairs,
excluding trailing SET options; its command implementation defines this layout,
and `COMMAND GETKEYS` is tested against it (this is not a Redis/Valkey command
compatibility claim). The pinned
Valkey 9.1.2 reference (`valkey/valkey:9.1.2`, reporting Redis compatibility
7.2.4) returns exactly those keys from `COMMAND GETKEYS LMPOP`, including with
`RIGHT COUNT n` and binary key bytes. The last `STORE`/`STOREDIST` destination is
checked for last-wins commands.
A read-write key needs both a read and a write grant, which may come from
different patterns. Patterns are cumulative and `resetkeys` clears all modes;
`%RW~x` is listed canonically as `~x`, and adding read and write grants for the
same pattern merges them into `~x`.

Keyless keyspace-wide commands receive an additional ACL scope check:
`KEYS`, `SCAN`, `RANDOMKEY` and `DBSIZE` need an all-key read grant; `FLUSHDB`
and `FLUSHALL` need an all-key write grant. Without it, the server returns
`NOPERM keyspace-wide commands require an all-key ACL grant with the required
access`; `ACL DRYRUN` returns the same result. This deliberately differs from
Redis/Valkey, where these keyless commands are not filtered by key patterns.
Rillcache denies them rather than expose or flush keys outside a scoped user's
patterns. A read-only all-key grant can inspect but cannot flush; a write-only
all-key grant can flush but cannot inspect. Root/all-key users retain normal
behavior.

`ACL SETUSER` is atomic: a bad rule leaves the previous user untouched. The
Rillcache extension `maxconnections=N` sets a concurrent authenticated-session
cap for that user (`0` through `1,000,000`; zero means unlimited). `ACL GETUSER`
reports the value and `ACL LIST`/`ACL SAVE`/`ACL LOAD` preserve it. AUTH, HELLO
AUTH and verified certificate-mapped TLS identities reserve a slot atomically;
when full, a valid login receives `-ERR max connections reached for user` and
retains any previous identity. Reauthenticating as the same user does not consume
another slot; switching users, `RESET` and disconnect release capacity. A TLS
`RESET` that would restore a certificate identity over its lowered cap fails
atomically and keeps the current identity. Lowering a cap does not evict existing
sessions, but blocks later admissions until the active count falls below the new value. Only authenticated ACL sessions count:
unauthenticated sessions using the implicit no-password default user are bounded
by global `maxclients` only. This is not a memory, CPU or per-command quota. A
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
over `PATH.1` without first removing the older generation. A failed rename
leaves the older generation in place. If rotation succeeds but reopening the
active path fails, the next local audit write retries the open without rotating
or appending to `PATH.1`. Failed records are not replayed. `ACL LOG RESET` clears only the in-memory
aggregation; the file is append-only and appends across restart. Runtime write failures leave the command
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
16..=1024-byte key loaded before accept. The server and collector strip one
trailing LF or CRLF from a raw key file and read at most 1027 bytes to detect
oversized input. This bounds bytes read, not the time spent on file I/O. The
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
rotated to `PATH.1` at `--max-bytes` (1 KiB..=1 GiB). Empty and oversized
records are dropped and counted; a line must also fit `--max-bytes`, including
any added newline. An oversized record does not rotate the output. A failed
rotation rename retains the previous `PATH.1`; this does not guarantee recovery
from a later reopen failure or a crash. An existing
oversized file is not rewritten on startup. `--max-datagrams N` exits after N accepted datagrams for
bounded tests or one-shot collection. `--hmac-key-file PATH` loads a shared
16..=1024-byte key and retains only lines with a valid per-line HMAC, while
`--hmac-keyring-file PATH` verifies against any listed rotation key; malformed,
tampered and wrong-key datagrams are dropped and counted, and a collector without
a key stores lines unverified. On Unix, pass `--secure-secret-files` with either
key option to require a regular file with no group or other permission bits
(for example, mode `0400` or `0600`). The collector checks the opened file before
reading and rejects invalid settings before binding. Use a trusted directory;
this checks mode bits, not owner identity or managed secret custody.
Verification does not authenticate sender
identity, encrypt traffic, prevent replay, retry, reorder, deduplicate or federate
multiple collectors; UDP loss and spoofing remain possible. The output is created
mode-`0600` on Unix, but directory permissions and key/log custody are operator
duties; the key is never written to the output or diagnostics. An output
append, flush, or rotation failure exits nonzero and prints the prior counts
with `output_failures 1`, without echoing the failed datagram. A failed write
is not counted as accepted and may have left partial output; the summary is
not an fsync or delivery guarantee.

`ACL SAVE` writes a mode-`0600` temporary file in the same directory, fsyncs it,
renames it over the target and fsyncs the directory (where supported); hashes are
saved as `#...`, never plaintext passwords. ACL files are bounded to 4 MiB,
10,000 users, 1,024 rules per user/operation and 1,024 key/channel patterns per
user; exceeding a bound is a visible error, not truncation.
Startup and `ACL LOAD` read at most 4 MiB plus one byte to detect overflow,
regardless of the size reported by file metadata. This caps bytes read, not
I/O time or total memory use. `ACL LOAD` parses a replacement map before
publishing it; a rejected file leaves the previous rules active. With
`--secure-secret-files`, startup and `ACL LOAD` also check that the opened ACL
file is regular and has no group or other permission bits before reading it.
Apply restrictive permissions to a replacement before publishing it. This is
not an owner-identity check or an I/O deadline. `ACL SAVE/LOAD` require a
configured ACL file. The `default` user cannot be deleted.

Authorization is evaluated before command dispatch and store access for known
commands with declared keys. `EXEC` re-checks every queued command against the
current ACL, so revoking or disabling a user between `MULTI` and `EXEC` cannot
commit the queued writes. Recovery commands (`AUTH`, `HELLO`, `QUIT`) remain
available to unauthenticated connections. Denied commands return `NOPERM`; denied
keys return the key-specific `NOPERM`; disabled/deleted users and `NOAUTH` are
re-evaluated on every command, so `ACL SETUSER`/`DELUSER`/`LOAD` changes apply to
live connections immediately. `ACL WHOAMI` is available to any authenticated user.

## Deliberate limits

The authoritative open security list is in
[production readiness gaps](production-readiness-gaps.md). The boundary below
describes current behavior, not a second checklist.

- **Not full Redis ACL.** Remaining subcommand-specific categories beyond the
  bounded `CLIENT`/`MEMORY`/`OBJECT`/`ACL`/`COMMAND`/`XGROUP`/`XINFO`/`CONFIG`/
  `LATENCY`/`SLOWLOG`/`PUBSUB` overrides,
  nested selectors, dynamic key-sandbox rules, collector sender
  authentication/TLS/federation, full-chain signing/key-ID/hot rotation,
  cross-process/distributed rate limiting and server-certificate OCSP validation
  are absent. Native mTLS client CRL checking is opt-in and fail-closed; see [TLS](tls.md).
  The per-user `maxconnections` extension limits authenticated sessions only; it
  does not isolate the shared keyspace, memory, CPU or unauthenticated default-user
  connections.
  Selectors, first-argument command rules, `ACL LOG`, `ACL DRYRUN`, `ACL GENPASS`
  and the optional local denial audit are implemented for the published command table;
  commands not present in that table remain unsupported. This is not full upstream
  category parity: the profile has local `@generic`/`@zset` aliases, does not accept
  `@scripting`, and retains bounded subcommand overrides plus parent-category
  fallback for families outside the published override table.
  Channel patterns gate PUBLISH/SPUBLISH, SUBSCRIBE/UNSUBSCRIBE,
  PSUBSCRIBE/PUNSUBSCRIBE and SSUBSCRIBE/SUNSUBSCRIBE. `PUBSUB CHANNELS` and
  `SHARDCHANNELS` return only names permitted by the caller's channel patterns
  (including selector-scoped grants). `NUMSUB` and `SHARDNUMSUB` reject the whole
  request if any supplied channel is unauthorized, avoiding partial counts.
  `NUMPAT` is denied unless a ruleset granting that subcommand also grants all
  channels, because its global count cannot be safely filtered. These introspection
  checks intentionally strengthen the pinned Valkey 9.1.2 behavior: upstream's
  PUBSUB handlers return global data and its generic ACL channel checks cover only
  channel-bearing command metadata. Other unsupported PUBSUB subcommands continue
  to return their existing unsupported errors. Sharded pub/sub is a separate
  standalone namespace and keyspace notifications are process-local and at-most-once;
  neither is cluster-routed or durable. `CLIENT LIST`/`KILL` reveal connection
  addresses/names/users and terminate sessions; grant the CLIENT command only to
  trusted administrators. Key patterns apply to declared key positions; the
  keyless `KEYS`, `SCAN`, `RANDOMKEY`, `DBSIZE`, `FLUSHDB` and `FLUSHALL` commands
  instead require the corresponding all-key read/write grant described above.
  Dynamic key lists are handled only for explicitly supported command specs,
  including counted `LMPOP`/`BLMPOP` keys and the `XREAD`/`XREADGROUP` `STREAMS`
  block; dynamic `SORT BY/GET` lookups remain all-key-only. This is not full Redis
  ACL/key-spec coverage, and commands outside the published registry remain
  unsupported.
- **Hashes are unsalted SHA-256.** This mirrors Redis ACL hash compatibility but
  is fast to brute-force offline; use long random passwords and protect the ACL
  file. File permissions, backups and secret distribution are operator duties.
- **No tenant isolation at the process/network level.** All users share one
  in-memory keyspace and one process. ACL permissions do not create separate
  databases, memory budgets, CPU budgets or network identities.
- **Native TLS via rustls.** The reviewed dependency exceptions are rustls,
  Tokio (with Mio as its readiness backend) and `core_affinity`, plus their
  required transitive crates, vendored for offline builds. Native TLS 1.3 is
  terminated in-process; there is no OpenSSL, cgo or
  sidecar. TLS 1.2 and older are rejected, and HTTP/2 or HTTP/3 are not offered
  because Redis clients speak RESP. The plaintext listener still exists and must
  be bound to loopback/private space or firewalled. See [native TLS](tls.md).
- **Per-connection failed-auth budget.** `--auth-failure-limit N` (default 10,
  `0` disables) closes a connection after `N` consecutive failed `AUTH` or
  `HELLO ... AUTH` attempts; a successful authentication or `RESET` clears the
  counter. The standard `WRONGPASS` reply is sent before the close. A client can
  reconnect, so this setting by itself is not a per-source or global lockout.
- **Per-peer-IP failed-auth throttle.** `--auth-source-limit N` (default `0`,
  disabled) counts failures by peer IP within `--auth-source-window-ms W` (default
  60000). Once a source reaches `N`, further `AUTH`/`HELLO AUTH` credential
  attempts receive `ERR too many authentication attempts, try later` until the
  window expires; a successful authentication resets that tracked source. The
  table is process-local and bounded. When full, new/untracked sources share a
  separate overflow budget instead of evicting tracked peer state; a successful
  authentication from an overflow source cannot individually reset that shared
  budget, which expires with its window. Shared NAT addresses and overflow peers
  can throttle each other. `--auth-global-limit N` (default `0`, disabled)
  additionally counts failed credentials across all peers in this process and
  locks all new credential attempts for the same window when `N` is reached.
  Successful authentication does not reset this shared budget. It is a
  process-local aggregate, not a cross-process/distributed limit; it can deny
  legitimate users after an attacker exhausts it. There is no delay or username
  lockout. Use network policy/firewalling and strong, unique passwords; do not
  expose the port directly to untrusted clients.
- **Live revocation is explicit.** Rotating or disabling the password affects new
  authentication decisions; a connection that already authenticated remains
  authenticated, as in Redis. The supported revocation procedure is
  `CLIENT KILL USER <name>` from an administrative connection: it closes every
  matching normal/pub/sub session, returns the number closed, and skips the
  caller by default (`SKIPME no` can include it). `CLIENT KILL ID`, `ADDR`,
  `LADDR`, `MAXAGE` and `TYPE` remain available for narrower or legacy clients.
  There is no automatic deauthentication when `CONFIG SET requirepass`,
  `ACL SETUSER`, or `ACL LOAD` changes a credential, so rotate-then-revoke is an
  operator step. `tests/acl.rs::client_kill_user_revokes_live_sessions` exercises
  the close/retain/reconnect path. Existing unauthenticated connections begin
  receiving `NOAUTH` immediately after a password is set.
- **Process arguments are visible.** `--requirepass VALUE` can appear in process
  listings and shell history. Prefer `--requirepass-file` or `--aclfile` with a
  mode-`0400` secret mount. Files are read at startup and by explicit `ACL LOAD`;
  later password rotation uses authenticated `CONFIG SET`, and user changes use
  `ACL SETUSER` plus `ACL SAVE`.
## Security acceptance gate for new features

Scripting/functions, replication, failover/Sentinel, cluster routing and loadable
modules are absent. They must not be added as "just another command": each one
changes the trust boundary, and the profile must gate it on all of the following
before it can be reachable:

1. **Threat model** — who supplies the code or peer input, what keys/commands it
   can reach, what network identity it trusts, and what it can do on failure.
2. **Authentication/authorization path** — the ACL category, key/channel patterns
   and connection state that gate it, including for peers and replicas.
3. **Resource bounds** — CPU/instruction, memory, frame size, fan-out and time
   limits with explicit errors instead of unbounded work.
4. **Negative tests** — unauthorized peer/script, hostile or malformed input,
   oversized payloads, partial failure and revocation while running.
5. **Audit and observability** — denial/acceptance events and counters that do
   not leak secrets.
6. **Independent review** where the profile requires it, plus the matching
   profile/checklist update.

`tests/security_gate.rs` is the tripwire for the current absence: in default,
fixed-one and four-worker modes it requires every execution
(`EVAL`/`EVALSHA`/`EVAL_RO`/`FCALL`/`FCALL_RO`, `SCRIPT LOAD`, `FUNCTION LOAD`),
replication (`REPLICAOF`/`SLAVEOF`/`REPLCONF`/`PSYNC`/`SYNC`/`FAILOVER`), cluster
(`SLOTS`/`NODES`) and module (`LOAD`) entry point to fail explicitly, with
`ROLE` reporting a standalone master and no state change. If one of those starts
succeeding, the test fails and the gate above must be satisfied first.

## Secret sources, permissions and rotation

Every secret this server reads has one source flag, one permission rule under
`--secure-secret-files`, and one rotation path. The same validator runs for
startup and `--check-config`, so preflight and production agree.

| Secret | Source | Strict-mode rule | Rotation |
| --- | --- | --- | --- |
| Default-user password | `--requirepass` (process arguments) or `--requirepass-file` | File must be regular with no group/other bits; an inline `--requirepass` must be at least 16 bytes | Restart with the new value; rotate-then-revoke per the guidance below |
| ACL users and hashes | `--aclfile` | Regular file, owner-only | `ACL LOAD` (parsed fully before publishing) or restart; failed load keeps the previous rules |
| Journal authentication key | `--aof-hmac-key-file` | Regular file, owner-only | Restart with the new key; migration is explicit, never implicit |
| Audit per-line HMAC key | `--audit-hmac-key-file` | Regular file, owner-only in both server and collector | Restart; collector keeps the previous key available while old lines are verified |
| Audit keyring | `--audit-hmac-keyring-file` | Regular file, owner-only; 16..=1024 bytes per key, at most eight keys | Replace the file and restart/reload the collector: the first key signs, all listed keys verify, so rotate by prepending the new key and dropping the retired one later |
| TLS private key (default, SNI) | `--tls-key-file`, `--tls-sni-key-file` | Regular file, owner-only | Atomic TLS generation reload: an invalid key rejects the whole generation and keeps the previous one |
| Client-certificate ACL map | `--tls-client-cert-users-file` | Regular file, owner-only | Reloads with the TLS generation; invalid or unknown ACL mappings retain the last good generation |
| Rollback anchor | `--aof-anchor-file` | Regular file, owner-only | Created by `rillcache-backup anchor`; rotation is a backup-and-replace operation |

Inline `--requirepass` and `--requirepass-file` content appear in process
arguments or on disk; prefer the file form with owner-only permissions and a
256-bit random secret. `--secure-secret-files` enforces mode bits and a minimum
length, not owner identity, entropy, secret-manager custody or automatic
rotation of every deployment. ACL password hashes are Redis-compatible unsalted
SHA-256, so an exposed ACL file is offline-bruteforceable; protect it as a
secret and prefer long random passwords.

`tests/secret_sources.rs` drives the real binary through
`--check-config --secure-secret-files` for the password file, ACL file, journal
key and TLS key: owner-only files are accepted, group/other-readable files are
refused, permissive mode is unchanged, and a short inline `--requirepass` is
rejected while a 16-byte one is accepted.

- **Owner-only secret files.** `--secure-secret-files` (Unix) refuses to start
  when a configured `--requirepass-file`, `--aclfile`, audit HMAC/keyring,
  AOF HMAC or TLS private-key path is not a regular file with no group/other
  permission bits, and requires a `requirepass` of at least 16 bytes. Use
  `chmod 600` or stricter. The flag is off by default for command-line
  compatibility; a production deployment should enable it. Generate passwords
  and HMAC keys with at least 32 random bytes (256 bits) from a CSPRNG, supply them through files or a secret mount rather than argv, and
  rotate with `CONFIG SET requirepass`/`ACL SETUSER` followed by an explicit
  `CLIENT KILL USER` revocation. This is permission validation, not a secret
  manager or automatic short-secret detection.
- **Credential persistence is separate from data persistence.** The password is
  not written to disk and does not survive a restart unless the deployment
  supplies the flag/file again. Data persistence is opt-in append-only journaling;
  replication is not implemented.
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
keyspace-wide all-key restrictions and selector scope, atomic failed `SETUSER`,
ACL-file hash/rule round-trips, exact Redis-compatible
errors, `HELLO` atomicity, `NOAUTH` before data/admin access, unauthenticated
`QUIT`, and `CONFIG` redaction/rotation. Wire tests run in thread-per-client,
one-worker and four-worker modes, including ACL-file startup failures, restricted
users, `ACL SAVE/LOAD`, deletion/deactivation of live users, pipelined
rejection/authentication/order and a full runtime rotation cycle.

References: [Redis AUTH](https://redis.io/docs/latest/commands/auth/),
[Redis security](https://redis.io/docs/latest/operate/oss_and_stack/management/security/),
[Redis TLS](https://redis.io/docs/latest/operate/oss_and_stack/management/security/encryption/).
