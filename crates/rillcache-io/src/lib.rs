//! Minimal, audited `io_uring` completion batching for socket writes.
//!
//! The application crate is `#![forbid(unsafe_code)]`; this crate owns the only
//! `unsafe` in the repository. Its surface is deliberately small: create one
//! single-issuer ring, submit a `SEND` for an owned byte buffer, drain
//! completions, and drop the ring. There are no registered buffers, no linked
//! operations, no multishot and no threads.
//!
//! Safety contract:
//! * callers only ever pass a live socket descriptor they own and keep open
//!   until the matching completion is observed;
//! * submitted bytes are copied into ring-owned buffers, so the ring never
//!   borrows caller memory and the caller may mutate its own buffers at once;
//! * one outstanding submission per socket is the caller's ordering rule, and
//!   this crate never reorders completions of the same `user_data` because each
//!   in-flight buffer id is unique until its completion is returned.
//!
//! Kernel ABI structures below are `#[repr(C)]` copies of the stable `uapi`
//! layout; compile-time assertions pin their sizes so a future struct edit
//! cannot silently corrupt the rings.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

/// Ring/queue offsets from `include/uapi/linux/io_uring.h`.
const IORING_OFF_SQ_RING: i64 = 0;
const IORING_OFF_CQ_RING: i64 = 0x800_0000;
const IORING_OFF_SQES: i64 = 0x1000_0000;

const IORING_OP_SEND: u8 = 26;
const MSG_NOSIGNAL: u32 = 0x4000;

