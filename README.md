# <img src="docs/assets/rillcache-logo.svg" alt="Rillcache" width="420">

**Fast caching. Familiar commands. Safe Rust.**

Rillcache is a Redis-compatible cache server written in safe Rust and developed
with AI. The goal is simple: a major step forward in performance, without making
caching harder to use.

- **Familiar tools:** the Redis wire protocol, `redis-cli`, and common data structures.
- **Measured speed:** published comparisons with Valkey and Dragonfly, including the workloads and limits.
- **A simple start:** one binary or container, with sensible defaults. No cluster or runtime tuning to get your first key stored.

## Try it

With Docker installed:

```sh
docker run --rm --init --name rillcache \
  -p 127.0.0.1:6379:6379 ghcr.io/macel94/rillcache:latest
```

With `redis-cli` installed, open another terminal:

```sh
redis-cli -2 PING                 # PONG
redis-cli -2 SET greeting hello   # OK
redis-cli -2 GET greeting         # "hello"
```

That's it: a local cache speaking a familiar protocol. The published port stays
on your machine's loopback interface.

Prefer a native binary? See [building from source](docs/getting-started.md#build-from-source).

## Explore

| I want to… | Start here |
| --- | --- |
| Run and configure Rillcache | [Getting started](docs/getting-started.md) |
| See supported commands and compatibility | [Compatibility guide](docs/compatibility.md) |
| Understand the performance results | [Benchmarks and scope](docs/performance.md) |
| Use Docker, Compose or Kubernetes | [Container guide](docs/containers.md) |
| Give a coding agent repository context | [OpenWiki](openwiki/quickstart.md) or its [published wiki](https://macel94.github.io/rillcache/) |
| Find a specific topic | [Documentation](docs/README.md) |

## Status and contributing

Rillcache is **early-stage**: use it for development and evaluation on rebuildable
data. Full Redis drop-in compatibility and production readiness are still in
progress; the [canonical gap checklist](docs/production-readiness-gaps.md) records
every remaining blocker, and the [compatibility guide](docs/compatibility.md)
explains what works today.

Try it, report unexpected behavior, or help shape what comes next.
[Contributing](CONTRIBUTING.md) explains the AI-led development process, tests and
review requirements.
