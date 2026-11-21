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
  `max-clients`.
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
- The file must be valid UTF-8 and at most 1 MiB. The reader stops after at
  most 1 MiB plus one byte and rejects oversized input before parsing any
  directives. This is a per-file byte limit, not an I/O timeout. An empty file
  is an error.

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

Startup configuration changes require a restart in this phase; atomic reload and
rollback are tracked in [production readiness gaps](production-readiness-gaps.md).
