# Security requirements

Part of the [requirements and incremental delivery index](../requirements.md). Requirements are preserved verbatim from the former monolithic EARS sections; add new requirements only in the matching area file.

## Password authentication (S27)

- **R109 (state-driven):** While a default-user password is configured, a
  connection shall remain unauthenticated until a valid `AUTH`/`HELLO AUTH`, and
  shall receive `NOAUTH Authentication required.` before data, transaction,
  discovery or administrative work for every other command; only `AUTH`, `HELLO`
  and `QUIT` may execute. With no password configured, data commands shall remain
  available and `AUTH` shall return Redis's explicit no-password error.
- **R110 (event-driven):** When `AUTH [username] password` or
  `HELLO <2|3> AUTH username password [SETNAME name]` is received, the server shall
  accept only the `default` user and a constant-time SHA-256 match, returning
  `+OK`/HELLO information on success or a generic `WRONGPASS` error on failure.
  A failed HELLO shall leave protocol, metadata and authentication state unchanged.
- **R111 (event-driven):** When authenticated `CONFIG SET requirepass <value>` is
  received, the server shall atomically set a new digest or disable authentication
  for an empty value, leaving thresholds and credential unchanged if any pair is
  invalid; `CONFIG GET requirepass` shall report unset/redacted state without
  exposing the secret. Startup flag/file parsing shall fail closed on empty or
  oversized files and never log the password.
- **R112 (ubiquitous):** Authentication tests shall run offline in all execution
  modes, cover pipelined rejection/order, runtime rotation, `QUIT`, RESP2/RESP3
  HELLO, secret redaction and disabled/rotated credentials. Documentation shall
  retain the missing global/cross-process rate limiting, centralized-audit, TLS-hardening, durability and
  live-deauthentication boundaries; a single password is not full Redis security.

## Multi-user ACLs (S28)

- **R113 (event-driven):** When `--aclfile` is configured, the server shall load
  users/rules before accepting clients, require an enabled `default` user, reject
  malformed or missing files at startup, reject combining `--aclfile` with
  `--requirepass`, and preserve the single-password `--requirepass` mode otherwise.
- **R114 (state-driven):** While a connection is authenticated as a user, the
  server shall evaluate ordered command/category rules and declared key positions
  before dispatch/store access and deny with `NOPERM`; disabled/deleted users shall
  fail with `NOAUTH` on the next command. `AUTH`, `HELLO`, `QUIT` and `ACL WHOAMI`
  shall retain their documented exceptions. Denied commands shall not mutate data
  and shall mark an active transaction dirty; `EXEC` shall re-check every queued
  command so later revocation cannot commit it.
- **R115 (event-driven):** When `ACL SETUSER`, `DELUSER`, `WHOAMI`, `USERS`, `LIST`,
  `GETUSER`, `CAT`, `SAVE` or `LOAD` is received with valid permission, the server
  shall apply Redis-shaped user rules atomically, protect `default` from deletion,
  expose hashes instead of plaintext, and save/load a complete ACL file through a
  mode-0600, fsynced temporary-file rename within documented file/user/rule/pattern
  bounds. Invalid modifiers shall not partially change a user.
- **R116 (ubiquitous):** ACL tests shall run offline in all execution modes and cover
  password/category/command/key rules, live revocation, transaction dirtiness,
  startup failures, file hash/rule round-trips and secret-free diagnostics.
  Documentation shall retain the remaining subcommand-specific categories beyond the bounded CLIENT/MEMORY/OBJECT/ACL/COMMAND/XGROUP/XINFO/CONFIG/LATENCY/SLOWLOG/PUBSUB overrides,
  centralized audit shipping, global/cross-process rate limiting, TLS hardening, key-spec-blind
  commands and process-level tenant-isolation boundaries.

## ACL credentials, dry-run and denial log (S41)

