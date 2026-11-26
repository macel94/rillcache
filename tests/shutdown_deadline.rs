#![forbid(unsafe_code)]
#![cfg(target_os = "linux")]

//! Supervisor-deadline exhaustion on every supported transport (D7).
//!
//! The shutdown path previously had a fixed 25-second grace period, so the
//! forced-termination branch could only be triggered by a second signal. These
//! tests configure a short deadline, leave an admitted reply unread so the
//! worker cannot join, and require the documented forced-termination outcome:
//! a logged reason, a nonzero exit, a bounded exit time and a clean restart
//! from the unchanged journal.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rillcache::protocol::encode_request;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

const DEADLINE_MS: &str = "300";
const VALUE_LEN: usize = 1024 * 1024;

#[derive(Clone, Copy, PartialEq)]
enum Transport {
    Mio,
    Tokio,
    Tls,
}

fn unique_dir(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rillcache-deadline-{label}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/tls")
        .join(name)
}

struct Server {
    child: Child,
    plain: String,
    tls: Option<String>,
    _stderr: BufReader<ChildStderr>,
}

impl Server {
    fn start(directory: &Path, transport: Transport) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rillcache"));
        command
            .args([
                "--bind",
                "127.0.0.1:0",
                "--appendonly",
                "yes",
                "--appendfsync",
                "always",
                "--dir",
            ])
            .arg(directory)
            .args(["--shutdown-deadline-ms", DEADLINE_MS])
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        match transport {
            Transport::Mio => {}
            Transport::Tokio => {
                command.args(["--io-mode", "tokio"]);
            }
            Transport::Tls => {
                command
                    .arg("--tls-bind")
                    .arg("127.0.0.1:0")
                    .arg("--tls-cert-file")
                    .arg(fixture("server.crt"))
                    .arg("--tls-key-file")
                    .arg(fixture("server.key"));
            }
        }
        let mut child = command.spawn().expect("spawn rillcache");
        let mut stderr = BufReader::new(child.stderr.take().expect("stderr"));
        let plain = read_listener(&mut stderr, false);
        let tls = (transport == Transport::Tls).then(|| read_listener(&mut stderr, true));
        Self {
            child,
            plain,
            tls,
            _stderr: stderr,
        }
    }

    fn address(&self, transport: Transport) -> String {
        match transport {
            Transport::Tls => self.tls.clone().expect("tls listener"),
            _ => self.plain.clone(),
        }
    }

    fn signal(&self) {
        let status = Command::new("kill")
            .args(["-TERM", &self.child.id().to_string()])
            .status()
            .expect("kill");
        assert!(status.success(), "SIGTERM was not delivered");
    }

    fn wait(mut self, limit: Duration) -> (std::process::ExitStatus, String) {
        let deadline = Instant::now() + limit;
        loop {
            if let Some(status) = self.child.try_wait().expect("try_wait") {
                let mut diagnostics = String::new();
                let _ = self._stderr.read_to_string(&mut diagnostics);
                return (status, diagnostics);
            }
            assert!(Instant::now() < deadline, "server exceeded the deadline");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn read_listener(stderr: &mut BufReader<ChildStderr>, tls: bool) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    let needle = if tls {
        "rillcache TLS 1.3 listening on "
    } else {
        " listening on "
    };
    let mut line = String::new();
    while Instant::now() < deadline {
        line.clear();
        if stderr.read_line(&mut line).expect("startup line") == 0 {
            panic!("server exited during startup");
        }
        if line.contains(needle) {
            return line
                .split(needle)
                .nth(1)
                .expect("listener address")
                .trim()
                .to_owned();
        }
    }
    panic!("listener was not reported: {line:?}");
}

enum Connection {
    Tcp(TcpStream),
    Tls(Box<StreamOwned<ClientConnection, TcpStream>>),
}

impl Read for Connection {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Tcp(stream) => stream.read(buffer),
            Self::Tls(stream) => stream.read(buffer),
        }
    }
}

