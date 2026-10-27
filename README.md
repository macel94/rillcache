# rillcache

A safe Rust server developed toward production Redis drop-in compatibility,
with each compatibility and operational capability demonstrated by tests.

## Current performance

**[Read the latest benchmark summary →](benchmarks/README.md)** — validated tables,
CPU/memory/tail-latency tradeoffs, and GitHub Copilot analysis (preferring GPT-6
Luna, with the actual model disclosed). Default-branch comparisons authenticate
with the built-in workflow token: no personal token or repository secret to set
up. Reports are proposed automatically through gated PRs; merged reports retain
per-run Markdown history without an artifact download. [Reporting details](docs/benchmark-reports.md).
[Profile-guided before/after results](benchmarks/studies/d57cb2e/README.md) preserve
all 60 native conditions, including gains, regressions and worker-tail tradeoffs.
The [kernel-guided I/O follow-up](benchmarks/studies/io-scheduling/README.md)
records receive/scheduling bottlenecks, measured fixes, large worker-tail reductions
and remaining regressions—not an all-workload performance claim. The
[single-CPU client I/O handoff study](benchmarks/studies/client-handoffs/README.md)
now records higher throughput and lower whole-batch p99 than Valkey in all 20
conditions of the latest five-workload CI cohort. Under an equal one-CPU budget,
rate leads are 8.2–10.8% at pipeline 1 and 34.8–54.6% at pipeline 16. This is scoped
evidence, not a universal performance or feature-equivalence claim; all matched
before/after regressions, CPU costs and raw records remain visible.

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

Bounded RESP2 and Redis inline-command TCP server, with binary-safe `PING`,
`ECHO`, `SET` with atomic `NX`/`XX`,
`GET`, `KEEPTTL`, `EX`/`PX`/`EXAT`/`PXAT` options; `GET`, `MGET`, `MSET`, `MSETNX`,
`DEL`, `UNLINK`, `EXISTS`, `QUIT`;
TTL/PTTL, expiry inspection/update/removal, SET/GET aliases and atomic
`INCR`/`DECR`/`INCRBY`/`DECRBY`; atomic single-database `FLUSHDB`/`FLUSHALL`,
strict empty-registry `SCRIPT FLUSH`/`FUNCTION FLUSH`, protocol `SHUTDOWN`,
`CONFIG GET`/`RESETSTAT`, `INFO` (including process-local connection/command/
key/field-expiration/pub-sub statistics and opt-in aggregate per-command
`commandstats` via `CONFIG SET commandstats-enabled yes`), bounded `SLOWLOG`
command timing, bounded `LATENCY` event series and logical
`MEMORY USAGE`/`STATS`/`DOCTOR` payload reporting. `CONFIG SET` supports
collection thresholds, `requirepass`, `maxmemory`, `maxclients`,
`maxmemory-policy`, slow-log threshold/length, runtime `appendfsync` and automatic
journal-compaction thresholds; script/function execution is not implemented.
Memory usage is an entry-size estimate, not allocator/RSS data.
See [string semantics](docs/strings.md) and
[multi-key string limits](docs/multikey-strings.md). The startup option
`--error-dialect redis|valkey` selects the verified MSETEX short-pair error wording
(default `valkey`); it does not change successful operations or claim a full
vendor compatibility mode.

Bitmap commands `SETBIT`/`GETBIT`/`BITCOUNT`/`BITPOS`/`BITOP` support binary
MSB-first bit access, BYTE/BIT ranges, and atomic AND/OR/XOR/NOT destinations.
`BITFIELD`/`BITFIELD_RO` add signed/unsigned fields, `#` offsets and
wrap/saturate/fail overflow policies. See
[bitmap/LCS/float limits](docs/string-algorithms.md).

Geospatial commands `GEOADD`/`GEOPOS`/`GEODIST`/`GEOHASH`/`GEOSEARCH`/
`GEOSEARCHSTORE` plus the legacy `GEORADIUS*` aliases store WGS-84 points as
sorted-set geohash scores, with radius/box searches, distance/coordinate/hash
options and atomic store destinations. Searches use a bounded linear scan rather
than a native neighbour index. See [geospatial limits](docs/geospatial.md).

