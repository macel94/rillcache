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
.venv-redis-py/bin/python -m pip install --require-hashes --only-binary=:all: --no-deps \
  -r scripts/redis-py-requirements.txt
.venv-redis-py/bin/python scripts/redis_py_conformance.py \
  --server target/release/rillcache
```

The check launches one loopback-only candidate at a time in each of three
synchronous modes: default Mio readiness, fixed one-worker mode, and Tokio
plain-TCP mode. The harness creates a per-run ACL file with a named user,
checks RESP2 and RESP3 handshakes/authentication, binary string round trips,
non-transactional pipelines, MULTI/EXEC transaction pipelines, binary Pub/Sub,
and redis-py's connection-pool disconnect/reconnect. For each protocol, a
client configured with `db=0` writes a sentinel. A separate `db=1` client must
receive `DB index is out of range` from its startup `SELECT` before its first
write, and the DB 0 sentinel must remain unchanged. The JSON names this check
separately for RESP2 and RESP3. The pinned client omits `SELECT` for `db=0`;
this case validates client startup, not an explicit `SELECT 0` command.

Each network operation has a two-second timeout; readiness is bounded to eight seconds. Processes are
terminated and waited, escalating to kill and wait on timeout. Per-mode server
stderr and the JSON result are written under
`target/upstream/redis-py-conformance/` by default. Each invocation creates a
unique `run-*` subdirectory, and failure logs/ACL inputs are retained there; the
summary JSON is written after all modes pass. Use `--output DIR` to select another
evidence directory. No external Redis/Valkey server is used.

Pass `--tls` to add the same RESP2/RESP3 client checks over the default native
TLS listener using the repository test CA and strict endpoint identity checks.
A second disposable server presents a certificate signed by that CA that does
not identify `127.0.0.1`. The pinned client must reject it specifically for an
IP mismatch; an unrelated connection failure does not pass.

For hosted evidence, manually dispatch [Upstream Tcl compatibility](../.github/workflows/upstream-compatibility.yml)
with `redis_py_tls=true`. The optional job consumes the exact same-run release
binary, installs only the hash-pinned wheel, and retains client results and
server logs. A failed TLS check exits nonzero and retains a per-run failure
result. This job does not replace any required PR check.

A focused lifecycle and fail-closed dependency test is available with:

```sh
python3 -m unittest scripts.upstream.test_redis_py_conformance -v
```

The tested surface is intentionally small. This is not the full C7 acceptance
matrix: client certificates, certificate status/revocation, other negative
paths, node-redis, go-redis and Redis-version coverage remain unverified by this
harness. Results apply only to the tested binary, client, protocol and fixture
certificates. A pass does not establish production support or approve a TLS
termination policy.
