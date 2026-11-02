//! Managed connected-stream reads, retaining TCP's signal-safe write semantics.
//!
//! On Linux transfer the original managed descriptor into File for read(2),
//! sharing the existing control clone for TCP writes/CLIENT KILL. Mio keeps the
//! original descriptor and the steady-state budget stays at two. Never write
//! through File (SIGPIPE). Legacy borrowed I/O leases retain their TCP path.

use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::sync::Arc;

#[cfg(unix)]
use std::os::fd::{AsRawFd, RawFd};
#[cfg(target_os = "linux")]
use std::{fs::File, os::fd::OwnedFd};

#[cfg(target_os = "linux")]
type Reader = File;
#[cfg(not(target_os = "linux"))]
type Reader = TcpStream;

pub(super) enum ClientSocket {
    Managed {
        reader: Reader,
        control: Arc<TcpStream>,
    },
    Direct(TcpStream),
}

impl ClientSocket {
    pub(super) fn new(stream: TcpStream) -> io::Result<(Self, Arc<TcpStream>)> {
        let control = Arc::new(stream.try_clone()?);
        #[cfg(target_os = "linux")]
        let reader = File::from(OwnedFd::from(stream));
        #[cfg(not(target_os = "linux"))]
        let reader = stream;
        Ok((
            Self::Managed {
                reader,
                control: Arc::clone(&control),
            },
            control,
        ))
    }

    pub(super) fn direct(stream: TcpStream) -> Self {
        Self::Direct(stream)
    }

    fn tcp(&self) -> &TcpStream {
        match self {
            Self::Managed { control, .. } => control,
            Self::Direct(stream) => stream,
        }
    }

    pub(super) fn local_addr(&self) -> io::Result<SocketAddr> {
        self.tcp().local_addr()
    }

    pub(super) fn shutdown(&self, how: Shutdown) -> io::Result<()> {
        self.tcp().shutdown(how)
    }

    pub(super) fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()> {
        self.tcp().set_nonblocking(nonblocking)
    }

    #[cfg(test)]
    pub(super) fn peek(&self, buffer: &mut [u8]) -> io::Result<usize> {
        self.tcp().peek(buffer)
    }

    #[cfg(test)]
    pub(super) fn try_clone(&self) -> io::Result<TcpStream> {
        self.tcp().try_clone()
    }
}

impl Read for ClientSocket {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Managed { reader, .. } => reader.read(buffer),
            Self::Direct(stream) => stream.read(buffer),
        }
    }
}

impl Write for ClientSocket {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        // Keep std TCP's MSG_NOSIGNAL path, never File::write on a socket.
        self.tcp().write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.tcp().flush()
    }
}

#[cfg(unix)]
impl AsRawFd for ClientSocket {
    fn as_raw_fd(&self) -> RawFd {
        // Never register/deregister the numeric descriptor of the control.
        match self {
            Self::Managed { reader, .. } => reader.as_raw_fd(),
            Self::Direct(stream) => stream.as_raw_fd(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::time::{Duration, Instant};

    fn pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let stream = listener.accept().unwrap().0;
        stream.set_nonblocking(true).unwrap();
        (stream, peer)
    }

    #[test]
    fn stream_bytes_wouldblock_half_close_and_reply_are_preserved() {
        let (stream, mut peer) = pair();
        let (mut socket, _) = ClientSocket::new(stream).unwrap();
        let mut buffer = [0; 16];
        assert_eq!(
            socket.read(&mut buffer).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        peer.write_all(b"a\0\r\nb").unwrap();
        peer.shutdown(Shutdown::Write).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut received = Vec::new();
        loop {
            match socket.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => received.extend_from_slice(&buffer[..n]),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline);
                    std::thread::yield_now();
                }
                Err(error) => panic!("{error}"),
            }
        }
        assert_eq!(received, b"a\0\r\nb");
        socket.write_all(b"+PONG\r\n").unwrap();
        let mut response = [0; 7];
        peer.read_exact(&mut response).unwrap();
        assert_eq!(&response, b"+PONG\r\n");
    }

    #[test]
    fn shared_control_shutdown_reaches_the_read_side() {
        let (stream, _peer) = pair();
        let (mut socket, control) = ClientSocket::new(stream).unwrap();
        control.shutdown(Shutdown::Both).unwrap();
        assert_eq!(socket.read(&mut [0; 1]).unwrap(), 0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn original_selector_fd_and_two_descriptor_budget_are_preserved() {
        let (stream, _peer) = pair();
        let original = stream.as_raw_fd();
        let inode = std::fs::read_link(format!("/proc/self/fd/{original}")).unwrap();
        let (socket, control) = ClientSocket::new(stream).unwrap();
        assert_eq!(socket.as_raw_fd(), original);
        assert_ne!(control.as_raw_fd(), original);
        let descriptors = std::fs::read_dir("/proc/self/fd")
            .unwrap()
            .filter_map(Result::ok)
            .filter_map(|entry| std::fs::read_link(entry.path()).ok())
            .filter(|target| target == &inode)
            .count();
        assert_eq!(
            descriptors, 2,
            "control sharing must not duplicate a descriptor"
        );
    }
}
