//! Native TLS 1.3 for RESP using the rustls-only dependency exception.
//!
//! There is no sidecar and no OpenSSL. Certificates and keys are loaded from
//! PEM files; clients speak ordinary RESP over the TLS stream. ALPN is left
//! empty because Redis clients do not use HTTP/2 or HTTP/3.

use rustls::pki_types::{CertificateDer, CertificateRevocationListDer, PrivateKeyDer};
use rustls::server::{ClientHello, ResolvesServerCert, WebPkiClientVerifier};
use rustls::sign::CertifiedKey;
use rustls::{RootCertStore, ServerConfig, ServerConnection, StreamOwned};
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufReader, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

#[derive(Debug)]
struct SniResolver {
    default: Arc<CertifiedKey>,
    identities: HashMap<String, Arc<CertifiedKey>>,
}

impl ResolvesServerCert for SniResolver {
    fn resolve(&self, client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        match client_hello.server_name() {
            None => Some(Arc::clone(&self.default)),
            Some(_) if self.identities.is_empty() => Some(Arc::clone(&self.default)),
            Some(name) => self.identities.get(&name.to_ascii_lowercase()).cloned(),
        }
    }
}

const MAX_TLS_FILE_BYTES: usize = 1024 * 1024;
const MAX_RELOAD_TOKEN_BYTES: usize = 64;

pub(crate) struct TlsSettings<'a> {
    pub(crate) cert_file: &'a Path,
    pub(crate) key_file: &'a Path,
    pub(crate) ocsp_file: Option<&'a Path>,
    pub(crate) client_ca_file: Option<&'a Path>,
    pub(crate) client_crl_file: Option<&'a Path>,
    pub(crate) sni_certificates: &'a [crate::server::TlsSniCertificate],
    pub(crate) reload_trigger: Option<&'a Path>,
    pub(crate) client_cert_users: &'a std::collections::BTreeMap<[u8; 32], Vec<u8>>,
    pub(crate) handshake_timeout: Duration,
    pub(crate) idle_timeout: Duration,
}

pub(crate) struct TlsAcceptor {
    config: RwLock<Arc<ServerConfig>>,
    cert_file: PathBuf,
    key_file: PathBuf,
    ocsp_file: Option<PathBuf>,
    client_ca_file: Option<PathBuf>,
    client_crl_file: Option<PathBuf>,
    sni_certificates: Vec<crate::server::TlsSniCertificate>,
    client_cert_users: std::collections::BTreeMap<[u8; 32], Vec<u8>>,
    reload_trigger: Option<PathBuf>,
    reload_token: Mutex<Option<Vec<u8>>>,
    reload_error_logged: AtomicBool,
    handshake_timeout: Duration,
    idle_timeout: Duration,
}

impl TlsAcceptor {
    pub(crate) fn from_files(settings: TlsSettings<'_>) -> io::Result<Self> {
        let config = Self::build_config(
            settings.cert_file,
            settings.key_file,
            settings.ocsp_file,
            settings.client_ca_file,
            settings.client_crl_file,
            settings.sni_certificates,
        )?;
        let reload_token = settings.reload_trigger.map(read_reload_token).transpose()?;
        Ok(Self {
            config: RwLock::new(config),
            cert_file: settings.cert_file.to_owned(),
            key_file: settings.key_file.to_owned(),
            ocsp_file: settings.ocsp_file.map(Path::to_owned),
            client_ca_file: settings.client_ca_file.map(Path::to_owned),
            client_crl_file: settings.client_crl_file.map(Path::to_owned),
            sni_certificates: settings.sni_certificates.to_vec(),
            client_cert_users: settings.client_cert_users.clone(),
            reload_trigger: settings.reload_trigger.map(Path::to_owned),
            reload_token: Mutex::new(reload_token),
            reload_error_logged: AtomicBool::new(false),
            handshake_timeout: settings.handshake_timeout,
            idle_timeout: settings.idle_timeout,
        })
    }

