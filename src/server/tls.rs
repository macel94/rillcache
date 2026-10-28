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
use std::time::Duration;

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
    pub(crate) fn accept(&self, mut socket: TcpStream) -> io::Result<Connection> {
        socket.set_read_timeout(Some(self.handshake_timeout))?;
        socket.set_write_timeout(Some(self.handshake_timeout))?;
        let mut conn = ServerConnection::new(Arc::clone(&self.config))
            .map_err(|error| invalid(format!("TLS connection setup: {error}")))?;
        while conn.is_handshaking() {
            conn.complete_io(&mut socket)?;
        }
        socket.set_read_timeout(Some(self.idle_timeout))?;
        socket.set_write_timeout(Some(self.idle_timeout))?;
        Ok(Connection::Tls(Box::new(StreamOwned {
            conn,
            sock: socket,
        })))
    }
}

/// Plain or TLS RESP transport. The default per-client worker path can use
/// either without changing session or protocol code.
pub(crate) enum Connection {
    Plain(TcpStream),
    Tls(Box<StreamOwned<ServerConnection, TcpStream>>),
}

impl Connection {
    pub(crate) fn plain(stream: TcpStream) -> Self {
        Self::Plain(stream)
    }

    pub(crate) fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        self.raw().set_read_timeout(timeout)
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

    fn raw(&self) -> &TcpStream {
        match self {
            Self::Plain(stream) => stream,
            Self::Tls(stream) => &stream.sock,
        }
    }
}

impl Read for Connection {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.read(buffer),
            Self::Tls(stream) => stream.read(buffer),
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
