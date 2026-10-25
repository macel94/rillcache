# Kernel-guided I/O and scheduling (S19–S20)

Baseline: `7b5f08b7d18bf86052fd4f93a5e5d92277ef8a67`. This continues the
[earlier hot-path investigation](hot-paths.md); fewer user instructions did not
remove the default single-CPU pipeline-1 gap or oversubscribed-worker tails.

## Bottlenecks found before changing code

Separate kernel-requested `perf` profiles covered disposable Rillcache PING/SET,
four-worker PING and pinned Valkey 9.1.2 PING on the same logical CPU. A frozen
baseline Rust client validated replies. Separate `strace -f -c` captures counted
syscalls; instrumented elapsed times/rates were not used as speed evidence.
No firewall, CPU affinity of unrelated processes, scheduler/sysctl, perf access
policy or global scheduler-statistics setting was changed.

- Blocking PING showed queued-spinlock samples below `sk_wait_data`,
  `schedule_timeout`, scheduling and socket wake-up paths. PING does not use the
  key map, so more map tuning would not address this cost. About 11% of samples
  were the queued-lock slow path in this capture, not an expected wall-time gain.
- Firewall/connection-tracking and ordinary TCP work appeared in both Rillcache
  and Valkey. Those are host/network costs, not justification to disable security
  policy or equate a user-space optimization with eliminating kernel cost.
- The four-worker trace had 16,375 receive calls, including 5,560 EAGAIN results,
  for 10,808 sends including setup/warmup. Source inspection showed that an empty
  input buffer was read in one pass but dispatched only in the next, followed by
  another receive attempt. This added work even when a complete frame was ready.
- The earlier matched study exposed four workers sharing one CPU with roughly
  4.4 ms pipeline-1 batch p99. The 50 µs post-progress window was not a continuous
  run budget: real progress could keep rearming it, leaving other lanes to the OS
  timeslice rather than an application scheduling boundary.

These are diagnostic observations. VM sampling can skid across privilege
boundaries and perf itself introduces counter/MSR/context-switch overhead.
`/proc` runqueue-delay statistics were disabled; zeros were not treated as proof
of no waiting. Raw profiles remain separate from native elapsed comparisons.

## Focused changes

### Short-reply hint for a CPU-limited blocking server

At bind time, safe `std::thread::available_parallelism` supplies a capacity hint.
Only a one-CPU hint enables a cooperative yield after a single-command flush,
and only while another client is admitted. Batched flushes and lone clients do
not take this path. No store lock is held across the yield. It remains blocking
thread-per-client I/O, with the same socket deadlines, limits and joined shutdown;
no extra workers, unsafe code, FFI, allocator or Rust dependency were added.

An **unconditional** after-reply yield prototype was rejected: an initial screen
helped pipeline 1 but hurt batched throughput substantially. The narrower policy
is tested with singleton, batch and multicore controls. A yield is only an OS
hint; correctness never depends on another thread running or a fairness promise.

### Separate cooperative run budget for fixed lanes

Multiple lanes request a yield after 100 µs of continuous active passes when
multiple clients are admitted. Progress rearms the existing 50 µs active-poll
window but **does not rearm this separate run budget**. Parking resets the run
budget. A single lane or lone client avoids periodic fairness yields.

Checks occur between existing bounded passes, not at a hard real-time deadline.
The OS may ignore a yield. Existing per-client command/I/O quanta, admission,
backpressure, ready-work-before-idle behavior and shutdown remain invariants.

### Receive and dispatch ready input in one pass

A fixed worker with an empty input buffer first makes one bounded read. A
complete valid request can then execute and flush in that pass, without an
unnecessary next-pass round trip. Partial input uses the existing retained buffer;
malformed/truncated frames and QUIT retain their error/close ordering. Each pass
still makes at most one bounded read and executes at most 64 commands, with the
same output soft bound and backpressure behavior. This is not zero-copy storage,
readiness notification, epoll or a new I/O dependency.

## Exact policy provenance and validation

`ServerHandle::scheduling_policy()` returns the immutable policy captured at bind
time, and embedded benchmark JSON serializes that snapshot rather than resampling
capacity after measurement. Fixed-worker configuration adds `run_quantum_us`;
per-client configuration adds `single_reply_yield`. Native external configuration
remains launcher-owned/null and source/arguments/affinity identify its policy.

New mode evidence has a separate versioned `cooperative-v1 (required)` run-budget
marker; v1's 100 µs value is interpreted independently of future runtime tuning. Reporting rejects
missing/wrong new budgets while retaining old no-poll and poll-only cohorts
unchanged. Tests cover guard decisions, permit release, immutable snapshots,
progress-independent budget expiry, post-park reset, same-pass execute-once,
64-command fairness, partial frames, slow readers and shutdown in all modes.

The optional [native A/B workflow](../.github/workflows/hotpaths.yml) now freezes
`7b5f08b` and its client for comparisons against this increment. Preserve all five
workloads, three modes, pipelines 1/16, inherited/one-CPU budgets and three rotated
repetitions, alongside CPU, whole-batch p99, spreads and hashes. Fresh reference
comparisons and genuine attributed Copilot interpretation remain separate.
Local VM screens were highly variable and are not a release-performance verdict;
retain only changes supported by the matched published evidence, not the best
local sample. No all-workload or production-readiness claim follows.

Sources: [Rust yield semantics](https://doc.rust-lang.org/std/thread/fn.yield_now.html),
[capacity-hint caveats](https://doc.rust-lang.org/std/thread/fn.available_parallelism.html),
[Linux yield caveats](https://man7.org/linux/man-pages/man2/sched_yield.2.html),
[scheduler statistics](https://docs.kernel.org/scheduler/sched-stats.html),
[socket timeouts](https://man7.org/linux/man-pages/man7/socket.7.html).
