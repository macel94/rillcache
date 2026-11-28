# Representative mixed workload (advisory, separate from the merge gate)

**Scope: one development host, advisory only.** The protected merge gates measure a
deliberately narrow surface (PING, SET, GET and two conditional writes, one key
shape, 64-byte values). This suite is the *separate* representative workload for the
declared profile, and it is never used to pass or fail a merge.

Reproduce:

```sh
python3 scripts/mixed_workload.py --binary target/release/rillcache \
    --seconds 15 --clients 8 --seed 20261128
```

## Workload

One sampled mix per operation (weights in `scripts/mixed_workload.py`):

| Op | Weight | Shape |
| --- | ---: | --- |
| `GET` hit | 45 % | 256-byte strings over a 512-key working set |
| `SET` | 20 % | same working set |
| `GET` miss | 10 % | unique missing keys (negative lookups) |
| `HSET` / `HGET` | 8 / 5 % | 128 hashes × 16 fields |
| `LPUSH` / `LRANGE 0 15` | 4 / 2 % | 64 lists growing under load |
| `SADD` / `ZADD` | 2 / 2 % | 64 sets, 64 sorted sets |
| `PEXPIRE` | 1 % | TTL churn on the string working set |
| `XADD` | 1 % | 8 streams, one field |
| 4 KiB `SET` | once per phase | large-value write path |

Two phases run the same mix: the default configuration and the journal with
`--appendfsync everysec`.

## Measured

| Phase | s | ops | ops/s | p50 ms | p95 ms | p99 ms | Errors | RSS KiB | 4 KiB SET ms |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| journal-everysec | 15.01 | 31196 | 2078 | 2.021 | 11.994 | 33.753 | 0 | 7745536 | 2.699 |

default mix: {'expire': 600, 'get-hit': 26590, 'get-miss': 5839, 'hget': 2995, 'hset': 4651, 'lpush': 2344, 'lrange': 1131, 'sadd': 1210, 'set': 11561, 'xadd': 584, 'zadd': 1170}

Per-op counts for the default phase (15 s): `get-hit` 26 590, `set` 11 561,
`get-miss` 5 839, `hset` 4 651, `hget` 2 995, `lpush` 2 344, `lrange` 1 131,
`sadd` 1 210, `zadd` 1 170, `expire` 600, `xadd` 584 — zero protocol errors in
either phase.

## What this shows, and what it does not

- A realistic mixed keyspace/collection/stream workload with TTL churn runs without
  protocol errors at this size, and the journal with `everysec` costs roughly half
  the client-bound throughput (3907 → 2078 ops/s) with p99 rising 15 → 34 ms.
- The harness is a **Python client on one host**, so throughput is a harness ceiling,
  not server capacity, and the phases are not matched against any competitor.
- Not covered here: TLS, pub/sub fan-out, multi-owner, large collections under
  memory pressure, controlled hardware, and any SLO. Those stay open under `P2`,
  `P5`, `P6`, `R6` and `O9`.

## Relationship to the merge gate

`docs/gate-scope.md` remains authoritative: no required context runs this suite, and
no result here substitutes for `Tests and offline CI` or either Valkey cohort. This
page exists so a representative workload is *reported separately*, as the criterion
asks.
