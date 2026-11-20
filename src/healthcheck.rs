//! Read-only application probe using safe std I/O and the reviewed rustls client.
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static NEXT_AOF_PROBE_ID: AtomicU64 = AtomicU64::new(0);

pub struct TlsSettings {
    config: Arc<rustls::ClientConfig>,
    server_name: rustls::pki_types::ServerName<'static>,
}

impl TlsSettings {
    pub fn from_ca_file(
        path: &str,
        name: Option<String>,
        address: SocketAddr,
    ) -> Result<Self, String> {
        let server_name = match name {
            Some(name) => rustls::pki_types::ServerName::try_from(name)
                .map_err(|_| "--healthcheck-server-name is not a valid DNS name or IP address")?,
            None => rustls::pki_types::ServerName::IpAddress(address.ip().into()),
        };
        let file = std::fs::File::open(path).map_err(|_| "could not open healthcheck CA file")?;
        let mut bytes = Vec::new();
        file.take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "could not read healthcheck CA file")?;
        if bytes.len() > 1024 * 1024 {
            return Err("healthcheck CA file exceeds 1 MiB".into());
        }
        let mut roots = rustls::RootCertStore::empty();
        let mut input = std::io::Cursor::new(bytes);
        for certificate in rustls_pemfile::certs(&mut input) {
            if roots.len() >= 128 {
                return Err("healthcheck CA file exceeds 128 certificates".into());
            }
            roots
                .add(certificate.map_err(|_| "invalid healthcheck CA certificate")?)
                .map_err(|_| "invalid healthcheck CA certificate")?;
        }
        if roots.is_empty() {
            return Err("healthcheck CA file contains no certificates".into());
        }
        let config = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|_| "could not configure healthcheck TLS")?
        .with_root_certificates(roots)
        .with_no_client_auth();
        Ok(Self {
            config: Arc::new(config),
            server_name,
        })
    }
}

// Enforce the remaining budget on every underlying syscall, including calls
// made inside rustls handshake/record loops rather than only application I/O.
struct DeadlineSocket {
    socket: TcpStream,
    deadline: Instant,
}

impl Read for DeadlineSocket {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.socket
            .set_read_timeout(Some(remaining(self.deadline)?))?;
        self.socket.read(bytes)
    }
}
impl Write for DeadlineSocket {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.socket
            .set_write_timeout(Some(remaining(self.deadline)?))?;
        self.socket.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        remaining(self.deadline)?;
        self.socket.flush()
    }
}

enum ProbeStream {
    Plain(DeadlineSocket),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, DeadlineSocket>>),
}
impl Read for ProbeStream {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.read(bytes),
            Self::Tls(stream) => stream.read(bytes),
        }
    }
}
impl Write for ProbeStream {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.write(bytes),
            Self::Tls(stream) => stream.write(bytes),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Plain(stream) => stream.flush(),
            Self::Tls(stream) => stream.flush(),
        }
    }
}

pub fn check(
    address: SocketAddr,
    username: Option<&[u8]>,
    password: Option<&[u8]>,
    require_aof: bool,
    tls: Option<TlsSettings>,
) -> io::Result<()> {
    check_with_timeout(
        address,
        username,
        password,
        require_aof,
        tls,
        Duration::from_secs(1),
    )
}

pub fn check_with_aof_write_probe(
    address: SocketAddr,
    username: Option<&[u8]>,
    password: Option<&[u8]>,
    require_aof: bool,
    aof_write_probe_directory: Option<&Path>,
    tls: Option<TlsSettings>,
) -> io::Result<()> {
    if aof_write_probe_directory.is_some() && !require_aof {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "AOF write probe requires the journal-status check",
        ));
    }
    check_with_timeout_and_aof_probe(
        address,
        username,
        password,
        require_aof,
        aof_write_probe_directory,
        tls,
        Duration::from_secs(1),
    )
}

fn remaining(deadline: Instant) -> io::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "health probe deadline exceeded"))
}

fn check_with_timeout(
    address: SocketAddr,
    username: Option<&[u8]>,
    password: Option<&[u8]>,
    require_aof: bool,
    tls: Option<TlsSettings>,
    timeout: Duration,
) -> io::Result<()> {
    check_with_timeout_and_aof_probe(address, username, password, require_aof, None, tls, timeout)
}

