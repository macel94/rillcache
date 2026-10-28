# Command and server discovery (incremental scope)

Rillcache implements a bounded RESP2 subset of Redis discovery. These commands
are explicitly registered alongside their implementations; this module does not
claim complete Redis command metadata or advertise commands that are not present
in this worktree's Store dispatcher.

## Supported behavior

- `COMMAND` returns metadata for each registered command.
- `COMMAND COUNT` returns the registered count.
- `COMMAND INFO [name ...]` returns all metadata when no names are supplied, or
  one metadata record per requested name; matching is ASCII case-insensitive and
  unknown names have null metadata.
- `COMMAND LIST` returns registered lower-case command names.
- `COMMAND LIST FILTERBY (PATTERN glob | ACLCAT category | MODULE name)` filters
  the same registry. PATTERN uses the server's byte-oriented glob matcher, ACLCAT
  accepts `@`-prefixed or bare categories and returns an empty list for unknown
  categories, and MODULE returns an empty list because no modules are loadable.
  Missing/extra/duplicate filters or unknown filter kinds return a syntax error.
- `COMMAND GETKEYS <command> [args ...]` returns the declared key arguments
  using the same extraction as ACL key-pattern enforcement, including counted
  key lists, STORE destinations and legacy GEORADIUS destinations. Invalid
  commands/arity, no-key commands and unextractable arguments return bounded
  errors.
- `COMMAND GETKEYSANDFLAGS <command> [args ...]` returns two-element
  `[key, flags]` entries using the same command resolution, arity validation and
  key extraction. Each flags array contains one primary access class: `RO` for
  read-only keys and `RW` for writable or read-modify-write keys, preserving
  source/destination ordering for commands such as `SORT ... STORE`, `COPY` and
  `GEORADIUS ... STORE`. Errors match `COMMAND GETKEYS`.
- `INFO [SERVER|MEMORY|KEYSPACE|DEFAULT|ALL]` returns a bounded RESP2 bulk string.
  With no section, the default output includes server, memory and keyspace.
- INFO reports `rillcache_version` from the package version and `rillcache_*`
  payload accounting/limit fields. Payload scope is key plus value bytes, not
  process RSS or allocator usage. Keyspace count fields use logically live keys
  at the store's current clock snapshot; aggregate TTL statistics are omitted
  because they are not maintained.

COMMAND metadata uses the RESP2 six-field legacy shape: bulk-string name,
integer arity, array of simple-string flags, and integer first key, last key and
key step. It is not the modern extended Redis metadata
schema. The values describe this implementation's command interfaces, not
execution-policy claims beyond the listed flags. In particular, no Redis release,
allocator, RSS, uptime, persistence, or memory-total values are fabricated.

Unsupported COMMAND subcommands, malformed/extra LIST filters and unsupported
INFO sections return bounded errors that do not echo user-supplied bytes. All generated output is bounded by the fixed registry/section set, aside
from the protocol encoder's standard bounded RESP framing.

## EARS acceptance criteria

- **When** a supported COMMAND form is received, **then** the server shall return
  its registered command count/list/metadata with case-insensitive names and
  correct basic arity, flags and key positions.
- **When** COMMAND GETKEYSANDFLAGS resolves a command, **then** each declared key
  shall be paired with exactly one `RO` or `RW` primary access flag in the same
  order as COMMAND GETKEYS, and invalid commands, arity or unextractable keys
  shall return the bounded GETKEYS errors.
- **When** COMMAND LIST FILTERBY is supplied, **then** PATTERN/ACLCAT/MODULE
  filtering shall return only matching registry names; MODULE and unknown ACL
  categories shall be empty, and malformed filters shall fail without reflection.
- **When** COMMAND INFO names an unknown command, **then** that element shall be
  null metadata without reflecting the supplied name.
- **When** a supported INFO section is requested, **then** the server shall return
  only its truthful version, payload-accounting and live-keyspace scope.
- **If** a subcommand, filter, section, or argument count is unsupported, **then**
  the server shall return an error and keep the connection usable without
  reflecting arbitrary bytes.
- **When** the same wire requests run in managed per-client, one-worker and
  four-worker modes, **then** replies shall preserve RESP2 framing and order.

## Registry module layout and ownership

Command metadata lives under `src/engine/discovery/`, with one file per feature
family (`strings.rs`, `hashes.rs`, `lists.rs`, `sets.rs`, `zsets.rs`,
`streams.rs`, `pubsub.rs`, `keyspace.rs`, `connections.rs`, `server.rs` and the
smaller `geo`, `bitmaps`, `hyperloglog` and `transactions` families). `mod.rs`
owns only the `Command` shape, the `GROUPS` list, the one-time flattened registry
and discovery behavior; each family file owns just its own metadata entries.

To register a command, add it to the family file that owns the feature (or add a
new family module and list its slice in `GROUPS`). Do not add entries to a
shared monolithic array. The registry is flattened and strictly name-sorted once
on first use, so parallel feature work only edits its own family file and never
reorders a shared table. `command_count`, command-at/index lookups, category
filters and `COMMAND`/`INFO` output all read that single sorted registry.

## Compatibility limits / coordinator integration

The registered command list includes implemented Store commands, including typed
collections, keyspace operations, MGET/MSET/MSETNX/MSETEX and conditional string
commands (DIGEST/DELEX/DELIFEQ). CONFIG metadata covers collection encoding-threshold
GET/SET forms and RESETSTAT; other runtime settings are unsupported. Every
advertised name is wired to this worktree's Store dispatcher; unimplemented Redis
commands remain unadvertised. The HELLO/CLIENT worker owns
connection-local state; this module does not implement or advertise it. Detailed
key specifications beyond the coarse `RO`/`RW` access class, granular
`GETKEYSANDFLAGS` sub-flags (for example `OW`, `RM`, `access`, `update`, `insert`,
`delete` and `variable_flags`), subcommand records and Redis extended metadata are
not supported. No network-facing production-readiness or
full Redis compatibility claim follows from this feature.