Binary-safe lists, sets, hashes and sorted sets support creation, mutation,
removal, bounded random selection (`SRANDMEMBER`/`HRANDFIELD`/`ZRANDMEMBER`),
HGETDEL atomic field reads/removal, per-field hash TTL controls
(`HEXPIRE`/`HPEXPIRE`/`*AT`, `HTTL`/`HPTTL`, `*EXPIRETIME`, `HPERSIST`),
combined `HSETEX`/`HGETEX` field writes/reads with FNX/FXX and TTL clauses, set
algebra/store commands, list position and
multi-key pop (LPOS/LMPOP/BLMPOP), atomic and blocking list moves
(LMOVE/RPOPLPUSH/BLMOVE/BRPOPLPUSH), score ranges and rank/range inspection. Sorted sets add unified `ZRANGE` score/lex/reverse/limit
selection, `ZRANGESTORE`, `ZLEXCOUNT`, `ZREMRANGEBYLEX`, `ZRANDMEMBER` and
weighted union/intersection/difference algebra with store and cardinality
variants plus multi-key `ZMPOP` and blocking `BZMPOP`. `SORT`/
`SORT_RO` order lists, sets and sorted sets with `ASC`/`DESC`, `ALPHA`, `LIMIT`,
dynamic `BY` weights, `GET` output patterns and atomic `STORE`. Dynamic patterns
require all-key ACL access. See [SORT limits](docs/sort.md).
Collection writes preserve expiry metadata where Redis does, delete empty keys,
and stage the complete value before payload admission. See
[typed collections](docs/collections.md).

`ROLE` and `INFO replication` report the truthful standalone master state with
zero replicas and no fabricated replid or replica rows; replication itself is
not implemented. See [replication status](docs/replication.md).

`CLIENT ID/SETNAME/GETNAME/SETINFO/INFO/LIST/KILL` (with `MAXAGE` age filtering)
`/PAUSE/UNPAUSE` expose bounded per-connection
metadata and administrative connection controls; `INFO clients` reports the
registry size. `TIME` returns wall-clock seconds/microseconds and `OBJECT
ENCODING`/`REFCOUNT`/`HELP` expose actual storage representations and ownership;
IDLETIME/FREQ report explicit unsupported errors instead of invented metrics.
See [runtime introspection](docs/introspection.md) and
[client sessions](docs/client-sessions.md).

`PFADD`/`PFCOUNT`/`PFMERGE` provide bounded HyperLogLog cardinality estimates
with exact small-cardinality hashes and a dense 2^14-register sketch after 4,096
elements. Values remain ordinary string keys for expiry, keyspace and journal
handling; the private payload is not Redis's HLL encoding. See
[HyperLogLog limits](docs/hyperloglog.md).

A bounded stream subset supports `XADD`/`XDEL`/`XLEN`, consumer groups
(`XGROUP CREATE/SETID/DESTROY/CREATECONSUMER/DELCONSUMER`, `XREADGROUP` with
`COUNT`/`BLOCK`/`NOACK` and pending history reads), `XACK`/`XPENDING`,
`XCLAIM`/`XAUTOCLAIM`, `XRANGE`/`XREVRANGE`, `XTRIM`, XADD
`NOMKSTREAM`/`MAXLEN`/`MINID` trimming, blocking/non-blocking multi-stream
`XREAD`, `XSETID`, and `XINFO STREAM` (with `FULL`)/`GROUPS`/`CONSUMERS`. Pending
delivery state is volatile across restart.
See [stream limits](docs/streams.md).

Bounded `MULTI`/`EXEC`/`DISCARD`/`WATCH`/`UNWATCH` queue supported data
commands and execute them under one store-wide gate, with expiry-aware optimistic
invalidation, RESP2/RESP3 replies and a documented fail-closed version bound. See
[transactions](docs/transactions.md).

