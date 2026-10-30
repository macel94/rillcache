# Owner-local storage: first usable execution increment

Status: **one owner, not multi-owner/thread-per-core completion**. This document
owns the design and limits of this PR; review does not depend on another worktree.
It implements the one-worker step of the completed target design reviewed on
2026-10-30 (target document SHA256
`326db47d02465156f8011f4b7a557ba5c2daa7018074963f574e5d0b950a9add`).

## Story and boundary

As a user of the existing server, I can run its entire supported command surface
on one socket/data owner without a shared data lock or per-command data-message
round trip, while existing synchronous embedders and fallback modes remain usable.

This is not a runtime swap. It retains the existing Mio readiness reactor and
fixed-worker socket machinery, with complete session/engine wiring on this branch.
There is no dependency on an unpublished transport implementation, no new Rust
crate, unsafe code, custom allocator, affinity change, or benchmark-only mode.

## State and execution

- `ServerServices` contains the existing shared authentication/configuration,
  counters, observation, client administration and pub/sub services. Their locks
  are **not** claimed to have disappeared.
- `ShardExecutor` contains `RefCell<Data>` and an `Rc` marker. It is neither
  `Send` nor `Sync`. Data includes the existing entries, expiry/field-TTL indexes,
  versions/epoch, eviction policy and payload accounting; it is constructed,
  accessed and dropped on the executor's thread.
- All command families use this executor. Internal borrows are synchronous and
  scoped; no public API exposes a guard, reference into Data, or mutable Data.
  A borrow must end before effect delivery or file operations. The production
  direct path uses the built-in clock; injectable embedding clocks retain their
  existing synchronous behavior on the compatibility owner.
- The default Unix plaintext reactor directly owns an executor. A configured
  single fixed worker constructs its executor on that worker. Sessions use one
  common generic execution boundary and the same auth/ACL, protocol, observer,
  slowlog/error epilogue in direct and compatibility paths.
- Local data execution has no mailbox, transaction gate, shared data lock or
  global transaction counter. Existing socket/pub/sub channels are unrelated to
  data routing. The fixed-worker mode retains its existing accept thread; this
  change adds no data thread to its one-worker path.
- `ShardRouter` retains its fixed startup routing contract. The active layout is
  **one shard**, routing every key to zero. Seeded XXH3 routing is not advertised
  as a cryptographic HashDoS defense. There is no multi-core scaling claim.

## Atomicity and effects

One synchronous owner turn cannot interleave with another turn on that owner.
Ordinary commands and complete EXEC batches use that same execution boundary.
WATCH validation and the complete EXEC loop run without yielding; runtime EXEC
errors still do not roll back earlier successful commands. A remote EXEC is one
request, not independently queued constituent commands. Atomic multi-key commands
keep their existing validation/admission and argument/duplicate-key ordering.

Blocking operations retain their existing check/recheck and session polling
semantics. Waiting sessions never hold a data borrow or occupy an owner turn.
Expiry, mutation versions, payload accounting, eviction and effect capture use
the same implementations as ordinary operations. Notification configuration uses
an epoch so changing flags clears old queued lifecycle events on the next turn.
Eviction configuration is refreshed before execution and budget enforcement.

The journal's existing single global order, replay, errors and rewrite format
remain unchanged. Mutating effects are captured before the next owner operation;
notification/pub/sub ordering remains the existing contract. No new rollback,
crash-atomic transaction, distributed commit or cross-shard visibility promise
is introduced.

## External Store compatibility and retention

`Store` remains the thread-safe synchronous public API, including its constructors,
method/UFCS entrypoints and append-to-output behavior. It shares control services
and lazily starts one joined executor thread for external data operations. It
submits **complete owned operations and owned replies**, never Data leases.

External admission is serialized *before argument copying*. A one-message bounded
mailbox and one reply slot retain at most one admitted operation/reply; waiting
callers do not create queued bridge-owned input copies. Ordinary argument lists
are copied into one byte frame plus ranges. EXEC retains one admitted copy of its
existing command queue. An owner may be installed beside a socket loop before any
external owner starts; ownership cannot migrate after activation.

