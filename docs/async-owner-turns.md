# Parallel sockets, one asynchronous data owner

`--io-mode auto` uses eligible multi-CPU Unix Tokio networking and retains Mio
on one effective CPU. Explicit `mio` and `tokio` remain available. This addresses
a serial networking bottleneck; it is not sharded storage, a universal scaling
claim or acceptance evidence. All40 protected comparisons/all14 contexts remain.

## Selection and actual resources

Eligible ordinary plaintext has no fixed-worker override, additional storage
owners, TLS listener, startup persistence, ACL file or synchronous audit sink.
Auto requires a known effective CPU count of at least two. Unsupported auto
configurations keep their existing frontend. Explicit Tokio retains its existing
mixed-TLS compatibility and cold-configuration fallback; explicit Mio retains
readiness. No command or benchmark pipeline chooses a different mode.

For eligible Tokio, N current-thread network runtimes use the once-captured
`available_parallelism` hint, clamped1..64. Runtime zero **accepts, serves local
sockets and owns the only `Rc<ShardExecutor>`**. N−1 remote network threads own
only sockets/session state, never Data. There is one additional bounded cold
service, not an accept-only runtime or another data-owner thread. One-CPU explicit
Tokio has one network runtime/cold service and direct local execution; one-CPU
auto remains Mio. Mixed TLS retains its separately counted compatibility owner.

Runtime zero preserves launcher affinity. Remote best-effort pins use distinct
allowed IDs without wrapping; failures retain OS scheduling. N is not a physical
core count or reservation. `SchedulingPolicy` and embedded benchmark JSON report
actual captured network count, shared acceptance, local/compatibility data, cold
service, `remote_dispatch` and `owner_turn_capacity`. External placement remains
launcher-owned and measured thread/CPU evidence is still required.

## Complete owned turns, not synchronous bridge calls

A remote connection acquires one of32 total owner credits **before** moving its
Session, input Vec and output Vec. A Tokio mailbox/oneshot wakes runtime zero and
returns those same owned values. No per-command frame copy, cold-worker hop or
synchronous Store wait is introduced for ordinary remote commands. A private
Session slot is empty only while awaiting the owned result; transfer does not
construct throwaway clients or change the original client ID.

Each turn parses/executes at most64 commands or reaches the existing soft64KiB
output threshold, then the owner yields to socket/lifecycle work. Complete
pipelines share a turn rather than one message per command; fragmented inputs
may need partial-input turns. Commands/EXEC use the same Session epilogue and
published executor: auth, observer/error accounting, expiry, multi-key atomicity,
RESP2/3, blocking state and transaction limits are not reimplemented. No Data
borrow crosses an await or thread. This remains one serial keyspace owner;
long atomic commands/control locks can delay all data work.

Input/read/connection/task limits remain; active task caps supplement bounded
routing mailboxes. No following command executes or new input read begins while
output is backpressured. One atomic reply can exceed64KiB; this is a soft batch
threshold, not a hard atomic-reply, transaction-byte or process-RSS limit. Existing
pub/sub queue/slow-consumer policy remains. Remote blocking polling uses owned
owner turns, while disk-capable ACL/paused work remains on the separately bounded
cold service; its compatibility inbox still runs on runtime zero.

## Cancellation, failure and joined lifetime

An admitted job owns its credit even if its response waiter disappears. Revoked
queued work is skipped before execution; a result cancelled during execution
cannot orphan pub/sub/client registration. Normal complete output is forwarded
before another input turn, preserving per-connection order and side-effect
backpressure. Every executed command still uses common accounting.

Shutdown closes socket controls before joins. Protocol SHUTDOWN has no success
ACK; an owned reply guard survives dispatch and remote scheduling until the
actual writer finishes (with a10ms write budget once scheduled). Socket closure
waits for that ownership, not an arbitrary global scheduling grace period.
External handle stop and failures interrupt the wait and close immediately.
Runtime zero keeps processing owner/compatibility inboxes until **all** remote
network workers finish, then drains owner/cold credit and joins both drivers.
Only afterward can local Data drop and the cold thread join. Synchronous joining
of a live remote thread while it awaits runtime zero would deadlock, so the root
awaits worker completion without blocking its executor.

Task panics publish fail-stop before further data turns; failure closes the
compatibility inbox rather than executing later requests. An earlier join error
does not skip other joins. A hung filesystem syscall/long atomic operation can
still delay shutdown; no hard real-time guarantee or detached-file-work substitute
is claimed. Ordinary network errors remain connection-local.

## Verification and limits

Deterministic regressions cover zero ordinary Store crossings and same-thread
owner construction/destruction with multiple network runtimes; one-envelope
pipelines/session identity/buffer reuse; full mailbox/cancellation credit; revoked
data mutations; command/output quanta; remote slow-reader side effects; remote
cold work using the bridge after stop; and protocol pipeline flush/join.
Existing binary/transaction/error/blocked/push/FD-reuse/half-close/topology wire
suites run this complete path. Cargo's unsafe prohibition and existing dependency
allowlist are unchanged: only already approved Tokio/Mio APIs are used here.

Historical Tokio single-runtime and Compio losses are preserved, not converted
into wins. Focused/all-mode tests do not replace full CI, release/mode validation,
a fresh matched advisory screen or current-head protected all40/all14 acceptance.
