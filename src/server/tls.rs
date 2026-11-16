//! Native TLS 1.3 for RESP using the rustls-only dependency exception.
//!
//! There is no sidecar and no OpenSSL. Certificates and keys are loaded from
//! PEM files; clients speak ordinary RESP over the TLS stream. ALPN is left
//! empty because Redis clients do not use HTTP/2 or HTTP/3.

use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::WebPkiClientVerifier;
use rustls::{RootCertStore, ServerConfig, ServerConnection, StreamOwned};
use std::fs::File;
use std::io::{self, BufReader, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub(crate) struct TlsAcceptor {
    config: Arc<ServerConfig>,
    handshake_timeout: Duration,
    idle_timeout: Duration,
}

impl TlsAcceptor {
    pub(crate) fn from_files(
        cert_file: &Path,
        key_file: &Path,
        client_ca_file: Option<&Path>,
        handshake_timeout: Duration,
        idle_timeout: Duration,
    ) -> io::Result<Self> {
        let certs = read_certs(cert_file)?;
        if certs.is_empty() {
            return Err(invalid("TLS certificate file contains no certificates"));
        }
        let key = read_private_key(key_file)?;
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let builder = ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|error| invalid(format!("TLS protocol configuration: {error}")))?;
        let builder = match client_ca_file {
            None => builder.with_no_client_auth(),
            Some(path) => {
                let mut roots = RootCertStore::empty();
                for cert in read_certs(path)? {
                    roots
                        .add(cert)
                        .map_err(|error| invalid(format!("TLS client CA: {error}")))?;
                }
                if roots.is_empty() {
                    return Err(invalid("TLS client CA file contains no certificates"));
                }
                let verifier = WebPkiClientVerifier::builder(Arc::new(roots))
                    .build()
                    .map_err(|error| invalid(format!("TLS client verifier: {error}")))?;
                builder.with_client_cert_verifier(verifier)
            }
        };
        let config = builder
            .with_single_cert(certs, key)
            .map_err(|error| invalid(format!("TLS certificate/key: {error}")))?;
        Ok(Self {
            config: Arc::new(config),
            handshake_timeout,
            idle_timeout,
        })
    }

    /// Complete the TLS handshake before returning a stream to the RESP loop.
    pub(crate) fn accept(&self, socket: TcpStream) -> io::Result<Connection> {
        let handshake_deadline = Instant::now()
            .checked_add(self.handshake_timeout)
            .ok_or_else(|| invalid("TLS handshake timeout is too large"))?;
        let mut sock = DeadlineSocket {
            stream: socket,
            baseline_read_timeout: Some(self.handshake_timeout),
            baseline_write_timeout: Some(self.handshake_timeout),
            handshake_deadline: Some(handshake_deadline),
            frame_deadline: None,
            plaintext_idle_timeout: None,
            plaintext_idle_deadline: None,
        };
        let mut conn = ServerConnection::new(Arc::clone(&self.config))
            .map_err(|error| invalid(format!("TLS connection setup: {error}")))?;
        while conn.is_handshaking() {
            conn.complete_io(&mut sock)?;
        }
        // rustls can finish processing buffered handshake bytes without another
        // socket operation, so completion itself must still be inside budget.
        sock.ensure_handshake_time()?;
        sock.baseline_read_timeout = Some(self.idle_timeout);
        sock.baseline_write_timeout = Some(self.idle_timeout);
        sock.handshake_deadline = None;
        sock.plaintext_idle_timeout = Some(self.idle_timeout);
        sock.reset_plaintext_idle_deadline();
        sock.stream.set_read_timeout(Some(self.idle_timeout))?;
        sock.stream.set_write_timeout(Some(self.idle_timeout))?;
        Ok(Connection::Tls(Box::new(StreamOwned::new(conn, sock))))
    }
}

/// Plain or TLS RESP transport. The default per-client worker path can use
/// either without changing session or protocol code.
pub(crate) enum Connection {
    Plain(TcpStream),
    Tls(Box<StreamOwned<ServerConnection, DeadlineSocket>>),
}

/// Recomputes socket timeouts before every TLS record read/write. rustls may
/// perform multiple underlying operations inside one call; absolute phase
/// deadlines therefore need to be enforced here, not only by the outer loop.
pub(crate) struct DeadlineSocket {
    stream: TcpStream,
    baseline_read_timeout: Option<Duration>,
    baseline_write_timeout: Option<Duration>,
    handshake_deadline: Option<Instant>,
    frame_deadline: Option<Instant>,
    plaintext_idle_timeout: Option<Duration>,
    plaintext_idle_deadline: Option<Instant>,
}

impl DeadlineSocket {
    fn ensure_handshake_time(&self) -> io::Result<()> {
        if self
            .handshake_deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            return Err(io::ErrorKind::TimedOut.into());
        }
        Ok(())
    }

    fn remaining_timeout(
        baseline: Option<Duration>,
        deadlines: impl Iterator<Item = Instant>,
    ) -> io::Result<Option<Duration>> {
        let remaining = deadlines
            .map(|deadline| deadline.saturating_duration_since(Instant::now()))
            .min();
        let timeout = match (baseline, remaining) {
            (Some(base), Some(remaining)) => Some(base.min(remaining)),
            (Some(base), None) => Some(base),
            (None, Some(remaining)) => Some(remaining),
            (None, None) => None,
        };
        if timeout.is_some_and(|timeout| timeout.is_zero()) {
            return Err(io::ErrorKind::TimedOut.into());
        }
        Ok(timeout)
    }

    fn read_timeout(&self) -> io::Result<Option<Duration>> {
        Self::remaining_timeout(
            self.baseline_read_timeout,
            self.handshake_deadline
                .into_iter()
                .chain(self.frame_deadline)
                .chain(self.plaintext_idle_deadline),
        )
    }

    fn reset_plaintext_idle_deadline(&mut self) {
        self.plaintext_idle_deadline = self.plaintext_idle_timeout.map(|timeout| {
            let now = Instant::now();
            now.checked_add(timeout).unwrap_or(now)
        });
    }

    fn write_timeout(&self) -> io::Result<Option<Duration>> {
        Self::remaining_timeout(
            self.baseline_write_timeout,
            self.handshake_deadline.into_iter(),
        )
    }

    fn check_after_handshake_io<T>(&self, result: io::Result<T>) -> io::Result<T> {
        self.ensure_handshake_time()?;
        result
    }
}

