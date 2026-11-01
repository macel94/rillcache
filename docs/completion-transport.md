# Linux completion TCP

`--io-mode auto` selects safe Compio 0.19.2 owned-buffer TCP on eligible Linux
configurations when an io_uring runtime can initialize. This is one socket/data
owner, **not** multi-core storage or a universal throughput claim. All original
Valkey, four-CPU Valkey and four-CPU Dragonfly comparisons remain mandatory.

## Selection and fallback

| Selection | Behavior |
| --- | --- |
| `auto` (default) | Prefer completion on eligible Linux; otherwise retain the existing frontend |
| `mio` | Explicit existing Unix readiness frontend |
| `compio` | Require eligible Linux and available io_uring; report unsupported instead of silently falling back |
| `tokio` | Explicit existing async frontend and its documented compatibility fallback |

Completion eligibility requires no explicit fixed workers, one storage owner,
no TLS listener, no startup append-only journal, no ACL file and no synchronous
audit file/syslog sink. Fixed workers, multiple storage owners, mixed/native TLS,
those disk/sink configurations and non-Linux retain their existing paths. An
unavailable/denied ring does not require a kernel, security or host-setting change.

Capability detection uses one short **joined startup-only probe thread**, not the
calling thread: tearing down a temporary ring on an embedded caller can interrupt
its unrelated blocking socket I/O. The serving runtime is constructed on its own
socket/data owner. Neither probe nor serving runtime changes launcher affinity.
A bind-time success is not a promise that later runtime allocation cannot fail;
startup errors still propagate rather than claim a backend that did not start.

## Ownership and bounded work

One local Compio runtime owns acceptance, client futures and `Rc<ShardExecutor>`.
Ordinary commands and blocking polling use the same complete `Session` epilogue
and executor, without a compatibility data-channel hop. No keyspace RefCell
borrow crosses an await; neither executor nor Data moves to the cold service. Existing authorization,
observers, RESP2/3, transactions/WATCH, expiry and error/payload accounting remain.

The steady application topology is one network/data runtime plus one bounded
cold-service thread, not an extra data owner or an accept-only runtime. Captured
`SchedulingPolicy.completion_transport`, startup diagnostics and embedded benchmark
JSON distinguish completion from readiness; measured process/thread evidence is
still required for actual resource claims. Kernel/runtime implementation internals
are not claimed to use no auxiliary resources.

- Each completion owns its `Vec` until the result returns it. Input is limited by
  `max_frame_bytes`, with reads of at most 16 KiB; write chunks are at most 64 KiB.
- Dispatch yields after 64 commands or the soft 64 KiB output batch threshold.
  One atomic reply/push may exceed that threshold. No later command executes and
  no new input read begins while that output is backpressured. This is not a hard
  atomic-reply, aggregate transaction-byte or process-RSS bound.
- Active registrations and task storage are both capped by the client limit;
  admission rejection has one bounded reply attempt and no new connection task.
- One cold permit precedes any extra owned frame/session transfer. Its holder
  lives through completion even if a waiter disappears. ACL and paused resumption
  run outside the network runtime; admitted work can use the same owner's bounded
  compatibility inbox. There is no unbounded blocking-task queue.
- Existing bounded pub/sub queues remain. A 1 ms control/async-delivery target,
  100 ms expiry target (at most 128 keys), ordinary-session inactivity/total-write
  deadlines and a 10 ms stop-check target are scheduling targets, not real-time
  guarantees. Long atomic commands, control locks or a hung filesystem syscall
  can still delay progress or shutdown. Waiting subscriptions, monitors, blocked
  and paused sessions keep their existing idle-timeout exemption; stalled writes
  still have a total deadline.

## Cancellation and shutdown

A periodic timeout must **not** drop a normal owned read or accept: the kernel may
already have committed bytes/a connection before its completion is delivered.
Control ticks borrow and retain those futures. Async replies can progress while
one input operation remains pending; there is no read cancellation every tick.
When unblocking must resume already-buffered commands, a single-read cancellation
token recovers the initialized prefix before continuing. A token never accumulates
operation keys over an entire long-lived connection.

Protocol-error draining uses only disposable owned buffers and one total drain
budget. Shutdown/failure closes registered socket controls before joining client
futures and explicit socket closes. The bridge and its local executor remain alive
until admitted cold credit drains, then the control task and cold service join.
Task panics publish fail-stop before another local command turn. Ordinary network
errors are connection-local. Dropping task handles is not the normal shutdown path.

## Tests and dependency boundary

Native tests cover retained accepts across ticks, owned-prefix cancellation,
caller-I/O/affinity isolation, cold-service saturation with bridge use during
shutdown, and slow-reply side-effect suppression/joining. Wire suites include
binary fragmentation, half-close, FD reuse, deferred pushes/blocked replies,
already-buffered blocked/paused pipelines, limits and joined shutdown. Force native
coverage on a Linux host with an available ring:

```sh
RILLCACHE_REQUIRE_COMPIO=1 cargo test --locked --offline --lib server::compio_io::tests
RILLCACHE_TEST_THREADS=compio cargo test --locked --offline --test server --test readiness --test network_modes
```

Unavailable native cases explicitly report that limitation; requiring Compio
makes it a failure, not a hidden skip. Automatic/fixed/fallback behavior remains
in the ordinary all-mode checks. Focused tests do not replace `make ci`, release/
mode checks or protected performance acceptance.

The only new direct exception is Linux Compio `=0.19.2`, default features off,
with runtime/net/io-uring/time and required pinned vendored transitives. Existing
locked versions are unchanged. Application `unsafe_code = "forbid"`, no handwritten
FFI/build script/allocator and the all-target dependency allowlist remain. Safe
API internals in approved dependencies are outside the application-code boundary.
