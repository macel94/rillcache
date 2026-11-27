# Startup configuration

Rillcache starts from command-line flags. `--config PATH` (or `-c PATH`) also
reads a bounded startup file, which is useful for containers and service
managers that prefer an immutable file over a long argument list.

This is a **Rillcache startup file**, not a complete `redis.conf`
implementation. Unknown directives, multiple values for one directive,
includes, `save` policies, replication, modules, `daemonize` and the rest of the
upstream configuration language fail closed before the listener binds. A future
profile decision can extend the schema; silently ignoring a directive is not
allowed.

## Syntax

- One directive and one value per line. To declare the startup file schema
  explicitly, put `config-version 1` as the first directive after any blank
  lines and comments. This is file metadata, not a CLI option. A later or
  repeated marker or an unsupported version rejects the file. Files without
  the marker retain the original version 1 syntax. A marker alone is not a
  configuration file; include at least one startup setting.
- Blank lines and lines whose first unquoted character is `#` are comments.
- `directive value` and `directive=value` are both accepted.
- Directive names are the long option names without `--`; `_` and `-` are
  equivalent. For example, `max-clients` and `max_clients` select the same
  setting.
- `maxmemory` is an alias for `max-data-bytes`, and `maxclients` is an alias for
  `max-clients`. At runtime, exact `CONFIG GET max-data-bytes` returns the
  current logical payload budget under `maxmemory`. The startup name is not
  listed by wildcard queries; use `CONFIG SET maxmemory` to change the budget.
- `auth-failure-limit`, `auth-source-limit`, `auth-global-limit` and
  `auth-source-window-ms` configure the bounded authentication budgets described
  in [security](security.md); the global budget is process-local and disabled by
  default.
- Values may be unquoted (one token), double-quoted with `\"`, `\\`, `\n`, `\r`
  and `\t` escapes, or single-quoted literally.
- `secure-secret-files yes` opts into owner-only secret-file checks on Unix;
  `secure-secret-files no` disables that optional policy (the default). Both
  values follow file/CLI precedence. On the CLI use `--secure-secret-files=yes`
  or `--secure-secret-files=no`; the existing bare flag means `yes`.
- `aof-hmac-migrate` accepts `yes` (request migration) or `no` (no request;
  does not cancel an earlier request).
- `aof-anchor-file` names the read-only external rollback anchor created by
  `rillcache-backup anchor`; it requires `appendonly yes` and
  `aof-hmac-key-file`, and a start fails closed when the journal is shorter than
  the anchored boundary or its chain tag differs. See
  [persistence](persistence.md#rollback-and-truncation-anchor).
- The file must be valid UTF-8 and at most 1 MiB. The reader stops after at
  most 1 MiB plus one byte and rejects oversized input before parsing any
  directives. All file arguments in one invocation share a 4 MiB total budget;
  repeated paths count each time. Each read stops after at most the smaller of
  the per-file limit and remaining total budget, plus one overflow byte. These
  are byte limits, not I/O timeouts. An empty file is an error.

```conf
# /etc/rillcache/rillcache.conf
config-version 1
bind 127.0.0.1:6379
maxclients 1024
max-data-bytes 268435456
appendonly yes
appendfilename appendonly.aof
appendfsync everysec
dir /var/lib/rillcache
requirepass-file /run/secrets/rillcache-password
```

```sh
rillcache --config /etc/rillcache/rillcache.conf --max-clients 2048
```

## Defaults and mutability

The enforced defaults below are the same values `Config::default()` uses; a test
(`tests/config_schema.rs`) fails when this table and the code drift apart. The
full directive list, including TLS, audit, journal-key and healthcheck options,
is in `rillcache --help`.

| Directive | Default | Runtime mutability |
| --- | --- | --- |
| `max-clients` | `128` | mutable (`CONFIG SET maxclients`) |
| `max-data-bytes` | `67108864` (64 MiB) | mutable (`CONFIG SET maxmemory`) |
| `memory-bound-bytes` | unset | startup-only |
| `memory-bound-headroom-bytes` | unset | startup-only |
| `appendfsync` | `everysec` | mutable (`CONFIG SET appendfsync`) |
| `max-bulk-bytes` | `1048576` (1 MiB) | startup-only |
| `max-frame-bytes` | `2097152` (2 MiB) | startup-only |
| `idle-timeout-ms` | `30000` | startup-only |
| `frame-assembly-timeout-ms` | `60000` | startup-only |
| `shutdown-deadline-ms` | `25000` | startup-only |
| `auth-failure-limit` | `10` | startup-only |
| `auth-source-limit` | `0` (off) | startup-only |
| `auth-global-limit` | `0` (off) | startup-only |
| `auth-source-window-ms` | `60000` | startup-only |
| `appendonly` | `no` | startup-only |
| `storage-owners` | one owner | startup-only |

The optional resident-memory bound is a pair: `memory-bound-bytes` and
`memory-bound-headroom-bytes` must be given together, the headroom must be
positive and smaller than the bound, and the bound must fit inside a readable
cgroup limit. Startup fails closed otherwise and the derived payload budget caps
`max-data-bytes`; see
[the memory bound](production-profile.md#resident-memory-bound-and-headroom-guidance).

Mutability here means "changeable at runtime through `CONFIG SET`". That
command validates every pair before publishing and the documented mutable subset
is listed in [the profile](production-profile.md#runtime-configuration-changes);
all other directives are startup-only and rejected at runtime. The schema is
versioned by `config-version 1`, validated before the listener binds, and
precedence is last-wins in argument order (see below).

## Order and precedence

Directives are processed at the position of the `--config` flag. Later CLI
options therefore override earlier file directives, and a file listed after a
CLI option overrides that option. Multiple `--config` files are applied in the
order written. Each file is checked for its own version marker, independently
of the other files. Duplicate settings follow the same last-value-wins rule.

All values pass through the normal startup validation and bind checks after the
full argument list is assembled. For example, the effective `bind` and
`tls-bind` must not both name the same fixed address, but a later override of
either can resolve a conflict. A missing, unreadable, oversized, non-UTF-8,
unknown or malformed file fails startup with a line-numbered error before the
listener binds.

`--config`, `--help`, `--version` and `--healthcheck` are not allowed inside a
file. `CONFIG REWRITE`, live file reload, includes, multi-value directives and
the upstream `redis.conf` schema are not implemented.

## Validate before startup

```sh
rillcache --check-config --config /etc/rillcache/rillcache.conf --max-clients 2048
```

Put `--check-config` first and then pass the same startup arguments you intend
to use. It checks the merged options with the same startup option validator,
including Unix secret-file permissions when `--secure-secret-files` is set.
It reads the configuration and any configured password file but does not bind
listeners, start workers or open journal or audit files. It does not validate
ACL, TLS or HMAC file contents, port availability, or startup I/O. Exit 0 means
the option check passed; exit 2 means invalid startup options. Failure output
omits argument values and file paths so it can be retained without logging
credentials. This does not change the configuration of a running server.

## Secrets

`requirepass` stores the password in the file and in process arguments if used
on the command line. Prefer `requirepass-file` with restrictive file
permissions, or an ACL file. Configuration errors name the line and directive,
not the supplied value.

Startup configuration changes require a restart in this phase. For runtime
`CONFIG SET`, startup-only journal options reject the entire command. A failed
[`appendfsync` policy transition](persistence.md#runtime-configuration-visibility)
also leaves the other settings in that command unchanged. This is not live
configuration-file reload or a guarantee of atomic visibility to concurrent
clients. Further work is tracked in the [production readiness gaps](production-readiness-gaps.md).
