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
fn tls_plaintext_frame_assembly_deadline_closes_trickling_resp() {
    let server = Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        tls_bind_addr: Some("127.0.0.1:0".parse().unwrap()),
        tls_cert_file: Some(fixture("server.crt")),
        tls_key_file: Some(fixture("server.key")),
        idle_timeout: Duration::from_secs(2),
        frame_assembly_timeout: Some(Duration::from_millis(250)),
        ..Config::default()
    })
    .unwrap()
    .spawn()
    .unwrap();
    let mut slow = connect(&server, false);
    slow.write_all(b"*").unwrap();
    slow.flush().unwrap();
    for byte in b"1\r\n$4\r\nPING\r\n" {
        std::thread::sleep(Duration::from_millis(60));
        if slow.write_all(&[*byte]).is_err() || slow.flush().is_err() {
            break;
        }
    }
    let mut response = [0u8; 1];
    assert!(slow.read(&mut response).is_err() || response[0] == 0);
    server.shutdown().unwrap();
}

#[test]
fn tls_record_trickle_after_resp_frame_start_obeys_absolute_deadline() {
    let server = Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        tls_bind_addr: Some("127.0.0.1:0".parse().unwrap()),
        tls_cert_file: Some(fixture("server.crt")),
        tls_key_file: Some(fixture("server.key")),
        idle_timeout: Duration::from_secs(2),
        frame_assembly_timeout: Some(Duration::from_millis(250)),
        ..Config::default()
    })
    .unwrap()
    .spawn()
    .unwrap();
    let mut control = connect(&server, false);
    control
        .write_all(b"*1\r\n$4\r\nPING\r\n*1\r\n$4\r\nPI")
        .unwrap();
    let mut pong = [0u8; 7];
    control.read_exact(&mut pong).unwrap();
    assert_eq!(&pong, b"+PONG\r\n");
    let started = std::time::Instant::now();
    let mut byte = [0u8; 1];
    let control_read = control.read(&mut byte);
    assert!(matches!(control_read, Err(_) | Ok(0)));
    assert!(started.elapsed() < Duration::from_millis(800));

    let mut tls = connect(&server, false);
    tls.write_all(b"*1\r\n$4\r\nPING\r\n*1\r\n$4\r\nPI")
        .unwrap();
    tls.read_exact(&mut pong).unwrap();
    assert_eq!(&pong, b"+PONG\r\n");

    // Generate one valid encrypted record containing the final plaintext bytes,
    // then drip its ciphertext into the socket. The server already parsed the
    // RESP prefix, so the absolute frame deadline must bound rustls' internal
    // record reads as well as the outer RESP read.
    tls.conn.writer().write_all(b"NG\r\n").unwrap();
    let mut record = Vec::new();
    tls.conn.write_tls(&mut record).unwrap();
    assert!(record.len() >= 24);
    let mut raw = tls.sock.try_clone().unwrap();
    let dripper = std::thread::spawn(move || {
        for byte in record {
            std::thread::sleep(Duration::from_millis(40));
            if raw.write_all(&[byte]).is_err() {
                break;
            }
        }
    });
    let started = std::time::Instant::now();
    let read = tls.read(&mut byte);
    let elapsed = started.elapsed();
    assert!(matches!(read, Err(_) | Ok(0)));
    assert!(
        elapsed < Duration::from_millis(800),
        "closed after {elapsed:?}"
    );
    dripper.join().unwrap();
    let mut healthy = connect(&server, false);
    ping(&mut healthy);
    server.shutdown().unwrap();
}

#[test]
fn completed_tls_frame_clears_deadline_for_later_commands() {
    let server = Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        tls_bind_addr: Some("127.0.0.1:0".parse().unwrap()),
        tls_cert_file: Some(fixture("server.crt")),
        tls_key_file: Some(fixture("server.key")),
        frame_assembly_timeout: Some(Duration::from_millis(250)),
        ..Config::default()
    })
    .unwrap()
    .spawn()
    .unwrap();
    let mut tls = connect(&server, false);
    tls.write_all(b"*1\r\n$4\r\nPING\r\n*1\r\n$4\r\nPI")
        .unwrap();
    let mut pong = [0u8; 7];
    tls.read_exact(&mut pong).unwrap();
    assert_eq!(&pong, b"+PONG\r\n");
    tls.write_all(b"NG\r\n").unwrap();
    tls.read_exact(&mut pong).unwrap();
    assert_eq!(&pong, b"+PONG\r\n");
    std::thread::sleep(Duration::from_millis(350));
    ping(&mut tls);
    server.shutdown().unwrap();
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