- **R161 (event-driven):** When `ACL GENPASS [bits]` receives a valid 1..=1024-bit
  request (default 256), the server shall return lowercase hex sourced from
  `/dev/urandom`; if the source is unavailable or bits are invalid, it shall return
  a bounded error without falling back to weak randomness.
- **R162 (event-driven):** When `ACL DRYRUN username command [arg ...]` is
  received, the server shall evaluate the named enabled user's ordered command,
  key and channel rules without executing the command, mutating state or recording
  a denial, returning `+OK`/`NOPERM` or a visible unknown-user error.
- **R163 (event-driven):** When permission is denied, the server shall record a
  bounded process-local log entry with reason (`noauth`/`command`/`key`/`channel`),
  context (`toplevel`/`multi`), object, username and age; identical tuples shall
  aggregate. `ACL LOG [count]` shall return newest entries as
  `[count, reason, context, object, username, age]` arrays and `ACL LOG RESET`
  shall clear them.
- **R164 (ubiquitous):** GENPASS/DRYRUN/LOG shall require the normal ACL command
  permission and all-mode tests shall cover random output length/hex, invalid bits,
  allowed/denied dry runs, unknown users, no-log DRYRUN behavior, key/channel/
  command/noauth aggregation, reset and bounded counts. Documentation shall retain
  the absence of replicated/centralized audit shipping, global/cross-process rate limiting, token/
  timeout metadata and exhaustive security-event coverage.

## ACL selectors (S86)

- **R328 (event-driven):** `ACL SETUSER` shall accept one or more parenthesized
  selector groups containing command (`+cmd`/`-cmd`), category (`+@cat`/`-@cat`),
  key (`~`) and channel (`&`) rules, plus `allkeys`/`resetkeys`,
  `allchannels`/`resetchannels`. `clearselectors` shall remove all selectors and
  `reset` shall clear them with the rest of the user. Invalid or oversized
  groups shall reject the whole `SETUSER` atomically.
- **R329 (state-driven):** While authorizing a command, the server shall allow it
  when the root rules or any selector permits the command, its declared keys and
  its channel arguments as one set; a selector that permits the command but not
  its keys or channels shall not grant it. Dynamic `SORT` BY/GET keys shall
  require all-keys in the matching set, and denials shall report command, key or
  channel consistently.
- **R330 (event-driven):** `ACL LIST` shall render selectors as parenthesized
  groups and `ACL GETUSER` shall expose a `selectors` array of commands/keys/
  channels maps. `ACL SAVE`/`LOAD` shall round-trip selector groups with spaces
  intact and preserve the documented bounds.
- **R331 (ubiquitous):** Tests shall cover root/selector alternatives, key-scoped
  writes with broad reads, `clearselectors`/`reset`, atomic rejection, LIST/
  GETUSER exposure, ACL-file restart round-trips and every worker mode.
  Documentation shall retain the absence of nested selectors, subcommand
  categories, global/cross-process rate limiting, centralized audit shipping and native TLS.

## ACL read/write key modes (S88)

- **R335 (event-driven):** `ACL SETUSER`, selector groups and ACL files shall
  accept `%R~pattern` (read), `%W~pattern` (write), `%RW~pattern` and legacy
  `~pattern` (read-write) key rules. `allkeys` shall grant read-write `*`;
  `resetkeys` and `reset` shall clear key patterns. Empty patterns, unknown
  `%`-prefixed forms and oversized patterns shall reject the whole update
  atomically.
- **R336 (state-driven):** While authorizing a command, each declared key shall be
  checked against the access required by its Redis key spec: read-only keys need
  a read grant, overwrite/update keys need a write grant, and read-write keys need
  both, which may come from different patterns. Mixed commands shall be checked
  per key (`BITOP` destination versus sources, `COPY` source versus destination,
  `RENAME` source versus destination, and store destinations of `SORT`,
  `GEORADIUS*` and `*STORE`). `SORT ... STORE` and `GEORADIUS ... STORE` shall
  require read on the source and write on the effective last-wins destination;
  `XREAD`/`XREADGROUP` shall check each key in the dynamic `STREAMS` block, and
  `PFCOUNT` shall require both accesses per Redis 7's RW key spec. `SET` with
  `GET` and mutating `BITFIELD` operations shall require both accesses.
