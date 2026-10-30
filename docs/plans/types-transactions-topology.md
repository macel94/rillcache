# Typed storage, transactions and topology: dependency plan

**Status: design only.** Nothing in this plan is implemented or implied to be complete. The current compatibility boundary is the string-only, in-memory RESP2 subset in [requirements](../requirements.md); this document proposes INVEST-sized next work. It deliberately does not claim Redis drop-in compatibility.

## Current architecture and constraints

`src/engine/data.rs` stores `HashMap<Vec<u8>, Entry>`, where `Entry` is a byte `Vec` plus an optional millisecond deadline. `Data` owns one exact `(key length + string value length)` payload counter and a `BTreeSet<(deadline,key)>` expiry index. `Data::{reserve,put,remove,set_expiry,reap}` maintain those invariants. `src/engine.rs::Store` wraps the entire `Data` in one `RwLock`; command methods in `src/engine/commands.rs` hold a read or write lock while they inspect/mutate a command, but release it before socket output. That single write lock currently gives each implemented command atomicity, not a multi-command transaction. Expiration is logically checked on reads and reclaimed in bounded maintenance; the injected `Clock` is an important deterministic-test seam.

RESP2 parsing in `src/protocol.rs` accepts bounded arrays of non-null bulk strings and borrows their bytes. `Store::execute(arguments, output)` has no client/session parameter. Thread-per-client connection state is local to `server::serve_client`; fixed-worker state is in `server::pool::Client`, which is transferred during bounded scheduling and may be moved through `server::handoff`. A per-connection transaction or subscription must therefore live with the connection in *both* modes and travel with `Client` handoffs; it must not be inferred from a worker/thread ID. Store locks are never to be held over socket I/O, blocking waits or a client-controlled delay. All Rust remains safe, bounded, joined on shutdown, and free of unreviewed external dependencies; the reviewed vendored exceptions are Tokio, core_affinity, Mio and rustls with required transitive crates.

The current payload counter is intentionally **not RSS accounting**. Typed storage must define its logical admission charge before adding types. A practical compatible extension is key bytes plus owned user payload bytes (field/member/value/stream bytes), with separately bounded metadata/index overhead; do not label that an absolute memory bound. All checked arithmetic, allocations/admission planning, type checks, expiry changes and writes must preserve old state on pre-execution errors/OOM. Type indexes must be updated atomically with their canonical data.

## Dependency DAG

```text
RESP2 response-value encoder + command metadata/key extraction (D0)
                  │
                  ▼
Typed Entry + shared key lifecycle/accounting + WRONGTYPE (D1)
        ┌─────────┼───────────────┬────────────────┐
        ▼         ▼               ▼                ▼
 Hashes (T1)   Lists (T2)      Sets (T3)       Sorted sets (T4)
                  │               │                │
       bounded blocking scheduler  │        numeric/order contract
                  └───────────────┴────────────────┘
        │         │               │                │
        ├─ Bitmaps (T5; string-like byte semantics)
        ├─ HLL (T6; probabilistic structure + deterministic test seam)
        └─ Geospatial (T7; sorted-set encoding/range/numeric boundaries)

D0 + D1 + per-connection state + key mutation generations + one-lock EXEC core
                  │
                  ▼
MULTI/EXEC/DISCARD (X1) ─── WATCH + expiry invalidation (X2)
                  │
                  └─────────────── requires every mutation path to be covered

D0 + connection-state/push-output scheduler + bounded subscriber queues
                  │
                  ▼
Pub/Sub (P1) ── independent of D1 value types, but not of connection/session plumbing

D1 + stream IDs/order/indexes + bounded block scheduler
                  │
                  ▼
Streams (S1) ── consumer groups/PEL/claim/ack (S2)

D0 + atomic command execution + read/write/key declarations + CPU/memory limits
                  │
                  ▼
Script runtime decision (L0) ── EVAL (L1) ── Functions/library lifecycle (L2)

Typed canonical mutation journal + snapshot format + durable recovery
                  │
                  ▼
Replication handshake/offset/backlog + snapshot/PSYNC (R1)
                  │
            ┌─────┴──────────┐
            ▼                ▼
   Sentinel failover (R2)  Cluster slots/client redirects/migration (R3)
```

