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

- One directive and one value per line.
- Blank lines and lines whose first unquoted character is `#` are comments.
- `directive value` and `directive=value` are both accepted.
- Directive names are the long option names without `--`; `_` and `-` are
  equivalent. For example, `max-clients` and `max_clients` select the same
  setting.
- `maxmemory` is an alias for `max-data-bytes`, and `maxclients` is an alias for
  `max-clients`.
- Values may be unquoted (one token), double-quoted with `\"`, `\\`, `\n`, `\r`
  and `\t` escapes, or single-quoted literally.
- `aof-hmac-migrate` is the one boolean directive and accepts `yes` or `no`.
- The file must be valid UTF-8 and at most 1 MiB. An empty file is an error.

```conf
# /etc/rillcache/rillcache.conf
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
order written. Duplicate directives follow the same last-value-wins rule.

All values pass through the normal startup validation and bind checks after the
full argument list is assembled. A missing, unreadable, oversized, non-UTF-8,
unknown or malformed file fails startup with a line-numbered error before the
listener binds.

`--config`, `--help`, `--version` and `--healthcheck` are not allowed inside a
file. `CONFIG REWRITE`, live file reload, includes, multi-value directives and
the upstream `redis.conf` schema are not implemented.

## Secrets

`requirepass` stores the password in the file and in process arguments if used
on the command line. Prefer `requirepass-file` with restrictive file
permissions, or an ACL file. Configuration errors name the line and directive,
not the supplied value.

Startup configuration changes require a restart in this phase; atomic reload and
rollback are tracked in [production readiness gaps](production-readiness-gaps.md).
