# Async plaintext transport increment (PR #143)

Status: usable async transport on the **published one-owner storage API**, not
complete multi-owner/thread-per-core execution or performance acceptance. The
storage implementation was published in PR #144 (`9a4df76`); this PR owns its
complete transport wiring without an unpublished sibling dependency.

## Story and target sequencing

As an operator, I can use default Unix async plaintext sockets without blocking
socket calls or synchronous Store-bridge waits on their runtime, preserving the
complete existing command surface, bounded admission and joined shutdown.

Relevant target-design decisions are reproduced here for self-contained review:

1. Transport may be delivered independently, with honest storage scope. Remove
   blocking disk work from runtimes or retain the proven fallback. Count network,
   accept, data-owner and cold-service threads separately. Verify one effective
   CPU, fairness, byte backpressure, cancellation and joined stop.
2. A one-worker socket/data owner must execute ordinary local commands directly,
   without a data-channel hop or a data/transaction lock. Keep the proven
   one-owner mode as default until complete multi-owner execution passes. Never
   introduce an unnecessary data thread behind each I/O thread.
3. Multi-owner promotion requires atomic multi-key/EXEC/WATCH, blocking semantics,
   global accounting, bounded control/data/bytes, journal order and joined
   failure/shutdown together. Independent fan-out and whole-Data lending are not
   substitutes. No partial command subset default or unpublished sibling wiring.
4. Throughput/scaling claims need matched evidence, CPU/tails and every loss,
   plus both unchanged protected gates. Runtime replacement is not a speed claim.

## Local-owner path and effective CPU accounting

Published `Store` is now a synchronous compatibility bridge, not the former
lock-backed keyspace. Calling it directly from Tokio would block a runtime.
Accordingly, ordinary eligible plaintext preserves the published **one-owner
default**: one current-thread Tokio runtime owns both its sockets and
`Rc<ShardExecutor>`. Acceptance shares runtime zero. `Session::execute_on` and
blocked-command polling use the complete common executor directly. All command
families, observers, auth/ACL, transactions, expiry and accounting retain their
published implementations. No Data borrow crosses an await.

`ShardExecutor` contains `RefCell<Data>` and is !Send/!Sync. It is installed,
accessed and dropped on the socket thread. A `LocalSet`/local JoinSet prevents its
migration. Ordinary network commands make zero bridge submissions; an inherited
regression verifies complete EXEC/error semantics, zero crossings and same-thread
owner construction/destruction. This is one executing shard, not a sharded server.
Shared control-plane locks remain.

The one-owner choice is a semantic reconciliation with published storage, not a
benchmark-only switch. One network runtime is within the effective CPU budget;
it does not claim to use every allowed core. The previous N-runtime/shared-Store
implementation and failed measurements remain in history. Multi-owner execution
is still separate work.

Mixed native TLS retains one external shared data owner for both frontends.
Its plaintext side may use N current-thread Tokio network runtimes, where N is
captured once by `available_parallelism` before any affinity changes and clamped
to 1..=64. Rust 1.99.1 Linux uses the minimum discoverable affinity/cgroup quota;
this is a hint, not a reservation or physical-core count. Only N−1 remote network
threads are added; acceptance shares runtime zero. Each connection is assigned
once. Complete compatibility Session dispatch, blocked/paused polling and data
maintenance go through the bounded cold service **outside** network runtimes;
the published Store bridge never synchronously waits on an async runtime.

Thread accounting:

| Configuration | Network runtimes | Cold service | Additional data owner | Other frontend threads |
| --- | ---: | ---: | ---: | --- |
| Eligible ordinary plaintext | 1, including acceptance | 1 | 0 | none |
| Mixed TLS plaintext compatibility | N, including acceptance | 1 | 1 shared with TLS | existing TLS accept/client threads |
| Persistence/ACL-file/audit, fixed lanes, non-Unix | existing fallback | no Tokio cold service | published compatibility/local-mode policy | existing accept/workers/TLS as configured |

`SchedulingPolicy` captures network count and compatibility selection. Embedded
benchmark JSON explicitly reports `network_runtime_threads`,
`accept_runtime_shared`, `cold_io_service_threads`, `storage_execution` and
`compatibility_data_owner_threads`; external placement remains launcher-owned.

Runtime zero preserves its calling thread's **launcher-owned affinity mask**,
including when it is the CLI process leader. Only remote compatibility network
workers attempt distinct allowed logical IDs via safe `core_affinity`, without
modulo wrapping. Missing/failed pins retain OS scheduling. Nonfatal diagnostics
record this policy and remote results. No cpuset/quota is rewritten.

## Disk boundary, cold service and bridge lifetime

AOF, ACL-file and synchronous audit file/UDP configurations retain the proven
non-Tokio plaintext path. ACL SAVE/LOAD holds an auth write lock across file I/O;
moving only that syscall would still indirectly block runtime authorization.
Startup ACL/key/certificate loading happens before runtimes start. Native TLS
retains its existing socket machinery and published compatibility owner.