**DAG notes.** D0 is a small command-boundary refactor, not a public extension API promise. D1 is the shared prerequisite for all value-type stories; no family should invent a second per-type lock/accounting/expiry implementation. Hashes, sets, sorted sets and bitmap basics can be separate modules after D1. Lists can be stored independently after D1, but blocking list commands must wait for a cancellation-aware bounded wait/notification scheduler. Geospatial commands should follow the sorted-set score/order and numeric-parsing contract (or prove an independent design); HLL needs an explicit representation/accuracy/serialization contract. Streams are their own ordered log type and do not depend on list implementation, but blocking reads depend on the same resource-safe scheduler. Pub/Sub does not require typed values, but cannot be safely implemented as a command-only module: it requires session mode, push framing, bounded delivery queues, output fairness and disconnect cleanup. Transactions require D0, D1 and all commands to run in a single isolation boundary; they cannot be composed by calling today’s lock-taking `execute` repeatedly. WATCH correctness cannot be claimed until *every* mutation/expiry/eviction path changes versions. Scripts/functions depend on transaction-grade execution and resource budgets; a real Lua-compatible runtime is currently blocked by the dependency policy, whose only exception is vendored rustls. Replication/topology are blocked on a canonical typed mutation stream and persistence/recovery design; they are not just socket commands added to the current dispatcher.

## Ownership and interface boundaries

| Boundary / owner | Proposed responsibility | Must not do |
| --- | --- | --- |
| `src/protocol.rs` (protocol owner) | Keep RESP framing/limits and borrowed input. Add RESP2 array/nested-array response encoding/value representation only through separately reviewed, bounded increments. | Parse commands into lossy UTF-8; permit unbounded replies; couple framing to engine data structures. |
| `src/engine/data.rs` (shared storage owner) | Own `Value`/typed `Entry`, key expiry, logical payload charges, admission planning, replacement/removal, expiry reaping, mutation version/journal hooks. One key lifecycle regardless of value family. | Expose mutable map entries to command modules; split accounting by type without atomic aggregate admission; count allocator RSS as payload. |
| `src/engine.rs` (dispatch/integration owner) | Parse command semantics, validate before mutation, dispatch to type-specific methods and translate typed outcomes to wire responses. Coordinator owns final integration. | Leave a new module unreferenced or bypass wrong-type/lifecycle checks. |
| `src/engine/<family>.rs` (one family owner per story) | Implement cohesive family semantics against narrow storage operations; own that family’s local indexes and invariant helpers. | Edit common storage/dispatch concurrently without coordination; retain unchecked aliases to stored mutable values. |
| `src/server.rs` + `src/server/pool.rs` + `src/server/handoff.rs` (connection owner) | Add bounded `ConnectionState` owned by each client; thread-per-client keeps it in the serve loop; pool puts it in `Client`; handoff transfers it with the rest of `Client`. Add push/wait scheduling only in explicit stories. | Key client state by worker, block a fixed worker indefinitely, or hold a store lock while waiting/writing. |
| tests/reference corpus (story owner) | Isolated unit tests plus ephemeral-port wire tests in per-client, 1-worker and 4-worker modes; optional pinned Redis/Valkey differential corpus. | Shared mutable test fixtures, timing sleeps as correctness, or treating missing server features as success. |
| persistence/topology (future designated owner) | Define format, journal ordering, recovery and fault contracts before command integration; separate network protocol from store internals. | Reuse in-memory `HashMap` layout as an implicit persistence format or assert HA without partition/failover evidence. |

### Shared typed-entry contract (D1)

Use one closed internal type tag/value boundary, conceptually `String(Vec<u8>)`, `Hash(...)`, `List(...)`, `Set(...)`, `SortedSet(...)`, `Hll(...)`, `Stream(...)`; geospatial data may initially share sorted-set representation only if Redis score/member semantics and metadata stay coherent. Keep `expires` on the key entry, not in each value. A type-specific command presented with another type returns the exact Redis `WRONGTYPE Operation against a key holding the wrong kind of value` error and performs no mutation. Missing keys follow each command’s Redis creation/no-op rule. Empty collection deletion and type replacement must remove expiry metadata and reclaim the key’s charge exactly once. Commands that convert strings (for example bit operations) require separate upstream-compatible decisions rather than silently treating every `Value` as bytes.