fn check_with_timeout_and_aof_probe(
    address: SocketAddr,
    username: Option<&[u8]>,
    password: Option<&[u8]>,
    require_aof: bool,
    aof_write_probe_directory: Option<&Path>,
    tls: Option<TlsSettings>,
    timeout: Duration,
) -> io::Result<()> {
    let deadline = Instant::now() + timeout;
    let socket = TcpStream::connect_timeout(&address, remaining(deadline)?)?;
    socket.set_nodelay(true)?;
    let socket = DeadlineSocket { socket, deadline };
    let mut stream = match tls {
        None => ProbeStream::Plain(socket),
        Some(tls) => {
            let connection =
                rustls::ClientConnection::new(tls.config, tls.server_name).map_err(|_| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "could not initialize healthcheck TLS",
                    )
                })?;
            let mut stream = rustls::StreamOwned::new(connection, socket);
            stream.conn.complete_io(&mut stream.sock)?;
            ProbeStream::Tls(Box::new(stream))
        }
    };
    if let Some(password) = password {
        let mut request = Vec::new();
        if let Some(username) = username {
            rillcache::protocol::encode_request(&[b"AUTH", username, password], &mut request);
        } else {
            rillcache::protocol::encode_request(&[b"AUTH", password], &mut request);
        }
        exchange(&mut stream, &request, b"+OK\r\n", deadline)?;
    }
    exchange(&mut stream, b"*1\r\n$4\r\nPING\r\n", b"+PONG\r\n", deadline)?;
    if require_aof {
        check_persistence(&mut stream, deadline)?;
    }
    if let Some(directory) = aof_write_probe_directory {
        check_aof_write_probe(directory)?;
    }
    Ok(())
}

fn check_aof_write_probe(directory: &Path) -> io::Result<()> {
    for _ in 0..8 {
        let id = NEXT_AOF_PROBE_ID.fetch_add(1, Ordering::Relaxed);
        let path = directory.join(format!(
            ".rillcache-aof-healthcheck-{}-{id}",
            std::process::id()
        ));
        let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(aof_probe_error(error)),
        };
        let mut cleanup = RemoveAofProbe(Some(path.clone()));
        let write_result = file.write_all(b"r").and_then(|()| file.sync_all());
        drop(file);
        let remove_result = fs::remove_file(&path);
        if let Err(error) = write_result {
            return Err(aof_probe_error(error));
        }
        remove_result.map_err(aof_probe_error)?;
        cleanup.0 = None;
        drop(cleanup);
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "AOF write probe could not reserve a temporary file",
    ))
}

fn aof_probe_error(error: io::Error) -> io::Error {
    io::Error::new(error.kind(), "AOF write probe failed")
}

struct RemoveAofProbe(Option<PathBuf>);

impl Drop for RemoveAofProbe {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = fs::remove_file(path);
        }
    }
}

fn send_request(stream: &mut ProbeStream, mut request: &[u8], deadline: Instant) -> io::Result<()> {
    while !request.is_empty() {
        remaining(deadline)?;
        match stream.write(request) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(size) => request = &request[size..],
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    stream.flush()?;
    remaining(deadline)?;
    Ok(())
}

fn read_exact(stream: &mut ProbeStream, reply: &mut [u8], deadline: Instant) -> io::Result<()> {
    let mut received = 0;
    while received < reply.len() {
        remaining(deadline)?;
        match stream.read(&mut reply[received..]) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(size) => received += size,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    remaining(deadline)?;
    Ok(())
}

fn exchange(
    stream: &mut ProbeStream,
    request: &[u8],
    expected: &[u8],
    deadline: Instant,
) -> io::Result<()> {
    send_request(stream, request, deadline)?;
    let mut reply = [0; 7];
    read_exact(stream, &mut reply[..expected.len()], deadline)?;
    if &reply[..expected.len()] != expected {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unexpected health probe reply",
        ));
    }
    Ok(())
}

fn invalid_persistence() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid or unhealthy journal status",
    )
}

fn check_persistence(stream: &mut ProbeStream, deadline: Instant) -> io::Result<()> {
    send_request(
        stream,
        b"*2\r\n$4\r\nINFO\r\n$11\r\npersistence\r\n",
        deadline,
    )?;
    // A bounded RESP2 bulk only: never allocate from an unchecked peer length.
    let mut header = Vec::new();
    while header.len() < 32 {
        let mut byte = [0];
        read_exact(stream, &mut byte, deadline)?;
        header.push(byte[0]);
        if header.ends_with(b"\r\n") {
            break;
        }
    }
    let length = header
        .strip_prefix(b"$")
        .and_then(|value| value.strip_suffix(b"\r\n"))
        .filter(|value| !value.is_empty() && value.iter().all(u8::is_ascii_digit))
        .and_then(|value| std::str::from_utf8(value).ok())
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|length| *length <= 16 * 1024)
        .ok_or_else(invalid_persistence)?;
    let mut body = vec![0; length + 2];
    read_exact(stream, &mut body, deadline)?;
    if !body.ends_with(b"\r\n") {
        return Err(invalid_persistence());
    }
    validate_persistence(&body[..length])
}