- **R337 (event-driven):** `ACL LIST` and `ACL GETUSER` shall render read-only
  patterns as `%R~`, write-only patterns as `%W~` and read-write patterns as `~`.
  Adding read and write grants for the same pattern shall merge into `~`. `ACL
  SAVE`/`LOAD` shall round-trip all modes, and `ACL DRYRUN` shall apply the same
  key-access decisions without executing the command.
- **R338 (ubiquitous):** Tests shall cover read-only, write-only and read-write
  grants, per-key mixed commands, read denial for a write-only credential, write
  denial for a read-only credential, listing, `ACL DRYRUN`, ACL-file restart
  round-trips and every worker mode. Documentation shall retain the
  subcommand-category, dynamic-key-sandbox, centralized-audit, rate-limiting and
  TLS-hardening boundaries.

## First-argument command rules (S102)

- **R378 (event-driven):** When an ACL rule uses `+command|subcommand` or
  `-command|subcommand`, the server shall match the command case-insensitively
  and the literal first argument case-insensitively. A subcommand rule shall not
  grant the bare command without that first argument, and a bare command rule
  shall continue to cover every subcommand unless a later ordered rule overrides
  it.
- **R379 (state-driven):** While authorizing a command, root rules and selector
  rules shall apply the same ordered last-match-wins evaluation for
  first-argument rules. `ACL LIST`, `ACL GETUSER`, `ACL SAVE` and `ACL LOAD` shall
  preserve and round-trip the `|`-qualified rule, including selector groups, and
  `ACL DRYRUN` shall report the same allow/deny decision without executing it.
- **R380 (unwanted behavior):** An empty first argument (`+command|`), an unknown
  command, an oversized rule or a malformed selector shall reject the whole
  update atomically without partially changing the user. Tests shall cover
  allow/deny, ordered overrides, case-insensitivity, selectors, LIST/GETUSER,
  DRYRUN, ACL-file restart round-trips and every worker mode; documentation shall
  retain the remaining subcommand-specific categories beyond the bounded CLIENT/MEMORY/OBJECT/ACL/COMMAND/XGROUP/XINFO/CONFIG/LATENCY/SLOWLOG/PUBSUB overrides, wildcards in the first
  argument and nested selectors.

## Persistent ACL denial audit (S104)

- **R384 (event-driven):** When `--audit-log PATH` is configured, every ACL
  denial (including `NOAUTH`) shall append one bounded line containing a Unix
  millisecond timestamp, reason, context, hex-encoded username and hex-encoded
  object. Binary, empty and newline-containing values shall be hex-safe; the
  line shall contain no plaintext password, key value or reflected request bytes.
  Each line shall be flushed before the denial returns, and the file shall be
  opened before the listener accepts clients.
- **R385 (state-driven):** The file shall be bounded by
  `--audit-log-max-bytes` (1 KiB..=1 GiB, default 8 MiB) with one-generation
  rotation to `<path>.1`; rotation shall preserve the previous generation and
  keep the current line intact. An unopenable path or invalid bound shall fail
  startup. A runtime write failure shall not change the command reply, shall
  increment `audit_log_write_failures` in `INFO stats`, and shall remain visible
  rather than silently claiming a durable trail.
- **R386 (ubiquitous):** `ACL LOG RESET` shall clear only the in-memory
  aggregation; the audit file is append-only and shall append across restart.
  Tests shall cover line shape, hex/binary safety, truncation bounds, one-generation
  rotation, restart appends, startup failure, write-failure accounting and every
  worker mode. Documentation shall retain the absence of fsync-per-line,
  full-chain signing/key-ID/hot rotation, global/cross-process rate limiting and
  centralized aggregation.

