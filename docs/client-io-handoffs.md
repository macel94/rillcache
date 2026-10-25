# Single-CPU client I/O handoffs (S21–S22)

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
one-CPU capacity hint permits a client that just flushed replies to lend its
socket/buffers to an existing client thread, while its own thread waits on a
return channel. One temporary leader round-robins busy loans using the same
bounded nonblocking `Client::poll` implementation as explicit fixed workers.
This amortizes server-thread scheduling across an active burst rather than
requesting a yield for every short response.

After 50 µs without real progress, the epoch closes and sockets return to their
blocking owners. There is no new permanent worker, no changed default thread
count, no unsafe readiness multiplexer, no flag selecting a different benchmark
mode, and no 1 ms fixed-worker idle park on this path. An initially lone client,
unknown/multicore capacity hint and explicit fixed modes do not enable handoffs.
Capacity is an immutable hint, not a CPU budget, load detector or OS guarantee.

The original worker does **no I/O while waiting for its lease**. Its temporary
socket clone shares options with the original handle, so returning restores
blocking mode before continuing. Input and output buffers move, not copy;
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
- At most one extra cloned socket handle, one outstanding lease and bounded
  channel metadata per lending client. This is not zero allocation or RSS-free.
- Active polling trades CPU for wake/scheduling work. Sparse multi-client bursts
  may pay handoff/channel/clone costs; quiet connections return to blocking I/O.
  A permanently busy epoch can last longer than its idle window. Checks occur
  between bounded passes, not at hard real-time deadlines.
- No correctness relies on CPU-hint accuracy or OS yield/fairness behavior.
- Deterministic tests cover epoch closure with queued/late senders, cancellation,
  unwind, exact 64-command round robin, partial reply suffixes, buffer retention,
  one return per loan and restoration of blocking reads.
- `make check`/`make ci` add the complete default TCP suite on one allowed Linux
  CPU when taskset is available; existing per-client/one/four-mode tests remain.
  The benchmark records the captured `client_handoff_idle_us` and
  `client_handoff_policy_version` rather than resampling CPU capacity later.

Fresh hosted A/B and current-reference results determine acceptance; local
screens are not a release speed claim. Keep all CPU/tail/rate regressions and
unequal feature sets visible. Native TLS, persistence, broad compatibility and
production acceptance remain separate [readiness gates](production-readiness.md).

Sources: [safe TCP cloning/options](https://doc.rust-lang.org/std/net/struct.TcpStream.html),
[capacity caveats](https://doc.rust-lang.org/std/thread/fn.available_parallelism.html),
[yield caveats](https://doc.rust-lang.org/std/thread/fn.yield_now.html),
[std channels](https://doc.rust-lang.org/std/sync/mpsc/),
[previous I/O investigation](io-scheduling.md).
