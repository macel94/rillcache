# Optional real redis-py conformance

This standalone compatibility check uses the real, exactly pinned `redis-py`
5.2.1 client against disposable Rillcache server processes. It is external
compatibility tooling, not a Rust/Cargo dependency and not a required CI gate.
Missing client tooling, a wrong client version, server startup failure or any
failed assertion returns nonzero; none is treated as a pass.

## Run

Build the candidate and install the pinned client in an isolated Python
environment, then run from the repository root:

```sh
cargo build --locked --offline --release --bin rillcache
python3 -m venv .venv-redis-py
.venv-redis-py/bin/python -m pip install 'redis==5.2.1'
.venv-redis-py/bin/python scripts/redis_py_conformance.py \
  --server target/release/rillcache
```

The check launches one loopback-only candidate at a time in each of three
synchronous modes: default Mio readiness, fixed one-worker mode, and Tokio
plain-TCP mode. The harness creates a per-run ACL file with a named user,
checks RESP2 and RESP3 handshakes/authentication, binary string round trips,
non-transactional pipelines, MULTI/EXEC transaction pipelines, binary Pub/Sub,
and redis-py's connection-pool disconnect/reconnect. Each network operation has
a two-second timeout; readiness is bounded to eight seconds. Processes are
terminated and waited, escalating to kill and wait on timeout. Per-mode server
stderr and the JSON result are written under
`target/upstream/redis-py-conformance/` by default. Each invocation creates a
unique `run-*` subdirectory, and failure logs/ACL inputs are retained there; the
summary JSON is written after all modes pass. Use `--output DIR` to select another
evidence directory. No external Redis/Valkey server is used.

A focused lifecycle and fail-closed dependency test is available with:

```sh
python3 -m unittest scripts.upstream.test_redis_py_conformance -v
```

The tested surface is intentionally small. This is not the full C7 acceptance
matrix: TLS/certificate identity, negative/error paths, node-redis, go-redis,
Redis-version coverage, and automated CI execution remain unverified. A single
successful run does not establish production support or drop-in compatibility.
The generated result is local evidence for the exact binary/client invocation,
not a cross-version compatibility claim.