## Per-connection failed-auth budget (S105)

- **R387 (event-driven):** When `--auth-failure-limit N` is greater than zero,
  the server shall count consecutive failed `AUTH` and `HELLO ... AUTH`
  credential attempts per connection. On the `N`th failure it shall send the
  standard bounded `WRONGPASS` reply and then close that connection. A successful
  authentication or `RESET` shall clear the counter; arity errors, `NoPassword`
  configuration errors and internal errors shall not consume the budget.
- **R388 (state-driven):** A zero limit shall disable the budget. The counter
  shall be connection-local, so exhausting one connection shall not close or
  throttle another. The server shall not claim a global or cross-process rate limit,
  delay, username lockout or credential-attempt audit beyond the optional local
  denial trail.
- **R389 (ubiquitous):** Tests shall cover the exact limit, reset-on-success,
  `RESET` clearing, `HELLO` sharing the same counter, zero disabling, connection
  isolation, no secret reflection and every worker mode. Documentation shall
  retain the absence of global/cross-process rate limiting, delayed responses,
  account lockout, centralized audit shipping and a complete brute-force
  deployment boundary.

## Per-source failed-auth throttling (S106)

- **R390 (event-driven):** When `--auth-source-limit N` is greater than zero, the
  server shall count failed `AUTH` and `HELLO ... AUTH` credential attempts by
  peer IP within `--auth-source-window-ms W`. When a source reaches `N` failures,
  further credential attempts from that source shall receive the bounded error
  `ERR too many authentication attempts, try later` without evaluating the
  credential until the window expires. A successful authentication shall reset
  that source's counter.
- **R391 (state-driven):** The source table shall be process-local, bounded to a
  fixed maximum number of peer addresses, and shall prune expired entries. A zero
  limit shall disable the feature. The key shall be the peer IP, not the username
  or connection; shared NAT addresses can therefore throttle each other, and the
  feature shall not claim a global, cross-process or username lockout.
- **R392 (ubiquitous):** Tests shall cover same-source lockout across connections,
  lockout expiry, zero disabling, success reset, `HELLO AUTH` sharing the counter,
  distinct-source state in unit tests, bounded table behavior, no secret
  reflection and every worker mode. Documentation shall retain the absence of
  global/cross-process rate limiting, delayed responses, username lockout,
  centralized audit shipping and a complete brute-force deployment boundary.

## Off-host audit shipping (S107)

- **R393 (event-driven):** When `--audit-syslog IP:PORT` is configured, every
  ACL denial shall send the same bounded, redacted line used by the local audit
  file as one UDP datagram to that destination. The sink shall work with or
  without `--audit-log`, shall validate a numeric address with a nonzero port at
  startup, and shall never include a plaintext password, key value or unbounded
  request bytes.
- **R394 (state-driven):** UDP delivery shall be best-effort with a bounded send
  deadline. A send failure shall not change the command reply or block retries;
  it shall increment `audit_syslog_write_failures` in `INFO stats`. The local
  file, when configured, remains the durable evidence path; the sink shall not
  claim reliable, ordered, authenticated or encrypted delivery.
- **R395 (ubiquitous):** Tests shall receive the datagram on a local UDP socket
  and verify exact line shape, hex/redaction, file-independent operation,
  invalid-destination startup failure and every worker mode. Documentation shall
  retain the absence of collector authentication, TLS, retries, ordering,
  backpressure, centralized aggregation, full-chain signing/key-ID/hot rotation and
  delivery guarantees.

## Bounded subcommand ACL categories (S108)

- **R396 (event-driven):** For `CLIENT`, `+@admin` and `+@dangerous` shall match
  only `KILL`, `LIST`, `NO-EVICT`, `PAUSE`, `UNBLOCK` and `UNPAUSE`; connection
  subcommands such as `SETNAME`, `GETNAME`, `ID`, `INFO`, `SETINFO`, `TRACKING`,
  `CACHING`, `REPLY`, `GETREDIR`, `TRACKINGINFO` and `HELP` shall remain governed
  by `+@connection`/`+@slow` and shall be denied by an `@admin`-only grant.