Storage access should be operation-oriented: validate/get typed immutable view; calculate a complete proposed mutation and checked logical charge; atomically apply data, expiry index and mutation-version update under the single store write guard. Avoid returning references beyond the guard. For commands touching multiple keys, validate types, options, numeric ranges, slot-independent key conditions, and all admission costs before any write; then mutate all keys atomically or leave them unchanged. Where Redis itself specifies partial per-element behavior, document/test that instead of applying blanket rollback assumptions.

## INVEST stories with EARS acceptance

Sizes are relative and intentionally small; they are not delivery estimates. `T` stories are not one combined “all types” increment.

### D0 — response and command boundary (S)

**Value:** Later transaction results can represent arrays and command metadata/key access can be reviewed without embedding key semantics in every feature.

- **When** a supported command returns a RESP2 scalar or array reply, **the server shall** encode exact nested framing with checked lengths and bounded output.
- **If** output would exceed the configured response budget or a length overflows, **then** the server shall fail the command/connection according to an agreed error policy without allocating unbounded memory.
- **When** the engine resolves a command, **it shall** expose explicit command traits/key positions and argument validation outcomes for transaction queueing and later key tracking; binary key bytes remain unchanged.
- **If** a command has dynamic/unknown key positions, **then** WATCH/journal/cluster eligibility shall fail closed rather than guessing.

**Evidence:** response boundary/property tests at zero, decimal boundaries and configured max; malformed/overflow tests; same wire tests in all modes. No invented success response.

### D1 — typed Entry, lifecycle, accounting and WRONGTYPE (M; foundation, no user-facing rich type required)

**Value:** A single auditable foundation for types and future atomic commands.

- **When** an existing supported string command encounters a non-string value, **the server shall** return exact WRONGTYPE and preserve value bytes, TTL, charge and indexes.
- **When** a mutation replaces/deletes/expires any typed value, **the store shall** update the key expiry index, value charge and mutation version atomically exactly once.
- **If** validation, checked charge arithmetic or budget admission fails, **then** the store shall preserve the complete prior typed value and deadline (except semantics explicitly defined as side effects before execution by Redis).
- **While** the store is under concurrent reads/writes in per-client, one-worker and four-worker modes, **the store shall** expose no mutable stored-value borrow outside its guard and remain binary-safe.
- **When** lazy expiration or bounded reaping removes a key, **the store shall** reclaim its charge and notify mutation tracking consistently without relying on a maintenance sleep for visibility.

**Evidence:** model-based interleavings; zero/empty/binary payload; max/overflow budgets; failed overwrite preserves bytes and TTL; expired type reclaimed without false OOM; expiry index cardinality equals expiring-key count; all-mode protocol tests; debug and release.

### T1 — hashes (S)

- **When** HSET/HGET/HMGET/HGETALL/HEXISTS/HLEN/HDEL/HINCRBY are supported for a hash key, **the server shall** preserve arbitrary binary field/value bytes, field/value pairing, reply shapes and specified count semantics; repeated fields update once and return the documented number of newly added fields. HGETALL iteration order shall not be promised unless Redis specifies it.
- **If** the key has another type, integer parsing/overflow fails, arity/options are invalid, or admission fails, **then** no unintended field/value or accounting change shall occur; HINCRBY shall preserve existing TTL.
- **When** the final field is removed, **the server shall** remove the key and expiry index as Redis specifies.

**Evidence:** binary/duplicate fields; missing-vs-empty values; HGETALL field/value pairing and allowed reply behavior versus pinned Redis and Valkey (do not assert unspecified ordering); signed 64-bit boundaries and malformed integer corpus; WRONGTYPE, OOM, TTL and concurrent HINCRBY; all modes.

### T2 — lists and blocking list commands (M split: list core S; blocking S/M)

