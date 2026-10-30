# Async plaintext transport increment (PR #143)

Status: transport implementation over the existing **shared `Store`**. This is
not shard-owned execution, lock-free storage, a multi-core scaling result, or
performance acceptance. The independent storage/ownership migration is not a
prerequisite for this usable increment.

## Story and boundary

As an operator, I can serve default Unix plaintext TCP through actual async
sockets within the effective CPU budget, without a slow socket, saturated
handoff, or synchronous filesystem work stalling every network task.

This implements the transport stage of the target architecture. Its relevant
sequencing decisions are reproduced here so review does not depend on another
unpublished worktree:

1. Deliver actual async plaintext reads/writes while retaining shared storage;
   remove disk work from runtimes or keep those configurations on the proven
   fallback. Count accept, network, and service threads separately. Test one
   effective CPU, fairness, byte backpressure, cancellation, and joined stop.
2. True owner-local execution is separate: construct data on its worker, never
   transfer/lend it, and dispatch local ordinary commands directly without a
   data-channel hop or global transaction/data lock. Do not introduce a dedicated
   data thread behind each network runtime or a partial command subset default.
3. Complete multi-owner execution must include atomic multi-key/EXEC/WATCH,
   blocking semantics, bounded control/data/byte admission, accounting, journal
   order, administration, and joined shutdown before promotion. Independent
   fan-out is not atomic execution. Never depend on an unpublished sibling.
4. Promotion/scaling claims require fresh matched 1/2/4-CPU evidence, including
   hot-key/cross-owner costs, CPU, tails and regressions, plus both unchanged
   protected Valkey gates. A runtime swap alone is not evidence of a win.

## Execution and thread accounting

`Server::bind` captures `std::thread::available_parallelism()` once, before any
pinning. The pinned Rust 1.99.1 Linux implementation takes the minimum of the
calling thread's allowed affinity and discoverable cgroup CPU quotas, including
ancestor quotas. Other platforms retain the standard library's documented
limits. Discovery is a hint, not a reservation or physical-core count; unknown
capacity selects one runtime. The network budget is clamped to 1..=64.

Runtime zero accepts connections **and** runs local client tasks. Only N−1
additional current-thread Tokio network threads are created. Each connection is
assigned once; ordinary command dispatch is synchronous `Session`/shared-`Store`
execution on that runtime, not a channel round trip per command. Shared data and
transaction locks can still block runtimes; this is not ownership isolation.

