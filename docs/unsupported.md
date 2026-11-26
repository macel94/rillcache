# Cluster, module and replication boundaries

Rillcache is a standalone server. It does not implement Redis Cluster routing,
replicas or loadable modules, and it does not pretend those features exist.

## CLUSTER

`CLUSTER <subcommand>` returns:

```text
-ERR This instance has cluster support disabled
```

This applies to `INFO`, `MYID`, `SLOTS`, `SHARDS`, `NODES`, `KEYSLOT` and the
other cluster subcommands. `CLUSTER HELP` returns one bounded explanatory row and
a bare `CLUSTER` returns the normal arity error. No slot table, node identity or
migration state is fabricated, and keys are never redirected with `MOVED`/`ASK`.

## MODULE

- `MODULE LIST` returns an empty array: this server has no loadable modules.
- `MODULE HELP` returns one bounded explanatory row.
- `MODULE LOAD`, `MODULE LOADEX` and `MODULE UNLOAD` return an explicit
  unsupported error; module code is never executed.
- Unknown subcommands return a bounded `unknown MODULE subcommand` error.

## WAIT and cluster connection commands

`WAIT numreplicas timeout` validates both integer arguments and returns `0`
without waiting because no replicas are configured; a negative timeout returns
the Redis timeout error. `READONLY`, `READWRITE` and `ASKING` return the same
cluster-disabled error as `CLUSTER`, since this server has no cluster read path.

These replies are explicit capability boundaries, not silent no-ops. Enabling
cluster routing, replication or module loading requires separate implementation,
configuration and validation work and is not implied by this surface.

## Serialization, migration and admin exclusions

The following surfaces return a named capability error instead of pretending to
work or falling through to an unqualified unknown-command reply:

- `DUMP`, `RESTORE` and `MIGRATE` report that the private append-only journal and
  the offline `rillcache-backup` tool are the supported durability path; there is
  no RDB payload or migration stream.
- `COMMAND DOCS` points at `COMMAND INFO`/`COMMAND LIST`; the full documentation
  payload is not implemented.
- `CONFIG REWRITE` reports that startup configuration is read-only; there is no
  generated `redis.conf`.

Arity is validated before each capability error. These exclusions are part of
the compatibility profile boundary; they are not implementations and do not
close the corresponding production-readiness items.

## Command inventory and absent names

[The command matrix](production-readiness/command-matrix.md) enumerates every
command and subcommand name reported by the pinned Redis and Valkey references
and records what this server does with it. A command name that is not returned
by `COMMAND` is absent from the profile; the head answers it with the standard
`ERR unknown command` (for example `DEBUG`, `LOLWUT`, the Redis 8 vector-set and
array commands, and the Valkey 9 `BACKUP`, `COMMANDLOG`, `HOTKEYS`, `HIMPORT`
and `TRIMSLOTS` extensions) or with one of the named capability errors above.
It never accepts an absent top-level command silently. `tests/command_matrix.rs`
fails if the committed matrix and the tested head disagree, if an absent command
answers successfully, or if a named capability error is not recorded here or in
the [production profile](production-profile.md).

The same matrix lists the subcommands of containers that do exist. Those rows
record the container's bounded answer or its rejection; they are not a claim that
every option is implemented, and the area documents linked from the
[documentation index](README.md) remain authoritative for option-level support.