fn validate_persistence(body: &[u8]) -> io::Result<()> {
    let text = std::str::from_utf8(body).map_err(|_| invalid_persistence())?;
    let required = [
        ("loading", "0"),
        ("aof_enabled", "1"),
        ("aof_last_write_status", "ok"),
        ("aof_last_bgrewrite_status", "ok"),
    ];
    let mut seen = [false; 4];
    for line in text.split("\r\n") {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (name, value) = line.split_once(':').ok_or_else(invalid_persistence)?;
        if let Some(index) = required.iter().position(|(field, _)| *field == name) {
            if seen[index] || value != required[index].1 {
                return Err(invalid_persistence());
            }
            seen[index] = true;
        }
    }
    if !seen.iter().all(|present| *present) {
        return Err(invalid_persistence());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn aof_write_probe_syncs_a_temporary_byte_and_removes_it() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "rillcache-aof-write-probe-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();

        check_aof_write_probe(&directory).unwrap();

        assert_eq!(fs::read_dir(&directory).unwrap().count(), 0);
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn aof_write_probe_fails_for_a_non_directory_without_modifying_it() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "rillcache-aof-write-probe-file-{}-{nonce}",
            std::process::id()
        ));
        fs::write(&directory, b"unchanged").unwrap();

        assert!(check_aof_write_probe(&directory).is_err());
        assert_eq!(fs::read(&directory).unwrap(), b"unchanged");
        fs::remove_file(directory).unwrap();
    }

    #[test]
    fn rejects_aof_write_probe_without_the_journal_status_check_before_connecting() {
        let directory = std::env::temp_dir();
        let error = check_with_aof_write_probe(
            "127.0.0.1:1".parse().unwrap(),
            None,
            None,
            false,
            Some(&directory),
            None,
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn validates_ping_and_fragmented_pong_but_rejects_bad_or_truncated_replies() {
        for reply in [
            b"+PONG\r\n".as_slice(),
            b"+PANG\r\n",
            b"-ERR no\r\n",
            b"+PON",
            b"",
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            std::thread::scope(|scope| {
                scope.spawn(|| {
                    let (mut stream, _) = listener.accept().unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(3)))
                        .unwrap();
                    let mut request = [0; 14];
                    stream.read_exact(&mut request).unwrap();
                    assert_eq!(&request, b"*1\r\n$4\r\nPING\r\n");
                    for byte in reply {
                        let _ = stream.write_all(&[*byte]);
                    }
                });
                assert_eq!(
                    check(address, None, None, false, None).is_ok(),
                    reply == b"+PONG\r\n"
                );
            });
        }
    }

    #[test]
    fn authenticates_binary_password_before_ping_and_never_pings_after_rejection() {
        let password = b"probe\0\r\n\xff";
        for (accepted, username) in [
            (true, None),
            (false, None),
            (true, Some(b"probe-user".as_slice())),
            (false, Some(b"probe-user".as_slice())),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            std::thread::scope(|scope| {
                scope.spawn(|| {
                    let (mut stream, _) = listener.accept().unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(3)))
                        .unwrap();
                    let mut expected = Vec::new();
                    if let Some(username) = username {
                        rillcache::protocol::encode_request(
                            &[b"AUTH", username, password],
                            &mut expected,
                        );
                    } else {
                        rillcache::protocol::encode_request(&[b"AUTH", password], &mut expected);
                    }
                    let mut request = vec![0; expected.len()];
                    stream.read_exact(&mut request).unwrap();
                    assert_eq!(request, expected);
                    if accepted {
                        for byte in b"+OK\r\n" {
                            stream.write_all(&[*byte]).unwrap();
                        }
                        let mut ping = [0; 14];
                        stream.read_exact(&mut ping).unwrap();
                        assert_eq!(&ping, b"*1\r\n$4\r\nPING\r\n");
                        stream.write_all(b"+PONG\r\n").unwrap();
                    } else {
                        stream
                            .write_all(b"-ERR credential-must-not-leak\r\n")
                            .unwrap();
                        let mut next = [0];
                        // The client closes without sending PING or reflecting the error.
                        match stream.read(&mut next) {
                            Ok(0) => {}
                            Err(error) if error.kind() == io::ErrorKind::ConnectionReset => {}
                            other => panic!("probe did not close after AUTH rejection: {other:?}"),
                        }
                    }
                });
                let result = check(address, username, Some(password), false, None);
                assert_eq!(result.is_ok(), accepted);
                if let Err(error) = result {
                    assert!(!error.to_string().contains("credential-must-not-leak"));
                }
            });
        }
    }

    const HEALTHY: &str = "# Persistence\r\nloading:0\r\naof_enabled:1\r\naof_last_write_status:ok\r\naof_last_bgrewrite_status:ok\r\n";

    #[test]
    fn persistence_fields_fail_closed_on_missing_duplicate_or_reported_errors() {
        assert!(validate_persistence(HEALTHY.as_bytes()).is_ok());
        assert!(
            validate_persistence(format!("{HEALTHY}aof_rewrite_in_progress:1\r\n").as_bytes())
                .is_ok()
        );
        for (field, value) in [
            ("loading", "0"),
            ("aof_enabled", "1"),
            ("aof_last_write_status", "ok"),
            ("aof_last_bgrewrite_status", "ok"),
        ] {
            let line = format!("{field}:{value}\r\n");
            for body in [
                HEALTHY.replace(&line, ""),
                HEALTHY.replace(&line, &format!("{field}:err\r\n")),
                format!("{HEALTHY}{line}"),
            ] {
                assert!(
                    validate_persistence(body.as_bytes()).is_err(),
                    "field {field}"
                );
            }
        }
        for body in [
            b"".as_slice(),
            b"not-info",
            b"\xff",
            b"loading:0\naof_enabled:1\n",
        ] {
            assert!(validate_persistence(body).is_err());
        }
    }

    #[test]
    fn persistence_probe_bounds_and_validates_fragmented_bulk_replies() {
        let healthy = [
            format!("${}\r\n", HEALTHY.len()).as_bytes(),
            HEALTHY.as_bytes(),
            b"\r\n",
        ]
        .concat();
        for (accepted, reply) in [
            (true, healthy),
            (false, b"$-1\r\n".to_vec()),
            (false, b"$16385\r\n".to_vec()),
            (false, b"$999999999999999999999999999999999999\r\n".to_vec()),
            (false, b"$3\r\nab".to_vec()),
            (false, b"$3\r\nabcxx".to_vec()),
            (false, b"-NOPERM secret-server-text\r\n".to_vec()),
            (false, b"*0\r\n".to_vec()),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            std::thread::scope(|scope| {
                scope.spawn(|| {
                    let (mut stream, _) = listener.accept().unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(3)))
                        .unwrap();
                    let mut ping = [0; 14];
                    stream.read_exact(&mut ping).unwrap();
                    assert_eq!(&ping, b"*1\r\n$4\r\nPING\r\n");
                    stream.write_all(b"+PONG\r\n").unwrap();
                    let expected = b"*2\r\n$4\r\nINFO\r\n$11\r\npersistence\r\n";
                    let mut request = vec![0; expected.len()];
                    stream.read_exact(&mut request).unwrap();
                    assert_eq!(request, expected);
                    for byte in &reply {
                        if stream.write_all(&[*byte]).is_err() {
                            break;
                        }
                    }
                });
                let result = check(address, None, None, true, None);
                assert_eq!(result.is_ok(), accepted);
                if let Err(error) = result {
                    assert!(!error.to_string().contains("secret-server-text"));
                }
            });
        }
    }

    #[test]
    fn persistence_read_uses_the_same_bounded_socket_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (release, wait) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            scope.spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut ping = [0; 14];
                stream.read_exact(&mut ping).unwrap();
                stream.write_all(b"+PONG\r\n").unwrap();
                let mut request = [0; 32];
                stream.read_exact(&mut request).unwrap();
                wait.recv_timeout(Duration::from_secs(5)).unwrap();
            });
            let started = Instant::now();
            let result =
                check_with_timeout(address, None, None, true, None, Duration::from_millis(100));
            assert!(result.is_err());
            assert!(started.elapsed() < Duration::from_secs(3));
            release.send(()).unwrap();
        });
    }

    #[test]
    fn a_silent_tls_peer_cannot_extend_the_handshake_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let ca = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tls/ca.crt");
        let tls = TlsSettings::from_ca_file(ca.to_str().unwrap(), None, address).unwrap();
        let (release, wait) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            scope.spawn(move || {
                let (_stream, _) = listener.accept().unwrap();
                wait.recv_timeout(Duration::from_secs(5)).unwrap();
            });
            let started = Instant::now();
            assert!(
                check_with_timeout(
                    address,
                    None,
                    None,
                    false,
                    Some(tls),
                    Duration::from_millis(100)
                )
                .is_err()
            );
            assert!(started.elapsed() < Duration::from_secs(3));
            release.send(()).unwrap();
        });
    }

    #[test]
    fn a_silent_peer_cannot_keep_the_probe_alive() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (release, wait) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            scope.spawn(move || {
                let (_stream, _) = listener.accept().unwrap();
                wait.recv_timeout(Duration::from_secs(5)).unwrap();
            });
            let started = Instant::now();
            assert!(
                check_with_timeout(address, None, None, false, None, Duration::from_millis(100))
                    .is_err()
            );
            assert!(started.elapsed() < Duration::from_secs(3));
            release.send(()).unwrap();
        });
    }
}
