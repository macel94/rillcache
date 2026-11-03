//! Initialized request storage with a separate visible length. Socket reads
//! append directly into owned storage; unread initialization is never exposed.

use std::fmt;
use std::io::{self, Read};
use std::ops::Deref;

#[derive(Default)]
pub(super) struct InputBuffer {
    storage: Vec<u8>,
    filled: usize,
}

impl InputBuffer {
    #[inline]
    pub(super) fn len(&self) -> usize {
        self.filled
    }

    #[inline]
    pub(super) fn is_empty(&self) -> bool {
        self.filled == 0
    }

    #[inline]
    pub(super) fn as_slice(&self) -> &[u8] {
        &self.storage[..self.filled]
    }

    #[cfg(test)]
    pub(super) fn capacity(&self) -> usize {
        self.storage.capacity()
    }

    pub(super) fn clear(&mut self) {
        self.filled = 0;
    }

    pub(super) fn consume(&mut self, count: usize) {
        assert!(count <= self.filled, "consumed beyond received input");
        if count != 0 && count < self.filled {
            self.storage.copy_within(count..self.filled, 0);
        }
        self.filled -= count;
    }

    /// Exactly one read, into initialized storage. `room` is the caller's
    /// remaining frame/quantum budget, not an advertised untrusted payload size.
    pub(super) fn read_from<R: Read>(
        &mut self,
        reader: &mut R,
        room: usize,
        frame_limit: usize,
    ) -> io::Result<usize> {
        let target = self
            .filled
            .checked_add(room)
            .filter(|&target| room != 0 && target <= frame_limit)
            .ok_or(io::ErrorKind::InvalidInput)?;
        if self.storage.len() < target {
            self.grow(target, frame_limit)?;
        }
        let size = reader.read(&mut self.storage[self.filled..target])?;
        if size > room {
            return Err(io::ErrorKind::InvalidData.into());
        }
        self.filled += size;
        Ok(size)
    }

    #[cold]
    fn grow(&mut self, target: usize, frame_limit: usize) -> io::Result<()> {
        // Geometric initialization avoids quadratic growth under bytewise
        // fragmented input. Exact reservation keeps capacity within the bound.
        let length = target
            .max(self.storage.len().saturating_mul(2))
            .min(frame_limit);
        self.storage
            .try_reserve_exact(length - self.storage.len())
            .map_err(|_| io::ErrorKind::OutOfMemory)?;
        self.storage.resize(length, 0);
        Ok(())
    }
}

impl From<Vec<u8>> for InputBuffer {
    fn from(storage: Vec<u8>) -> Self {
        let filled = storage.len();
        Self { storage, filled }
    }
}

impl Deref for InputBuffer {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl AsRef<[u8]> for InputBuffer {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl<T: AsRef<[u8]>> PartialEq<T> for InputBuffer {
    fn eq(&self, other: &T) -> bool {
        self.as_slice() == other.as_ref()
    }
}

impl fmt::Debug for InputBuffer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("InputBuffer")
            .field(&self.as_slice())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn reader_writes_directly_into_owned_free_storage_once() {
        struct Capture {
            calls: usize,
            pointer: usize,
            room: usize,
        }
        impl Read for Capture {
            fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
                self.calls += 1;
                self.pointer = bytes.as_ptr() as usize;
                self.room = bytes.len();
                bytes[..4].copy_from_slice(b"tail");
                Ok(4)
            }
        }
        let mut input = InputBuffer::from(b"head".to_vec());
        let mut reader = Capture {
            calls: 0,
            pointer: 0,
            room: 0,
        };
        assert_eq!(input.read_from(&mut reader, 8, 32).unwrap(), 4);
        assert_eq!(reader.calls, 1);
        assert_eq!(reader.room, 8);
        assert_eq!(reader.pointer, input.storage[4..].as_ptr() as usize);
        assert_eq!(input.as_slice(), b"headtail");
        assert_eq!(input.len(), 8);
    }

    #[test]
    fn consuming_preserves_pipeline_tail_and_never_exposes_stale_storage() {
        let mut input = InputBuffer::from(b"firstsecond".to_vec());
        input.consume(5);
        assert_eq!(input.as_slice(), b"second");
        input.read_from(&mut Cursor::new(b"third"), 8, 64).unwrap();
        assert_eq!(input.as_slice(), b"secondthird");
        let capacity = input.storage.capacity();
        input.consume(input.len());
        assert!(input.is_empty());
        assert_eq!(input.storage.capacity(), capacity);
        input.read_from(&mut Cursor::new(b"x"), 8, 64).unwrap();
        assert_eq!(input.as_slice(), b"x");
        input.clear();
        assert!(input.as_slice().is_empty());
    }

    #[test]
    fn eof_and_errors_preserve_visible_input() {
        struct Failure(io::ErrorKind);
        impl Read for Failure {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(self.0.into())
            }
        }
        let mut input = InputBuffer::from(b"partial".to_vec());
        for kind in [
            io::ErrorKind::WouldBlock,
            io::ErrorKind::Interrupted,
            io::ErrorKind::ConnectionReset,
        ] {
            assert_eq!(
                input
                    .read_from(&mut Failure(kind), 16, 64)
                    .unwrap_err()
                    .kind(),
                kind
            );
            assert_eq!(input.as_slice(), b"partial");
        }
        assert_eq!(input.read_from(&mut Cursor::new([]), 16, 64).unwrap(), 0);
        assert_eq!(input.as_slice(), b"partial");
    }

    #[test]
    fn invalid_budgets_and_dishonest_readers_do_not_extend_input() {
        struct TooMuch;
        impl Read for TooMuch {
            fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
                Ok(bytes.len() + 1)
            }
        }
        let mut input = InputBuffer::from(b"a".to_vec());
        for (room, limit) in [(0, 16), (16, 8), (usize::MAX, usize::MAX)] {
            assert_eq!(
                input
                    .read_from(&mut TooMuch, room, limit)
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::InvalidInput
            );
            assert_eq!(input.as_slice(), b"a");
        }
        assert_eq!(
            input.read_from(&mut TooMuch, 4, 16).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(input.as_slice(), b"a");
    }

    #[test]
    fn fragmented_growth_is_geometric_and_frame_bounded() {
        let mut input = InputBuffer::default();
        let mut growths = 0;
        for _ in 0..2048 {
            let before = input.storage.len();
            let room = 16.min(2048 - input.len());
            input.read_from(&mut Cursor::new(b"x"), room, 2048).unwrap();
            growths += usize::from(input.storage.len() != before);
            assert!(input.storage.len() <= 2048);
            assert!(input.storage.capacity() <= 2048);
        }
        assert!(growths <= 8, "growth was not amortized: {growths}");
        assert_eq!(input.as_slice(), vec![b'x'; 2048]);
    }

    #[test]
    fn deterministic_read_consume_sequences_match_a_plain_vector() {
        let mut input = InputBuffer::default();
        let mut model = Vec::new();
        let mut seed = 42u64;
        for _ in 0..5000 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            if !model.is_empty() && (seed & 3 == 0 || model.len() == 256) {
                let count = seed as usize % (model.len() + 1);
                input.consume(count);
                model.drain(..count);
            } else if model.len() < 256 {
                let room = 16.min(256 - model.len());
                let size = seed as usize % (room + 1);
                let bytes = vec![seed as u8; size];
                input
                    .read_from(&mut Cursor::new(&bytes), room, 256)
                    .unwrap();
                model.extend_from_slice(&bytes);
            }
            assert_eq!(input.as_slice(), model);
            assert!(input.storage.len() <= 256);
            assert!(input.storage.capacity() <= 256);
        }
    }
}
