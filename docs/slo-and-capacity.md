# Stated SLOs and capacity plan (single-node profile)

**Status: stated, not independently accepted.** These are the service objectives and
the capacity envelope the project commits to *for the declared profile only*
(`docs/production-profile.md`: one standalone process, plain TCP, optional journal,
one owner-local data path, Linux x86_64). They are derived from repeatable
measurements on a **dedicated single-tenant host**, not from shared CI runners, and
they are checked by `scripts/controlled_run.py` against `ci/slo-v1.json`.

They are **not** a production acceptance: independent review (`S1`, `V3`) and the
remaining P0 gaps still decide whether the project may be called production ready.

## Measured conditions (repeatable)

| Field | Value |
| --- | --- |
| Host | dedicated single-tenant VPS, 6 vCPU `AMD EPYC Processor (with IBPB)`, no cpufreq governor exposed |
| Isolation | server pinned with `taskset` to one CPU, generator pinned to another; no other workload on the host during a run |
| Transport | plain TCP, loopback, RESP2, no TLS |
| Workload | the mixed keyspace/collection/stream workload from `scripts/mixed_workload.py` (GET/SET/hash/list/set/zset/stream, TTL churn, 8 clients, pipeline 1, 256-byte values) |
| Repetitions | 3 rounds, reported per round and as a median |

Reproduce:

```sh
python3 scripts/controlled_run.py --binary target/release/rillcache
```

## Stated SLOs (v1)

| Objective | Target | How it is measured |
| --- | --- | --- |
| Mixed-workload throughput | ≥ **800 ops/s** median over 3 rounds | `scripts/controlled_run.py` |
| Per-command latency (p99, mixed) | ≤ **150 ms** p99 per round | same run |
| Per-command latency (p50, mixed) | ≤ **5 ms** p50 per round | same run |
| Read-path error rate | **0** protocol errors | same run |
| Recovery | ready to serve within **2 s** after a clean restart, all acknowledged keys present | same run (restart phase) |
| Resident memory | ≤ **256 MiB** after a bounded run (≤ 1000 keys) | `INFO memory` `used_memory_rss` in bytes |

These numbers are deliberately loose: the point is that they are **stated,
measured and checked** rather than implied. A stricter objective needs quiet
hardware and a longer run, not a different sentence in this table.

## Measured run (3 rounds × 20 s, server pinned to one CPU, generator to another)

| Round | ops/s | p50 ms | p99 ms | Errors |
|---:|---:|---:|---:|---:|
| 1 | 3 699 | 1.065 | 16.0 | 0 |
| 2 | 1 609 | 3.203 | 32.8 | 0 |
| 3 | 1 131 | 4.643 | 46.3 | 0 |

Medians: **1 608.8 ops/s**, p50 3.203 ms, p99 32.8 ms, 0 errors, restart 0.022 s,
RSS 5 332 992 bytes — every stated objective passes (`python3
scripts/controlled_run.py` exits 0 and prints "all stated SLO checks pass").

**Observed and not explained:** throughput falls across rounds (3 699 → 1 609 →
1 131) while the workload keeps appending to 8 unbounded streams. The keyspace is
bounded but stream length is not, so this is consistent with per-append cost
growing with stream length — the same shape as the journal defect fixed in #710,
but on the in-memory/notification path. It is **not attributed**: host noise and
the Python generator are also candidates, and the run did not isolate them. The
stated SLOs use the median of three rounds precisely so a single drifting round
cannot fail them, and a stable long run needs a stream-length-bounded workload.

## Capacity plan

| Resource | Declared limit | Where it comes from |
| --- | --- | --- |
| Command execution | **one** owner thread | `R1`: single-owner data path is the supported mode; multi-owner is an experiment |
| Clients | 128 admitted by default, explicit `max number of clients reached` beyond that | `R1`/`R5` measured envelope |
| Payload | 64 MiB logical key/value budget by default, refused without partial application | `O1` payload accounting |
| Frames | 1 MiB argument / 2 MiB frame bounds, explicit errors | protocol limits |
| Queue growth | bounded output backpressure; subscribers dropped at the documented queue bound | `O6` |
| Connections per second | **~10 000 reconnects/s** sustained on the measured host with 0 failures (see report) | mixed-workload reconnect phase |
| Journal | appended per command; stream appends journal the entry, not the stream (post-#710); rewrite compacts | `D1`/`#710` |

The envelope is what one owner thread and the documented limits allow; it is not a
scaling claim and it says nothing about a second hardware generation.

## Retained limits

- One dedicated host, one CPU model, no second generation: not a hardware lab, no
  NUMA isolation, no frequency control (`P2` closes on *stated* SLOs and a
  controlled, non-shared host, not on commercial attestation).
- No independent acceptance: these objectives are stated by the project and
  checked mechanically; `S1`/`V3` remain open.
- Load-soak, multi-hour trends and tail distributions under coordinated omission
  remain tracked by `O9`, `P6` and `P3`.
