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

## Serialization, migration, HLL diagnostics and admin exclusions

The following surfaces return a named capability error instead of pretending to
work or falling through to an unqualified unknown-command reply:

- `DUMP`, `RESTORE` and `MIGRATE` report that the private append-only journal and
  the offline `rillcache-backup` tool are the supported durability path; there is
  no RDB payload or migration stream.
- `PFDEBUG` and `PFSELFTEST` report that HyperLogLog uses this project's private
  bounded representation and does not expose Redis diagnostic payloads.
- `COMMAND DOCS` points at `COMMAND INFO`/`COMMAND LIST`; the full documentation
  payload is not implemented.
- `CONFIG REWRITE` reports that startup configuration is read-only; there is no
  generated `redis.conf`.
- `MEMORY MALLOC-STATS` and `MEMORY PURGE` report that memory reporting covers
  payload accounting and process RSS, not allocator internals.

Arity is validated before each capability error. These exclusions are part of
the compatibility profile boundary; they are not implementations and do not
close the corresponding production-readiness items.
