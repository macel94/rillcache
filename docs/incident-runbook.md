# Incident runbook: disk, memory, credential and journal faults

**Scope: the declared single-node profile** (`docs/production-profile.md`). Every
bound below was measured by executing the drill, not assumed;
`scripts/incident_drills.py --binary target/release/rillcache` reproduces them.

```sh
python3 scripts/incident_drills.py --binary target/release/rillcache          # all
python3 scripts/incident_drills.py --binary ... --only disk-full --json       # one, machine readable
```

Each drill starts the real binary in a disposable directory, injects one fault
through a supported control, and asserts the documented recovery bound. A drill
prints `SKIP` with the exact missing privilege instead of pretending to pass.

## Disk full (`disk-full`)

**Inject:** a 2 MiB ext4 image mounted through a loop device, journal inside it,
`--appendonly yes --appendfsync always`; write 8 KiB values until the write fails.

**Measured:** `-ERR append-only write failed: No space left on device (os error 28)`
after 104 acknowledged writes. The process stays up, earlier acknowledged values
remain readable, and writes resume once space is freed.

**Operator response:** stop admitting large writes, free journal space or rotate to
a new `--dir`, verify with `INFO persistence` and a read of a known key, then resume.
**Bound:** no acknowledged-write loss is claimed for a crash *during* ENOSPC, and
fsync latency on a nearly full filesystem is not measured.
**Requires:** mount privileges; without them the drill reports `SKIP`.

## Memory bound (`memory-bound`)

**Inject:** `--memory-bound-bytes 8388608 --memory-bound-headroom-bytes 2097152`,
then grow the keyspace.

**Measured:** `-OOM configured payload limit exceeded`; the server stays up and
`INFO memory` keeps reporting `used_memory_rss` plus
`rillcache_memory_bound_bytes`, the derived `rillcache_memory_bound_payload_limit_bytes`
and `rillcache_memory_bound_scope:rss_bound_derived_payload_limit_not_hard_rss_cap`.

**Operator response:** treat the bound as a payload ceiling derived from RSS, not a
hard RSS cap; size the bound with headroom for stacks, allocator and sockets, and
watch `used_memory_rss` against the bound. **Bound:** no host OOM-score or cgroup
eviction behaviour is claimed.

## Credential lockout (`acl-lockout`)

**Inject:** an ACL file whose default user is `off`.

**Measured:** startup refuses the file —
`cannot load ACL file ...: MissingDefault`, so the process fails closed instead of
serving a locked-out or silently permissive state.

**Operator response:** repair the ACL file offline (or use a second credential with
`ACL LOAD` while it is still valid), re-run `--check-config`, then restart. Verify
with an authenticated `PING` and a denied-command probe.
**Bound:** an operator who removes the last authorized admin must use offline file
repair; the process cannot be trusted to restore access by itself.

## Journal corruption (`corruption-recovery`)

**Inject:** truncate the journal to half its bytes.

**Measured:** startup succeeds and replays `0/50` keys — the truncation landed
inside the first recorded group, so the whole replay was discarded, which is the
documented torn-tail behaviour (`docs/persistence.md`, "truncated to the previous
verified boundary"). Restoring the intact bytes reloads all 50 keys.

**Operator response:** stop the process, restore the last verified archive
(`docs/persistence.md#offline-backup-and-restore`), replay into a disposable
directory, verify keys and TTLs, then start. **Bound:** the loss is bounded by the
last verified boundary, not by "only the tail": a mid-file truncation can discard
everything after it. Truncation at a verified boundary is indistinguishable from a
rollback and cannot be detected from the file alone.

## Certificate rotation

Covered by `tests/native_tls.rs`: atomic generation reload, invalid replacement
retained, stale/unusable OCSP staple refused. Not re-implemented in the drill
harness; the test names are the evidence.

## Retained limits

- These drills run on one host in disposable directories; they are not a staging
  deployment, and none of them measures throughput or failure rates.
- `disk-full` needs mount privileges; `memory-bound` does not model cgroup
  eviction, and `corruption-recovery` does not model a corrupted *and* unrecoverable
  journal (that needs the offline archive).
- No high-availability, replication or multi-node incident is covered: the profile
  is one standalone process (`A1`, `A2` are explicit exclusions).
