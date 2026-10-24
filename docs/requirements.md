# Requirements and incremental delivery

The README is the product charter. This document turns it into small, testable
increments; it is not a claim of full Redis compatibility.

## Non-negotiable constraints

- **Safe Rust only:** every project target forbids `unsafe_code`. No FFI, raw
  pointer manipulation, custom allocator, native extension or build script.
  Safe standard-library APIs are allowed; the standard library/runtime/OS have
  internals outside this project's code and are not claimed to contain no unsafe.
- **Zero external Rust dependencies**, including build, development, test and
  benchmark dependencies. The dependency-policy test inspects all targets/features.
- All project code is AI-generated. Human review, product judgment and test-based
  validation remain central. Automation does not replace that validation.
- Compatibility is incremental, byte-oriented and demonstrated by tests.
- Rust alone is not evidence of speed. Performance claims require comparable,
  repeated end-to-end measurements against current Redis and Valkey releases.

## INVEST stories: implemented increments

Each story provides a separately testable outcome. Estimates are relative scope
(XS/S), not delivery promises. Internal design remains negotiable; the constraints
above do not. Shared RESP plumbing is an explicit technical prerequisite, not a
reason to couple all future commands into a single large story.

| ID | Story and value | Size | Acceptance evidence |
| --- | --- | --- | --- |
| S01 | As a client author, I can check connectivity and echo binary data so that I can validate my transport. | S | R10–R13; protocol unit tests and TCP tests |
| S02 | As a cache user, I can store, overwrite and retrieve strings so that independent clients can share cached bytes. | S | R20–R22; engine and TCP tests |
| S03 | As a cache user, I can check and delete keys so that I can invalidate data. | XS | R23–R24; duplicate-key and shared-connection tests |
| S04 | As a developer, I can run the same offline checks locally and on GitHub so that each change receives fast feedback. | S | R30–R32; `make ci` and CI workflow |
| S05 | As a performance investigator, I can reproduce validated workloads against all three servers so that optimization starts with evidence. | S | R40–R43; benchmark tests, smoke and comparison targets |
| S06 | As a cache user, I can create a value only when absent or replace it only when present so that concurrent clients do not race a separate existence check. | S | R25–R29; engine, pipeline, racing-client and current-reference tests |
| S07 | As a performance investigator, I can compare equal CPU budgets using unchanged binaries so that parallelism and build churn do not masquerade as implementation improvements. | S | R44–R46; recorded affinities, rotated order and cross-profile digest checks |

## EARS acceptance requirements

### Policy and transport

- **R01 (ubiquitous):** The project shall compile with unsafe project code forbidden
  and without any external dependency in any target or feature.
- **R10 (event-driven):** When a complete supported RESP2 command is received, the
  server shall execute it once and return one correctly framed response.
- **R11 (state-driven):** While a command is incomplete, the server shall retain
  its bytes without executing it; when completed, it shall process the command.
- **R12 (event-driven):** When multiple commands arrive in a pipeline, the server
  shall return their responses in command order, without waiting for another read.
- **R13 (event-driven):** When `PING [message]` or `ECHO message` is received with
  valid arity, the server shall return Redis-compatible PONG or binary bulk data.
- **R14 (unwanted behavior):** If framing or configured request limits are invalid,
  then the server shall emit a protocol error and close only that connection.
- **R15 (unwanted behavior):** If a command is unknown or has invalid arguments,
  then the server shall return an error without mutating data or closing the
  connection. Unknown-command errors shall not reflect arbitrary control bytes.
- **R16 (event-driven):** When `QUIT` is received, the server shall flush `+OK`,
  close the connection and not execute subsequent pipelined commands.
- **R17 (unwanted behavior):** If the client capacity is exhausted, then the server
  shall reject additional connections without creating additional workers.
- **R18 (event-driven):** When an embedded server is shut down or its handle is
  dropped, the server shall interrupt client I/O and join every worker.
- **R19 (state-driven):** While a client performs no socket I/O for the configured
  inactivity timeout, the server shall release that connection.

### Data

- **R20 (event-driven):** When plain `SET key value` is received within the payload
  budget, the server shall atomically replace the value and reply `+OK`.
- **R21 (event-driven):** When `GET key` is received, the server shall return the
  stored bytes or a null bulk for a missing key, distinguishing an empty value.
- **R22 (unwanted behavior):** If an admitted SET would exceed the key/value
  payload budget, then the server shall return OOM and preserve the previous
  data. Overwrite and deletion shall reclaim payload budget. This is not an absolute RSS limit.
- **R23 (event-driven):** When `DEL key [key ...]` is received, the server shall
  atomically remove existing keys and count each removed key only once.
- **R24 (event-driven):** When `EXISTS key [key ...]` is received, the server shall
  count each existing argument, including duplicates, as Redis does.
- **R25 (event-driven):** When `SET key value NX` is received, the server shall
  atomically create the key and return `+OK` if absent; otherwise it shall return
  a null bulk and preserve the existing value, including an empty value.
- **R26 (event-driven):** When `SET key value XX` is received, the server shall
  atomically replace the value and return `+OK` if present; otherwise it shall
  return a null bulk without creating the key.
- **R27 (unwanted behavior):** If SET options contain both NX and XX or an
  unsupported token, then the server shall return a syntax error before checking
  conditions or changing data. Complete ASCII NX/XX tokens shall be
  case-insensitive; repeated identical conditions shall be accepted.