There is **one additional, bounded, joined cold-I/O service thread**. ACL commands
(including GENPASS's `/dev/urandom` read) and paused-command resumption execute
there with the original Session observation/authorization epilogues. Ordinary
requests and EXEC do not use it. Existing Session queueing rejects ACL commands
inside MULTI, so EXEC cannot perform ACL file I/O; this rejection and EXECABORT
behavior are regression-tested, not silently changed.
A single async permit is acquired before copying a cold frame or moving session/
reply state. The job owns the permit until completion, even if its receiver goes
away. At most one cold operation retains an extra frame (bounded by
`max_frame_bytes`); there is no unbounded blocking-pool/task queue. This
compatibility service is not a global database actor.

Thus one effective CPU has one network runtime plus one cold service, not two
network runtimes. `Server::spawn` uses its managed accept thread as runtime zero;
CLI `run` uses the calling thread. `SchedulingPolicy::tokio_worker_threads`
reports **all network runtimes including acceptance**, not the service count.
Embedded benchmark metadata additionally reports `network_runtime_threads`,
`accept_runtime_shared: true`, and `cold_io_service_threads: 1`; external-server
placement remains launcher-owned, not inferred by the load generator.
TLS, when configured, separately retains its existing accept thread and managed
client threads. Fixed-worker mode is unchanged and separately accounted.

Distinct allowed logical CPU IDs are attempted once per network worker using
safe `core_affinity`; IDs are never modulo-wrapped. Startup stderr records each
attempt and success/failure. A missing ID or failed pin leaves OS scheduling in
control. Affinity does not imply physical-core independence or a performance win.

## Disk/maintenance fallback

Append-only persistence, ACL-file configuration, or synchronous audit file/UDP
sinks select the existing managed-client plaintext path instead of Tokio. Their
synchronous append, sync, rewrite and audit behavior is preserved until proper
offloading is implemented. ACL SAVE/LOAD holds the auth write lock during file
I/O: merely moving the syscall to another thread would still block runtime
authorization, so ACL-file configurations deliberately stay on fallback. Without
an ACL file, SAVE/LOAD return their existing NoAclFile error without disk I/O;
GENPASS does not hold a transaction/data/auth guard while reading randomness.
This is an intentional configuration boundary, not a hidden disk-isolation
claim. Non-Unix
plaintext and native TLS also retain their previous socket machinery. Startup
ACL/key/certificate loading occurs before network runtimes are started.

The async accept loop only performs the existing bounded in-memory expiry pass
(128 keys, 100 ms scheduling target). It does not call journal sync or rewrite.
The shared store may still serialize work, including work from fallback/TLS
clients. Neither disk latency nor long atomic commands acquire a new hard
real-time bound.

## Admission, bytes and fairness

- The client registry's configured max-client admission bounds total plaintext
  registrations. Each worker task set is explicitly capped by the startup
  `max_clients` value; a 32-socket routing mailbox alone is not a task bound.
  A runtime increase of CONFIG maxclients does not grow this startup task bound;
  excess admitted sockets backpressure routing until capacity returns.
- Remote mailboxes contain sockets/session registrations, **not parsed request
  frames**. They are capped at 32 entries each. At most one pending accepted
  socket is retained by the acceptor when all lanes are full. It is retried
  round-robin without loss; no further socket is accepted until it is routed.
  Maintenance, worker failure and stop are checked at most each 10 ms wait,
  subject to scheduler/command slack. Existing healthy tasks remain runnable.
- Each connection input is hard-capped by `max_frame_bytes`, with reads of at
  most 16 KiB. Deferred/MULTI state retains the existing Session limits. There is
  no unbounded queue of owned data requests on this transport path.
- Dispatch stops after 64 commands or a 64 KiB output batch, and yields between
  buffered quanta. An existing atomic command/push can exceed 64 KiB; this is a
  **soft batch threshold, not a hard maximum reply size or total RSS bound**.
  Atomic reply size remains determined by existing command/data/frame limits.
  No following command executes and no further input is read while that reply
  is blocked. Writes use at most 64 KiB chunks with yields between chunks and a
  total configured write deadline. One slow reader cannot accumulate its whole
  pipeline of replies in userspace.
- Pub/sub retains the existing bounded message queue and slow-consumer policy;
  message capacity is not misrepresented as a 64 KiB byte bound. Push size is
  constrained by existing request/command limits. This increment does not add a
  process-wide network-memory reservation mechanism.

## Cancellation and failure

On stop/error: close acceptance; set the shared stop flag; shut down registered
socket control clones; drop the pending accepted socket and queued registrations;
join every local task and every remote worker; close/drain the cold queue; join
its service. Worker/task failures propagate, while ordinary socket errors stay
connection-local. A failed join never skips joining the remaining tasks/threads.

Already admitted cold jobs finish; dropping an async waiter is not cancellation
of a filesystem syscall. There is no hard shutdown deadline for a hung kernel
filesystem call. Tests use a controlled delayed service to verify that the
network remains runnable and stop does not silently detach the service.
Protocol-error draining has one total deadline rather than a new deadline per
chunk, preventing an indefinitely streaming peer from holding teardown open.

## Acceptance and evidence

Requirements R-TOKIO-NET1–9 are in
[policy and transport](requirements/policy-and-transport.md). Tests include:

- tiny per-worker task limits and full remote mailboxes: pending-client retention,
  healthy-client progress, capacity return, and joined stop with idle clients;
- a one-runtime slow reader: later pipeline mutation is not executed while its
  large preceding replies are blocked;
- bounded cold-service saturation: healthy GET (which takes the data/transaction
  gate) progresses on the same sole runtime, not just gate-bypassing PING;
  ACL-in-MULTI rejection/EXECABORT and task panic preserve semantics and joins;
- Linux subprocess restricted to one allowed CPU: exactly the main network
  runtime plus one cold thread, GENPASS/pipeline correctness, and protocol stop;
- existing all-mode binary/fragmented pipelines, blocked clients, pub/sub,
  persistence, audit, ACL, TLS, and shutdown suites.

`make test`, `make ci`, and the live policy audit remain required before review.
Hosted acceptance remains all **20 original** and **10 official four-CPU** strict
median wins on the up-to-date exact head, with all 14 live-required contexts.
Prior four-CPU run [41039159717](https://github.com/macel94/rillcache/actions/runs/41039159717)
failed 5/10: every P=1 case lost while every P=16 case passed. It is preserved,
not retried or relabeled. Async networking does not fix a failed gate by definition.

The subsequent local 72-sample advisory control/candidate cohort overlapped a
full CI build in another worktree on the same host. It is explicitly
**contention-affected and inconclusive for causal throughput claims**, not a
controlled speedup result. All rows and raw samples are retained, including
one-CPU SET/P1 −26.2% and PING/P16 −41.8%. Under its predeclared rejection rule,
that screen does not support performance promotion. Host interference prevents
attribution, not preservation of losses; no winning-sample selection or
unchanged required-gate retry follows. The PR evidence reports the entire table.

Sources: [Rust capacity API](https://doc.rust-lang.org/1.99.1/std/thread/fn.available_parallelism.html),
[version-matched Linux implementation](https://github.com/rust-lang/rust/blob/58cf8f9c3/library/std/src/sys/thread/unix.rs),
[Tokio 1.54.1 bounded channel](https://docs.rs/tokio/1.54.1/tokio/sync/mpsc/index.html),
[protected PR gates](pr-quality-gate.md).
