# Durability-mode performance profile

**Scope: one development host, matched workload, advisory only.** Each table's
rows were produced by one command against the same release binary, so the modes
within a table are matched with each other. They are not matched with published
benchmarks and are not a capacity plan. Reproduce the matched cohort with:

```sh
TMPDIR=/var/tmp/d10 python3 scripts/durability_profile.py \
    --binary target/release/rillcache --writes 20000 --clients 8 \
    --rewrite-keys 200000
```

Measured on 2026-11-28 from the release binary built at candidate `83a02763`
(SHA-256 `9d3d18026e72bc93c977a67d0e631ebe62b056bc0fd8f9e35ff7f9be55d6b02f`) on
a 6-vCPU AMD EPYC host, Debian 13, kernel 6.12.113. `TMPDIR` was pointed at an
ext4 directory (`/dev/sda1`) because the host defaults `/tmp` to tmpfs; a tmpfs
journal does not perform durable storage work, so it cannot measure fsync cost.
The script disables automatic rewrites (`CONFIG SET auto-aof-rewrite-percentage
0`) so the measured `BGREWRITEAOF` is the only rewrite and the throughput rows
are not perturbed by one.

Workload: 8 concurrent Python clients, pipeline depth 1, 64-byte values, plain TCP,
one owner worker, no TLS, no eviction pressure. The generator is a single-CPU
Python process and is **the bottleneck for the `none` row**, so absolute
throughput is a ceiling of this harness, not of the server. Only the *relative*
cost of durability is interpreted. D10's accepted **relative regression budgets**
are recorded in [durability benchmarks](durability-benchmarks.md); no absolute
throughput or latency SLO follows from these tables (`P2`, `P3` stay open).

| Durability mode | Writes | req/s | p50 ms | p99 ms | p99.9 ms | Server CPU s | CPU s/write | RSS KiB | Restart to ready s | Key recovered | Keys match |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|---|
| none | 20000 | 4652 | 0.673 | 17.110 | 68.399 | 1.98 | 0.000099 | 16179200 | n/a | n/a | n/a |
| always | 20000 | 111 | 40.162 | 465.169 | 2420.348 | 27.67 | 0.001384 | 16842752 | 0.949 | yes | yes |
| everysec | 20000 | 1318 | 2.743 | 62.988 | 163.960 | 6.73 | 0.000337 | 16760832 | 1.581 | yes | yes |
| no-fsync | 20000 | 1608 | 2.302 | 49.653 | 153.254 | 6.07 | 0.000304 | 16728064 | 1.345 | yes | yes |

## What the numbers show

- On real storage, `always` is fsync-bound: 111 req/s, p50 40.2 ms and p99
  465 ms, against 1318 req/s and p99 63 ms for `everysec`. The per-write CPU
  cost is also highest (0.001384 s/write, ~14× the `none` row), because each
  acknowledgement waits for the device. A deployment using `always` must be
  sized for storage fsync latency; the earlier tmpfs numbers did not measure it.
- `everysec` and `no-fsync` are generator-bound in this harness (1318 and 1608
  req/s). One sample cannot rank them, and no such ranking is claimed.
- Restart-to-ready now includes journal replay and is not a bare `PING`: 0.95 s
  for `always`, 1.35 s for `no-fsync` and 1.58 s for `everysec` at 200,000 keys,
  and 6.48 s for the 1,000,000-key `everysec` run below. Every mode recovers the
  acknowledged key and the exact key count (`Keys match`), so the recovery path
  works at these sizes; it is not a recovery SLO (`D11` and `O9` remain open).
- RSS is flat across modes (16.1–16.8 MiB) at 20k keys, i.e. the journal path adds
  no measurable resident overhead at this size.

## Rewrite pause by durability mode