- **When** LPUSH/RPUSH/LPOP/RPOP/LRANGE/LLEN/LREM/LSET (selected bounded subset) execute, **the server shall** match left/right ordering, negative index/trim boundaries and empty-key deletion on binary elements.
- **If** a list command sees a wrong type or invalid index/arity, **then** it shall return upstream-compatible error with no partial mutation.
- **When** BLPOP/BRPOP is later enabled, **the server shall** honor argument-key priority, FIFO waiter service for a contended key, fractional timeout semantics, null timeout reply and disconnect/cancellation; **while** clients wait, unrelated clients shall continue in all worker modes.

**Evidence:** lengths 0/1/2, duplicates, all index boundaries, key-priority across multiple nonempty keys, two queued waiters, timeout 0/fractional/deadline, disconnect, stop/join with waiters, slow consumer/backpressure. Do not implement blocking calls in `Store::execute` by sleeping while holding its lock.

### T3 — sets (S)

- **When** SADD/SREM/SISMEMBER/SCARD/SMEMBERS/SMOVE/SINTER/SUNION/SINTERSTORE (scoped selection) execute, **the server shall** treat members as binary values, suppress duplicates, match integer counts, and produce documented deterministic/reference-valid reply behavior.
- **If** any multi-key input has a wrong type or output admission fails, **then** destination and source sets shall remain unchanged unless Redis documents different behavior.
- **When** a source/destination key is repeated or is the same key, **the server shall** match Redis reference behavior and charge the destination only once.

**Evidence:** binary/duplicate members, empty sets, self-move and duplicate keys, output ordering comparison (set order is not an application order), atomic multi-key OOM/type failure, TTL and randomized-hash safety.

### T4 — sorted sets (M)

- **When** ZADD/ZREM/ZSCORE/ZCARD/ZRANGE/ZRANK/ZCOUNT execute, **the server shall** preserve binary members, IEEE-754 score parsing/range rules, score then binary lexicographic tie order and documented rank/reverse/range boundaries.
- **If** a score is NaN/infinite/invalid or option combinations conflict, **then** the command shall fail atomically without changing score, ordering indexes, TTL or charge.
- **When** a member score is updated, **the server shall** update every ordered index exactly once and return counts honoring NX/XX/CH semantics.

**Evidence:** negative/zero/positive, equal scores, signed zero/NaN/infinity corpus, score precision boundaries, binary lex ordering, inclusive/exclusive ranges, reverse order, duplicate updates, random index/model invariants, OOM rollback and Redis/Valkey oracle.

### T5 — bitmaps (S, after string-vs-typed conversion decision)

- **When** GETBIT/SETBIT/BITCOUNT/BITPOS/BITOP selected commands operate on byte strings, **the server shall** use Redis MSB-first bit numbering and bit-operation semantics, including implicit zero extension and byte-range boundaries.
- **If** offset multiplication/length or destination charge overflows/exceeds configured budget, **then** no destination or source mutation shall occur.
- **When** BITOP writes to a destination that aliases a source, **the server shall** compute the result from pre-command source values and apply it atomically.

**Evidence:** offsets 0/7/8, empty strings, all bit values, ranges/negative indexes, alias cases, high-offset limits, OOM unchanged, reference corpus. Must decide if strings remain a separate `String` value variant and how type checks for bit commands behave.

### T6 — HyperLogLog (S/M)

- **When** PFADD/PFCOUNT/PFMERGE are supported, **the server shall** document and test Redis-compatible command/reply behavior, stable cardinality error bounds and bounded memory admission.
- **If** merge/type/admission validation fails, **then** no input sketch shall be changed and destination semantics shall match Redis.
- **When** tests run, **they shall** use deterministic element sets and statistical acceptance over fixed seeds, never demand exact cardinality from an approximate algorithm.

**Evidence:** duplicate/idempotent add reply, empty/missing keys, overlapping/disjoint cardinality, merge, deterministic serialized state decision, bounded memory and repeated distribution tests against Redis tolerance. Exact algorithm/encoding interoperability remains an explicit scope decision.

### T7 — geospatial (M; after sorted-set numeric/order contract)