Channel, pattern and sharded-channel pub/sub
(`SUBSCRIBE`/`PSUBSCRIBE`/`SSUBSCRIBE`/`PUBLISH`/`SPUBLISH`/`PUBSUB`) delivers
bounded, at-most-once messages to live connections, with separate normal/sharded
namespaces, RESP3 push frames and ACL channel patterns. There is no cluster
hash-slot routing, keyspace notification integration, persistence or replication
of messages. Cluster/module/replica probing returns explicit unsupported replies;
see [cluster/module boundaries](docs/unsupported.md). See [pub/sub](docs/pubsub.md).

Keyspace/keyevent notifications publish changed-key events to normal pub/sub
channels under `K`/`E` and `g$lshztxe`/`A` flags, including bounded at-most-once
`expired`/`evicted` lifecycle events, with runtime `CONFIG SET` toggling.
Keymiss/new-key and `FLUSH*` event classes are explicitly unsupported, and
delivery is non-persistent. See [keyspace notifications](docs/notifications.md).

Optional payload-budget eviction (`--maxmemory-policy noeviction|allkeys-random|
allkeys-lru|volatile-random|volatile-lru|volatile-ttl`, runtime `CONFIG SET`) keeps
writes succeeding by evicting eligible non-target keys; the budget is still
key/value bytes, not RSS, and a volatile policy never removes persistent keys. See
[eviction](docs/eviction.md).

Opt-in append-only persistence (`--appendonly yes --dir /data`) writes a
RESP-command journal of changed-key final states with absolute TTLs, supports
`always`/`everysec`/`no` fsync policies, replays before listening, truncates only
an incomplete tail, fails startup on mid-file corruption and syncs joined shutdown.
`BGREWRITEAOF` performs a synchronous, full-state compacting rewrite (it briefly
holds the exclusive gate). `auto-aof-rewrite-percentage` and
`auto-aof-rewrite-min-size` configure maintenance-triggered use of that same
synchronous compaction. There is no RDB format, background/incremental rewrite,
stream-pending recovery or backup drill yet. See
[persistence](docs/persistence.md).

Access control starts with a single operator password for the `default` user
(`--requirepass-file`/`--requirepass`, `AUTH`, atomic `HELLO ... AUTH`, runtime
`CONFIG SET requirepass` rotation and a redacted `CONFIG GET`) or a Redis-style
multi-user ACL file (`--aclfile`) with hashed passwords, ordered
command/category rules, declared read/write key patterns and atomic `ACL SAVE`/`LOAD`.
`NOAUTH`/`NOPERM` are checked before store access and apply to live connections;
`%R~`/`%W~`/`%RW~` scope reads and writes per key pattern. Persistent/centralized
audit, brute-force controls and native TLS remain separate security gates; ACL
selectors group command/key/channel rules and channel patterns are enforced for
pub/sub.
`ACL GENPASS`, `ACL DRYRUN` and a bounded process-local `ACL LOG` support
credential operations and policy debugging. See
[security](docs/security.md).
Ordered mixed-protocol pipelines and fragmented requests; concurrent clients,
payload/connection/request limits and inactivity timeouts. Conservative request
bounds can be raised with finite `--max-bulk-bytes` and `--max-frame-bytes` options.
It is already multithreaded by default (one managed worker per client). Optional
`--threads 1` or `--threads N` selects exactly 1..64 shared I/O/execution workers
plus the accept/control thread; idle clients do not occupy an entire worker.

This is **not yet** a drop-in Redis replacement or a production service. It
supports a bounded `HELLO`/RESP3 session subset, common `CLIENT` metadata,
`SELECT 0`, disconnect-equivalent `RESET`, `CLIENT LIST`/`KILL` and honest client
statistics; single-password and multi-user ACL authentication are implemented.
Audit controls, native TLS, brute-force protection, RDB/background-AOF
interoperability, backup/restore drills, scripts/functions, sharded pub/sub,
keyspace notifications, replication, clustering, and real client-library
interoperability remain production blockers. RESP3 replies are limited to the current command
subset's strings, integers, errors, arrays and nulls. Multi-key string operations
are atomic under the store lock; MGET replies are capped at 2 MiB to bound
duplicate-key amplification. It binds to loopback by default. See
[client sessions](docs/client-sessions.md),
[multi-key string limits](docs/multikey-strings.md),
[security boundaries](docs/security.md),
[INVEST stories and EARS requirements](docs/requirements.md), and the
[production-readiness plan](docs/production-readiness.md) and
[observability counters](docs/observability.md) for exact compatibility
boundaries; a verified subset is not a drop-in claim.