- **R397 (state-driven):** For `MEMORY`, `USAGE` shall match `@read`, while
  `STATS`, `DOCTOR`, `PURGE` and `MALLOC-STATS` shall match `@admin` and not
  `@read`; `@dangerous` shall not grant memory subcommands. For `OBJECT`,
  `ENCODING`, `REFCOUNT`, `IDLETIME`, `FREQ` and `HELP` shall match `@keyspace`;
  the data-inspection subcommands shall match `@read`, while `HELP` shall not;
  `@admin` and `@dangerous` shall not grant object subcommands.
- **R398 (ubiquitous):** Tests shall cover the over-grant and under-grant cases
  above through unit authorization and all-mode wire tests, with unknown
  subcommands falling back to the command-level approximation. Documentation
  shall retain that category listings enumerate the bounded subcommand table,
  that category membership is approximate rather than byte-for-byte Redis
  metadata, and that other command families still use command-level categories.

## Per-line audit HMAC (S109)

- **R399 (event-driven):** When `--audit-hmac-key-file PATH` is configured, the
  server shall load a 16..=1024-byte key before accepting clients, stripping one
  trailing newline. A missing, unreadable, empty, too-short or oversized key shall
  fail startup. The key shall never appear in `Debug`, logs, errors, replies or
  the audit line.
- **R400 (state-driven):** While a key is configured, every denial line written
  to the local file or the UDP sink shall append one space and a 64-character
  lowercase hex HMAC-SHA256 over the exact line body (all preceding fields,
  including the timestamp). Without a key, the five-field line remains unchanged.
  Per-line authentication shall not claim ordering, deletion, replay or
  full-chain protection, key IDs, hot rotation, HSM/KMS custody or authenticated
  collector delivery.
- **R401 (ubiquitous):** Tests shall cover RFC 4231 HMAC-SHA256 vectors, signed
  line verification and tamper detection, five-field compatibility without a key,
  file and UDP shape, missing/short key startup failure, secret absence and every
  worker mode. Documentation shall retain the absence of hot rotation, key IDs, chain
  sequencing, centralized verification, tamper-proof storage and key-custody
  guarantees.

## Bounded UDP audit collector (S110)

- **R402 (event-driven):** The `rillcache-audit-collector` binary shall accept
  numeric `--bind` and nonempty `--output` options, receive UDP datagrams, append
  each nonempty datagram up to `--max-datagram` as one newline-terminated line,
  flush every line, and rotate the output to `<path>.1` at `--max-bytes`
  (1 KiB..=1 GiB). Missing values, invalid addresses, empty output paths and
  out-of-range bounds shall fail before serving.
- **R403 (state-driven):** Delivery shall remain best-effort UDP: empty or
  oversized datagrams shall be counted and dropped without growing the file,
  `--max-datagrams N` shall exit after `N` accepted datagrams (zero runs
  indefinitely), and the collector shall not verify HMACs, authenticate senders,
  encrypt traffic, retry, reorder or deduplicate. The output file shall be created
  mode-0600 on Unix and protected by operator filesystem policy.
- **R404 (ubiquitous):** Tests shall spawn the collector on an ephemeral port,
  send datagrams, verify exact newline-normalized output, invalid configuration
  failure, option bounds and one-generation rotation. Documentation shall retain
  the absence of sender authentication, HMAC verification, TLS, delivery
  guarantees, multi-host federation, backpressure and chain/key management.

## Authenticated UDP collector lines (S111)