- **When** GEOADD/GEOPOS/GEODIST/GEOSEARCH selected commands execute, **the server shall** validate finite longitude/latitude bounds, preserve binary member names and match units, distance rounding, radius/box filtering and requested ordering.
- **If** invalid coordinates, malformed unit/options, or nonfinite numeric results occur, **then** the command shall produce the pinned Redis result/error and preserve all prior members.
- **When** geospatial metadata is represented using sorted-set scores, **the server shall** keep GEO operations interoperable with documented zset-visible membership/score behavior where applicable.

**Evidence:** exact bounds, antimeridian/high-latitude cases, zero distance, unit conversion, member tie/order, WITHCOORD/WITHDIST/WITHHASH, binary members, bad numeric values and native reference comparisons. Approximation algorithms need explicit tolerance.

### X1 — MULTI/EXEC/DISCARD connection-local queue (M)

- **When** a connection enters MULTI, **the server shall** queue eligible commands, reply QUEUED, and execute the bounded accepted queue only on EXEC, returning one ordered RESP array reply per queued command.
- **When** DISCARD is received, **the server shall** drop the queue and restore normal connection state without running any queued command.
- **If** a command is malformed/unknown/not queueable or queue command/byte bounds are exceeded, **then** the server shall mark the transaction dirty or reject according to Redis’s queue-time rule, return EXECABORT where required, and run no queued effects.
- **When** a queued command has a runtime error (including WRONGTYPE/OOM), **the server shall** return that element’s error and continue later queued commands; it shall not claim rollback. All commands in a successful EXEC shall have no other client command interleave.
- **While** a queue is open, only that connection’s bounded session owns it; a disconnect/drop/shutdown shall discard it. Transaction state shall survive fixed-worker reassignment and I/O handoff with the connection.

**Evidence:** order and visibility across clients; two-command write/read; runtime error followed by successful command; syntax/unknown/arity queue failure; nested MULTI, DISCARD, empty EXEC, disconnect, queue bounds, QUIT; concurrent barrier test proving no interleaving; wire array framing; all worker modes and handoff.

**Implementation boundary:** do not invoke current `Store::execute` recursively under its `RwLock` (deadlock) or execute commands one by one under separate locks (interleaving). Factor command execution into lock-aware validated operations and hold the store write guard over the entire EXEC; reply encoding and output budget must be reserved/checked to prevent a partial externally visible execution due to reply allocation failure. No socket I/O under the lock.

### X2 — WATCH and expiry invalidation (M; only after X1 and D1 mutation hooks)

- **When** WATCH names one or more keys, **the server shall** capture their mutation generations, including absent keys; duplicate watch keys shall not create unbounded duplicate state.
- **If** a watched key is created, updated, deleted, expired, evicted, or changed by another transaction before EXEC, **then** EXEC shall abort with Redis’s null response and execute none of its queued commands.
- **When** an expiration deadline passes, **the server shall** invalidate a watch based on logical expiration even if background reaping has not run; clock rollback shall not resurrect an already observed expired key.
- **When** the watching client itself queues mutations, **the server shall** not invalidate its watch merely by queueing them; mutation occurs only at EXEC and Redis semantics determine subsequent watch state.
- **If** any write or expiry path lacks a generation update, **then** the feature is not ready for release; generation wrap/ABA shall be prevented by checked monotonic generations or a safe epoch design.

**Evidence:** deterministic injected-clock tests for exact deadline boundary/no sleeps; absent-key creation; replacement/delete/expire/persist, lazy read and reaper paths; concurrent mutation barrier; all data-type mutation paths; expiry index and watch invalidation under OOM; version overflow policy; pinned Redis comparison. WATCH support is blocked until all store writers are audited.

### P1 — Pub/Sub basic channels (M; blocked on session/push output)

- **When** a connection subscribes/unsubscribes, **the server shall** send the correct RESP2 subscription acknowledgement and maintain exact channel counts; a subscribed RESP2 client shall accept only the documented allowed command subset.
- **When** PUBLISH is issued, **the server shall** deliver binary channel/message bytes to currently subscribed clients and return the number of matching subscribers.
- **If** a subscriber disconnects, times out, becomes backpressured or is shut down, **then** it shall be removed without a leaked registry reference or unbounded queued messages.
- **When** a message cannot be delivered due to disconnect/backpressure, **the server shall** not claim durable delivery; compatibility is at-most-once. Pub/Sub is not a Streams substitute.