impl Write for Connection {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Tcp(stream) => stream.write(buffer),
            Self::Tls(stream) => stream.write(buffer),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Tcp(stream) => stream.flush(),
            Self::Tls(stream) => stream.flush(),
        }
    }
}

fn roots() -> RootCertStore {
    let mut store = RootCertStore::empty();
    let file = std::fs::File::open(fixture("ca.crt")).unwrap();
    for cert in rustls_pemfile::certs(&mut std::io::BufReader::new(file)) {
        store.add(cert.unwrap()).unwrap();
    }
    store
}

fn tls_stream(address: &str) -> StreamOwned<ClientConnection, TcpStream> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_root_certificates(roots())
        .with_no_client_auth();
    let name = rustls::pki_types::ServerName::try_from("127.0.0.1").unwrap();
    let connection = ClientConnection::new(Arc::new(config), name).unwrap();
    let socket = TcpStream::connect(address).unwrap();
    socket.set_nodelay(true).unwrap();
    StreamOwned::new(connection, socket)
}

fn exercise(transport: Transport) {
    let label = match transport {
        Transport::Mio => "mio",
        Transport::Tokio => "tokio",
        Transport::Tls => "tls",
    };
    let directory = unique_dir(label);
    let server = Server::start(&directory, transport);
    let address = server.address(transport);
    let value = vec![b'v'; VALUE_LEN];
    let mut request = Vec::new();
    encode_request(&[b"SET", b"payload", &value], &mut request);

    let mut stream = match transport {
        Transport::Tls => Connection::Tls(Box::new(tls_stream(&address))),
        _ => {
            let socket = TcpStream::connect(&address).unwrap();
            socket.set_nodelay(true).unwrap();
            Connection::Tcp(socket)
        }
    };
    stream.write_all(&request).unwrap();
    let mut reply = [0u8; 5];
    stream.read_exact(&mut reply).unwrap();
    assert_eq!(&reply, b"+OK\r\n");
    // Queue more reads than any socket buffer can absorb, then read one byte so
    // the worker is provably inside a blocked write when the signal lands. The
    // connection stays open until after the deadline has been observed.
    let mut pipeline = Vec::new();
    for _ in 0..64 {
        encode_request(&[b"GET", b"payload"], &mut pipeline);
    }
    stream.write_all(&pipeline).unwrap();
    let mut first = [0u8; 1];
    stream.read_exact(&mut first).unwrap();
    assert_eq!(first, [b'$']);
    server.signal();

    let (status, diagnostics) = server.wait(Duration::from_secs(15));
    assert!(
        !status.success(),
        "{label}: forced shutdown exited successfully"
    );
    assert!(
        diagnostics.contains("forced termination: shutdown deadline exceeded"),
        "{label}: missing forced-termination reason: {diagnostics:?}"
    );

    // The acknowledged write must still replay from the unchanged journal.
    let server = Server::start(&directory, Transport::Mio);
    let mut stream = TcpStream::connect(server.address(Transport::Mio)).unwrap();
    stream.set_nodelay(true).unwrap();
    let mut get = Vec::new();
    encode_request(&[b"STRLEN", b"payload"], &mut get);
    stream.write_all(&get).unwrap();
    let mut header = [0u8; 1];
    stream.read_exact(&mut header).unwrap();
    assert_eq!(header, [b':']);
    let mut digits = Vec::new();
    loop {
        let mut byte = [0u8; 1];
        stream.read_exact(&mut byte).unwrap();
        if byte[0] == b'\n' {
            break;
        }
        digits.push(byte[0]);
    }
    let length: usize = std::str::from_utf8(&digits[..digits.len() - 1])
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(length, VALUE_LEN, "{label}: acknowledged value was lost");
    server.signal();
    let _ = server.wait(Duration::from_secs(15));
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn deadline_exhaustion_forces_exit_on_plain_mio() {
    exercise(Transport::Mio);
}

#[test]
fn deadline_exhaustion_forces_exit_on_plain_tokio() {
    exercise(Transport::Tokio);
}

#[test]
fn deadline_exhaustion_forces_exit_on_native_tls() {
    exercise(Transport::Tls);
}
