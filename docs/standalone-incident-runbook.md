# Standalone incident runbook (operator guidance, not an HA drill)

This runbook applies only to one Rillcache process and its owner-local data path. It does not provide replication, promotion, reconciliation, automatic failover, or a proven durability/RPO guarantee. A successful client reply or journal fsync setting is not evidence of survival across every process, host, power, filesystem, or device failure. Do not use this service as the sole copy of irreplaceable data.

## Service or node unavailable

1. Confirm the intended host and service using your service manager (`systemctl status <unit>` where systemd is used) and inspect its journal (`journalctl -u <unit> --since '<time>'`). Check host reachability separately; a failed client connection cannot distinguish process, node, route, or firewall failure.
2. Preserve logs and the configured data directory before attempting repair. Do not start a second process against the same data path or point another instance at it as a promotion.
3. If the process is stopped and the host/storage are healthy, use the deployment's normal service manager to start the same instance and inspect startup logs and client connectivity. A restart may lose in-memory state and does not establish a bounded recovery time or acknowledged-write durability.
4. If the host is lost, recovery requires restoring that host's data from an independently maintained backup, if one exists and has been validated. There is no replica to promote or state to reconcile.

## Disk errors or suspected corruption

Treat write, sync, filesystem, and device errors as possible data loss. Stop application writes and preserve the original device/image and logs; do not delete, truncate, repair-in-place, or replay uncertain files as an experiment. Follow the platform/storage incident procedure and restore only from a separately verified backup using a recovery procedure validated for the exact format/version. This project has not validated ENOSPC/device-failure recovery or backup/restore as a complete operator drill. If there is no known-good backup, do not claim recovery; retain the evidence and escalate.

The configured journal, when enabled, is not a backup. `max_data_bytes` is not a disk quota. See [persistence limitations](persistence.md) and the [durability plan](plans/durability-and-recovery.md).

## Network interruption at one node

A network partition isolates clients from this one process; it does not create another node, a quorum, or a failover candidate. Do not start a replacement and accept writes while the original may still be running. Once connectivity returns, reconnect to the same authoritative instance. There is no replication/rejoin protocol, conflict detection, or automatic reconciliation for writes made elsewhere.

## Client reconnect

After confirming the original instance is healthy, clients may reconnect using their normal endpoint and authentication configuration. Re-establish connection-local state (including selected protocol, authentication, subscriptions and transactions) and retry only operations whose outcome is known to be safe: a disconnect after sending a mutation leaves its outcome ambiguous. Do not blindly replay non-idempotent commands. Pub/Sub subscriptions are process-local and are not replayed after disconnect. See [client compatibility](compatibility.md) and [Pub/Sub](pubsub.md).

## Observable status and evidence

On a reachable server, `ROLE` and `INFO replication` report truthful standalone status; these are status probes, not health, disk-writability, backup-validity, recovery-readiness, or failover checks. Capture timestamps, service/host logs, storage alerts, client error context, and the exact data/backup identities during an incident. No promotion or reconciliation procedure is supported or exercised by this runbook.

The hosted `make ci` smoke stage runs `python3 scripts/test_standalone_incident.py` against the just-built release binary. It starts an isolated loopback process with a fresh temporary journal, checks PING and truthful ROLE/INFO replication status, writes one key, stops and confirms listener loss, reconnects after restart, and verifies journal replay. Process output is retained at `target/benchmarks/standalone-runbook/incident.log` (included in the CI benchmark artifact). This exercises orderly local lifecycle/reconnect only; it does not simulate node loss, network partitions, disk faults, or crash/power-loss durability.

Structural documentation checks: `python3 scripts/test_standalone_runbook.py`. They verify required boundaries and linked source references only; they do not establish production readiness. The standalone profile remains a draft; see [production readiness](production-readiness.md).