**Evidence:** multiple subscribers, duplicate subscribe, unsubscribe all, binary payload, PUBLISH count, publish/disconnect races, RESP2 command restrictions, bounded slow-reader queue policy, connection cleanup in all modes, shutdown/join. Pattern/sharded Pub/Sub are separate stories. Implement shared subscriber registry + bounded per-client mailboxes and a fair output scheduler; never synchronously write to peers under registry/store locks.

### S1 — Streams core; S2 — consumer groups (M each)

- **When** XADD/XRANGE/XREVRANGE/XLEN/XDEL/XTRIM selected commands execute, **the server shall** preserve stream ID ordering, monotonic/explicit ID validation, binary field/value pairs, trim behavior and bounded retention.
- **If** an ID is malformed/non-monotonic or a stream key has another type, **then** the server shall return the reference error and preserve log/index/accounting.
- **When** S2 adds XGROUP CREATE/XREADGROUP/XACK/XPENDING/XCLAIM, **the server shall** track last-delivered IDs and per-group pending ownership/idle/delivery counts consistently; acknowledgement/removal/claim shall update PEL and stream state atomically.
- **If** blocked XREAD/XREADGROUP is enabled, **then** it shall use bounded cancellable waiter scheduling and not stall a worker or hold a storage guard.

**Evidence:** generated ID monotonicity at clock rollback/same millisecond; binary repeated fields; trimming with pending IDs; group creation from `$`/`0`; pending redelivery, ack, claim timeout, consumer cleanup; recovery/expiry policy; blocked disconnect/shutdown; randomized model checks and Redis/Valkey differential suite. Streams cannot be represented as a list of strings without losing IDs/consumer metadata.

### L0/L1/L2 — programmability decision, EVAL, Functions (XL / unresolved)

- **Before** accepting script syntax, the project shall make and document a human-reviewed runtime/compatibility decision that resolves the dependency policy (only vendored rustls is allowed), Lua version/standard library, deterministic behavior, and security boundary. A hand-written “Lua-like” subset shall not be advertised as EVAL compatibility.
- **When** a supported script/function runs, **the server shall** execute atomically with declared KEYS/ARGV, exact result/error conversion, replication/journal semantics, bounded instruction/CPU and memory use, and no filesystem/network/OS escape.
- **If** a script exceeds resource limits or runtime errors, **then** the server shall terminate safely with Redis-compatible errors and leave state according to Redis script semantics; it shall not let a client wedge every worker indefinitely.
- **When** Functions are added, **the server shall** define library load/replace/persistence/replication and cluster key rules separately from volatile script cache semantics.

**Evidence/gate:** threat model and independent security review; adversarial infinite loops, allocation exhaustion, deeply nested input, binary args, atomic writes, deterministic replication, cancellation limits, poisoned interpreter state, restart/reload, cross-version corpus. Current dependency policy is a blocker, not permission to introduce a dependency or claim a fake no-op.

### R1 — Replication/PSYNC (XL; after durability/journal)

- **When** a replica attaches, **the server shall** authenticate/authorize according to the accepted security design, negotiate protocol/version, establish replication identity/offset, and transfer a consistent snapshot plus subsequent ordered changes without gaps/duplicates.
- **When** a known replication ID/offset remains in bounded backlog, **the server shall** resume partial synchronization; otherwise it shall select a documented full-sync snapshot path and continue buffering changes during transfer.
- **If** snapshot transfer, disk/network I/O, checksum, version negotiation or replica apply fails, **then** the replica shall not expose a silently partial dataset; recovery shall restart/resync or fail visibly.
- **When** asynchronous acknowledgements/WAIT are in scope, **the server shall** describe their exact durability/visibility guarantee; no synchronous durability claim follows from an acknowledgement count alone.

**Evidence:** deterministic mutation journal order across strings/types/transactions/expiry; interruption at each snapshot/journal boundary; partial/full sync and backlog rollover; duplicate/out-of-order/corrupt records; master restart/replica restart; mixed supported versions; partition/rejoin and data divergence; soak and bounded backlog/RSS. Requires stable snapshot/restore and a canonical write stream first.