- **R405 (event-driven):** When `rillcache-audit-collector` receives
  `--hmac-key-file PATH`, it shall load a 16..=1024-byte shared key before binding,
  stripping one trailing newline, and reject missing/unreadable/empty/short/
  oversized keys at startup. With a key configured, each datagram shall be a
  UTF-8 audit line whose final field is a valid 64-character hex HMAC-SHA256 over
  the exact preceding body; tampered, malformed or wrong-key lines shall be
  dropped and counted. Without a key, lines are stored without verification.
- **R406 (state-driven):** Only verified lines shall increment the accepted count
  and `--max-datagrams`; rejected HMAC lines shall not grow the output. Verification
  shall not authenticate sender identity, encrypt transport, prevent replay,
  deduplicate, reorder or provide multi-host federation. The shared key and
  output-file custody remain operator duties and the key shall never be written
  to the output or diagnostics.
- **R407 (ubiquitous):** Tests shall cover valid, tampered and wrong-key lines,
  unsigned compatibility, missing/short key startup failure, exact output and
  every collector mode. Documentation shall retain the absence of sender
  authentication, TLS, replay protection, full-chain verification, centralized
  key distribution and delivery guarantees.

## Remaining bounded subcommand categories (S112)

- **R408 (event-driven):** For `ACL`, `@admin` and `@dangerous` shall match only
  `DELUSER`, `DRYRUN`, `GETUSER`, `LIST`, `LOAD`, `LOG`, `SAVE`, `SETUSER` and
  `USERS`; `CAT`, `GENPASS`, `HELP` and `WHOAMI` shall not be granted by those
  categories. `WHOAMI` remains available to every authenticated user as before.
- **R409 (state-driven):** All implemented `COMMAND` subcommands shall match
  `@connection` and `@slow`. For `XGROUP`, `@write` shall match
  `CREATE`, `CREATECONSUMER`, `DELCONSUMER`, `DESTROY` and `SETID`, but not
  `HELP`; for `XINFO`, `@read` shall match `STREAM`, `GROUPS` and `CONSUMERS`,
  but not `HELP`. Unexpected categories shall fall back to the command-level
  approximation.
- **R410 (ubiquitous):** Unit and all-mode wire tests shall cover the allowed and
  denied cases above, including `ACL USERS` versus `ACL CAT`/`GENPASS`,
  `COMMAND COUNT`, `XGROUP CREATE` versus `HELP`, and `XINFO STREAM`/`GROUPS`
  versus `HELP`. Documentation shall retain that category listings enumerate the bounded
  subcommand table and that other command families still use the command-level
  approximation.

## Subcommand category listings (S113)

- **R411 (event-driven):** `ACL CAT <category>` and
  `COMMAND LIST FILTERBY ACLCAT <category>` shall enumerate the implemented
  subcommand entries (`command|subcommand`) for the bounded families where a
  subcommand list exists, using the same category overrides as authorization.
  Commands without a known subcommand list shall keep their parent command entry;
  unknown categories shall produce the existing empty/error behavior.
- **R412 (state-driven):** Only implemented subcommands present in the table shall
  be listed, in lower-case `command|subcommand` form, deduplicated and consistent
  with the authorization decision for that category. `@all` shall include all
  listed subcommands. The output shall not invent unsupported Redis subcommands or
  claim byte-for-byte Redis metadata.
- **R413 (ubiquitous):** Tests shall cover `ACL CAT` and `COMMAND LIST FILTERBY
  ACLCAT` for admin/connection/read categories, parent fallback, unknown
  categories and every worker mode. Documentation shall retain the bounded family
  table, command-level fallback for other commands and approximate category
  membership.

## Audit HMAC keyring rotation (S115)

- **R417 (event-driven):** `--audit-hmac-keyring-file PATH` and the collector's
  `--hmac-keyring-file PATH` shall parse a text keyring of hex-encoded keys,
  ignoring blank lines and `#` comments, accepting at most eight unique
  16..=1024-byte keys and ignoring repeated keys even after eight unique keys.
  Reading shall be bounded to 64 KiB of text including comments. Empty, non-hex,
  undersized, oversized and over-limit keyrings shall fail startup; the keyring and single-key options shall
  be mutually exclusive.
