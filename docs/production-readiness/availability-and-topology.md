# Availability and topology

Part of the [production-readiness plan](../production-readiness.md). This file owns
the capability row so independent feature increments edit only their own area.

| Capability | Current boundary / evidence required before closing |
| --- | --- |
| Availability and topology | Standalone status only: `ROLE` and `INFO replication` truthfully report `master`, zero replicas and offset 0 without a replid or replica rows; `REPLICAOF`/`SLAVEOF NO ONE` succeed as no-op promotion while host/port forms, `PSYNC`, `SYNC`, `REPLCONF` and `FAILOVER` return explicit replication-unsupported errors (S92). `MOVE`/`SWAPDB` explicitly report that only database 0 exists. Still required: replication/PSYNC, failover/Sentinel, Redis Cluster slots/redirections/migration, partition/rejoin/recovery tests and mixed-version interoperability. Multiple independent servers do not imply replication or clustering. |
