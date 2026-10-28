#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use rillcache::{Config, Server, ServerHandle};
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/tls")
        .join(name)
}

fn start(workers: Option<usize>, client_ca: bool) -> ServerHandle {
    Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        tls_bind_addr: Some("127.0.0.1:0".parse().unwrap()),
        tls_cert_file: Some(fixture("server.crt")),
        tls_key_file: Some(fixture("server.key")),
        tls_client_ca_file: client_ca.then(|| fixture("ca.crt")),
        worker_threads: workers,
        ..Config::default()
    })
    .unwrap()
    .spawn()
    .unwrap()
}

fn roots() -> RootCertStore {
    let mut store = RootCertStore::empty();
    let file = std::fs::File::open(fixture("ca.crt")).unwrap();
    for cert in rustls_pemfile::certs(&mut std::io::BufReader::new(file)) {
        store.add(cert.unwrap()).unwrap();
    }
    store
}

fn client_config(with_client_cert: bool, tls12: bool) -> Arc<ClientConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let versions: &[&rustls::SupportedProtocolVersion] = if tls12 {
        &[&rustls::version::TLS12]
    } else {
        &[&rustls::version::TLS13]
    };
    let builder = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(versions)
        .unwrap()
        .with_root_certificates(roots());
    let config = if with_client_cert {
        let mut cert_reader =
            std::io::BufReader::new(std::fs::File::open(fixture("client.crt")).unwrap());
        let certs = rustls_pemfile::certs(&mut cert_reader)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let mut key_reader =
            std::io::BufReader::new(std::fs::File::open(fixture("client.key")).unwrap());
        let key = rustls_pemfile::private_key(&mut key_reader)
            .unwrap()
            .unwrap();
        builder.with_client_auth_cert(certs, key).unwrap()
    } else {
        builder.with_no_client_auth()
    };
    Arc::new(config)
}

fn connect(
    server: &ServerHandle,
    with_client_cert: bool,
) -> StreamOwned<ClientConnection, TcpStream> {
    let tcp = TcpStream::connect(server.tls_local_addr().unwrap()).unwrap();
    tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    tcp.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
    let connection = ClientConnection::new(
        client_config(with_client_cert, false),
        ServerName::try_from("localhost").unwrap(),
    )
    .unwrap();
    StreamOwned::new(connection, tcp)
}

fn ping(stream: &mut StreamOwned<ClientConnection, TcpStream>) {
    stream.write_all(b"*1\r\n$4\r\nPING\r\n").unwrap();
    let mut reply = [0u8; 7];
    stream.read_exact(&mut reply).unwrap();
    assert_eq!(&reply, b"+PONG\r\n");
}

#[test]
fn tls13_resp_works_in_all_execution_modes_and_plaintext_is_unchanged() {
    for workers in [None, Some(1), Some(4)] {
        let server = start(workers, false);
        let mut stream = connect(&server, false);
        ping(&mut stream);
        assert_eq!(
            stream.conn.alpn_protocol(),
            None,
            "HTTP ALPN must not be negotiated"
        );
        assert_eq!(
            stream.conn.protocol_version(),
            Some(rustls::ProtocolVersion::TLSv1_3)
        );

        let mut plain = TcpStream::connect(server.local_addr()).unwrap();
        plain
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        plain.write_all(b"*1\r\n$4\r\nPING\r\n").unwrap();
        let mut reply = [0u8; 7];
        plain.read_exact(&mut reply).unwrap();
        assert_eq!(&reply, b"+PONG\r\n");

        server.shutdown().unwrap();
    }
}

#[test]
fn tls12_is_rejected_and_client_certificates_can_be_required() {
    let server = start(None, false);
    let tcp = TcpStream::connect(server.tls_local_addr().unwrap()).unwrap();
    tcp.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    tcp.set_write_timeout(Some(Duration::from_secs(3))).unwrap();
    let connection = ClientConnection::new(
        client_config(false, true),
        ServerName::try_from("localhost").unwrap(),
    )
    .unwrap();
    let mut stream = StreamOwned::new(connection, tcp);
    let mut reply = [0u8; 1];
    assert!(stream.write_all(b"*1\r\n$4\r\nPING\r\n").is_err() || stream.read(&mut reply).is_err());
    drop(stream);
    server.shutdown().unwrap();

    let server = start(None, true);
    let tcp = TcpStream::connect(server.tls_local_addr().unwrap()).unwrap();
    tcp.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    tcp.set_write_timeout(Some(Duration::from_secs(3))).unwrap();
    let connection = ClientConnection::new(
        client_config(false, false),
        ServerName::try_from("localhost").unwrap(),
    )
    .unwrap();
    let mut missing = StreamOwned::new(connection, tcp);
    assert!(
        missing
            .write_all(b"*1\r\n$4\r\nPING\r\n")
            .and_then(|()| {
                let mut byte = [0u8; 1];
                missing.read(&mut byte).map(|_| ())
            })
            .is_err()
    );
    drop(missing);

    let mut authenticated = connect(&server, true);
    ping(&mut authenticated);
    server.shutdown().unwrap();
}

#[test]
fn invalid_tls_configuration_fails_before_listening() {
    let base = Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        tls_bind_addr: Some("127.0.0.1:0".parse().unwrap()),
        tls_cert_file: Some(fixture("server.crt")),
        tls_key_file: Some(fixture("server.key")),
        ..Config::default()
    };
    assert!(Server::bind(base.clone()).is_ok());
    let mut missing_key = base.clone();
    missing_key.tls_key_file = None;
    assert!(Server::bind(missing_key).is_err());
    let mut same_address = base.clone();
    same_address.bind_addr = "127.0.0.1:65000".parse().unwrap();
    same_address.tls_bind_addr = Some(same_address.bind_addr);
    assert!(Server::bind(same_address).is_err());
    let ca_without_bind = Config {
        tls_client_ca_file: Some(fixture("ca.crt")),
        ..Config::default()
    };
    assert!(Server::bind(ca_without_bind).is_err());
    let mut missing_cert = base;
    missing_cert.tls_cert_file = Some(fixture("missing.crt"));
    assert!(Server::bind(missing_cert).is_err());
}