`--rewrite-keys` fills the keyspace to the requested size, then issues one
`BGREWRITEAOF` while a second client keeps issuing `PING` for the whole rewrite
window. The table records the start pause (time to the
`+Background append only file rewriting started` reply), the worst client-observed
`PING` stall over the window (including the start pause), the p99 stall, the
rewrite duration, the number of samples, and whether the run is inside the
reviewed bound in [persistence](persistence.md#compaction-with-bgrewriteaof):
**at most 1 s start pause and at most 1 s worst concurrent stall for keyspaces up
to 1,000,000 keys**. A client-observed stall includes Python-client scheduling
and loopback round trips, so it over-approximates server-only time; that is the
conservative direction for this bound.

| Durability mode | Keyspace | Start pause ms | Worst concurrent stall ms | p99 stall ms | Rewrite duration s | Samples | Within 1 s bound |
|---|---:|---:|---:|---:|---:|---:|---|
| none | n/a | n/a | n/a | n/a | n/a | n/a | no journal |
| always | 199,993 | 253.702 | 261.276 | 5.568 | 40.396 | 76,226 | yes |
| everysec | 199,993 | 88.456 | 179.196 | 5.818 | 40.661 | 73,863 | yes |
| no-fsync | 199,993 | 82.538 | 742.981 | 6.724 | 42.321 | 73,121 | yes |
| everysec | 999,993 | 693.834 | 723.784 | 6.099 | 205.943 | 359,486 | yes |

The 1,000,000-key row is a separate run with the same binary, host and client
shape:

```sh
TMPDIR=/var/tmp/d10 python3 scripts/durability_profile.py \
    --binary target/release/rillcache --writes 20000 --clients 8 \
    --rewrite-keys 1000000 --max-data-bytes 268435456 --only everysec
```

It measured 1434 req/s, p50 2.371 ms, p99 56.356 ms, p99.9 240.583 ms, 6.84
server CPU s, RSS 16.7 MiB and 6.477 s restart-to-ready, with the exact keyspace
recovered. `--max-data-bytes 268435456` is required because the 64 MiB default
payload budget cannot hold one million 64-byte values plus keys: the first
ceiling attempt stopped at 551,773 keys with
`-OOM configured payload limit exceeded`. That failure is retained here; it is
the documented budget, not a rewrite defect, and the budget was raised explicitly
for the ceiling run rather than by changing the default.

What the rewrite rows show:

- Every measured start pause and every worst client stall is inside the reviewed
  1 s bound. The closest row is the 200,000-key `no-fsync` outlier at 743 ms
  (p99 6.72 ms); the 1,000,000-key `everysec` row is 694 ms start pause and
  724 ms worst stall. The margin is not large enough to infer behaviour for a
  larger keyspace, a different disk, a write storm or a different client.
- Start pause grows with the live key count (the implementation materialises one
  key name per live key); the 1,000,000-key everysec pause is 693.834 ms, inside
  the accepted linear bound but the largest measured value.
- Rewrite completion still scales with the dataset (40–42 s at 200,000 keys,
  206 s at 1,000,000). There is no fork, copy-on-write sharing or incremental
  generation, so this is a service-time cost, not a bounded pause.

## Retained limits

- One host, one generator process, 8 clients, 64-byte values, ext4 on a cloud
  volume: no controlled hardware, no second storage generation, no disk-latency
  distribution, no fault injection and no TLS (`P2`, `P5`, `P6`).
- The 1,000,000-key rewrite ceiling was measured for `everysec` only. `always`
  and `no-fsync` were measured at 200,000 keys; their stalls are per-slice
  effects, but a 1M claim for those two policies is not made.
- The rewrite window had a `PING` client, not a sustained write storm or mixed
  workload. The reconcile phase can lengthen under a write storm; that case is
  unmeasured (`O9`, `R6`).
- Rewrite stall numbers are client-observed with a Python `PING` loop on
  loopback, so they include client scheduling and round-trip time. No
  server-internal stall histogram is published.
- No absolute throughput or latency SLO follows from either table (`P2`, `P3`
  and `R6` remain open). The accepted bounds used here are the reviewed rewrite
  start-pause bound above and D10's relative regression budgets in
  [durability benchmarks](durability-benchmarks.md).
- Replication lag, failover time and cluster migration are **not applicable**:
  those topologies are explicit exclusions (`A1`, `A2`), not unimplemented rows.
