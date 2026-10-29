# CPU-limited client I/O handoffs (S21–S22, S-IO4)

## Historical single-CPU result

The [complete study and raw evidence](../benchmarks/studies/client-handoffs/README.md)
identify measured source `0157177`. Its latest reference CI cohort leads Valkey
in median throughput and whole-batch p99 in all 20 five-workload conditions.
Equal-one-CPU rate leads are 8.2–10.8% at pipeline 1 and 34.8–54.6% at pipeline 16.
The separate matched A/B study reports both improvements and regressions; those
runner/cohort percentages must not be blended. This does not establish all-traffic
performance, feature equivalence or production readiness.

## Small-multicore policy v7

V3 enabled bounded handoffs for known two-, three- and four-CPU bind-time
capacity hints. V4's unconditional two-coordinator experiment at four logical
CPUs completed with [5/10 wins](https://github.com/macel94/rillcache/actions/runs/40794219718),
not merge acceptance. On the two-core/four-thread EPYC runner its PING P1 median
was 99,022 versus Valkey's 113,922 req/s, with worse p99 and about 21 server CPU
seconds per ten-second sample. More busy pollers did not solve that bottleneck.
This failed source/evidence retains its actual version.
V5's SMT/backoff follow-up reached [7/10](https://github.com/macel94/rillcache/actions/runs/40796958078),
with three small P1 losses remaining; it is also retained as failed evidence.

V6 retains one temporary coordinator at 1–3 CPUs, and also at four CPUs while SMT
is active or unknown. Two groups are allowed at four CPUs only with an explicit
bind-time Linux `smt/active` value of zero. This is a conservative machine-wide
hint, not an affinity-specific physical-core count, CPU-model allowlist or host
setting change; unavailable/non-Linux/malformed status falls back to one group.
Admission rotates connections between the selected groups; each connection keeps
its group across leases, independently of session IDs or other disconnects.
All groups use the same shared store and transaction locks: this is I/O ownership
partitioning, not data sharding or thread pinning.

After an empty multicore scan, processor hints back off from 1 through at most
64 `spin_loop` instructions, resetting immediately on real progress. This avoids
hammering empty socket reads while clients/SMT siblings need execution resources.
There is no added OS yield/sleep and no rearming of the idle/maximum epoch on an
empty scan. V6 additionally defers one rescan of a normal leased client after an
actual no-progress poll, clearing the hint before every decision so it cannot
skip consecutive passes without another empty poll. Buffered input, pending
output, EOF/closing, blocked/paused, subscription and MONITOR states are never
deferred. A quiet borrower still polls ready I/O before its existing idle return;
epoch/shutdown checks still run on every pass. This avoids repeated empty socket
syscalls in partly busy scans, not just wholly empty scans. It is not a readiness
notification or a fixed-time latency guarantee. One-CPU v2 and explicit fixed
workers do not use either multicore backoff.

V7 then raises the multicore quiet window to 100 µs while leaving the one-CPU
window at 50 µs. At the measured pipeline-1 rate the eight clients issue roughly
one request every 65–80 µs per connection, so a 50 µs window returned each client
just before its next request and rebuilt the lease for every reply. The longer
window keeps a busy client in the coordinator across that interval. The 100 ms
maximum epoch, managed thread-per-client admission and absence of multicore
single-reply yields are unchanged. No permanent worker is added. One-CPU
scheduling remains v2 with one group and a 50 µs window. Fixed lanes, hints above
four or unknown capacity, and TLS transport retain their previous paths. More
concurrent I/O can spend more CPU; two coordinators are not an efficiency or
linear-scaling guarantee.

The motivating official four-CPU cohort loses all five pipeline-1 medians while
winning all five pipeline-16 medians. Worker profiles identify TCP/syscall and
wake/scheduling cost rather than command dispatch as the main bottleneck. The
extension amortizes ready-client I/O using the existing bounded polling path;
profiles are diagnostics, not performance acceptance. Fresh unchanged 20-case and
10-case Valkey gates must pass before claiming acceptance of this source.

A leased connection now carries its actual bounded push receiver with its socket,
buffers and session. The original worker waits without receiving. Unread queued
messages return with that same receiver; MONITOR and subscription state must not
be paired with a disconnected substitute. Tests cover pushes during a lease,
unread messages at return and subsequent owner-side delivery, without duplicates.

## Where Valkey still won

The prior validated [CI reference cohort](../benchmarks/reports/40230615995-1.md)
used application source equivalent to baseline `6cea8cc`. Under an equal single
logical CPU at pipeline 1, default Rillcache trailed Valkey throughput in all five
workloads by roughly 7.8–9.5%. For SET, Rillcache delivered 52406 commands/s versus
57330, using 19.2 versus 17.5 sampled CPU µs/command. At pipeline 16 Rillcache
usually led throughput, but all its batch-p99 values were worse than Valkey's.
The higher inherited-budget rates were not equal-per-core wins.

The default path still had roughly one matched-thread context switch per command
at pipeline 1. Its 92–94% system-CPU share meant that more key-map tuning could
not explain or remove the whole gap. PING, which does not touch that map, was a
necessary control.

## Fresh diagnosis before code changes

The frozen `6cea8cc` server/client were profiled separately from elapsed-time
comparisons. A fresh 499 Hz kernel-requested PING capture showed receive-wait,
scheduler selection and scheduling-lock paths. A separate syscall trace recorded
5808 sends/5816 receives and 5707 scheduling yields across 5000 timed commands,
800 warmups and eight readiness PINGs. Those are whole-invocation syscall counts,
not request-only CPU costs or instrumented-throughput evidence.

Fixed-one-worker and Valkey controls used the same host/CPU budget. Network,
firewall, connection-tracking, wake-up and profiler MSR costs appeared too; no
host-global firewall, scheduler, perf policy or affinity of unrelated processes
was changed. The shared VM gave large timing variation. An interrupted oversized
local matrix is incomplete, not a release result; exploratory screens only guide
which candidate gets clean-source hosted comparisons.

## The focused change

Default admission still creates one **managed thread per client**. A bind-time
one-through-four-CPU capacity hint permits a client that just flushed replies to lend its
socket/buffers to an existing client thread, while its own thread waits on a
return channel. Each group's temporary leader round-robins busy loans using the same
bounded nonblocking `Client::poll` implementation as explicit fixed workers.
This amortizes server-thread scheduling across an active burst rather than
requesting a yield for every short response.

V2 starts a new epoch only after at least two reply-progress observations within
50 µs, or joins an already active epoch. Isolated sparse replies therefore avoid
per-reply lease/channel/clone allocation and post-reply active polling. V1 activated after
every eligible reply; its separate results remain attributed to that source.
After 50 µs without real progress, the epoch closes and sockets return to their
blocking owners. V2 also returns individually quiet borrowers while others stay
busy, avoiding an O(idle-clients) scan after a mixed burst. The coordinator keeps
its own socket serviced rather than waiting on itself. A separate 100 ms maximum
epoch, checked between bounded passes, prevents continuous traffic from retaining
a closed coordinator's admission slot indefinitely. There is no new permanent
worker, no changed default thread count, no unsafe readiness multiplexer, no flag
selecting a different benchmark
mode, and no 1 ms fixed-worker idle park on this path. An initially lone client,
unknown/above-four capacity hint and explicit fixed modes do not enable handoffs.
Capacity is an immutable hint, not a CPU budget, load detector or OS guarantee.

The original worker does **no I/O while waiting for its lease**. Its retained
socket clone shares options with the original handle, so returning restores
blocking mode before continuing. Each connection caches an inactive lease shell:
the clone, disconnected placeholder receiver and placeholder session are created
only on its first lease and reused thereafter. Real session, receiver and buffer
ownership swap into the lease and back; the cached shell has no pending bytes
and must not shut down the original socket when dropped. Input and output buffers move, not copy;
already-sent reply bytes are removed once. Returned ready input is parsed before
another read, preserving execute-once behavior and QUIT/error/EOF ordering.
Store locking and data semantics are unchanged.

One outstanding loan per admitted connection bounds both message directions.
RAII returns queued/active loans even when an epoch closes between cloning a
sender and sending, a send fails or leadership unwinds. Cancellation closes
before waking the waiting owner. The accept/control shutdown still shuts down
all client control handles and joins every started worker.

## Bounds, costs and validation

- Existing frame, argument, client and payload limits, 64-command quanta, bounded
  reads/writes and backpressure remain; no lock spans network I/O.
- At most one retained extra cloned socket handle, one outstanding lease and
  bounded channel metadata per lending client. The inactive clone is released
  when its connection worker exits. This is not zero allocation or RSS-free.
- Active polling trades CPU for wake/scheduling work. Sparse multi-client bursts
  may pay handoff/channel/clone costs; quiet connections return to blocking I/O.
  A busy epoch can outlast its idle window, but progress does not extend its
  separate 100 ms maximum. Checks occur between bounded passes, not at hard
  real-time deadlines. At most the coordinator's own quiet socket stays polled
  while other borrowers return to their blocking owners.
- No correctness relies on CPU-hint accuracy or OS yield/fairness behavior.
- Deterministic tests cover epoch closure with queued/late senders, cancellation,
  unwind, exact 64-command round robin, partial reply suffixes, buffer retention,
  one return per loan and restoration of blocking reads. Additional tests cover
  lane-local leadership closure, cross-lane transaction atomicity, repeated shell
  reuse/partial replies/push ownership and eight-client wire/shutdown behavior.
  SMT-probe tests cover short/error/malformed/oversized reads and a three-byte
  read bound; policy/backoff tests cover conservative selection, cap and reset.
- `make check`/`make ci` add the complete default TCP suite on one allowed Linux
  CPU when taskset is available; existing per-client/one/four-mode tests remain.
  The benchmark records captured `client_handoff_idle_us`,
  `client_handoff_policy_version`, `client_handoff_max_epoch_ms` and
  `client_handoff_lanes` rather than
  resampling CPU capacity later.

The [first handoff reference cohort](../benchmarks/reports/40243686729-1.md)
beat Valkey in rate and batch p99 across all five equal-one-CPU workloads at
pipelines 1 and 16. That result identifies source `9062847` (v1), not an unmeasured
claim for v2 or all traffic. A paced local diagnostic exposed extra sparse CPU
cost in v1, motivating the activation guard and deterministic boundary tests.

Both hosted A/B studies and current-reference publications were independently
validated; local screens remain diagnostic rather than release speed claims.
The archived reference cohort regenerates its published measurements byte-for-byte
in an offline regression test. All CPU/tail/rate regressions and unequal feature
sets remain visible. Native TLS, persistence, broad compatibility and
production acceptance remain separate [readiness gates](production-readiness.md).

Sources: [safe TCP cloning/options](https://doc.rust-lang.org/std/net/struct.TcpStream.html),
[capacity caveats](https://doc.rust-lang.org/std/thread/fn.available_parallelism.html),
[yield caveats](https://doc.rust-lang.org/std/thread/fn.yield_now.html),
[std channels](https://doc.rust-lang.org/std/sync/mpsc/),
[Linux SMT status ABI](https://www.kernel.org/doc/Documentation/ABI/testing/sysfs-devices-system-cpu),
[bounded processor hints](https://doc.rust-lang.org/std/hint/fn.spin_loop.html),
[previous I/O investigation](io-scheduling.md).