    fn build_config(
        cert_file: &Path,
        key_file: &Path,
        ocsp_file: Option<&Path>,
        client_ca_file: Option<&Path>,
        client_crl_file: Option<&Path>,
        sni_certificates: &[crate::server::TlsSniCertificate],
    ) -> io::Result<Arc<ServerConfig>> {
        let certs = read_certs(cert_file)?;
        if certs.is_empty() {
            return Err(invalid("TLS certificate file contains no certificates"));
        }
        let key = read_private_key(key_file)?;
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut default_key = CertifiedKey::from_der(certs, key, &provider)
            .map_err(|error| invalid(format!("TLS certificate/key: {error}")))?;
        default_key.ocsp = ocsp_file.map(read_ocsp_response).transpose()?;
        let default_key = Arc::new(default_key);
        let mut identities = HashMap::new();
        for identity in sni_certificates {
            let certs = read_certs(&identity.cert_file)?;
            if certs.is_empty() {
                return Err(invalid(format!(
                    "TLS certificate file for SNI name {} contains no certificates",
                    identity.server_name
                )));
            }
            let key = read_private_key(&identity.key_file)?;
            let mut certified_key =
                CertifiedKey::from_der(certs, key, &provider).map_err(|error| {
                    invalid(format!(
                        "TLS certificate/key for SNI name {}: {error}",
                        identity.server_name
                    ))
                })?;
            certified_key.ocsp = identity
                .ocsp_file
                .as_deref()
                .map(read_ocsp_response)
                .transpose()?;
            identities.insert(
                identity.server_name.to_ascii_lowercase(),
                Arc::new(certified_key),
            );
        }
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
                let verifier_builder = WebPkiClientVerifier::builder(Arc::new(roots));
                let verifier_builder = if let Some(path) = client_crl_file {
                    let crls = read_crls(path)?;
                    if crls.is_empty() {
                        return Err(invalid("TLS client CRL file contains no CRLs"));
                    }
                    verifier_builder
                        .with_crls(crls)
                        .enforce_revocation_expiration()
                } else {
                    verifier_builder
                };
                let verifier = verifier_builder
                    .build()
                    .map_err(|error| invalid(format!("TLS client verifier: {error}")))?;
                builder.with_client_cert_verifier(verifier)
            }
        };
        Ok(Arc::new(builder.with_cert_resolver(Arc::new(
            SniResolver {
                default: default_key,
                identities,
            },
        ))))
    }

    fn refresh_if_requested(&self) {
        let Some(trigger) = &self.reload_trigger else {
            return;
        };
        let Ok(mut previous_token) = self.reload_token.lock() else {
            eprintln!("rillcache: TLS reload state is unavailable; retaining active certificates");
            return;
        };
        let token = match read_reload_token(trigger) {
            Ok(token) => token,
            Err(error) => {
                if !self.reload_error_logged.swap(true, Ordering::AcqRel) {
                    eprintln!("rillcache: cannot read TLS reload trigger: {error}");
                }
                return;
            }
        };
        self.reload_error_logged.store(false, Ordering::Release);
        if previous_token.as_ref() == Some(&token) {
            return;
        }
        match Self::build_config(
            &self.cert_file,
            &self.key_file,
            self.ocsp_file.as_deref(),
            self.client_ca_file.as_deref(),
            self.client_crl_file.as_deref(),
            &self.sni_certificates,
        ) {
            Ok(config) => match self.config.write() {
                Ok(mut active) => {
                    *active = config;
                    *previous_token = Some(token);
                    eprintln!("rillcache: TLS certificate generation reloaded");
                }
                Err(_) => {
                    eprintln!("rillcache: TLS config lock poisoned; retaining active certificates")
                }
            },
            Err(error) => {
                *previous_token = Some(token);
                eprintln!(
                    "rillcache: TLS certificate reload rejected; retaining active certificates: {error}"
                );
            }
        }
    }

    /// Complete the TLS handshake before returning a stream to the RESP loop.
    pub(crate) fn accept(&self, socket: TcpStream) -> io::Result<(Connection, Option<Vec<u8>>)> {
        self.refresh_if_requested();
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
        let config = self
            .config
            .read()
            .map_err(|_| io::Error::other("TLS config lock poisoned"))?
            .clone();
        let mut conn = ServerConnection::new(config)
            .map_err(|error| invalid(format!("TLS connection setup: {error}")))?;
        while conn.is_handshaking() {
            conn.complete_io(&mut sock)?;
        }
        // rustls can finish processing buffered handshake bytes without another
        // socket operation, so completion itself must still be inside budget.
        sock.ensure_handshake_time()?;
        let client_user = if self.client_cert_users.is_empty() {
            None
        } else {
            let certificate = conn
                .peer_certificates()
                .and_then(|chain| chain.first())
                .ok_or_else(|| invalid("TLS client certificate is required for ACL mapping"))?;
            let fingerprint = crate::engine::tls_certificate_fingerprint(certificate.as_ref());
            Some(
                self.client_cert_users
                    .get(&fingerprint)
                    .cloned()
                    .ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            "TLS client certificate has no ACL user mapping",
                        )
                    })?,
            )
        };
        sock.baseline_read_timeout = Some(self.idle_timeout);
        sock.baseline_write_timeout = Some(self.idle_timeout);
        sock.handshake_deadline = None;
        sock.plaintext_idle_timeout = Some(self.idle_timeout);
        sock.reset_plaintext_idle_deadline();
        sock.stream.set_read_timeout(Some(self.idle_timeout))?;
        sock.stream.set_write_timeout(Some(self.idle_timeout))?;
        Ok((
            Connection::Tls(Box::new(StreamOwned::new(conn, sock))),
            client_user,
        ))
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