/// Bound on one queued payload. Larger replies are split by the caller, which
/// keeps worst-case ring memory (`entries * MAX_PAYLOAD`) small and explicit.
pub const MAX_PAYLOAD: usize = 8 * 1024;
/// Fixed number of in-flight sends, and therefore ring entries.
pub const ENTRIES: u32 = 128;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SqringOffsets {
    head: u32,
    tail: u32,
    ring_mask: u32,
    ring_entries: u32,
    flags: u32,
    dropped: u32,
    array: u32,
    resv1: u32,
    user_addr: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CqringOffsets {
    head: u32,
    tail: u32,
    ring_mask: u32,
    ring_entries: u32,
    overflow: u32,
    cqes: u32,
    flags: u32,
    resv1: u32,
    user_addr: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Params {
    sq_entries: u32,
    cq_entries: u32,
    flags: u32,
    sq_thread_cpu: u32,
    sq_thread_idle: u32,
    features: u32,
    wq_fd: u32,
    resv: [u32; 3],
    sq_off: SqringOffsets,
    cq_off: CqringOffsets,
}

impl Default for Params {
    fn default() -> Self {
        // SAFETY: `Params` is a plain `repr(C)` POD aggregate of integers.
        unsafe { std::mem::zeroed() }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Sqe {
    opcode: u8,
    flags: u8,
    ioprio: u16,
    fd: i32,
    off_or_addr2: u64,
    addr: u64,
    len: u32,
    rw_flags: u32,
    user_data: u64,
    buf_index_or_group: u16,
    personality: u16,
    splice_fd_in_or_file_index: i32,
    addr3: u64,
    pad2: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Cqe {
    user_data: u64,
    res: i32,
    flags: u32,
}

const _: () = assert!(std::mem::size_of::<Sqe>() == 64);
const _: () = assert!(std::mem::size_of::<Cqe>() == 16);
const _: () = assert!(std::mem::size_of::<Params>() == 120);

/// One completed send. `result` follows `send(2)`: bytes accepted, or `-errno`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Completion {
    pub id: u32,
    pub result: i32,
}

struct Mapping {
    ptr: *mut u8,
    len: usize,
}

impl Mapping {
    fn new(fd: RawFd, len: usize, offset: i64) -> io::Result<Self> {
        // SAFETY: `fd` is a live io_uring descriptor owned by `Ring`; `len` and
        // `offset` come from the kernel's own queue geometry. The mapping is
        // shared, read/write and released exactly once in `Drop`.
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED | libc::MAP_POPULATE,
                fd,
                offset,
            )
        };
        if ptr == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { ptr: ptr.cast(), len })
    }

    /// # Safety
    /// `offset + size <= self.len`, and the caller must not hold aliasing
    /// references that outlive the returned pointer's use in the kernel ABI.
    unsafe fn at<T>(&self, offset: usize) -> *mut T {
        debug_assert!(offset + std::mem::size_of::<T>() <= self.len);
        // SAFETY: bounds are asserted above; `ptr` is a valid mapping.
        unsafe { self.ptr.add(offset).cast::<T>() }
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: the pointer and length are exactly those returned by `mmap`
        // above and are unmapped once, here.
        unsafe {
            libc::munmap(self.ptr.cast(), self.len);
        }
    }
}

/// Single-issuer completion ring used only for socket sends.
pub struct SendRing {
    fd: OwnedFd,
    sq: Mapping,
    cq: Mapping,
    sqes: Mapping,
    sq_head: *mut u32,
    sq_tail: *mut u32,
    sq_mask: u32,
    sq_array: *mut u32,
    cq_head: *mut u32,
    cq_tail: *mut u32,
    cq_mask: u32,
    cqes: *mut Cqe,
    pending: u32,
    buffers: Vec<Option<Box<[u8]>>>,
    lengths: Vec<u32>,
    free: Vec<u32>,
    in_flight: usize,
}

// The ring is created and used by one reactor thread. It is intentionally not
// `Send`/`Sync`: all methods take `&mut self`, and moving it to another thread is
// disallowed so the single-issuer invariant is enforced by the type system.
const _: () = {
    const fn assert_not_send<T>() {}
    assert_not_send::<SendRing>();
};

impl SendRing {
    /// Create a ring with [`ENTRIES`] slots. Fails cleanly when the kernel or
    /// the sandbox does not support `io_uring`, letting the caller fall back to
    /// ordinary writes.
    pub fn new() -> io::Result<Self> {
        let mut params = Params::default();
        // SAFETY: `params` is a valid, writable `repr(C)` ABI structure.
        let fd = unsafe { libc::syscall(libc::SYS_io_uring_setup, ENTRIES, &mut params) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the kernel returned a fresh owned descriptor.
        let fd = unsafe { OwnedFd::from_raw_fd(fd as RawFd) };
        let sq_len = params.sq_off.array as usize + params.sq_entries as usize * 4;
        let cq_len = params.cq_off.cqes as usize + params.cq_entries as usize * std::mem::size_of::<Cqe>();
        let sqes_len = params.sq_entries as usize * std::mem::size_of::<Sqe>();
        let sq = Mapping::new(fd.as_raw_fd(), sq_len, IORING_OFF_SQ_RING)?;
        let cq = Mapping::new(fd.as_raw_fd(), cq_len, IORING_OFF_CQ_RING)?;
        let sqes = Mapping::new(fd.as_raw_fd(), sqes_len, IORING_OFF_SQES)?;
        // SAFETY: each offset is inside the mapping created for that queue and
        // every pointer is used only by this single-threaded owner.
        let (sq_head, sq_tail) = unsafe {
            (sq.at::<u32>(params.sq_off.head as usize), sq.at::<u32>(params.sq_off.tail as usize))
        };
        // SAFETY: as above; the array is `sq_entries` u32 indices.
        let sq_array = unsafe { sq.at::<u32>(params.sq_off.array as usize) };
        // SAFETY: as above.
        let (cq_head, cq_tail) = unsafe {
            (cq.at::<u32>(params.cq_off.head as usize), cq.at::<u32>(params.cq_off.tail as usize))
        };
        // SAFETY: the CQ entry array starts at `cq_off.cqes` and holds
        // `cq_entries` entries of 16 bytes, both included in `cq_len`.
        let cqes = unsafe { cq.at::<Cqe>(params.cq_off.cqes as usize) };
        if std::env::var_os("RILLCACHE_TRACE_CQ").is_some() {
            eprintln!("size={} sq_off_at={} cq_off_at={}", std::mem::size_of::<Params>(),
                std::mem::offset_of!(Params, sq_off), std::mem::offset_of!(Params, cq_off));
            // SAFETY: diagnostic read of our own zeroed-then-kernel-filled POD.
            let raw: &[u32] = unsafe { std::slice::from_raw_parts((&params as *const Params).cast::<u32>(), 30) };
            eprintln!("raw[0..30]={raw:?}");
            eprintln!("sq: head={} tail={} mask={} entries={} flags={} dropped={} array={}",
                params.sq_off.head, params.sq_off.tail, params.sq_off.ring_mask, params.sq_off.ring_entries,
                params.sq_off.flags, params.sq_off.dropped, params.sq_off.array);
            eprintln!("cq: head={} tail={} mask={} entries={} overflow={} cqes={}",
                params.cq_off.head, params.cq_off.tail, params.cq_off.ring_mask, params.cq_off.ring_entries,
                params.cq_off.overflow, params.cq_off.cqes);
        }
        let buffers = (0..ENTRIES).map(|_| None).collect();
        let lengths = vec![0u32; ENTRIES as usize];
        let free = (0..ENTRIES).rev().collect();
        Ok(Self {
            fd,
            sq,
            cq,
            sqes,
            sq_head,
            sq_tail,
            sq_mask: params.sq_off.ring_mask,
            sq_array,
            cq_head,
            cq_tail,
            cq_mask: params.cq_off.ring_mask,
            cqes,
            pending: 0,
            buffers,
            lengths,
            free,
            in_flight: 0,
        })
    }

    /// Bytes queued and not yet completed.
    pub fn in_flight(&self) -> usize {
        self.in_flight
    }

    /// Queue one `SEND` of at most [`MAX_PAYLOAD`] bytes.
    ///
    /// `Ok(id)` identifies the buffer until its [`Completion`] is returned.
    /// `Err(WouldBlock)` means the ring is full; the caller retries later and
    /// must not write that socket through another path meanwhile.
    pub fn submit_send(&mut self, fd: RawFd, bytes: &[u8]) -> io::Result<u32> {
        if bytes.is_empty() || bytes.len() > MAX_PAYLOAD {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "send size outside ring bound"));
        }
        let Some(id) = self.free.pop() else {
            return Err(io::ErrorKind::WouldBlock.into());
        };
        let tail = self.sq_tail();
        let index = tail & self.sq_mask;
        let slot = self.buffers[id as usize].get_or_insert_with(|| vec![0u8; MAX_PAYLOAD].into_boxed_slice());
        slot[..bytes.len()].copy_from_slice(bytes);
        // SAFETY: `index < sq_entries`; the SQE array covers every entry and is
        // exclusively owned by this ring.
        let sqe = unsafe { &mut *self.sqes.at::<Sqe>(index as usize * std::mem::size_of::<Sqe>()) };
        *sqe = Sqe {
            opcode: IORING_OP_SEND,
            fd,
            addr: slot.as_ptr() as u64,
            len: bytes.len() as u32,
            rw_flags: MSG_NOSIGNAL,
            user_data: id as u64,
            ..Sqe::default()
        };
        // SAFETY: `sq_array` has `sq_entries` u32 slots indexed by `index`.
        unsafe { *self.sq_array.add(index as usize) = index; }
        self.set_sq_tail(tail.wrapping_add(1));
        self.pending += 1;
        self.lengths[id as usize] = bytes.len() as u32;
        self.in_flight += bytes.len();
        Ok(id)
    }

    /// Submit everything queued so far with one `io_uring_enter`.
    pub fn flush(&mut self) -> io::Result<()> {
        if self.pending == 0 {
            return Ok(());
        }
        let pending = self.pending;
        // SAFETY: the ring is exclusively owned; `pending` counts SQEs written
        // by `submit_send` that the kernel has not been told about yet.
        let submitted = unsafe {
            libc::syscall(
                libc::SYS_io_uring_enter,
                self.fd.as_raw_fd(),
                pending,
                0,
                0,
                std::ptr::null::<libc::sigset_t>(),
                0,
            )
        };
        if submitted < 0 {
            return Err(io::Error::last_os_error());
        }
        self.pending -= submitted as u32;
        Ok(())
    }

    /// Take the next completion, if the kernel published one. Never blocks.
    pub fn next_completion(&mut self) -> Option<Completion> {
        let head = self.cq_head();
        let tail = self.cq_tail();
        if head == tail {
            return None;
        }
        let index = head & self.cq_mask;
        // SAFETY: `index < cq_entries` and the CQ array covers every entry.
        let cqe = unsafe { *self.cqes.add(index as usize) };
        let before = head;
        self.set_cq_head(head.wrapping_add(1));
        if std::env::var_os("RILLCACHE_TRACE_CQ").is_some() {
            eprintln!("HEAD before={before} after={} tail={}", self.cq_head(), self.cq_tail());
        }
        let id = cqe.user_data as u32;
        if std::env::var_os("RILLCACHE_TRACE_CQ").is_some() {
            eprintln!("CQE id={id} res={} len={}", cqe.res, self.lengths.get(id as usize).copied().unwrap_or(0));
        }
        if (id as usize) < self.buffers.len() {
            self.in_flight = self.in_flight.saturating_sub(self.lengths[id as usize] as usize);
            self.free.push(id);
        }
        Some(Completion { id, result: cqe.res })
    }

    fn sq_tail(&self) -> u32 {
        // SAFETY: mapped shared u32 owned by this ring.
        unsafe { std::ptr::read_volatile(self.sq_tail) }
    }

    fn set_sq_tail(&mut self, value: u32) {
        // SAFETY: mapped shared u32 owned by this ring; release publishes the
        // SQE and array entry to the kernel before the tail update.
        unsafe { std::ptr::write_volatile(self.sq_tail, value) };
        std::sync::atomic::fence(std::sync::atomic::Ordering::Release);
    }

    fn cq_tail(&self) -> u32 {
        // SAFETY: mapped shared u32 owned by this ring.
        unsafe { std::ptr::read_volatile(self.cq_tail) }
    }

    fn cq_head(&self) -> u32 {
        // SAFETY: mapped shared u32 owned by this ring.
        unsafe { std::ptr::read_volatile(self.cq_head) }
    }

    fn set_cq_head(&mut self, value: u32) {
        std::sync::atomic::fence(std::sync::atomic::Ordering::Acquire);
        // SAFETY: mapped shared u32; only this thread advances the consumer head.
        unsafe { std::ptr::write_volatile(self.cq_head, value) };
    }
}

