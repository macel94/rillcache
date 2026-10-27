# Replication status: standalone only

Rillcache does not implement replication, PSYNC, replicas, Sentinel or cluster
failover. `ROLE` and `INFO replication` exist so clients and operators can read
the truthful standalone state instead of receiving an unknown command or a
fabricated topology.

```text
ROLE
1) "master"
2) (integer) 0
3) (empty array)
```

`INFO replication` (also included in `INFO all`) reports:

```text
# Replication
role:master
connected_slaves:0
master_failover_state:no-failover
master_repl_offset:0
```

The offset is zero because no replication stream exists. No `master_replid`,
`slaveN`, `master_link_status` or backlog fields are reported; they would be
inventions. `INFO` without arguments keeps its existing sections and does not
add a replication block.

`ROLE` is classified `@admin`, `@fast` and `@dangerous` like Redis, and rejects
extra arguments with the normal arity error. This is status reporting only: it
closes no replication, failover, split-brain, backup or durability gate. See
[production readiness](production-readiness.md).