fn read_bounded_tls_file(path: &Path, description: &str) -> io::Result<Vec<u8>> {
    let file = File::open(path).map_err(|error| {
        invalid(format!(
            "cannot read TLS {description} {}: {error}",
            path.display()
        ))
    })?;
    let mut bytes = Vec::new();
    file.take((MAX_TLS_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_TLS_FILE_BYTES {
        return Err(invalid(format!(
            "TLS {description} {} exceeds the {MAX_TLS_FILE_BYTES}-byte limit",
            path.display()
        )));
    }
    Ok(bytes)
}

fn read_ocsp_response(path: &Path) -> io::Result<Vec<u8>> {
    let bytes = read_bounded_tls_file(path, "OCSP response")?;
    if !valid_der_sequence(&bytes) {
        return Err(invalid(format!(
            "TLS OCSP response {} is not one bounded DER sequence",
            path.display()
        )));
    }
    Ok(bytes)
}

// Validate complete DER TLV framing. rustls treats OCSP as opaque bytes; this
// does not validate OCSP semantics, signatures, status, certificate identity, or freshness.
fn valid_der_sequence(bytes: &[u8]) -> bool {
    if bytes.first() != Some(&0x30) {
        return false;
    }
    der_value_end(bytes, 0, 0) == Some(bytes.len())
}

fn der_value_end(bytes: &[u8], start: usize, depth: usize) -> Option<usize> {
    if depth > 64 {
        return None;
    }
    let tag = *bytes.get(start)?;
    if tag & 0x1f == 0x1f || tag == 0 {
        return None; // High-tag-number form and EOC are not used in OCSPResponse DER.
    }
    let first_len = *bytes.get(start.checked_add(1)?)?;
    let (header_len, content_len) = if first_len < 0x80 {
        (2usize, first_len as usize)
    } else {
        let octets = (first_len & 0x7f) as usize;
        if octets == 0 || octets > std::mem::size_of::<usize>() {
            return None; // DER forbids indefinite lengths.
        }
        let length_start = start.checked_add(2)?;
        let length_end = length_start.checked_add(octets)?;
        let encoded = bytes.get(length_start..length_end)?;
        if encoded.first() == Some(&0) {
            return None;
        }
        let mut length = 0usize;
        for byte in encoded {
            length = length.checked_mul(256)?.checked_add(*byte as usize)?;
        }
        if length < 128 {
            return None; // DER requires short-form for lengths below 128.
        }
        (2 + octets, length)
    };
    let content_start = start.checked_add(header_len)?;
    let end = content_start.checked_add(content_len)?;
    if end > bytes.len() {
        return None;
    }
    if tag & 0x20 != 0 {
        let mut child = content_start;
        while child < end {
            child = der_value_end(&bytes[..end], child, depth + 1)?;
        }
        if child != end {
            return None;
        }
    }
    Some(end)
}

fn read_certs(path: &Path) -> io::Result<Vec<CertificateDer<'static>>> {
    let bytes = read_bounded_tls_file(path, "certificate")?;
    rustls_pemfile::certs(&mut BufReader::new(std::io::Cursor::new(bytes))).collect()
}

fn read_crls(path: &Path) -> io::Result<Vec<CertificateRevocationListDer<'static>>> {
    let file = File::open(path).map_err(|error| {
        invalid(format!(
            "cannot read TLS client CRL {}: {error}",
            path.display()
        ))
    })?;
    let length = file.metadata()?.len();
    if length == 0 || length > MAX_TLS_FILE_BYTES as u64 {
        return Err(invalid("TLS client CRL file must be 1..=1048576 bytes"));
    }
    let mut bytes = Vec::with_capacity(length as usize);
    file.take(MAX_TLS_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 != length || bytes.len() > MAX_TLS_FILE_BYTES {
        return Err(invalid(
            "TLS client CRL file changed or exceeds 1048576 bytes",
        ));
    }
    Ok(vec![CertificateRevocationListDer::from(bytes)])
}

fn read_private_key(path: &Path) -> io::Result<PrivateKeyDer<'static>> {
    let bytes = read_bounded_tls_file(path, "key")?;
    rustls_pemfile::private_key(&mut BufReader::new(std::io::Cursor::new(bytes)))?.ok_or_else(
        || {
            invalid(format!(
                "TLS key file {} contains no private key",
                path.display()
            ))
        },
    )
}

