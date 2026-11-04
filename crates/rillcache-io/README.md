# rillcache-io — the repository's only `unsafe` boundary

This crate exists for one reason: Linux `io_uring` lets the reactor hand many
socket writes to the kernel with a single `io_uring_enter` instead of one `write`
syscall per reply. The application crate keeps `#![forbid(unsafe_code)]` and sees
only the safe surface below.

## Surface

```rust
let mut ring = SendRing::new()?;            // None on failure -> caller falls back
let id = ring.submit_send(fd, &bytes)?;     // copies into a ring-owned buffer
ring.flush()?;                              // one enter for every queued SQE
while let Some(done) = ring.next_completion() { /* (id, result) */ }
```

* `submit_send` copies the payload, so the caller may mutate or drop its own
  buffer immediately. Payloads are bounded by `MAX_PAYLOAD`; larger replies are
  split by the caller.
* `Err(WouldBlock)` means the ring is full. The caller must retry later and must
  not write that socket through another path in the meantime, or reply order
  would change.
* `next_completion` never blocks and is the only owner of buffer recycling.
* One ring belongs to one thread and is deliberately `!Send`/`!Sync` by
  construction (no `unsafe impl`, all methods take `&mut self`).

## Why the unsafe is acceptable

* The kernel ABI structures are `#[repr(C)]` copies of the stable `uapi`
  layout, with compile-time size assertions (`Sqe` 64, `Cqe` 16, `Params` 120).
* `io_uring_params` reports *offsets into the mmap*, not values: `ring_mask` is
  read from the mapping and validated against the advertised queue size at
  construction, and construction fails closed on any mismatch.
* Every pointer is derived from an `mmap` this crate owns and releases exactly
  once in `Drop`. No global state, no aliasing between rings, no user pointers.
* Memory ordering: SQ tail is published with a release fence after the SQE and
  array entry are written; CQ head is advanced after an acquire fence.
* `unsafe_op_in_unsafe_fn` is denied: each block carries a `SAFETY:` comment and
  does one thing.

## Tests

`cargo test -p rillcache-io` covers ordered completions with byte counts,
empty/oversized/bounded submission, error completion on a closed peer, clean drop
with queued work, and a **2,000-send partial-write stress test** that verifies the
peer receives byte-exact, correctly ordered data (this test found the queue-mask
bug that shipped in the first draft).

## Failure behaviour

If `io_uring_setup` or the mmap fails (old kernel, sandbox, seccomp), `new()`
returns an error and the reactor keeps its original direct-`write` path. There is
no fallback inside the ring and no silent partial batching.
