# Availability and topology

Open items are tracked in the canonical [production-readiness gaps](../production-readiness-gaps.md); this file records current behavior and boundaries, not a second checklist.

Part of the [production-readiness plan](../production-readiness.md). This file owns
the capability row so independent feature increments edit only their own area.

| Capability | Current boundary / evidence required before closing |
| --- | --- |
| Availability and topology | Standalone status only: `ROLE` and `INFO replication` truthfully report `master`, zero replicas and offset 0 without a replid or replica rows; `REPLICAOF`/`SLAVEOF NO ONE` succeed as no-op promotion while host/port forms, `PSYNC`, `SYNC`, `REPLCONF` and `FAILOVER` return explicit replication-unsupported errors (S92). `MOVE`/`SWAPDB` explicitly report that only database 0 exists. Open work (replication, failover, cluster, partition/rejoin and mixed-version evidence) is tracked in the canonical gaps file (A1–A5). Multiple independent servers do not imply replication or clustering. |