### R2 — Sentinel/failover (XL; separate deployment/control-plane story)

- **When** monitored master failure is suspected, **Sentinel-compatible control plane shall** distinguish quorum failure detection from majority authorization and publish a versioned promoted-master configuration only after a valid election.
- **If** network partition, stale configuration, flapping node, split vote or no promotable replica occurs, **then** clients/operators shall receive a clear unavailable/stale outcome; two writable primaries shall not be represented as safe automatic failover.
- **When** failover completes, **the system shall** test client discovery/reconnect and document acknowledged-write loss limits from asynchronous replication.

**Evidence:** multi-process/multi-host fault harness, packet loss/delay/partition, quorum versus majority cases, leader loss, stale epochs, replica promotion, reconnect, split-brain observation, restart and long soak. Sentinel requires working replication plus authenticated control plane; embedding a single-process timer is not Sentinel.

### R3 — Cluster slots, redirects and migration (XL; after key hashing/journal/replication)

- **When** a cluster command is routed, **the node/client contract shall** calculate the 16,384-slot mapping including Redis hash tags and return correct MOVED/ASK redirections during stable and migrating/importing states.
- **If** a multi-key operation spans slots, **then** the server shall reject it with the proper CROSSSLOT-style response before mutation; same-slot operations shall preserve atomicity.
- **When** slots migrate/fail over, **the system shall** preserve bounded availability and avoid duplicate/lost keys under documented async replication limits; it shall support topology discovery and refresh without forged version metadata.
- **If** mixed software versions disagree on hash, command, replication or migration format, **then** deployment gates shall block upgrade until compatibility is proven.

**Evidence:** hash vectors/hashtag edge cases, all slot boundaries, redirect loops/stale maps, ASKING state, same/cross-slot transactions and type commands, concurrent reshard/migration, node/replica failure, partition/rejoin, mixed-version rolling upgrade and load soak. Do not begin until the single-node command/key metadata surface and replication are stable.

## Realistic next wave and sequencing

1. **D0 response/key metadata boundary**, only the minimum needed for later session and transaction framing.
2. **D1 typed-entry/accounting/WRONGTYPE foundation**, without claiming rich-type compatibility; preserve all existing string/TTL behavior.
3. **T1 hashes** as the first independently useful type, with T3 basic sets in a separately reviewed follow-on if D1 survives tests. Do not start every type lane simultaneously against unstable accounting interfaces.
4. Reassess measured product demand, test costs and payload budget policy; then choose one of T2 nonblocking list core, T4 sorted sets, or T5 bitmaps. Blocking lists/streams are not in this small next wave.
5. Only after connection state and response arrays are established, consider X1; X2 follows only after a complete mutation/expiry audit. Pub/Sub and blocking features require their own server scheduling project and must not be bundled into X1.
6. Streams, HLL and geospatial each need explicit fidelity/resource decisions. Script runtime, persistence, auth/TLS, replication and HA remain separate roadmap decisions; no implementation begins by assuming dependencies or deployment guarantees that current policy excludes.

A parallel type-module lane is safe only after D1’s exact internal API and file ownership are frozen. Separate owners can then implement `src/engine/hash.rs`, `set.rs`, etc. and their family tests against it, but must not concurrently alter `data.rs`, common dispatch, storage accounting, the protocol codec, or server state. Hashes and sets are the best first independent modules; geospatial is coupled to zset numeric ordering, blocking list/stream operations are coupled to server wait scheduling, and WATCH is cross-cutting across every writer.

## Cross-cutting failure, security, and release gates

Every story must define maximum arguments, input bytes, response bytes, queued transaction bytes/commands, collection cardinality/work, index overhead and waiters before implementation. Arithmetic uses checked operations in debug and release; errors and OOM must not partially mutate data. Test duplicate keys/members/fields, aliasing, empty/binary payloads, integer/float boundaries, stable ordering, exact expiration visibility, and output allocation failure policy. Test in per-client, one-worker and four-worker configurations using ephemeral ports and managed server handles; do not use readiness sleeps. Fault tests must cover disconnect during partial frame/reply, slow readers, saturated queues, shutdown while waiting, worker panic/failure, partial start and joined cleanup. Never hold store/registry locks while waiting or on socket I/O.