impl Read for DeadlineSocket {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.ensure_handshake_time()?;
        self.stream.set_read_timeout(self.read_timeout()?)?;
        let result = self.stream.read(buffer);
        self.check_after_handshake_io(result)
    }
}

impl Write for DeadlineSocket {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.ensure_handshake_time()?;
        self.stream.set_write_timeout(self.write_timeout()?)?;
        let result = self.stream.write(buffer);
        self.check_after_handshake_io(result)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

impl DeadlineSocket {
    fn shutdown(&self, how: Shutdown) -> io::Result<()> {
        self.stream.shutdown(how)
    }
}

impl Connection {
    pub(crate) fn plain(stream: TcpStream) -> Self {
        Self::Plain(stream)
    }

    pub(crate) fn set_read_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        match self {
            Self::Plain(stream) => stream.set_read_timeout(timeout),
            Self::Tls(stream) => {
                stream.sock.baseline_read_timeout = timeout;
                stream.sock.stream.set_read_timeout(timeout)
            }
        }
    }

    pub(crate) fn set_frame_assembly_deadline(&mut self, deadline: Option<Instant>) {
        if let Self::Tls(stream) = self {
            stream.sock.frame_deadline = deadline;
        }
    }

    pub(crate) fn shutdown(&mut self, how: Shutdown) -> io::Result<()> {
        match self {
            Self::Plain(stream) => stream.shutdown(how),
            Self::Tls(stream) => {
                stream.conn.send_close_notify();
                let _ = stream.conn.write_tls(&mut stream.sock);
                stream.sock.shutdown(how)
            }
        }
    }

    pub(crate) fn plain_mut(&mut self) -> Option<&mut TcpStream> {
        match self {
            Self::Plain(stream) => Some(stream),
            Self::Tls(_) => None,
        }
    }
}

impl Read for Connection {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.read(buffer),
            Self::Tls(stream) => {
                let result = stream.read(buffer);
                if result.as_ref().is_ok_and(|&size| size > 0) {
                    stream.sock.reset_plaintext_idle_deadline();
                }
                result
            }
        }
    }
}

impl Write for Connection {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.write(buffer),
            Self::Tls(stream) => stream.write(buffer),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Plain(stream) => stream.flush(),
            Self::Tls(stream) => stream.flush(),
        }
    }
}

fn read_certs(path: &Path) -> io::Result<Vec<CertificateDer<'static>>> {
    let file = File::open(path).map_err(|error| {
        invalid(format!(
            "cannot read TLS certificate {}: {error}",
            path.display()
        ))
    })?;
    rustls_pemfile::certs(&mut BufReader::new(file)).collect()
}

fn read_private_key(path: &Path) -> io::Result<PrivateKeyDer<'static>> {
    let file = File::open(path)
        .map_err(|error| invalid(format!("cannot read TLS key {}: {error}", path.display())))?;
    rustls_pemfile::private_key(&mut BufReader::new(file))?.ok_or_else(|| {
        invalid(format!(
            "TLS key file {} contains no private key",
            path.display()
        ))
    })
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn plaintext_idle_deadline_is_absolute_and_refreshable_on_progress() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        let mut socket = DeadlineSocket {
            stream,
            baseline_read_timeout: Some(Duration::from_secs(2)),
            baseline_write_timeout: Some(Duration::from_secs(2)),
            handshake_deadline: None,
            frame_deadline: None,
            plaintext_idle_timeout: Some(Duration::from_millis(500)),
            plaintext_idle_deadline: None,
        };
        socket.reset_plaintext_idle_deadline();
        let first = socket.plaintext_idle_deadline.unwrap();
        thread::sleep(Duration::from_millis(20));
        socket.reset_plaintext_idle_deadline();
        let refreshed = socket.plaintext_idle_deadline.unwrap();
        assert!(refreshed > first);

        socket.plaintext_idle_deadline = Some(Instant::now() + Duration::from_millis(30));
        let before = socket.read_timeout().unwrap().unwrap();
        thread::sleep(Duration::from_millis(40));
        assert_eq!(
            socket.read_timeout().unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        assert!(before <= Duration::from_millis(30));
        drop(peer);
    }

    #[test]
    fn handshake_deadline_shrinks_read_and_write_budgets() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        let deadline = Instant::now() + Duration::from_millis(300);
        let socket = DeadlineSocket {
            stream,
            baseline_read_timeout: Some(Duration::from_secs(2)),
            baseline_write_timeout: Some(Duration::from_secs(2)),
            handshake_deadline: Some(deadline),
            frame_deadline: None,
            plaintext_idle_timeout: None,
            plaintext_idle_deadline: None,
        };

        let first_read = socket.read_timeout().unwrap().unwrap();
        let first_write = socket.write_timeout().unwrap().unwrap();
        thread::sleep(Duration::from_millis(20));
        let later_read = socket.read_timeout().unwrap().unwrap();
        let later_write = socket.write_timeout().unwrap().unwrap();
        assert!(later_read < first_read, "read budget did not shrink");
        assert!(later_write < first_write, "write budget did not shrink");
        drop(peer);
    }
}