Eligible async paths have one bounded joined cold service for ACL (GENPASS reads
`/dev/urandom`) and paused resumption; compatibility paths also execute complete
Session operations there. One async permit is acquired **before** copying a
frame or moving session/reply state, and is owned by the admitted job until it
finishes, even if its reply receiver disappears. At most one extra owned frame
is retained, bounded by `max_frame_bytes`. There is no unbounded blocking pool.
Ordinary local requests and EXEC do not use this service. Existing Session
queueing rejects ACL inside MULTI; its error/EXECABORT contract is unchanged.

The local owner's external bridge inbox is serviced in bounded turns on that
same runtime (1 ms scheduling target) for cold paused resumption. This is not an
ordinary local-command hop. Its driver stays alive through socket-task joins
and drains outstanding cold admission before Data is destroyed, including when
a cold reply waiter was cancelled. The cold thread is then closed and joined.
Mixed TLS's shared external owner outlives both frontends and their cold work.

The local expiry pass removes at most 128 keys on a 100 ms scheduling target.
Compatibility data maintenance runs on the cold service. No async accept loop
calls synchronous journal sync/rewrite or waits on Store data operations.
Shared control locks, long atomic commands, fallback clients and a hung OS file
syscall can still delay progress/shutdown; no hard real-time deadline is claimed.

## Admission, bytes, fairness and stop

- Explicit active-task caps supplement the registry's max-client admission and
  32-socket remote mailboxes. Mailboxes retain sockets/session registrations, not
  parsed data frames. At most one pending accepted socket is retained when all
  lanes are full; it is retried round-robin, not dropped on capacity return.
  Existing healthy tasks and lifecycle checks continue during saturation.
- Per-connection input is capped by `max_frame_bytes`; reads are at most 16 KiB.
  Existing Session/MULTI limits remain; no stronger aggregate transaction-byte or
  process-wide network-memory budget is invented.
- Dispatch stops after 64 commands or a 64 KiB output batch, yielding between
  buffered quanta. One existing atomic reply/push may exceed 64 KiB: this is a
  **soft batch threshold, not a hard reply-size or total RSS limit**. No following
  command executes and no input is read while that reply is backpressured.
  Writes use 64 KiB chunks/yields and one configured total write deadline.
- Pub/sub retains existing bounded message queues and slow-consumer policy;
  message capacity is not a 64 KiB byte claim. Protocol-error drain has one total
  deadline, not a renewed deadline per incoming chunk.
- Stop closes acceptance and registered socket controls before joining tasks,
  dropping pending/queued registrations, joining remote workers, draining cold
  work/bridge lifetime, and joining services. Panics propagate; an earlier join
  error does not skip other joins. Aborting a waiter does not cancel file work.

## Tests and evidence

Deterministic tests cover tiny local/remote capacity, retained pending clients,
healthy progress, slow-reader pipeline side-effect suppression, service
saturation with same-runtime healthy GET, task failure, owner lifetime and
all-mode binary/protocol/blocking/pubsub/TLS/persistence/audit semantics.
Subprocess tests verify inherited mask preservation, one-CPU 1+1 thread count,
and separately counted mixed-TLS compatibility workers within their cpuset.

The pre-main `b85d2d2` head passed 13/14 contexts, including all 20 original
medians. Its official four-CPU run
[41053623142](https://github.com/macel94/rillcache/actions/runs/41053623142)
completed **8/10**, losing PING/P1 and NX-existing/P1. It is not retried or reused
as acceptance for reconciled source. Earlier numeric loss 41039159717 and setup
affinity failures 41049312936/41049312759 also remain preserved.

Two local advisory cohorts remain visible, with all raw rows/CPU/tails and source
identities in the owning worktree ledger and PR evidence. The first 72-sample
cohort overlapped full CI and is causally inconclusive (including SET/P1 −26.2%
and PING/P16 −41.8%). The read-timer diagnostic found one new timeout per P1 read
versus 16× amortization at P16; its reusable-timer candidate reduced construction
to one per connection. But its matched normal-release screen failed its
predeclared rule: detected cargo/version-tool overlap and only 1/3 paired NX/P1
rate wins. That candidate is preserved and **reverted**, not promoted or called
a throughput win. Test-only diagnostic counters are absent from normal builds.

Fresh `make ci`, live protection and all 14 protected contexts remain required
on the up-to-date exact source. All **20 original** and **10 official four-CPU**
medians must strictly beat Valkey; no loss, stale pass, bypass, or unchanged retry
qualifies. This is transport delivery, not completion of the full target.

Sources: [published owner design](plans/owner-local-storage.md),
[Rust capacity implementation](https://github.com/rust-lang/rust/blob/58cf8f9c3/library/std/src/sys/thread/unix.rs),
[Tokio 1.54.1 LocalSet](https://docs.rs/tokio/1.54.1/tokio/task/struct.LocalSet.html),
[protected PR gates](pr-quality-gate.md).
