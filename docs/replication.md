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

## A1 profile decision: replication excluded

**Decision (2026-11-27):** replication is **excluded** from the supported
profile. The reference decision is recorded by the protected review path that
merged this section with the evidence below; it is not an implementation claim
and it does not cover failover or cluster scope.

Rationale, in the profile's own terms:

- The data path is single-owner and one journal per process. Replication would
  introduce a second writer's consistency problem, an offsets/backlog model,
  snapshot transfer, partial resynchronization, replica apply and
  acknowledged-write semantics on both ends, plus mixed-version interoperability
  with pinned Redis and Valkey replicas. None of that exists today.
- Shipping a partial replication path would be a compatibility claim without
  two-end evidence, which the readiness policy forbids. The verified recovery
  path is the quiescent offline copy plus the external rollback anchor, and it is
  measured (see [the restore drill](persistence.md#isolated-restore-drill-rporto-and-retention)).
- A deployment that needs read scaling or automatic promotion needs a design
  with pinned two-end evidence, not a best-effort link.

Alternatives considered and rejected for this profile: full+partial
synchronization with a bounded backlog (large surface, unverifiable mixed-version
interop inside the profile); streaming a snapshot to an online replica (requires
a consistent snapshot point and a network protocol that do not exist); and
routing clients to independent primaries with a proxy (would be presented as
replication while serving divergent data).

Revisit when a deployment requires multi-node reads or automatic failover, a
reviewed design covers offsets, backlog, snapshot, partial resync, replica apply
and acknowledgement-loss bounds, and both ends can be pinned for tests.

### What the boundary proves

Every replication entry point fails closed with one explicit error, and no
replica state can be faked:

| Probe | Required result |
| --- | --- |
| `REPLICAOF host port`, `SLAVEOF host port` | replication-unsupported error; connection stays usable |
| `PSYNC replid offset`, `SYNC`, `REPLCONF ...`, `FAILOVER` | replication-unsupported error |
| `SLAVEOF NO ONE` | the declared no-op: `+OK`, no state change |
| Arity errors on `REPLICAOF`/`PSYNC`/`SYNC` | normal wrong-number-of-arguments error |
| `ROLE`, `INFO replication` | `master`, `connected_slaves:0`, `master_repl_offset:0`, no `slaveN`/`master_link_status` fields, before and after local writes |
| `WAIT`, `WAITAOF` | zero acknowledged replicas; `WAITAOF` describes only local journal state |
| Second Rillcache instance told to replicate | stays an independent master; each keyspace keeps only its own keys; no snapshot transfer |

Evidence: `tests/replication_boundary.rs` (all worker modes, every probe, the
no-op, arity, two-instance isolation) and `scripts/upstream/replication_exclusion.py`
(pinned Redis 8.10.2 and Valkey 9.1.2 started with `--replicaof` against the
candidate: `master_link_status:down`, replica `DBSIZE 0`, no `+FULLRESYNC`, no
`+CONTINUE`, candidate still `master` with `connected_slaves:0` and offset `0`).
The pinned-native job is a declared dependency of the required Tcl contexts, so a
regression fails the protected check.

## Single-node availability and data-loss statement

This profile is **single-node**. Availability is the availability of one process
on one host and disk; there is no standby, no automatic promotion and no
split-brain protection beyond the absence of a second writer (the journal's
exclusive ownership lock rejects a second process).

- **Outage on node loss:** the service is unavailable until an operator restores
  it. The supported recovery is the documented quiescent restore: verify an
  archive, restore into an isolated instance, validate data/TTLs/accounting, then
  switch traffic. The restore drill measures that path per archive size; the
  deployment's own RTO must be re-measured on its hardware.
- **Recovery point:** an acknowledged write is in the journal before its reply
  (`appendfsync always`), so a process crash loses no acknowledged write. Loss of
  the node or disk loses every write after the newest **off-host verified**
  archive — the archive is the recovery point, and taking one requires a freeze.
- **In-flight and client-visible effects:** connections drop, unacknowledged
  commands may or may not be present after restart, and clients must reconnect
  and retry according to their own contract. There is no read-your-write promise
  across a failed node.
- **Prohibited deployments:** two independent instances behind one address,
  proxy or load balancer are not replication and will serve divergent data; do
  not present them as a cluster or as high availability. A shared or network
  filesystem is unsupported for the same reason (no distributed lease).

Operational requirements that follow: liveness/readiness probes
(`--healthcheck`), monitoring of `aof_last_write_status`/`aof_last_bgrewrite_status`,
off-host retention of verified archives and their rollback anchors, and a tested
restore on the deployment's own hardware. Failover (A2) and cluster routing (A3
is excluded by its own decision) are separate items and are not implied by this
statement.


`ROLE` is classified `@admin`, `@fast` and `@dangerous` like Redis, and rejects
extra arguments with the normal arity error. This is status reporting only: it
closes no replication, failover, split-brain, backup or durability gate. See
[production readiness](production-readiness.md).
