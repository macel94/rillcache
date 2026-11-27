# Availability and topology

Open items are tracked in the canonical [production-readiness gaps](../production-readiness-gaps.md); this file records current behavior and boundaries, not a second checklist.

Part of the [production-readiness plan](../production-readiness.md). This file owns
the capability row so independent feature increments edit only their own area.

| Capability | Current boundary / evidence required before closing |
| --- | --- |
| Availability and topology | Sentinel and automatic failover are excluded by a reviewed decision ([A2 decision](#a2-profile-decision-sentinel-and-automatic-failover-excluded)): every `SENTINEL` form fails closed, `FAILOVER` and host/port promotion are refused, the no-op promotion changes no state, and `INFO replication` reports `master_failover_state:no-failover`. Standalone status only: `ROLE` and `INFO replication` truthfully report `master`, zero replicas and offset 0 without a replid or replica rows; `REPLICAOF`/`SLAVEOF NO ONE` succeed as no-op promotion while host/port forms, `PSYNC`, `SYNC`, `REPLCONF` and `FAILOVER` return explicit replication-unsupported errors (S92). `MOVE`/`SWAPDB` explicitly report that only database 0 exists. Open work (replication, failover, cluster, partition/rejoin and mixed-version evidence) is tracked in the canonical gaps file (A1–A5). Multiple independent servers do not imply replication or clustering. See the [standalone incident runbook](../standalone-incident-runbook.md) for service/node loss, disk-error boundaries, single-node network interruption and reconnect guidance; it is not a promotion, reconciliation, fault-drill or durability guarantee. |

## A2 profile decision: Sentinel and automatic failover excluded

**Decision (2026-11-27):** high availability is **out of the first profile**.
There is no Sentinel-compatible control plane, no quorum failure detection, no
election, no promotion, no fencing and no client rediscovery. The reviewed
single-node availability boundary is recorded here and in the canonical
[production readiness gaps](../production-readiness-gaps.md); the replication
link itself is covered by the separate A1 decision.

Rationale:

- Promotion without replication cannot preserve data. A standby that never
  received the primary's writes has nothing to promote, and promoting a stale or
  diverged copy would silently discard acknowledged writes.
- A control plane needs failure detection, quorum, fencing and rediscovery to be
  safer than an operator. Implementing it without those properties would be a
  claim that cannot be tested against partitions, stale configuration or flapping
  nodes, which the readiness policy forbids.
- The verified recovery path already exists and is measured: a quiescent archive
  restored and validated into an isolated instance (see the
  [restore drill](../persistence.md#isolated-restore-drill-rporto-and-retention)).
  That is the supported way to move a dataset to replacement hardware.

Alternatives rejected for this profile: a Sentinel-compatible sidecar that
promotes an empty instance (data loss presented as HA); orchestration that
restarts the same single-owner instance on another host from local disk (loses
the un-backed-up tail and silently drops the archive/validation step); and
active-active writers behind a proxy (divergent datasets).

Revisit when a deployment requires automatic promotion, a reviewed design covers
replication (A1) plus quorum, fencing, split-brain prevention and client
rediscovery, and the failure drills below can run with both ends pinned.

### Failure detection and recovery contract

| Concern | Supported behavior |
| --- | --- |
| Sentinel protocol | every `SENTINEL` form fails closed on the first call (`ERR unknown command`); no monitoring, election or fencing surface exists |
| Operator failover request | `FAILOVER` returns the explicit replication-unsupported error |
| Promotion source | `REPLICAOF host port`/`SLAVEOF host port` are refused; `REPLICAOF NO ONE`/`SLAVEOF NO ONE` are the declared no-ops and change no state |
| Reported state | `ROLE` is `master`; `INFO replication` stays `master_failover_state:no-failover` with zero replicas, no replid and no replica rows |
| Detection | operator probes (`--healthcheck`), client connection errors and `aof_last_write_status`/`aof_last_bgrewrite_status`; there is no quorum detector |
| Recovery | verified archive → isolated restore → validate data/TTLs/accounting → switch clients; RPO is the newest off-host verified archive, RTO is the measured restore plus validation and switch |
| Split brain | no second writer can exist for one journal (exclusive ownership lock); independent instances serve divergent data and must never be presented as one deployment |

Hard rules that follow: never promote a stale or diverged copy, never point two
writers at one keyspace, never front independent instances with one address or
proxy, keep archives and rollback anchors outside the failure domain, and
re-measure the restore path on the deployment's own hardware.

Evidence: `tests/failover_boundary.rs` proves the entry-point, no-op and reported
state above, and runs a two-instance drill in which the primary is killed: its
address is not served by anything else, the survivor stays a master over its own
keyspace only, and it does not adopt the primary's keys. The replication link
itself, including its all-mode protocol surface, is the separate A1 decision and
is not claimed by this control-plane decision.

Scope: this decision covers the control plane and promotion only. Upgrades and
rollback (A4), topology runbooks and failure exercises (A5) and the replication
link (A1) remain separate items.
