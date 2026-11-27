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

## A4 upgrade, downgrade and rollback policy

**Policy (2026-11-27):** upgrades are supported only for **recorded binary
pairs** that the drill exercises, and the guaranteed rollback path is the
pre-upgrade archive plus the preserved journal, never an assumed downgrade.

- **Supported pairs.** `ci/upgrade-pins.json` records the first pair: baseline
  commit `a98e2959` (`0.1.0-rc-baseline-a4`) → current head
  (`0.1.0-rc-current`). Each future release must pin its predecessor there; a
  pair is claimed only for the journal format and behavior the drill exercises on
  that exact pair. Arbitrary version skew, rolling cluster upgrades and
  cross-version replication are not supported.
- **Format compatibility.** The journal carries checksum groups and optional
  HMAC markers, and upgrade compatibility means the target binary opens the
  source journal, serves identical data, absolute expirations and accounting, and
  appends its own writes. The drill records the observed downgrade outcome per
  pair; the current pair downgrades by **serving** the upgraded journal because
  the markers are ordinary RESP commands the older binary replays harmlessly. A
  future on-disk format change must be documented here with its migration and may
  set the pin to `downgrade: refused`, in which case the archive is the only
  rollback path.
- **Rollback.** Before any upgrade attempt the procedure preserves the journal and
  creates a verified archive with the source binary. Rollback restores that
  archive into an isolated directory, validates it and starts the previous
  binary. The drill proves this restores the exact pre-upgrade state and that the
  preserved journal and archive are never modified by the attempt.
- **Failed upgrade guarantee.** An upgrade that dies after the target binary
  accepted writes still leaves the preserved journal and archive restorable by the
  previous binary (the pre-upgrade state), while the target binary recovers its
  own acknowledged prefix from the interrupted copy. No acknowledged write is
  silently reinterpreted across the pair.
- **Traffic cutover.** Announce a freeze, stop the writer cleanly, verify the
  preserved archive, install the target binary, start it, validate data/TTLs/
  accounting, then resume traffic; keep the previous binary and the prepared
  journal until the new instance is accepted.

Evidence: `scripts/upgrade_drill.py` (exact binaries from the pinned pair;
upgrade, downgrade, interrupted upgrade and archive rollback; per-step timings in
`result.json`/`summary.md`) and `scripts/test_upgrade_drill.py` (offline pin and
evidence checks). The fail-closed `Upgrade and rollback drill` job builds both
commits from source and runs the drill with only the ordinary test gates.
Non-claims: no rolling multi-node upgrade, no automatic format migration, no
downgrade guarantee outside the pinned pair, and no mixed-version replication.