fn read_reload_token(path: &Path) -> io::Result<Vec<u8>> {
    let file = File::open(path).map_err(|error| {
        invalid(format!(
            "cannot read TLS reload trigger {}: {error}",
            path.display()
        ))
    })?;
    let mut token = Vec::new();
    file.take((MAX_RELOAD_TOKEN_BYTES + 1) as u64)
        .read_to_end(&mut token)?;
    if token.is_empty()
        || token.len() > MAX_RELOAD_TOKEN_BYTES
        || !token.iter().all(|byte| (b'!'..=b'~').contains(byte))
    {
        return Err(invalid(format!(
            "TLS reload trigger {} must contain 1..={MAX_RELOAD_TOKEN_BYTES} visible ASCII bytes",
            path.display()
        )));
    }
    Ok(token)
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
    fn reload_tokens_are_bounded_nonempty_visible_ascii() {
        let path = std::env::temp_dir().join(format!(
            "rillcache-tls-token-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::write(&path, b"generation-1").unwrap();
        assert_eq!(read_reload_token(&path).unwrap(), b"generation-1");
        std::fs::write(&path, b"\n").unwrap();
        assert!(read_reload_token(&path).is_err());
        std::fs::write(&path, vec![b'x'; MAX_RELOAD_TOKEN_BYTES + 1]).unwrap();
        assert!(read_reload_token(&path).is_err());
        let _ = std::fs::remove_file(path);
    }

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