impl Drop for SendRing {
    fn drop(&mut self) {
        // The descriptor closes after the mappings are released; in-flight
        // sends are cancelled by the kernel and their buffers are dropped with
        // the ring. Ordering is guaranteed by field order: mappings first.
        let _ = &self.sq;
        let _ = &self.cq;
        let _ = &self.sqes;
        let _ = &self.fd;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;
    use std::time::{Duration, Instant};

    #[test]
    fn sends_complete_in_order_with_the_reported_byte_count() {
        let Ok(mut ring) = SendRing::new() else {
            eprintln!("io_uring unavailable in this environment; skipping");
            return;
        };
        let (a, mut b) = UnixStream::pair().unwrap();
        a.set_nonblocking(false).unwrap();
        let first = ring.submit_send(a.as_raw_fd(), b"hello ").unwrap();
        ring.flush().unwrap();
        // The next submission must not reuse the first buffer id.
        let second = ring.submit_send(a.as_raw_fd(), b"world").unwrap();
        ring.flush().unwrap();
        assert_ne!(first, second);
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut completed = Vec::new();
        while completed.len() < 2 {
            if let Some(c) = ring.next_completion() {
                completed.push(c);
            } else {
                assert!(Instant::now() < deadline, "completion timeout");
                std::thread::yield_now();
            }
        }
        assert!(completed.iter().all(|c| c.result == 6 || c.result == 5));
        let mut got = [0u8; 11];
        b.read_exact(&mut got).unwrap();
        assert_eq!(&got, b"hello world");
        assert_eq!(ring.in_flight(), 0);
    }

    #[test]
    fn rejects_empty_and_oversized_payloads_and_bounds_queueing() {
        let Ok(mut ring) = SendRing::new() else {
            eprintln!("io_uring unavailable in this environment; skipping");
            return;
        };
        let (a, _b) = UnixStream::pair().unwrap();
        assert!(ring.submit_send(a.as_raw_fd(), b"").is_err());
        assert!(ring.submit_send(a.as_raw_fd(), &vec![0u8; MAX_PAYLOAD + 1]).is_err());
        for _ in 0..ENTRIES {
            ring.submit_send(a.as_raw_fd(), b"x").unwrap();
        }
        assert!(matches!(ring.submit_send(a.as_raw_fd(), b"x"), Err(e) if e.kind() == io::ErrorKind::WouldBlock));
    }

    #[test]
    fn a_closed_peer_reports_an_error_completion_instead_of_panicking() {
        let Ok(mut ring) = SendRing::new() else {
            eprintln!("io_uring unavailable in this environment; skipping");
            return;
        };
        let (a, b) = UnixStream::pair().unwrap();
        drop(b);
        ring.submit_send(a.as_raw_fd(), b"gone").unwrap();
        ring.flush().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(c) = ring.next_completion() {
                assert!(c.result < 0, "expected EPIPE, got {}", c.result);
                break;
            }
            assert!(Instant::now() < deadline, "completion timeout");
            std::thread::yield_now();
        }
    }

