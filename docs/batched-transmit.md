# Bounded batched transmit

The one-owner Unix Mio frontend retains one socket-read/session/data owner. At
bind time, at least two effective CPUs enable one additional reusable transmit
thread. One effective CPU, explicit fixed workers, Tokio, non-Unix and the
separate multi-owner reactor retain their existing paths. Native TLS sockets are
not sent to this worker. This is not a second data owner or a claim of many-core
storage scaling.

## Work and ownership

The normal bounded reactor turn still reads and executes commands. If at least
two live ready reply buffers can share a handoff, their existing `Vec<u8>` buffers
move to one batch. A lone reply stays inline. Admission is nonblocking: a full
queue falls back to inline writes, rather than stalling the command owner. No
command-specific or benchmark-specific path selects the helper.

Each connection permits one in-flight output. While that buffer is away, the
reactor cannot read/dispatch later commands or drain pushes for that connection.
The helper never receives a Session, executor, Data or authorization state; it
only holds a duplicate nonblocking socket handle and an already-produced buffer.
The same buffer returns on completion, retaining its allocation and unsent offset.
No response bytes are copied for handoff.

A job makes at most one existing 64 KiB write. Partial writes, `WouldBlock` and
`Interrupted` return to the existing runnable/writable-interest machinery; the
helper never waits on one slow reader. A shared Mio Waker signals completion
batches; full selector tokens prevent stale completions from touching recycled
slots. Errors close only the affected connection; helper failure stops the run.

## Bounds and lifecycle

There is one queue slot for a batch plus one batch being written; a batch contains
at most the 256-service-turn quantum. Completions are drained before further
socket service, and only admitted in-flight outputs can produce completions.
The completion channel is nonblocking so shutdown cannot deadlock behind it.
Existing per-connection output/data limits still apply: these message bounds are
not a tighter total-RSS or atomic-reply-size guarantee.

On shutdown, original sockets are deregistered/closed before the helper is joined;
duplicates therefore cannot retain a blocked connection. No detached worker or
per-batch thread is created. `SchedulingPolicy.mio_write_threads` and embedded
benchmark `write_worker_threads` count this extra thread separately from the one
socket/data owner, using the captured decision rather than resampling capacity.

Tests exercise worker bounds/queue ownership/error continuation, actual batched
TCP replies, suppression of later side effects, partial/WouldBlock/stale-token
completion, lone inline replies, large slow-reader pipelines, healthy-client
progress and joined shutdown. Performance acceptance remains the unchanged
protected Valkey and Dragonfly comparisons; passing correctness tests does not
establish a throughput win.