## Run with Docker or Kubernetes

```sh
docker run --rm --init -p 127.0.0.1:6379:6379 ghcr.io/macel94/rillcache:latest
# Or: docker compose up -d
# Or: kubectl apply -f deploy/kubernetes.yaml
```

The thin multi-architecture image runs non-root, binds container port 6379 without
extra arguments, and has a built-in PING health check—no shell or redis-cli inside.
Append options normally, e.g. `--threads 4`, `--requirepass-file
/run/secrets/rillcache`, `--aclfile /run/secrets/users.acl` or
`--appendonly yes --dir /data` with a writable volume.
**Experimental: optional password/ACL-file authentication and opt-in append-only
persistence; no native TLS, RDB format, background compaction or replication.** Keep
authentication and network isolation in place for anything beyond loopback. See [container delivery](docs/containers.md)
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
window after progress, plus a separate 100 µs cooperative run-budget hint for
multiple lanes/clients. This trades CPU work for fewer premature sleeps, not
free CPU, kernel readiness or an OS fairness guarantee. Earlier four-worker,
one-CPU tail regressions motivated this separate budget; measure real traffic
rather than assuming more workers improve latency. Default threading is unchanged;
a guarded bind-time single-CPU hint lets busy clients briefly lend I/O to an
existing client thread while their owners wait. Quiet sockets return to blocking
owners; fixed modes, multicore/unknown hints and initially lone clients keep their
existing paths. This is bounded active polling, not readiness or free CPU.
[Ownership, resource costs and tests](docs/client-io-handoffs.md). Socket deadlines
and joined shutdown remain intact. Linux `make check`/`make ci` also run the full
default TCP suite on one allowed CPU when taskset is available.

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

## Original upstream evidence

[Original Tcl compatibility and native performance workflows](docs/upstream-ci.md)
run on `ubuntu-latest`: PR/manual Redis and Valkey Tcl controls followed by the
unchanged candidate selection, plus a separate manual-only, same-runner comparison
using pinned upstream `redis-benchmark`. Prerequisite blockers, exclusions,
unreached tests and invalid measurements remain visible failures—not passes or
full compatibility claims. Expensive execution stays in GitHub Actions.

## GitHub feedback and delivery

**`main` is PR-only, including generated reports and documentation.** The active
[ruleset](https://github.com/macel94/rillcache/rules/25863639) requires passing
**Tests and offline CI** and **Valkey performance gate** on an up-to-date PR.
Rillcache must have strictly higher median throughput than pinned Valkey in **all
20 tested cases**; any tie, loss or invalid/incomplete measurement blocks merge.
No admin/bot bypass or direct main pushes. [Exact policy and scope](docs/pr-quality-gate.md).

[CI](.github/workflows/ci.yml) runs the same `make ci` on PRs and main pushes,
cancels superseded runs, and publishes benchmark evidence. Successful main builds
also provide a Linux binary archive and SHA-256 checksum as Actions artifacts for
immediate testing. [Container delivery](.github/workflows/container.yml) separately
tests and publishes thin amd64/arm64 images to GHCR using the workflow token.
There is no automatic public deployment of this experimental server. Authentication
is opt-in; the packaged container is unauthenticated unless a password is supplied.

[Comparative benchmarks](.github/workflows/benchmarks.yml) run manually or weekly
with genuine Copilot review and open report PRs instead of pushing main. The
[mandatory PR comparison](.github/workflows/pr-quality.yml) is separate from the
fast offline path and uses a frozen base client/harness, alternating native
processes, three repetitions and both inherited/equal-one-CPU profiles. Shared
runner noise can block a PR; passing this scoped throughput policy is not a
universal performance, stable-hardware, full-compatibility or production claim.
