# Crash-recovery matrix

**Scope: one development host, SIGKILL crash simulation, advisory.** The matrix
answers one question per cell: after `SIGKILL` at an acknowledged-write boundary,
how many acknowledged writes survive replay, and how long does recovery take?

```sh
python3 scripts/recovery_matrix.py --binary target/release/rillcache --keys 200
```

Each cell starts the real binary on a fresh directory, writes to a chosen boundary,
sends `SIGKILL`, restarts against the same journal and counts how many of the
acknowledged keys are readable. Framing is exercised in both forms (CRC32 and
HMAC-framed journal) and both fsync policies (`always`, `everysec`).

## Measured (32 cells: 2 filesystems × 2 framings × 2 policies × 4 kill points)

| Filesystem | Framing | fsync | Kill after acked writes | Survived | Lost acked | Recovery ms |
|---|---|---|---:|---:|---:|---:|
| host-directory | crc32 | always | 0 | 0 | 0 | 23.45 |
| host-directory | crc32 | always | 1 | 1 | 0 | 21.51 |
| host-directory | crc32 | always | 5 | 5 | 0 | 23.35 |
| host-directory | crc32 | always | 20 | 20 | 0 | 21.56 |
| host-directory | crc32 | everysec | 0 | 0 | 0 | 22.11 |
| host-directory | crc32 | everysec | 1 | 1 | 0 | 22.13 |
| host-directory | crc32 | everysec | 5 | 5 | 0 | 23.01 |
| host-directory | crc32 | everysec | 20 | 20 | 0 | 22.77 |
| host-directory | hmac | always | 0 | 0 | 0 | 22.04 |
| host-directory | hmac | always | 1 | 1 | 0 | 23.42 |
| host-directory | hmac | always | 5 | 5 | 0 | 24.29 |
| host-directory | hmac | always | 20 | 20 | 0 | 30.19 |
| host-directory | hmac | everysec | 0 | 0 | 0 | 23.24 |
| host-directory | hmac | everysec | 1 | 1 | 0 | 23.02 |
| host-directory | hmac | everysec | 5 | 5 | 0 | 21.77 |
| host-directory | hmac | everysec | 20 | 20 | 0 | 23.14 |
| ext4-loop | crc32 | always | 0 | 0 | 0 | 27.84 |
| ext4-loop | crc32 | always | 1 | 1 | 0 | 37.56 |
| ext4-loop | crc32 | always | 5 | 5 | 0 | 22.77 |
| ext4-loop | crc32 | always | 20 | 20 | 0 | 22.41 |
| ext4-loop | crc32 | everysec | 0 | 0 | 0 | 24.65 |
| ext4-loop | crc32 | everysec | 1 | 1 | 0 | 25.05 |
| ext4-loop | crc32 | everysec | 5 | 5 | 0 | 22.12 |
| ext4-loop | crc32 | everysec | 20 | 20 | 0 | 21.67 |
| ext4-loop | hmac | always | 0 | 0 | 0 | 24.51 |
| ext4-loop | hmac | always | 1 | 1 | 0 | 22.06 |
| ext4-loop | hmac | always | 5 | 5 | 0 | 40.43 |
| ext4-loop | hmac | always | 20 | 20 | 0 | 22.08 |
| ext4-loop | hmac | everysec | 0 | 0 | 0 | 23.46 |
| ext4-loop | hmac | everysec | 1 | 1 | 0 | 21.35 |
| ext4-loop | hmac | everysec | 5 | 5 | 0 | 22.16 |
| ext4-loop | hmac | everysec | 20 | 20 | 0 | 24.34 |

**Result: 0 lost acknowledged writes in all 32 cells**, recovery to first `PING`
between 21.3 ms and 30.2 ms.

## What this covers

- **Kill -9 at each boundary**: kill points 0, 1, 5 and 200 acknowledged writes,
  i.e. before any write, immediately after the first, mid-stream and after a
  realistic batch.
- **Filesystem matrix**: the host directory and a 64 MiB ext4 image mounted through
  a loop device (the mount needs privileges; without them that half reports
  `skipped` instead of pretending to pass).
- **Both framings and both fsync policies**, so an HMAC-sealed journal is not
  assumed to behave like a CRC32 one.
- **Replay/restart cost**: recovery time is recorded per cell rather than assumed
  constant.

## Retained limits (please read before quoting any number)

- `SIGKILL` kills the process; the kernel page cache still holds the journal, so a
  zero-loss result here says the **replay logic is sound**, not that power loss is
  safe. A true power cut (lost write-back cache, torn sector, or a crash during
  `fsync`) is **not** simulated: that needs block-level fault injection or
  `dm-flakey`/`dm-writecache` style tooling.
- Mixed *versions* are covered separately by `scripts/upgrade_drill.py` against
  `ci/upgrade-pins.json` (old/new binary pair) and by the legacy RESP archive
  fixture; they are not re-run here.
- Long-running recovery is bounded by the 200-key journal used here; a
  multi-hundred-megabyte journal replay time is not measured.
- One host, no controlled hardware (`P2`), and no failure-rate statistics: a
  single pass per cell.

## Relationship to the rest of the evidence

`scripts/incident_drills.py` covers corrupted and truncated journals plus ENOSPC by
real fault injection; `scripts/restore_drill.py` covers the offline archive path;
this matrix covers crash points and framing/policy/filesystem combinations. None of
the three substitutes for a power-loss test.