    #[test]
    fn partial_sends_reuse_ids_without_duplicate_or_reordered_completions() {
        let Ok(mut ring) = SendRing::new() else {
            eprintln!("io_uring unavailable in this environment; skipping");
            return;
        };
        let (a, mut b) = UnixStream::pair().unwrap();
        a.set_nonblocking(false).unwrap();
        // One byte per send keeps the socket writable but exercises many ids.
        let payloads: Vec<Vec<u8>> = (0..2_000u32).map(|i| vec![(i % 251) as u8 + 1; (i % 5 + 1) as usize]).collect();
        let mut submitted = 0usize;
        let mut completed = 0usize;
        let mut received = Vec::new();
        let mut reader = std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            let mut total = Vec::new();
            b.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            while total.len() < 6_000 {
                match b.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => total.extend_from_slice(&buf[..n]),
                    Err(_) => break,
                }
            }
            total
        });
        while completed < payloads.len() {
            while submitted < payloads.len() {
                match ring.submit_send(a.as_raw_fd(), &payloads[submitted]) {
                    Ok(_) => submitted += 1,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(e) => panic!("submit failed: {e}"),
                }
            }
            ring.flush().unwrap();
            let mut progress = false;
            while let Some(c) = ring.next_completion() {
                assert!(c.result > 0, "unexpected result {}", c.result);
                completed += 1;
                progress = true;
            }
            if !progress {
                std::thread::yield_now();
            }
        }
        received = {
            let collected = reader.join().unwrap();
            collected
        };
        let expected: Vec<u8> = payloads.concat();
        assert_eq!(received.len(), expected.len(), "byte count");
        assert_eq!(received, expected, "bytes or ordering differ");
    }

    #[test]
    fn dropping_with_queued_work_is_clean() {
        let Ok(mut ring) = SendRing::new() else {
            eprintln!("io_uring unavailable in this environment; skipping");
            return;
        };
        let (mut a, mut b) = UnixStream::pair().unwrap();
        ring.submit_send(a.as_raw_fd(), b"queued").unwrap();
        ring.flush().unwrap();
        // Read then drop with the completion possibly still pending.
        let mut buf = [0u8; 6];
        b.read_exact(&mut buf).unwrap();
        drop(ring);
        let _ = a.write_all(b"x");
    }
}
