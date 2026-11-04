# AGENTS.md — rillcache-io

This crate is the repository's single `unsafe` boundary. Read this before
changing it.

## Rules

- Keep the public surface safe and minimal. Do not expose raw pointers, file
  descriptors without owners, or anything that lets a caller hold a ring buffer
  across a completion.
- One ring per thread, one issuer. Do not add `unsafe impl Send/Sync`, worker
  threads, registered buffers, linked SQEs or multishot without a new review:
  each changes the ordering and lifetime contract that the reactor relies on.
- Every `unsafe` block needs a `SAFETY:` comment that names the invariant it
  depends on (mapping bounds, publication order, single ownership, or kernel ABI
  layout). `unsafe_op_in_unsafe_fn` stays denied.
- Kernel ABI structs are `#[repr(C)]` mirrors of `include/uapi/linux/io_uring.h`.
  Any change must keep the compile-time size assertions and must re-verify the
  field offsets against the header on the target kernel.
- `io_uring_params` queue fields are **offsets into the mapping**. The mask value
  must be read from the mapping and validated against the queue size; treating an
  offset as a mask corrupts slot indexing and duplicates replies.
- Fail closed: any setup, geometry or submission error returns `io::Error`. The
  caller falls back to direct writes; this crate never retries silently and never
  batches partially.
- Tests are not optional. `cargo test -p rillcache-io` must pass, including the
  partial-write stress test that compares the peer's byte stream against the
  submitted payloads.

## Non-goals

- Not a general async runtime, not a file-I/O abstraction, not a replacement for
  Mio's readiness loop.
- Not a place to hide unsafe used by other parts of the application. If another
  subsystem needs FFI, it gets its own reviewed crate and its own contract.
