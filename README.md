# rillcache

A safe Rust server developed toward production Redis drop-in compatibility,
with each compatibility and operational capability demonstrated by tests.

## Current performance

**[Read the latest benchmark summary →](benchmarks/README.md)** — validated tables,
CPU/memory/tail-latency tradeoffs, and GitHub Copilot analysis (preferring GPT-6
Luna, with the actual model disclosed). Default-branch comparisons authenticate
with the built-in workflow token: no personal token or repository secret to set
up. Reports publish automatically with per-run Markdown history; no artifact
download is needed. [Reporting details](docs/benchmark-reports.md).
[Profile-guided before/after results](benchmarks/studies/d57cb2e/README.md) preserve
all 60 native conditions, including gains, regressions and worker-tail tradeoffs.

## Project vision

The initial goal is to implement a useful Redis-compatible server using only Rust’s standard library and runtime—no external dependencies. Over time, the project may explore newer and more advanced functionality, including streams and richer pub/sub capabilities.

Compatibility and behavior will be guided by tests, with the aim of making the server dependable and understandable rather than merely checking off features.

## How we’ll work

This is a collaboration between a human and AI:

- I (the human) will bring the ideas, direction, and product judgment.
- I will validate behavior and proposals with tests.
- All project code must be written by AI; only AI-generated code will be accepted.
- Tests and specifications are how ideas are checked and refined.

## Constraints

- **Language:** Rust
- **Dependencies:** None beyond the Rust standard library and runtime, including tests and benchmarks
- **Safety:** Safe Rust only; `unsafe_code = "forbid"` for every project target. No FFI or native extensions. Safe standard-library APIs are allowed; their internals are outside this policy's scope.
- **Compatibility target:** Redis protocol and behavior, developed incrementally
- **Performance:** Aim to outperform current Redis/Valkey implementations, demonstrated by reproducible measurements—not assumed from the language choice.

## Implemented so far

Bounded RESP2 TCP server, binary-safe `PING`, `ECHO`, `SET` with atomic `NX`/`XX`,
`GET`, `KEEPTTL`, `EX`/`PX`/`EXAT`/`PXAT` options; `GET`, `DEL`, `EXISTS`, `QUIT`;
TTL/PTTL, expiry inspection/update/removal, SET/GET aliases and atomic
`INCR`/`DECR`/`INCRBY`/`DECRBY`. See [string semantics](docs/strings.md).
Ordered pipelines and fragmented
requests; concurrent clients, payload/connection limits and inactivity timeouts.
It is already multithreaded by default (one managed worker per client). Optional
`--threads 1` or `--threads N` selects exactly 1..64 shared I/O/execution workers
plus the accept/control thread; idle clients do not occupy an entire worker.

This is **not yet** a drop-in Redis replacement or a production service.
RESP3/client negotiation, authentication/ACL, TLS, durable persistence, richer
data types, transactions, replication and clustering remain production blockers. It binds
to loopback by default. See [INVEST stories and EARS requirements](docs/requirements.md)
for acceptance criteria and exact compatibility boundaries. The
[production-readiness plan](docs/production-readiness.md) tracks every remaining
blocker; the goal is production compatibility, not silently redefining a subset
as a drop-in replacement.

## Run with Docker or Kubernetes

```sh
docker run --rm --init -p 127.0.0.1:6379:6379 ghcr.io/macel94/rillcache:latest
# Or: docker compose up -d
# Or: kubectl apply -f deploy/kubernetes.yaml
```

The thin multi-architecture image runs non-root, binds container port 6379 without
extra arguments, and has a built-in PING health check—no shell or redis-cli inside.
Append options normally, e.g. `--threads 4`. **In-memory/experimental: no AUTH, TLS
or persistence.** Keep it private. See [container delivery](docs/containers.md)
for resource bounds, Kubernetes networking, digest pinning, shutdown limitations
and GitHub's one-time public-package visibility setting.

## Quick development loop

Install [Rust via rustup](https://rust-lang.org/tools/install/) and GNU Make. The
repository pins Rust, rustfmt and Clippy in `rust-toolchain.toml`. Once that
toolchain is installed, all application checks/builds run offline with no crates
to download. No API keys, `.env` or services are required.

```sh
make test                 # correctness in per-client, one-worker and four-worker modes
make check                # formatting, all-target Clippy, tests and policy checks
make ci                   # same checks + release build + validated benchmark smoke
make run                  # server on 127.0.0.1:6379
cargo run -- --help        # bind address, payload/client limits and idle timeout
```

`cargo run -- --threads 4` selects four I/O/execution workers; `--threads 1`
uses one shared worker, not one-client-at-a-time admission. Fixed workers use
portable nonblocking polling with a 1 ms idle wait and a bounded 50 µs active
window after progress; this trades CPU work for fewer premature sleeps, not free
CPU or kernel readiness. Lower thread counts are not an assertion of lower
latency or higher throughput. Four workers sharing one CPU worsened batch p99
in the matched study despite higher throughput; evaluate `--threads 1` for that
budget. The default is unchanged.

A Redis CLI in RESP2 mode can exercise the supported commands. Some modern
clients issue unsupported discovery commands; compatibility is not yet claimed
for arbitrary client libraries.

## Measure from day one

```sh
make bench                # microbenchmarks + per-client/one/four-worker TCP runs
make bench-threads        # fixed one/four-worker scaling runs with validated replies
make compare              # build pinned competitors once, compare all three servers
make compare BENCH_ARGS='--requests 10000 --clients 4 --pipeline 1 --runs 3'
make compare PROFILE=1    # optional Linux CPU/RSS/thread-context and idle samples
```

The benchmark client is also safe, dependency-free Rust. It validates replies,
seeds GET data, excludes setup/warmup, and writes JSON with throughput and
**batch** latency percentiles. `make compare` uses native standalone processes
for all three servers. Competitor C build tools are optional benchmark tooling,
never Rillcache dependencies. Conditional SET benchmarks validate both rejected
NX and admitted XX operations on seeded existing keys. Comparisons can pin equal
server/client CPU budgets and rotate server order. Only one comparison server
runs at a time; optional safe, std-only Linux accounting records process CPU,
RSS and matched-thread context switches without changing the request timer. See
[benchmark methodology](docs/benchmarks.md) and the [profile-guided hot-path
study](docs/hot-paths.md), including an optional frozen-client before/after workflow.

## GitHub feedback and delivery

[CI](.github/workflows/ci.yml) runs the same `make ci` on PRs and main pushes,
cancels superseded runs, and publishes benchmark evidence. Successful main builds
also provide a Linux binary archive and SHA-256 checksum as Actions artifacts for
immediate testing. [Container delivery](.github/workflows/container.yml) separately
tests and publishes thin amd64/arm64 images to GHCR using the workflow token.
There is no automatic public deployment of this experimental, unauthenticated server.

[Comparative benchmarks](.github/workflows/benchmarks.yml) run manually or weekly,
separately from the fast feedback path. Timing on shared runners is evidence, not
a flaky performance gate. No claim of beating Redis or Valkey is made yet.