The historical direct Store API has no protocol frame-size limit. This increment
does not silently reject previously valid large API calls. Its memory contract is
**one admitted input copy plus one result**, in addition to caller-owned inputs
and output buffers, not a constant byte cap independent of caller input size.
Network paths retain their parser/frame, transaction-command-count, admission and
output-backpressure bounds. The existing MULTI queue has a command-count bound;
a stronger aggregate transaction-byte budget is not claimed here. Unrestricted
embedding inputs/caller threads are the embedder's responsibility. Tests pause
admission and the owner to verify copying cannot precede admission and concurrent
callers cannot accumulate retained bridge copies.

Owner/mailbox failures produce existing internal-error/empty/error outcomes,
request fail-stop and propagate a storage-owner failure to the server supervisor.
They do not resume service with partial Data. External shutdown closes admission,
finishes admitted work and joins the owner; Data does not leave its thread.

## Cold configurations and lifecycle

AOF, audit-file/syslog and `aclfile` configurations retain compatibility execution,
not the direct local path. Mixed plaintext+TLS explicitly starts the external
owner **before either frontend starts**. Both frontends retain its shared handle;
the owner is joined only after all frontend/client workers have finished. The
plaintext reactor cannot destroy an inbox or Data underneath an admitted TLS
request. Portable and multi-fixed-worker modes also use compatibility execution.
This is one data owner, not multiple independently executing owners.

Compatibility operations can synchronously wait for their owner, including file
work. ACL SAVE/LOAD retain their existing auth-lock/filesystem semantics. With no
ACL file they retain their existing errors. ACL GENPASS still reads at most 128
bytes from `/dev/urandom` as a synchronous **cold control-plane operation**, outside
any data borrow; it can block the socket thread. Control-service locks and OS file
calls can also delay shutdown. This PR does **not** claim that the entire worker,
all control commands, persistence, or the complete server are nonblocking.
Bounded asynchronous cold services and the final Tokio topology are later work.

The rejected whole-Data lease experiment is preserved, not used: its source patch
SHA256 is `2c94c7870b124c604498822f3281828f579ca2d0e1378df285ef4e99a91399fe`.
The original failed four-CPU run
[41035897482](https://github.com/macel94/rillcache/actions/runs/41035897482)
remains failed evidence. Its test-only head is not retried as this implementation.

## Acceptance and evidence

- **OWN:** While active, executor state shall be !Send/!Sync and shall be
  constructed/dropped on the same owner; requests shall never contain Data.
- **LOCAL:** When a local session executes, it shall use complete common dispatch
  with zero bridge submissions and no shared data/transaction lock. Counters and
  lifecycle probes exist only in test builds, not normal performance binaries.
- **SEM:** Existing all-mode command, transaction/WATCH, expiry/accounting,
  notification, blocking, persistence, TLS, observer and shutdown tests shall pass.
- **BOUND:** While admission/owner execution are paused, concurrent API callers
  shall retain only the single admitted bridge copy; binary/empty/large argument
  boundaries shall survive the copied frame.
- **STOP:** When a TLS operation is admitted and shutdown starts, joining shall
  wait for that operation and both frontends; closed or failed owners shall not
  panic public API callers or strand reply waits.
- **PERF:** Record exact source/diff/binary identities and matched normal controls.
  Advisory Rillcache A/B evidence is not either Valkey gate, a scaling study or a
  universal performance claim. Both unchanged protected gates remain mandatory:
  all 20 original and all 10 official four-CPU medians must strictly beat Valkey.
  Preserve losses and invalid evidence; do not retry unchanged candidates.

Multi-owner routing/reservations, cancellation, global budget credits, joined
asynchronous journal/cold services and scaling evidence remain required before
claiming the completed thread-per-core target. Independent fan-out, per-shard
quotas and whole-Data lending are not substitutes for that work.
