# Tested version matrix and client load model

**Scope: evidence for the exact tested head only.** This page is the single
published answer to "which upstream releases, client libraries and load shape does
the project actually exercise?" It is not a support window (`docs/release-lifecycle.md`
owns that), not a compatibility claim beyond the declared profile
(`docs/production-profile.md`), and not a production benchmark (`docs/gate-scope.md`).

`python3 scripts/check_compat_matrix.py` keeps this page tied to the pinned files:
if a version, a client or an exercised workflow drifts, the required CI graph
fails through `tests/documentation.rs`.

## Upstream servers

| Upstream | Version | Pinned source | Exercised by |
| --- | --- | --- | --- |
| Redis | 8.10.2 | `ci/upstream/pins.json` (`revision 498ecd0d`) | `.github/workflows/upstream-compatibility.yml` — Tcl matrix rows and the command matrix |
| Valkey | 9.1.2 | `ci/upstream/pins.json` (`revision 7f1dffed`) | `.github/workflows/upstream-compatibility.yml` — Tcl matrix rows; `.github/workflows/upstream-performance.yml` and the protected Valkey gates |
| Dragonfly | 2.0.2 | `ci/daily-pins.json` | `.github/workflows/benchmarks.yml` — daily comparison only, never a gate |
| Garnet | 2.2.5 | `ci/daily-pins.json` | `.github/workflows/benchmarks.yml` — daily comparison only, never a gate |
| dotnet | 10.0.14 | `ci/daily-pins.json` | `.github/workflows/benchmarks.yml` — daily comparison only, never a gate |

No other Redis, Valkey, Dragonfly or Garnet release is tested. A newer or older
release has no evidence here; adding one means updating the pin and exercising it.

## Client libraries

| Client | Version | Language | Harness | Exercised by |
| --- | --- | --- | --- | --- |
| redis-py | 5.2.1 | python | `scripts/redis_py_conformance.py` | `.github/workflows/upstream-compatibility.yml` — `client-matrix` job |
| node-redis | 6.4.0 | javascript | `ci/clients/node-redis/harness.mjs` | `.github/workflows/upstream-compatibility.yml` — `client-matrix` job |
| go-redis | v9.14.0 | go | `ci/clients/go-redis/main.go` | `.github/workflows/upstream-compatibility.yml` — `client-matrix` job |

Each client runs the case list in `ci/upstream/clients-v1.json` against the tested
head with declared expectations and exclusions; `docs/language-clients.md` records
the per-client result rows. No other client version is exercised, and a client
result is not a support promise for that library.

## Client load model

The protected performance gates and the daily comparison use one documented load
shape (`ci/upstream/pins.json`, `ci/upstream/scope.json`):

- **Generator:** the pinned upstream `redis-benchmark` from `ci/upstream/pins.json`
  over RESP2, keepalive, loopback, with the same runner and disjoint server/client
  CPU affinity.
- **Clients and pipelines:** 16 clients at pipeline 1 and pipeline 16.
- **Workload:** `ping`, `set`, `get-hit`, `get-miss`, `incr` with 64-byte values over
  a 1024-key uniform random namespace; 30,000 warmup requests then 300,000
  measured requests per condition, 6 repetitions, one fixed server worker.
- **What this shape does not model:** per-request tails under coordinated omission,
  client-side saturation and queueing, mixed read/write or TTL-churn workloads,
  large values, TLS or persistence modes, and any second hardware generation. Those
  gaps are tracked by `P1`, `P3`, `P5` and `P6` in the readiness checklist and are
  not claimed here.

## Retained limits

- A single hardware generation is exercised: shared GitHub runners, `x86_64` Linux only (`P2`, `L5`).
- No client-side saturation model or coordinated-omission correction (`P3`).
- Competitor rows exist only for the daily comparison; they never gate a merge.
- The matrix records what is exercised, not what is supported: the support window
  lives in `docs/release-lifecycle.md`, and the canonical readiness list in
  `docs/production-readiness-gaps.md` wins over this page.
