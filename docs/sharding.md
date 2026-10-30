# Shard ownership: routing and one-owner execution

Status: **usable one-owner execution**, not multiple executing shards. The owning
[implementation design and limits](plans/owner-local-storage.md) are included in
this PR; they supersede the earlier whole-store lease proposal.

`ShardRouter` supports power-of-two layouts in `1..=64`, with a process-stable
random seed and allocation-free routing of borrowed binary keys. The active
layout remains one shard and all keys route to zero. The seeded XXH3/finalizer is
not a cryptographic HashDoS guarantee; entry maps retain randomized hashers.

`ShardExecutor` owns `RefCell<Data>` and is !Send/!Sync. All command families use
synchronous owner-local execution. The default Unix plaintext reactor and the
single configured fixed worker directly execute local session requests, with no
data-channel hop or shared data/transaction lock. Shared control-plane locks
remain. Existing Mio/socket scheduling is retained; no Tokio/dependency setup or
unpublished sibling change is required.

The public thread-safe `Store` API is a bounded synchronous compatibility bridge:
complete operations run on its owner, and only owned arguments/results cross the
boundary. Data never travels in a message or returns to a caller. Admission
precedes argument copying. The API retains at most one admitted input copy and
reply, not a fixed byte budget independent of caller-provided input size. Existing
network input/admission/output limits remain in force.

AOF, audit and ACL-file configurations use compatibility execution. Mixed TLS and
plaintext frontends explicitly share an external owner that outlives both and is
joined after their workers finish. Portable/multi-fixed-worker modes also retain
compatibility execution. Cold control-plane operations, notably ACL GENPASS, can
still perform synchronous I/O outside data borrows. **The whole server is not
claimed nonblocking or thread-per-core complete.**

Transactions, WATCH, atomic multi-key operations, expiry, accounting, journal
ordering/replay, pub/sub and notifications retain their existing implementations
and semantics on the one owner. Blocking clients do not hold data borrows while
waiting. No cross-owner protocol, distributed transaction, new durability or
multi-core scaling guarantee is implied.

Verification includes type-level locality assertions, owner construction/drop and
local bridge-crossing probes, direct/compatibility dispatch equivalence, bounded
admission/copy tests, failed-owner behavior, deterministic mixed TLS shutdown, and
the repository's existing all-mode semantic suites. Both exact-head protected
Valkey gates remain required; advisory local measurements cannot replace them.

Next: a self-contained multi-owner mode must deliver deterministic partition
placement, ordered reservations, WATCH/blocking coordination, global accounting,
journal/control services and joined failure/shutdown together. Independent fan-out
and whole-Data lending are explicitly rejected.
