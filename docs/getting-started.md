# Getting started

Choose Docker or a native build. Neither needs a configuration file, API key or
another database. Start with local, rebuildable data while you evaluate Rillcache.

## Run with Docker

```sh
docker run --rm --init --name rillcache \
  -p 127.0.0.1:6379:6379 ghcr.io/macel94/rillcache:latest
```

The published image supports Linux amd64 and arm64, runs non-root and includes a
PING health check. The port mapping exposes it only on your own machine.
`latest` follows successful main builds; it is not a stable release tag.

In another terminal, with [Redis's CLI](https://redis.io/docs/latest/develop/tools/cli/) installed:

```sh
redis-cli -2 PING
redis-cli -2 SET greeting hello
redis-cli -2 GET greeting
```

Expected replies are `PONG`, `OK` and `"hello"`. `-2` selects RESP2, the simplest
starting point; see [client compatibility](compatibility.md#clients-and-protocol).
To try expiration, use `redis-cli -2 SET greeting hello EX 60` and then
`redis-cli -2 TTL greeting`.

Stop the example with `docker stop rillcache`. The default cache is in memory:
its contents disappear when the process exits. Protocol `SHUTDOWN` can join
workers and flush an enabled journal; container signals do not provide that same
graceful drain. See [container lifecycle and persistence](containers.md).

Already using port 6379? Publish `127.0.0.1:6380:6379` instead, and add `-p 6380`
to the `redis-cli` commands. Don't run the Docker and native examples on the same
port at the same time.

## Build from source

Install [Rust through rustup](https://rust-lang.org/tools/install/) and Git:

```sh
git clone https://github.com/macel94/rillcache.git
cd rillcache
cargo run --locked --offline --release --bin rillcache
```

The repository's `rust-toolchain.toml` selects the compiler version. Install that
toolchain first if it is not available locally; the application dependencies are
vendored, so Cargo does not need to download crates. The native server listens
on `127.0.0.1:6379` by default. Use the same CLI commands above to try it.

For subsequent starts, run `./target/release/rillcache`. A compiled Linux archive
and checksum are also available under **Artifacts** in successful
[main CI runs](https://github.com/macel94/rillcache/actions/workflows/ci.yml?query=branch%3Amain).

## Configure only what you need

```sh
./target/release/rillcache --help
./target/release/rillcache --bind 127.0.0.1:6380 --max-data-bytes 134217728
```

| Setting | Default | Change it with |
| --- | --- | --- |
| Native listener | `127.0.0.1:6379` | `--bind` |
| Concurrent clients | 128 | `--max-clients` |
| Key/value payload budget | 64 MiB | `--max-data-bytes` |
| Request argument / frame bound | 1 MiB / 2 MiB | `--max-bulk-bytes` / `--max-frame-bytes` |
| Socket inactivity timeout | 30 seconds | `--idle-timeout-ms` |
| Authentication / persistence | Off | [Security](security.md) / [persistence](persistence.md) |

The payload budget is not a process-memory limit: metadata, indexes, buffers,
threads and sockets also use memory. Start with the defaults, then measure your
workload before adjusting limits. Normal startup chooses the networking layout;
you do not need to pick an I/O engine or worker count to use the cache.
[Runtime architecture and advanced choices](hot-paths.md) are contributor-level
reference, not a prerequisite for getting started.

## Next steps

- [Compatibility](compatibility.md): command families, clients and current boundaries.
- [Containers](containers.md): Compose, Kubernetes, resource limits and digest pinning.
- [Security](security.md) and [TLS](tls.md): configure access before exposing a listener beyond a private interface. Authentication and TLS are opt-in.
- [Persistence](persistence.md): the optional journal is not yet a durable system of record.
- [Contributing](../CONTRIBUTING.md): development setup, tests and review.

[Documentation home](README.md) · [Project home](../README.md)