- **R418 (state-driven):** The server shall sign each denial with the first
  keyring key; the collector shall accept a line when any key in its keyring
  verifies it. The signed line format shall remain unchanged, so an operator can
  restart the server with the new key first while the collector retains the old
  key until old lines age out. Keys shall never appear in diagnostics or output.
  There is no automatic reload, key ID, chain sequencing or HSM/KMS custody.
- **R419 (ubiquitous):** Tests shall cover keyring parsing/comments/duplicates/
  bounds, server first-key signing, collector any-key verification, mixed
  single-key/keyring rejection, invalid-startup failure and exact output.
  Documentation shall retain the manual restart/retention procedure and the
  absence of full-chain signing, key IDs, hot rotation and centralized custody.

## Native TLS 1.3 with rustls (S101)

- **R374 (event-driven):** The server shall expose an optional native TLS listener
  that accepts TLS 1.3 only, rejects TLS 1.2 and older handshakes, carries RESP
  directly, and offers no HTTP/2, HTTP/3 or ALPN protocol because Redis clients
  speak RESP, not HTTP. A plaintext listener may coexist on a distinct address.
- **R375 (state-driven):** While a certificate and private key are configured, the
  listener shall serve TLS with that chain; an optional client-CA file shall require
  and verify client certificates. It shall enforce positive handshake bounds and
  shared client admission, and join TLS workers on shutdown.
- **R376 (unwanted behavior):** Missing/invalid certificate material, invalid CA
  bundles, a TLS listener on the plaintext address, non-positive handshake
  timeouts or TLS files without a listener shall fail startup with a bounded error.
  Only the reviewed rustls, Tokio and `core_affinity` dependency graphs, with
  required transitive crates, shall be allowed; project code remains unsafe-free
  and FFI-free. Mio is used transitively by Tokio for I/O readiness.
- **R377 (ubiquitous):** Tests shall cover TLS 1.3 RESP in every execution mode,
  plaintext coexistence, TLS 1.2 rejection, absence of HTTP ALPN, required
  client-certificate authentication and invalid configuration. Documentation shall
  retain the absence of SNI/hot reload, OCSP/CRL, client-cert-to-ACL mapping,
  replication TLS and any durability/HA claim.

## Remaining subcommand categories for administration and pub/sub (S117)

- **R423 (event-driven):** For `CONFIG`, `@admin`, `@dangerous` and `@slow` shall
  match the implemented `GET`, `SET` and `RESETSTAT` subcommands, while `@read`,
  `@write`, `@fast`, `@connection`, `@keyspace` and `@pubsub` shall not grant
  them. Unimplemented subcommands shall keep the parent command approximation.
- **R424 (state-driven):** For `LATENCY` and `SLOWLOG`, `@admin` and
  `@dangerous` shall match every implemented subcommand except `HELP`; `HELP`
  shall match `@slow` only among the broad categories. For `PUBSUB`,
  `CHANNELS`, `NUMSUB`, `NUMPAT`, `SHARDCHANNELS` and `SHARDNUMSUB` shall match
  `@pubsub` and `@slow`, but not `@fast`, `@admin`, `@dangerous`, `@read`,
  `@write`, `@connection` or `@keyspace`. Unknown subcommands and unexpected
  categories shall fall back to the command-level approximation.
- **R425 (ubiquitous):** Unit authorization and all-mode wire tests shall cover
  the allowed and denied cases above, including `LATENCY HELP` and
  `SLOWLOG HELP` versus their administrative subcommands and `PUBSUB`
  introspection versus an admin-only grant. `ACL CAT` and
  `COMMAND LIST FILTERBY ACLCAT` shall list the same bounded subcommand entries
  for `admin`, `dangerous`, `slow` and `pubsub`. Documentation shall retain the
  bounded family table and command-level fallback for other families.