- **R28 (event-driven):** When concurrent clients perform NX writes to one absent
  key with sufficient payload budget and no intervening deletion, the server
  shall accept exactly one write and preserve that winner's bytes.
- **R29 (state-driven):** While a SET condition is not satisfied, the server shall
  neither allocate stored payload nor consume its budget; admitted writes shall
  retain R22's atomic OOM behavior and budget reclamation.

### Development and delivery

- **R30 (event-driven):** When `make check` is run, the project shall check formatting,
  lint every target, enforce the dependency policy and execute its tests offline.
- **R31 (event-driven):** When a pull request or main push is checked on GitHub, CI
  shall run the same `make ci` command as local development, including a release
  benchmark smoke test that validates actual replies.
- **R32 (event-driven):** When main passes CI, the workflow shall make a Linux
  release archive, checksum and benchmark evidence available as downloadable
  artifacts, without publishing to a registry or deploying an unauthenticated
  experimental server. No deployment environment has been specified.

### Performance evidence

- **R40 (event-driven):** When a benchmark is run, the tool shall validate every
  measured response and fail on incorrect responses, I/O failure or worker failure.
- **R41 (event-driven):** When a result is reported, it shall identify workload,
  request count, clients, pipeline, keyspace, value size, warmup, run number,
  elapsed time, throughput and batch-latency percentiles in machine-readable JSON.
- **R42 (event-driven):** When GET is benchmarked, the tool shall seed and verify
  data before timing; setup, connection creation and warmup shall be excluded.
- **R43 (ubiquitous):** Comparative results shall use the same client and workload
  parameters for all servers and record source/compiler/hardware and competitor
  versions. Shared-host CI timings shall not be treated as stable performance gates.
- **R44 (event-driven):** When unchanged reference sources are reused across
  profiles, the comparison workflow shall verify unchanged executable digests.
  Source-cache bookkeeping shall not discover the enclosing project's Git history
  or turn generated build output into source-version changes.
- **R45 (event-driven):** When CPU budgets are configured, the comparison shall
  apply them to every server and the benchmark client and record requested and
  actual affinities. CI shall include equal single-core server budgets with
  clients on separate CPUs, alongside default-threading measurements.
- **R46 (event-driven):** When a comparison is rerun, the harness shall discard
  previous result files before starting; if the run fails, it shall not leave
  another server's stale JSON presented as that run's evidence.

## Compatibility boundary (v0.1)

Supported: RESP2 nonempty arrays of non-null bulk strings, case-insensitive ASCII
command names, binary keys/values, multiple connections and ordered pipelines;
`PING`, `ECHO`, `SET key value [NX | XX]`, `GET`, `DEL`, `EXISTS`, `QUIT`.
NX/XX conditions are atomic, case-insensitive and repeatable when identical.
Option tokens must match completely; embedded NULs/prefix matches are rejected.

Deliberately not supported: inline/telnet commands, empty/null command arrays,
RESP3/HELLO, other SET options/TTL (including GET/EX/PX), other types, SELECT,
AUTH/ACL/TLS, persistence, replication, clustering, transactions, pub/sub and
streams. Unsupported SET options return `ERR syntax error` and do not write.
Redis normally ignores empty request arrays; this strict initial subset rejects
them. Unknown-command error text is shorter than Redis's text. Limits are deliberately smaller than Redis defaults.

Default limits: 128 clients, 30-second socket inactivity timeout, 1 MiB bulk,
2 MiB complete request frame, 1024 arguments, 64 MiB key/value payload. Payload
accounting excludes map capacity, allocator and connection/thread buffers.
The CLI uses loopback by default. Do not expose it to untrusted clients.

## Next independently deliverable stories

Not implemented or implied by the completed increments:

1. EX/PX expiration with an injectable clock, no sleep-dependent expiry tests.
   NX alone is not a distributed-lock implementation; TTL and token-safe release
   are not implemented.
2. INCR/DECR with Redis integer parsing and overflow behavior.
3. Common client handshake metadata, selected only after client compatibility tests.
4. Prioritize the measured single-core, low-pipeline and tail-latency weakness:
   measure server CPU/RSS/context switches with PING as a no-key-map control,
   then evaluate bounded I/O scheduling before speculative sharding. Include idle
   CPU, backpressure, shutdown and multi-key atomicity in acceptance criteria;
   unsafe code and external dependencies remain forbidden.
5. Controlled-hardware performance budgets and repeated current-release comparisons;
   later, richer types, pub/sub and streams driven by product judgment.

Definition of done for each future story: EARS acceptance cases and limits agreed,
unit and wire tests added, dependency/safety policy intact, `make ci` passing,
relevant benchmark captured, compatibility documentation updated, human validation.

## Sources

- [INVEST](https://agilealliance.org/glossary/invest/)
- [EARS patterns](https://alistairmavin.com/ears/)
- [RESP specification](https://redis.io/docs/latest/develop/reference/protocol-spec/)
- [Redis command reference](https://redis.io/docs/latest/commands/)
- [Safe/unsafe boundary](https://doc.rust-lang.org/nomicon/safe-unsafe-meaning.html)
- [Cargo lint policy](https://doc.rust-lang.org/cargo/reference/manifest.html#the-lints-section)
