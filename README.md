# rillcache

An experiment in building a Redis-compatible server from the ground up in Rust.

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

Bounded RESP2 TCP server, binary-safe `PING`, `ECHO`, `SET` with atomic `NX`/`XX`
conditions, `GET`, `DEL`, `EXISTS`, and `QUIT`; ordered pipelines and fragmented
requests; concurrent clients, payload/connection limits and inactivity timeouts.

This is **not** a drop-in Redis replacement or a production service. No TTL,
other SET options, RESP3, authentication, TLS, persistence or other data types yet. It binds
to loopback by default. See [INVEST stories and EARS requirements](docs/requirements.md)
for acceptance criteria, exact compatibility boundaries and the next small stories.

## Quick development loop

Install [Rust via rustup](https://rust-lang.org/tools/install/) and GNU Make. The
repository pins Rust, rustfmt and Clippy in `rust-toolchain.toml`. Once that
toolchain is installed, all application checks/builds run offline with no crates
to download. No API keys, `.env` or services are required.

```sh
make test                 # fastest behavior feedback
make check                # formatting, all-target Clippy, tests and policy checks
make ci                   # same checks + release build + validated benchmark smoke
make run                  # server on 127.0.0.1:6379
cargo run -- --help        # bind address, payload/client limits and idle timeout
```

A Redis CLI in RESP2 mode can exercise the supported commands. Some modern
clients issue unsupported discovery commands; compatibility is not yet claimed
for arbitrary client libraries.

## Measure from day one

```sh
make bench                # parser/store microbenchmarks + repeated local TCP tests
make compare              # build pinned competitors once, compare all three servers
make compare BENCH_ARGS='--requests 10000 --clients 4 --pipeline 1 --runs 3'
```

The benchmark client is also safe, dependency-free Rust. It validates replies,
seeds GET data, excludes setup/warmup, and writes JSON with throughput and
**batch** latency percentiles. `make compare` uses native standalone processes
for all three servers. Competitor C build tools are optional benchmark tooling,
never Rillcache dependencies. Conditional SET benchmarks validate both rejected
NX and admitted XX operations on seeded existing keys. Comparisons can pin equal
server/client CPU budgets and rotate server order. See
[benchmark methodology](docs/benchmarks.md).

## GitHub feedback and delivery

[CI](.github/workflows/ci.yml) runs the same `make ci` on PRs and main pushes,
cancels superseded runs, and publishes benchmark evidence. Successful main builds
also provide a Linux binary archive and SHA-256 checksum as Actions artifacts for
immediate testing. There is no automatic public deployment of this experimental,
unauthenticated server.

[Comparative benchmarks](.github/workflows/benchmarks.yml) run manually or weekly,
separately from the fast feedback path. Timing on shared runners is evidence, not
a flaky performance gate. No claim of beating Redis or Valkey is made yet.
