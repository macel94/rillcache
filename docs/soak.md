# Mixed-workload soak and recovery drill (O9)

**Status: repeatable harness and the declared supported duration.** The declared
supported soak duration for the standalone profile is **60 minutes** of mixed
workload followed by the restart/recovery check; a 24-hour run is recommended
release-week practice but is not part of the declared supported duration.

```sh
make bench-threads                      # builds target/release/rillcache
python3 scripts/soak.py --duration 900 --clients 8 \
  --json target/soak/report.json | tee target/soak/summary.md
```

The drill starts the built server from a disposable directory with
`--appendonly yes --appendfsync everysec` and runs, for `--duration` seconds:

- writer/reader clients driving `SET`/`GET`/`INCR`/`EXPIRE`/`LPUSH`/`LRANGE`/
  `HSET`/`HGET` (all eight operations rotate per client);
- a **reconnect storm** client that opens, sends `PING` and closes continuously;
- a **slow client** that subscribes and never drains its socket until the end;
- an **admin probe** sampling `INFO memory`, `INFO clients`, `VmRSS` and server CPU
  every `--interval` seconds and triggering `BGREWRITEAOF` every
  `--rewrite-every` samples.

It then stops the server with `SIGTERM`, restarts it on the same directory and
verifies the acknowledged keys, so the run ends with recovery evidence rather than
only a liveness check.

## What it fails on

The harness exits non-zero on liveness or correctness problems only: the server
exiting during the soak, a command returning an unexpected error, a shutdown that
misses its deadline, or a recovery restart that fails. Throughput, latency and
memory growth are **recorded, not asserted**, because the accepted budgets are the
owner decision this harness is meant to inform.

## Acceptance for the declared duration

1. A **60-minute** run must finish with zero harness errors and every acknowledged
   key recovered after the restart.
2. The retained report must contain the RSS/CPU/client series and the rewrite
   triggers for the whole window.
3. A 24-hour run is recommended practice and is not claimed as measured.

## Evidence retention

`target/soak/report.json` holds the per-sample RSS/CPU/client series plus the
recovery result; `summary.md` is the human-readable table. Both are written by the
harness so a run keeps its own measurements and failures.