Before any production-facing claim: security threat model and independent human/security review (authentication/authorization, transport boundary, denial-of-service, script execution, replication credentials); fault injection for process crash, disk full/corruption, dropped/duplicated/reordered packets, partitions and restart; extended mixed workload/expiry/client saturation soak with bounded RSS, queues, threads and recovery time; defined service SLOs for availability, command latency tails, durability RPO/RTO, memory and upgrade/rollback; supported Redis/Valkey/client/version matrix; backups/restore and operational runbooks; current reference test corpus and reproducible controlled-hardware comparisons. The existing Valkey merge throughput gate remains a code-quality gate, not an SLO or compatibility proof. Human review is required for design/security and evidence.

Unresolved: exact logical charge for type metadata and stream/group state; maximum response and EXEC reply reservation; locking strategy once collection reads/writes grow; key-generation overflow/ABA policy; blocking waiter fairness/cancellation on the current API; push-output integration with RESP2 restrictions; script runtime versus no-dependency rule; persistence format/crash guarantees; authentication and native TLS under current dependency constraints; eviction policy and whether eviction is in scope at all; replication durability/acknowledgement semantics; cluster client responsibilities and mixed-version matrix; support level for Redis 8+ command/encoding extensions. Resolve each before its dependent story is estimated or marked supported.

## Official upstream references

Links below are official Redis documentation, reviewed for the semantic constraints above (documentation is versioned as “latest”; a future implementation must pin/test an explicit Redis and Valkey release corpus):

- [Command reference](https://redis.io/docs/latest/commands/)
- [HSET](https://redis.io/docs/latest/commands/hset/), [HGETALL](https://redis.io/docs/latest/commands/hgetall/), [WRONGTYPE command examples](https://redis.io/docs/latest/commands/hget/)
- [BLPOP](https://redis.io/docs/latest/commands/blpop/) (key priority, waiter order, timeout, transaction interactions)
- [ZADD](https://redis.io/docs/latest/commands/zadd/), [ZRANGE](https://redis.io/docs/latest/commands/zrange/)
- [BITOP](https://redis.io/docs/latest/commands/bitop/), [PFADD](https://redis.io/docs/latest/commands/pfadd/), [GEOSEARCH](https://redis.io/docs/latest/commands/geosearch/)
- [Transactions](https://redis.io/docs/latest/develop/using-commands/transactions/), [WATCH](https://redis.io/docs/latest/commands/watch/), [EXEC](https://redis.io/docs/latest/commands/exec/), [MULTI](https://redis.io/docs/latest/commands/multi/), [DISCARD](https://redis.io/docs/latest/commands/discard/)
- [Pub/Sub](https://redis.io/docs/latest/develop/pubsub/) (RESP2 subscribed-state restrictions, at-most-once delivery)
- [Streams](https://redis.io/docs/latest/develop/data-types/streams/), [XREADGROUP](https://redis.io/docs/latest/commands/xreadgroup/), [XACK](https://redis.io/docs/latest/commands/xack/), [XCLAIM](https://redis.io/docs/latest/commands/xclaim/)
- [Lua scripting](https://redis.io/docs/latest/develop/programmability/eval-intro/), [Redis Functions](https://redis.io/docs/latest/develop/programmability/functions-intro/)
- [Replication](https://redis.io/docs/latest/operate/oss_and_stack/management/replication/) (replication IDs/offsets, backlog partial sync, full sync, asynchronous acknowledgement)
- [Sentinel](https://redis.io/docs/latest/operate/oss_and_stack/management/sentinel/) (quorum detection, majority authorization, configuration epochs)
- [Cluster specification](https://redis.io/docs/latest/operate/oss_and_stack/reference/cluster-spec/) (16,384 slots, hash tags, MOVED/ASK, migration, async replication limits)

These upstream docs describe Redis, not capabilities of Rillcache. The local current limits, dependency/safety policy and production gates remain authoritative for project scope.
