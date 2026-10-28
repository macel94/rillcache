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
  retain the missing brute-force, audit, native-TLS, durability and
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
  Documentation shall retain the missing subcommand-specific categories,
  persistent audit, rate limiting, native TLS, key-spec-blind commands and
  process-level tenant-isolation boundaries.

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
  the absence of persistent/replicated/centralized audit, rate limiting, token/
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
  categories, rate limiting, persistent audit and native TLS.

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
  subcommand-category, dynamic-key-sandbox, persistent-audit, rate-limiting and
  native-TLS boundaries.

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
  retain the absence of subcommand-specific categories, wildcards in the first
  argument and nested selectors.

## TLS 1.3 sidecar (S101)

- **R374 (event-driven):** The Go standard-library sidecar shall terminate TLS
  only at version 1.3, reject TLS 1.2 and older handshakes, and proxy RESP bytes
  bidirectionally to the configured plaintext backend. It shall not offer HTTP/2,
  HTTP/3 or any ALPN protocol because Redis clients speak RESP, not HTTP.
- **R375 (state-driven):** While a certificate and private key are configured, the
  sidecar shall serve TLS with that chain; an optional client-CA file shall require
  and verify client certificates. It shall enforce positive connection, handshake
  and backend-dial bounds, close excess connections, and handle SIGINT/SIGTERM by
  stopping accepts, draining active proxies up to the shutdown deadline, then
  force-closing leftovers.
- **R376 (unwanted behavior):** Missing/invalid certificate material, invalid CA
  bundles, non-positive limits or invalid durations shall fail startup with a
  bounded error. The sidecar shall use no OpenSSL, cgo or external Go modules, and
  the Rust server dependency/safety policy shall remain unchanged.
- **R377 (ubiquitous):** Tests shall cover a TLS 1.3 RESP echo, TLS 1.2 rejection,
  absence of HTTP ALPN, required client-certificate authentication, configuration
  validation and graceful shutdown documentation. Documentation shall retain the
  absence of native Rust TLS, SNI/hot reload, backend authentication, proxy-aware
  client addresses and any durability/HA claim.
