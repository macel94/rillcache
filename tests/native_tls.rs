#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rillcache::{Config, Server, ServerHandle};
use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{
    ClientConfig, ClientConnection, DigitallySignedStruct, Error, RootCertStore, SignatureScheme,
    StreamOwned,
};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/tls")
        .join(name)
}

fn start(workers: Option<usize>, client_ca: bool) -> ServerHandle {
    start_with_timeouts(
        workers,
        client_ca,
        Duration::from_secs(5),
        Duration::from_secs(30),
        128,
    )
}

fn start_with_timeouts(
    workers: Option<usize>,
    client_ca: bool,
    handshake_timeout: Duration,
    idle_timeout: Duration,
    max_clients: usize,
) -> ServerHandle {
    Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        tls_bind_addr: Some("127.0.0.1:0".parse().unwrap()),
        tls_cert_file: Some(fixture("server.crt")),
        tls_key_file: Some(fixture("server.key")),
        tls_client_ca_file: client_ca.then(|| fixture("ca.crt")),
        tls_handshake_timeout: handshake_timeout,
        idle_timeout,
        max_clients,
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

#[derive(Debug)]
struct OcspCaptureVerifier {
    inner: Arc<WebPkiServerVerifier>,
    responses: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl ServerCertVerifier for OcspCaptureVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        self.responses.lock().unwrap().push(ocsp_response.to_vec());
        let verify_name = if matches!(server_name, ServerName::IpAddress(_)) {
            ServerName::try_from("localhost").unwrap()
        } else {
            server_name.to_owned()
        };
        self.inner
            .verify_server_cert(end_entity, intermediates, &verify_name, &[], now)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

fn client_config_with_ocsp_capture(
    responses: Arc<Mutex<Vec<Vec<u8>>>>,
    with_client_cert: bool,
) -> Arc<ClientConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier =
        WebPkiServerVerifier::builder_with_provider(Arc::new(roots()), Arc::clone(&provider))
            .build()
            .unwrap();
    let verifier = Arc::new(OcspCaptureVerifier {
        inner: verifier,
        responses,
    });
    let builder = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(verifier);
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
    connect_named(server, with_client_cert, "localhost")
}

fn connect_named(
    server: &ServerHandle,
    with_client_cert: bool,
    server_name: &str,
) -> StreamOwned<ClientConnection, TcpStream> {
    let tcp = TcpStream::connect(server.tls_local_addr().unwrap()).unwrap();
    tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    tcp.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
    let connection = ClientConnection::new(
        client_config(with_client_cert, false),
        ServerName::try_from(server_name.to_owned()).unwrap(),
    )
    .unwrap();
    StreamOwned::new(connection, tcp)
}

fn start_with_sni(client_ca: bool) -> ServerHandle {
    let mut config = Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        tls_bind_addr: Some("127.0.0.1:0".parse().unwrap()),
        tls_cert_file: Some(fixture("server.crt")),
        tls_key_file: Some(fixture("server.key")),
        tls_client_ca_file: client_ca.then(|| fixture("ca.crt")),
        tls_sni_certificates: vec![
            rillcache::TlsSniCertificate {
                server_name: "alt.test".into(),
                cert_file: fixture("alternate.crt"),
                key_file: fixture("alternate.key"),
                ocsp_file: None,
            },
            rillcache::TlsSniCertificate {
                server_name: "localhost".into(),
                cert_file: fixture("server.crt"),
                key_file: fixture("server.key"),
                ocsp_file: None,
            },
        ],
        ..Config::default()
    };
    config.idle_timeout = Duration::from_secs(30);
    Server::bind(config).unwrap().spawn().unwrap()
}

fn connect_capturing_ocsp(
    server: &ServerHandle,
    server_name: &str,
    responses: Arc<Mutex<Vec<Vec<u8>>>>,
) -> StreamOwned<ClientConnection, TcpStream> {
    connect_capturing_ocsp_with_client_cert(server, server_name, responses, false)
}

fn connect_capturing_ocsp_with_client_cert(
    server: &ServerHandle,
    server_name: &str,
    responses: Arc<Mutex<Vec<Vec<u8>>>>,
    with_client_cert: bool,
) -> StreamOwned<ClientConnection, TcpStream> {
    let tcp = TcpStream::connect(server.tls_local_addr().unwrap()).unwrap();
    tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    tcp.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
    let connection = ClientConnection::new(
        client_config_with_ocsp_capture(responses, with_client_cert),
        ServerName::try_from(server_name.to_owned()).unwrap(),
    )
    .unwrap();
    StreamOwned::new(connection, tcp)
}

fn connect_without_sni_capturing_ocsp(
    server: &ServerHandle,
    responses: Arc<Mutex<Vec<Vec<u8>>>>,
) -> StreamOwned<ClientConnection, TcpStream> {
    let tcp = TcpStream::connect(server.tls_local_addr().unwrap()).unwrap();
    tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    tcp.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
    let connection = ClientConnection::new(
        client_config_with_ocsp_capture(responses, false),
        ServerName::IpAddress("127.0.0.1".parse::<std::net::IpAddr>().unwrap().into()),
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

fn assert_tls_acl_user(stream: &mut StreamOwned<ClientConnection, TcpStream>, username: &[u8]) {
    stream
        .write_all(b"*2\r\n$3\r\nACL\r\n$6\r\nWHOAMI\r\n")
        .unwrap();
    let mut expected = format!("${}\r\n", username.len()).into_bytes();
    expected.extend_from_slice(username);
    expected.extend_from_slice(b"\r\n");
    let mut actual = vec![0; expected.len()];
    stream.read_exact(&mut actual).unwrap();
    assert_eq!(actual, expected);
}

fn read_resp_line(stream: &mut impl Read) -> Vec<u8> {
    let mut line = Vec::new();
    loop {
        let mut byte = [0];
        stream.read_exact(&mut byte).unwrap();
        line.push(byte[0]);
        if byte[0] == b'\n' {
            return line;
        }
    }
}

#[test]
fn ocsp_staples_follow_the_selected_default_or_sni_identity_and_absence_is_compatible() {
    let directory = std::env::temp_dir().join(format!(
        "rillcache-ocsp-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let default_ocsp = directory.join("default.ocsp.der");
    let alternate_ocsp = directory.join("alternate.ocsp.der");
    let default_response = [0x30, 0x03, 0x0a, 0x01, 0x00];
    let alternate_response = [0x30, 0x03, 0x0a, 0x01, 0x01];
    std::fs::write(&default_ocsp, default_response).unwrap();
    std::fs::write(&alternate_ocsp, alternate_response).unwrap();

    let mut config = Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        tls_bind_addr: Some("127.0.0.1:0".parse().unwrap()),
        tls_cert_file: Some(fixture("server.crt")),
        tls_key_file: Some(fixture("server.key")),
        tls_ocsp_file: Some(default_ocsp.clone()),
        tls_sni_certificates: vec![
            rillcache::TlsSniCertificate {
                server_name: "localhost".into(),
                cert_file: fixture("server.crt"),
                key_file: fixture("server.key"),
                ocsp_file: Some(default_ocsp),
            },
            rillcache::TlsSniCertificate {
                server_name: "alt.test".into(),
                cert_file: fixture("alternate.crt"),
                key_file: fixture("alternate.key"),
                ocsp_file: Some(alternate_ocsp),
            },
        ],
        ..Config::default()
    };
    config.idle_timeout = Duration::from_secs(30);
    for workers in [None, Some(1), Some(4)] {
        let mut mode_config = config.clone();
        mode_config.worker_threads = workers;
        let server = Server::bind(mode_config).unwrap().spawn().unwrap();
        for (name, expected) in [
            ("localhost", default_response),
            ("alt.test", alternate_response),
        ] {
            let responses = Arc::new(Mutex::new(Vec::new()));
            let mut stream = connect_capturing_ocsp(&server, name, Arc::clone(&responses));
            ping(&mut stream);
            assert_eq!(responses.lock().unwrap().as_slice(), &[expected]);
        }
        let responses = Arc::new(Mutex::new(Vec::new()));
        let mut no_sni = connect_without_sni_capturing_ocsp(&server, Arc::clone(&responses));
        ping(&mut no_sni);
        assert_eq!(responses.lock().unwrap().as_slice(), &[default_response]);
        let mut wrong_sni = connect_named(&server, false, "wrong.test");
        let handshake = wrong_sni.write_all(b"*1\r\n$4\r\nPING\r\n").and_then(|()| {
            let mut reply = [0u8; 7];
            wrong_sni.read_exact(&mut reply)
        });
        assert!(
            handshake.is_err(),
            "unknown SNI must not receive a different OCSP identity"
        );
        server.shutdown().unwrap();
    }

    let server = start(None, false);
    let responses = Arc::new(Mutex::new(Vec::new()));
    let mut stream = connect_capturing_ocsp(&server, "localhost", Arc::clone(&responses));
    ping(&mut stream);
    assert_eq!(responses.lock().unwrap().as_slice(), &[Vec::<u8>::new()]);
    server.shutdown().unwrap();
    std::fs::remove_dir_all(directory).unwrap();
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
fn fragmented_tls_pipeline_keeps_reply_order_and_resumes_a_partial_frame() {
    use rillcache::protocol::encode_request;

    for workers in [None, Some(1), Some(4)] {
        let server = start(workers, false);
        for fragment_bytes in [1, 7, 31] {
            let mut stream = connect(&server, false);
            stream.sock.set_nodelay(true).unwrap();
            ping(&mut stream);

            let binary = b"\0\r\n\xff";
            let mut pipeline = Vec::new();
            for args in [
                &[b"SET".as_slice(), b"tls:binary", binary][..],
                &[b"SET".as_slice(), b"tls:counter", b"0"][..],
                &[b"INCR".as_slice(), b"tls:counter"][..],
                &[b"GET".as_slice(), b"tls:binary"][..],
            ] {
                encode_request(args, &mut pipeline);
            }
            let mut trailing = Vec::new();
            encode_request(&[b"INCR", b"tls:counter"], &mut trailing);
            let last = trailing.pop().unwrap();
            pipeline.extend_from_slice(&trailing);
            for fragment in pipeline.chunks(fragment_bytes) {
                stream.write_all(fragment).unwrap();
                stream.flush().unwrap();
            }

            // Complete replies must not wait for the next request's missing LF.
            let expected = b"+OK\r\n+OK\r\n:1\r\n$4\r\n\0\r\n\xff\r\n";
            let mut replies = vec![0; expected.len()];
            stream.read_exact(&mut replies).unwrap();
            assert_eq!(replies.as_slice(), expected);

            // Complete the final INCR and bracket it with an independent GET.
            let mut rest = vec![last];
            encode_request(&[b"GET", b"tls:counter"], &mut rest);
            stream.write_all(&rest).unwrap();
            stream.flush().unwrap();
            let mut replies = [0; 11];
            stream.read_exact(&mut replies).unwrap();
            assert_eq!(&replies, b":2\r\n$1\r\n2\r\n");
            ping(&mut stream);
        }
        server.shutdown().unwrap();
    }
}

#[test]
fn sni_selects_exact_certificate_and_preserves_required_mtls() {
    for client_ca in [false, true] {
        let server = start_with_sni(client_ca);
        let mut alternate = connect_named(&server, client_ca, "alt.test");
        ping(&mut alternate);
        assert_eq!(
            alternate.conn.protocol_version(),
            Some(rustls::ProtocolVersion::TLSv1_3)
        );

        let mut default = connect(&server, client_ca);
        ping(&mut default);
        server.shutdown().unwrap();
    }
}

#[test]
fn unknown_sni_is_rejected_instead_of_receiving_the_default_certificate() {
    let server = start_with_sni(false);
    let mut stream = connect_named(&server, false, "unknown.test");
    let result = stream.conn.complete_io(&mut stream.sock);
    assert!(result.is_err(), "unknown SNI must abort the handshake");
    server.shutdown().unwrap();
}

#[test]
fn tls_reload_trigger_rotates_new_handshakes_and_keeps_last_valid_config_on_failure() {
    let directory = std::env::temp_dir().join(format!(
        "rillcache-tls-reload-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let cert_file = directory.join("server.crt");
    let key_file = directory.join("server.key");
    let trigger = directory.join("generation");
    let ocsp_file = directory.join("server.ocsp.der");
    let response_one = [0x30, 0x03, 0x0a, 0x01, 0x00];
    let response_two = [0x30, 0x03, 0x0a, 0x01, 0x01];
    std::fs::copy(fixture("server.crt"), &cert_file).unwrap();
    std::fs::copy(fixture("server.key"), &key_file).unwrap();
    std::fs::write(&ocsp_file, response_one).unwrap();
    std::fs::write(&trigger, b"generation-1").unwrap();

    let server = Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        tls_bind_addr: Some("127.0.0.1:0".parse().unwrap()),
        tls_cert_file: Some(cert_file.clone()),
        tls_key_file: Some(key_file.clone()),
        tls_ocsp_file: Some(ocsp_file.clone()),
        tls_reload_trigger: Some(trigger.clone()),
        idle_timeout: Duration::from_secs(30),
        ..Config::default()
    })
    .unwrap()
    .spawn()
    .unwrap();
    let initial_responses = Arc::new(Mutex::new(Vec::new()));
    let mut established =
        connect_capturing_ocsp(&server, "localhost", Arc::clone(&initial_responses));
    ping(&mut established);
    assert_eq!(
        initial_responses.lock().unwrap().as_slice(),
        &[response_one]
    );

    let cert_next = directory.join("server.crt.next");
    let key_next = directory.join("server.key.next");
    std::fs::copy(fixture("alternate.crt"), &cert_next).unwrap();
    std::fs::copy(fixture("alternate.key"), &key_next).unwrap();
    std::fs::rename(cert_next, &cert_file).unwrap();
    std::fs::rename(key_next, &key_file).unwrap();
    std::fs::write(&ocsp_file, response_two).unwrap();
    let trigger_next = directory.join("generation.next");
    std::fs::write(&trigger_next, b"generation-2").unwrap();
    std::fs::rename(trigger_next, &trigger).unwrap();

    let rotated_responses = Arc::new(Mutex::new(Vec::new()));
    let mut rotated = connect_capturing_ocsp(&server, "alt.test", Arc::clone(&rotated_responses));
    ping(&mut rotated);
    assert_eq!(
        rotated_responses.lock().unwrap().as_slice(),
        &[response_two]
    );
    ping(&mut established);

    std::fs::write(&cert_file, b"invalid replacement certificate").unwrap();
    std::fs::write(&ocsp_file, b"invalid replacement response").unwrap();
    std::fs::write(&trigger, b"generation-3").unwrap();
    let retained_responses = Arc::new(Mutex::new(Vec::new()));
    let mut retained = connect_capturing_ocsp(&server, "alt.test", Arc::clone(&retained_responses));
    ping(&mut retained);
    assert_eq!(
        retained_responses.lock().unwrap().as_slice(),
        &[response_two]
    );
    ping(&mut rotated);
    ping(&mut established);

    server.shutdown().unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn tls_reload_replaces_configured_sni_identity_for_new_handshakes() {
    let directory = std::env::temp_dir().join(format!(
        "rillcache-tls-sni-reload-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let trigger = directory.join("generation");
    let cert_file = directory.join("sni.crt");
    let key_file = directory.join("sni.key");
    std::fs::copy(fixture("alternate.crt"), &cert_file).unwrap();
    std::fs::copy(fixture("alternate.key"), &key_file).unwrap();
    std::fs::write(&trigger, b"generation-1").unwrap();

    let mut config = Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        tls_bind_addr: Some("127.0.0.1:0".parse().unwrap()),
        tls_cert_file: Some(fixture("server.crt")),
        tls_key_file: Some(fixture("server.key")),
        tls_reload_trigger: Some(trigger.clone()),
        tls_sni_certificates: vec![rillcache::TlsSniCertificate {
            server_name: "alt.test".into(),
            cert_file: cert_file.clone(),
            key_file: key_file.clone(),
            ocsp_file: None,
        }],
        ..Config::default()
    };
    config.idle_timeout = Duration::from_secs(30);
    let server = Server::bind(config).unwrap().spawn().unwrap();
    let mut initial = connect_named(&server, false, "alt.test");
    ping(&mut initial);

    let cert_next = directory.join("sni.crt.next");
    let key_next = directory.join("sni.key.next");
    std::fs::copy(fixture("server.crt"), &cert_next).unwrap();
    std::fs::copy(fixture("server.key"), &key_next).unwrap();
    std::fs::rename(cert_next, &cert_file).unwrap();
    std::fs::rename(key_next, &key_file).unwrap();
    let trigger_next = directory.join("generation.next");
    std::fs::write(&trigger_next, b"generation-2").unwrap();
    std::fs::rename(trigger_next, &trigger).unwrap();

    let mut rotated = connect_named(&server, false, "alt.test");
    let handshake = rotated.write_all(b"*1\r\n$4\r\nPING\r\n").and_then(|()| {
        let mut reply = [0u8; 7];
        rotated.read_exact(&mut reply)
    });
    assert!(handshake.is_err(), "new SNI certificate must be served");
    ping(&mut initial);

    server.shutdown().unwrap();
    std::fs::remove_dir_all(directory).unwrap();
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
fn established_tls_record_only_trickle_is_bounded_by_plaintext_idle_timeout() {
    let idle_timeout = Duration::from_millis(250);
    let server = start_with_timeouts(None, false, Duration::from_secs(2), idle_timeout, 1);
    let mut idle_control = connect(&server, false);
    ping(&mut idle_control);
    std::thread::sleep(Duration::from_millis(150));
    ping(&mut idle_control);
    std::thread::sleep(Duration::from_millis(300));
    let started = std::time::Instant::now();
    let mut response = [0u8; 1];
    let idle_result = idle_control.read(&mut response);
    assert!(matches!(idle_result, Err(_) | Ok(0)));
    assert!(started.elapsed() < Duration::from_millis(700));
    drop(idle_control);

    let mut tls = connect(&server, false);
    ping(&mut tls);

    // A valid TLS 1.3 KeyUpdate is a complete authenticated record but carries
    // no application plaintext. Drip its bytes more frequently than idle_timeout
    // so restarting the relative socket timeout would retain the connection.
    tls.conn.refresh_traffic_keys().unwrap();
    let mut record = Vec::new();
    tls.conn.write_tls(&mut record).unwrap();
    assert!(
        record.len() >= 20,
        "KeyUpdate record too short: {}",
        record.len()
    );
    let mut raw = tls.sock.try_clone().unwrap();
    let dripper = std::thread::spawn(move || {
        for byte in record {
            std::thread::sleep(Duration::from_millis(30));
            if raw.write_all(&[byte]).is_err() {
                break;
            }
        }
    });

    let started = std::time::Instant::now();
    let result = tls.read(&mut response);
    let elapsed = started.elapsed();
    assert!(
        matches!(result, Err(_) | Ok(0)),
        "unexpected plaintext: {response:?}"
    );
    assert!(
        elapsed < Duration::from_millis(700),
        "record-only TLS connection closed after {elapsed:?}"
    );
    dripper.join().unwrap();

    // The hostile client occupied the sole admission slot; closure must release
    // it so an independent TLS client can still use the server.
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
fn handshake_deadline_closes_silent_and_clienthello_trickle_peers() {
    let server = start_with_timeouts(
        None,
        false,
        Duration::from_millis(250),
        Duration::from_secs(2),
        1,
    );
    let silent = TcpStream::connect(server.tls_local_addr().unwrap()).unwrap();
    silent
        .set_read_timeout(Some(Duration::from_millis(800)))
        .unwrap();
    let started = std::time::Instant::now();
    let mut byte = [0u8; 1];
    assert!(matches!((&silent).read(&mut byte), Err(_) | Ok(0)));
    assert!(started.elapsed() < Duration::from_millis(800));
    drop(silent);

    let mut trickle = TcpStream::connect(server.tls_local_addr().unwrap()).unwrap();
    trickle
        .set_read_timeout(Some(Duration::from_millis(800)))
        .unwrap();
    trickle
        .set_write_timeout(Some(Duration::from_millis(800)))
        .unwrap();
    let mut hello = ClientConnection::new(
        client_config(false, false),
        ServerName::try_from("localhost").unwrap(),
    )
    .unwrap();
    let mut client_hello = Vec::new();
    hello.write_tls(&mut client_hello).unwrap();
    assert!(client_hello.len() > 30);
    let mut dripper = trickle.try_clone().unwrap();
    let sender = std::thread::spawn(move || {
        let mut sent = 0;
        for byte in client_hello {
            std::thread::sleep(Duration::from_millis(40));
            if dripper.write_all(&[byte]).is_err() {
                break;
            }
            sent += 1;
        }
        sent
    });
    let started = std::time::Instant::now();
    let read = trickle.read(&mut byte);
    let elapsed = started.elapsed();
    assert!(matches!(read, Err(_) | Ok(0)));
    assert!(
        elapsed < Duration::from_millis(800),
        "closed after {elapsed:?}"
    );
    let sent = sender.join().unwrap();
    assert!(sent > 1, "peer made no fragmented ClientHello progress");
    drop(trickle);

    let mut healthy = connect(&server, false);
    ping(&mut healthy);
    server.shutdown().unwrap();
}

#[test]
fn fragmented_clienthello_completes_and_handshake_budget_is_cleared() {
    let server = start_with_timeouts(
        None,
        false,
        Duration::from_millis(250),
        Duration::from_secs(2),
        128,
    );
    let mut tcp = TcpStream::connect(server.tls_local_addr().unwrap()).unwrap();
    tcp.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    tcp.set_write_timeout(Some(Duration::from_secs(3))).unwrap();
    let mut conn = ClientConnection::new(
        client_config(false, false),
        ServerName::try_from("localhost").unwrap(),
    )
    .unwrap();
    let mut client_hello = Vec::new();
    conn.write_tls(&mut client_hello).unwrap();
    for chunk in client_hello.chunks(64) {
        tcp.write_all(chunk).unwrap();
        std::thread::sleep(Duration::from_millis(2));
    }
    let mut stream = StreamOwned::new(conn, tcp);
    ping(&mut stream);

    // A successful handshake must not leave its short deadline active for
    // ordinary authenticated protocol traffic on the established connection.
    std::thread::sleep(Duration::from_millis(350));
    ping(&mut stream);
    server.shutdown().unwrap();
}

#[test]
fn client_crl_policy_allows_current_nonrevoked_and_rejects_revoked_or_expired() {
    for workers in [None, Some(1), Some(4)] {
        let mut good_config = Config {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            tls_bind_addr: Some("127.0.0.1:0".parse().unwrap()),
            tls_cert_file: Some(fixture("server.crt")),
            tls_key_file: Some(fixture("server.key")),
            tls_client_ca_file: Some(fixture("ca.crt")),
            tls_client_crl_file: Some(fixture("client-good.crl.der")),
            worker_threads: workers,
            ..Config::default()
        };
        good_config.idle_timeout = Duration::from_secs(5);
        let good = Server::bind(good_config).unwrap().spawn().unwrap();
        let mut client = connect(&good, true);
        ping(&mut client);
        good.shutdown().unwrap();

        for crl in [
            "client-revoked.crl.der",
            "client-expired.crl.der",
            "client-unrelated.crl.der",
        ] {
            let server = Server::bind(Config {
                bind_addr: "127.0.0.1:0".parse().unwrap(),
                tls_bind_addr: Some("127.0.0.1:0".parse().unwrap()),
                tls_cert_file: Some(fixture("server.crt")),
                tls_key_file: Some(fixture("server.key")),
                tls_client_ca_file: Some(fixture("ca.crt")),
                tls_client_crl_file: Some(fixture(crl)),
                worker_threads: workers,
                ..Config::default()
            })
            .unwrap()
            .spawn()
            .unwrap();
            let mut client = connect(&server, true);
            let result = client.write_all(b"*1\r\n$4\r\nPING\r\n");
            let mut reply = [0u8; 7];
            let result = result.and_then(|()| client.read_exact(&mut reply));
            assert!(result.is_err(), "accepted mTLS client under CRL {crl}");
            server.shutdown().unwrap();
        }
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
fn verified_client_certificate_maps_to_acl_user_without_password_and_survives_reset() {
    let directory = std::env::temp_dir().join(format!(
        "rillcache-tls-acl-map-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let acl = directory.join("users.acl");
    std::fs::write(
        &acl,
        b"user default on -@all\nuser cert-reader on nopass ~* &* +@all -@write\n",
    )
    .unwrap();
    let fingerprint = [
        0xc2, 0x51, 0x50, 0xac, 0x12, 0x67, 0xbf, 0x09, 0xe7, 0x54, 0xf1, 0xc1, 0xcc, 0xbd, 0x78,
        0x3c, 0x6a, 0x49, 0xaa, 0xaa, 0xb4, 0xc6, 0x6f, 0xe1, 0xf0, 0xb2, 0xe2, 0x17, 0x8b, 0xdd,
        0x8e, 0x0c,
    ];
    let server = Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        tls_bind_addr: Some("127.0.0.1:0".parse().unwrap()),
        tls_cert_file: Some(fixture("server.crt")),
        tls_key_file: Some(fixture("server.key")),
        tls_client_ca_file: Some(fixture("ca.crt")),
        tls_client_cert_users: std::collections::BTreeMap::from([(
            fingerprint,
            b"cert-reader".to_vec(),
        )]),
        aclfile: Some(acl),
        idle_timeout: Duration::from_secs(30),
        ..Config::default()
    })
    .unwrap()
    .spawn()
    .unwrap();

    let mut stream = connect(&server, true);
    ping(&mut stream);
    stream
        .write_all(b"*3\r\n$3\r\nSET\r\n$8\r\napp:key1\r\n$5\r\nvalue\r\n")
        .unwrap();
    let denied = b"-NOPERM this user has no permissions to run the 'set' command\r\n";
    let mut denial = vec![0; denied.len()];
    stream.read_exact(&mut denial).unwrap();
    assert_eq!(denial, denied);
    stream
        .write_all(b"*2\r\n$3\r\nACL\r\n$6\r\nWHOAMI\r\n")
        .unwrap();
    let mut identity = [0u8; 18];
    stream.read_exact(&mut identity).unwrap();
    assert_eq!(&identity, b"$11\r\ncert-reader\r\n");

    stream.write_all(b"*1\r\n$5\r\nRESET\r\n").unwrap();
    let mut reset_reply = [0u8; 8];
    stream.read_exact(&mut reset_reply).unwrap();
    assert_eq!(&reset_reply, b"+RESET\r\n");
    stream
        .write_all(b"*2\r\n$3\r\nACL\r\n$6\r\nWHOAMI\r\n")
        .unwrap();
    stream.read_exact(&mut identity).unwrap();
    assert_eq!(&identity, b"$11\r\ncert-reader\r\n");

    server.shutdown().unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn mapped_client_certificate_obeys_per_user_connection_caps_and_disconnect_releases_slot() {
    let fingerprint = [
        0xc2, 0x51, 0x50, 0xac, 0x12, 0x67, 0xbf, 0x09, 0xe7, 0x54, 0xf1, 0xc1, 0xcc, 0xbd, 0x78,
        0x3c, 0x6a, 0x49, 0xaa, 0xaa, 0xb4, 0xc6, 0x6f, 0xe1, 0xf0, 0xb2, 0xe2, 0x17, 0x8b, 0xdd,
        0x8e, 0x0c,
    ];
    for workers in [None, Some(1), Some(4)] {
        let acl = std::env::temp_dir().join(format!(
            "rillcache-tls-user-cap-{}-{workers:?}.acl",
            std::process::id()
        ));
        std::fs::write(
            &acl,
            b"user default on -@all\nuser cert-reader on nopass ~* &* +@all maxconnections=1\n",
        )
        .unwrap();
        let server = Server::bind(Config {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            tls_bind_addr: Some("127.0.0.1:0".parse().unwrap()),
            tls_cert_file: Some(fixture("server.crt")),
            tls_key_file: Some(fixture("server.key")),
            tls_client_ca_file: Some(fixture("ca.crt")),
            tls_client_cert_users: std::collections::BTreeMap::from([(
                fingerprint,
                b"cert-reader".to_vec(),
            )]),
            aclfile: Some(acl.clone()),
            worker_threads: workers,
            ..Config::default()
        })
        .unwrap()
        .spawn()
        .unwrap();

        let mut first = connect(&server, true);
        ping(&mut first);
        let mut rejected = connect(&server, true);
        let rejected_result = rejected.write_all(b"*1\r\n$4\r\nPING\r\n").and_then(|()| {
            let mut reply = [0; 7];
            rejected.read_exact(&mut reply)
        });
        assert!(rejected_result.is_err(), "second mTLS session exceeded cap");
        drop(rejected);
        drop(first);

        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            let mut replacement = connect(&server, true);
            let result = replacement
                .write_all(b"*1\r\n$4\r\nPING\r\n")
                .and_then(|()| {
                    let mut reply = [0; 7];
                    replacement.read_exact(&mut reply)?;
                    if reply == *b"+PONG\r\n" {
                        Ok(())
                    } else {
                        Err(std::io::Error::other("unexpected TLS PING reply"))
                    }
                });
            if result.is_ok() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "disconnect did not release mTLS slot: {result:?}"
            );
            drop(replacement);
            std::thread::yield_now();
        }
        server.shutdown().unwrap();
        std::fs::remove_file(acl).unwrap();
    }
}

#[test]
fn tls_reset_cannot_overbook_a_full_certificate_user_cap() {
    let fingerprint = [
        0xc2, 0x51, 0x50, 0xac, 0x12, 0x67, 0xbf, 0x09, 0xe7, 0x54, 0xf1, 0xc1, 0xcc, 0xbd, 0x78,
        0x3c, 0x6a, 0x49, 0xaa, 0xaa, 0xb4, 0xc6, 0x6f, 0xe1, 0xf0, 0xb2, 0xe2, 0x17, 0x8b, 0xdd,
        0x8e, 0x0c,
    ];
    for workers in [None, Some(1), Some(4)] {
        let acl = std::env::temp_dir().join(format!(
            "rillcache-tls-reset-cap-{}-{workers:?}.acl",
            std::process::id()
        ));
        std::fs::write(
            &acl,
            b"user default on nopass ~* &* +@all\nuser cert-reader on nopass ~* &* +@all maxconnections=1\nuser bob on nopass ~* &* +@all\n",
        )
        .unwrap();
        let server = Server::bind(Config {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            tls_bind_addr: Some("127.0.0.1:0".parse().unwrap()),
            tls_cert_file: Some(fixture("server.crt")),
            tls_key_file: Some(fixture("server.key")),
            tls_client_ca_file: Some(fixture("ca.crt")),
            tls_client_cert_users: std::collections::BTreeMap::from([(
                fingerprint,
                b"cert-reader".to_vec(),
            )]),
            aclfile: Some(acl.clone()),
            worker_threads: workers,
            ..Config::default()
        })
        .unwrap()
        .spawn()
        .unwrap();

        let mut mapped = connect(&server, true);
        ping(&mut mapped);
        mapped
            .write_all(b"*3\r\n$4\r\nAUTH\r\n$3\r\nbob\r\n$0\r\n\r\n")
            .unwrap();
        let mut ok = [0; 5];
        mapped.read_exact(&mut ok).unwrap();
        assert_eq!(&ok, b"+OK\r\n");

        let mut plain = TcpStream::connect(server.local_addr()).unwrap();
        plain
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        plain
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        plain
            .write_all(b"*3\r\n$4\r\nAUTH\r\n$11\r\ncert-reader\r\n$0\r\n\r\n")
            .unwrap();
        plain.read_exact(&mut ok).unwrap();
        assert_eq!(&ok, b"+OK\r\n");

        mapped.write_all(b"*1\r\n$5\r\nRESET\r\n").unwrap();
        let reply = read_resp_line(&mut mapped);
        assert_eq!(reply, b"-ERR max connections reached for user\r\n");
        mapped
            .write_all(b"*2\r\n$3\r\nACL\r\n$6\r\nWHOAMI\r\n")
            .unwrap();
        let mut identity = [0; 9];
        mapped.read_exact(&mut identity).unwrap();
        assert_eq!(&identity, b"$3\r\nbob\r\n");

        drop(plain);
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            mapped.write_all(b"*1\r\n$5\r\nRESET\r\n").unwrap();
            let reset = read_resp_line(&mut mapped);
            if reset == b"+RESET\r\n" {
                break;
            }
            assert_eq!(reset, b"-ERR max connections reached for user\r\n");
            assert!(
                std::time::Instant::now() < deadline,
                "disconnected user kept its slot"
            );
            std::thread::yield_now();
        }
        mapped
            .write_all(b"*2\r\n$3\r\nACL\r\n$6\r\nWHOAMI\r\n")
            .unwrap();
        let mut identity = [0; 18];
        mapped.read_exact(&mut identity).unwrap();
        assert_eq!(&identity, b"$11\r\ncert-reader\r\n");

        server.shutdown().unwrap();
        std::fs::remove_file(acl).unwrap();
    }
}

#[test]
fn valid_but_unmapped_client_certificate_is_rejected_and_mapping_requires_mtls_and_enabled_user() {
    let acl = std::env::temp_dir().join(format!(
        "rillcache-tls-acl-map-invalid-{}-{:?}.acl",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::write(
        &acl,
        b"user default on -@all\nuser cert-reader on nopass ~* &* +@all\n",
    )
    .unwrap();
    let mapping = std::collections::BTreeMap::from([([0u8; 32], b"cert-reader".to_vec())]);
    let base = Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        tls_bind_addr: Some("127.0.0.1:0".parse().unwrap()),
        tls_cert_file: Some(fixture("server.crt")),
        tls_key_file: Some(fixture("server.key")),
        tls_client_ca_file: Some(fixture("ca.crt")),
        tls_client_cert_users: mapping.clone(),
        aclfile: Some(acl.clone()),
        ..Config::default()
    };
    let server = Server::bind(base.clone()).unwrap().spawn().unwrap();
    let mut stream = connect(&server, true);
    let request = stream.write_all(b"*1\r\n$4\r\nPING\r\n");
    let mut reply = [0u8; 7];
    assert!(request.is_err() || stream.read_exact(&mut reply).is_err());
    server.shutdown().unwrap();

    let mut no_mtls = base.clone();
    no_mtls.tls_client_ca_file = None;
    assert!(Server::bind(no_mtls).is_err());
    let mut missing_user = base;
    missing_user.tls_client_cert_users =
        std::collections::BTreeMap::from([([0u8; 32], b"missing-user".to_vec())]);
    assert!(Server::bind(missing_user).is_err());
    std::fs::remove_file(acl).unwrap();
}

#[test]
fn client_certificate_mapping_reload_publishes_atomic_generation_and_preserves_sessions() {
    let directory = std::env::temp_dir().join(format!(
        "rillcache-tls-acl-map-reload-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let acl = directory.join("users.acl");
    let mapping = directory.join("client-users.map");
    let ocsp = directory.join("server.ocsp.der");
    let trigger = directory.join("generation");
    let response_one = [0x30, 0x03, 0x0a, 0x01, 0x00];
    let response_two = [0x30, 0x03, 0x0a, 0x01, 0x01];
    let response_three = [0x30, 0x03, 0x0a, 0x01, 0x02];
    let fingerprint = "c25150ac1267bf09e754f1c1ccbd783c6a49aaaab4c66fe1f0b2e2178bdd8e0c";
    std::fs::write(
        &acl,
        b"user default on nopass ~* &* +@all\nuser reader on nopass ~* &* +@all\nuser writer on nopass ~* &* +@all\n",
    )
    .unwrap();
    std::fs::write(&mapping, format!("{fingerprint}=reader\n")).unwrap();
    std::fs::write(&ocsp, response_one).unwrap();
    std::fs::write(&trigger, b"generation-1").unwrap();
    let config = Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        tls_bind_addr: Some("127.0.0.1:0".parse().unwrap()),
        tls_cert_file: Some(fixture("server.crt")),
        tls_key_file: Some(fixture("server.key")),
        tls_client_ca_file: Some(fixture("ca.crt")),
        tls_ocsp_file: Some(ocsp.clone()),
        tls_client_cert_users_file: Some(mapping.clone()),
        tls_reload_trigger: Some(trigger.clone()),
        aclfile: Some(acl),
        ..Config::default()
    };
    let server = Server::bind(config.clone()).unwrap().spawn().unwrap();
    let reader_responses = Arc::new(Mutex::new(Vec::new()));
    let mut established_reader = connect_capturing_ocsp_with_client_cert(
        &server,
        "localhost",
        Arc::clone(&reader_responses),
        true,
    );
    assert_tls_acl_user(&mut established_reader, b"reader");
    assert_eq!(reader_responses.lock().unwrap().as_slice(), &[response_one]);

    let replace = |path: &std::path::Path, bytes: &[u8], suffix: &str| {
        let staged = path.with_extension(format!("stage-{suffix}"));
        std::fs::write(&staged, bytes).unwrap();
        std::fs::rename(staged, path).unwrap();
    };
    replace(
        &mapping,
        format!("{fingerprint}=writer\n").as_bytes(),
        "valid",
    );
    replace(&ocsp, &response_two, "valid");
    replace(&trigger, b"generation-2", "valid");
    let writer_responses = Arc::new(Mutex::new(Vec::new()));
    let mut established_writer = connect_capturing_ocsp_with_client_cert(
        &server,
        "localhost",
        Arc::clone(&writer_responses),
        true,
    );
    assert_tls_acl_user(&mut established_writer, b"writer");
    assert_eq!(writer_responses.lock().unwrap().as_slice(), &[response_two]);
    assert_tls_acl_user(&mut established_reader, b"reader");
    established_reader
        .write_all(b"*1\r\n$5\r\nRESET\r\n")
        .unwrap();
    assert_eq!(read_resp_line(&mut established_reader), b"+RESET\r\n");
    assert_tls_acl_user(&mut established_reader, b"reader");

    replace(
        &mapping,
        format!("{fingerprint}=missing-user\n").as_bytes(),
        "invalid",
    );
    replace(&ocsp, &response_three, "invalid");
    replace(&trigger, b"generation-3", "invalid");
    let rejected_responses = Arc::new(Mutex::new(Vec::new()));
    let mut after_rejected_generation = connect_capturing_ocsp_with_client_cert(
        &server,
        "localhost",
        Arc::clone(&rejected_responses),
        true,
    );
    assert_tls_acl_user(&mut after_rejected_generation, b"writer");
    assert_eq!(
        rejected_responses.lock().unwrap().as_slice(),
        &[response_two]
    );
    assert_tls_acl_user(&mut established_writer, b"writer");

    replace(
        &mapping,
        format!("{fingerprint}=reader\n").as_bytes(),
        "recovered",
    );
    replace(&ocsp, &response_three, "recovered");
    replace(&trigger, b"generation-4", "recovered");
    let recovered_responses = Arc::new(Mutex::new(Vec::new()));
    let mut recovered = connect_capturing_ocsp_with_client_cert(
        &server,
        "localhost",
        Arc::clone(&recovered_responses),
        true,
    );
    assert_tls_acl_user(&mut recovered, b"reader");
    assert_eq!(
        recovered_responses.lock().unwrap().as_slice(),
        &[response_three]
    );

    drop(recovered);
    drop(after_rejected_generation);
    drop(established_writer);
    drop(established_reader);
    server.shutdown().unwrap();

    replace(
        &mapping,
        format!("{fingerprint}=missing-user\n").as_bytes(),
        "startup-invalid",
    );
    assert!(
        Server::bind(config).is_err(),
        "unknown mapped ACL user must fail startup"
    );
    std::fs::remove_dir_all(directory).unwrap();
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
    let trigger_without_bind = Config {
        tls_reload_trigger: Some(fixture("generation")),
        ..Config::default()
    };
    assert!(Server::bind(trigger_without_bind).is_err());
    let crl_without_ca = Config {
        tls_bind_addr: Some("127.0.0.1:0".parse().unwrap()),
        tls_cert_file: Some(fixture("server.crt")),
        tls_key_file: Some(fixture("server.key")),
        tls_client_crl_file: Some(fixture("client-good.crl.der")),
        ..Config::default()
    };
    assert!(Server::bind(crl_without_ca).is_err());
    let mut malformed_crl = base.clone();
    malformed_crl.tls_client_ca_file = Some(fixture("ca.crt"));
    malformed_crl.tls_client_crl_file = Some(fixture("client.crt"));
    assert!(Server::bind(malformed_crl).is_err());
    let mut missing_trigger = base.clone();
    missing_trigger.tls_reload_trigger = Some(fixture("missing-generation"));
    assert!(Server::bind(missing_trigger).is_err());
    let mut missing_cert = base.clone();
    missing_cert.tls_cert_file = Some(fixture("missing.crt"));
    assert!(Server::bind(missing_cert).is_err());
    let ocsp_file =
        std::env::temp_dir().join(format!("rillcache-invalid-ocsp-{}.der", std::process::id()));
    let mut invalid_ocsp = base.clone();
    invalid_ocsp.tls_ocsp_file = Some(ocsp_file.clone());
    assert!(
        Server::bind(invalid_ocsp.clone()).is_err(),
        "missing OCSP file must fail closed"
    );
    std::fs::write(&ocsp_file, b"not DER").unwrap();
    assert!(Server::bind(invalid_ocsp.clone()).is_err());
    std::fs::write(&ocsp_file, [0x30, 0x01, 0x00]).unwrap();
    assert!(Server::bind(invalid_ocsp.clone()).is_err());
    std::fs::write(&ocsp_file, []).unwrap();
    assert!(Server::bind(invalid_ocsp.clone()).is_err());
    std::fs::write(&ocsp_file, vec![0x30; 1024 * 1024 + 1]).unwrap();
    assert!(Server::bind(invalid_ocsp).is_err());
    std::fs::remove_file(ocsp_file).unwrap();
    let mut malformed_name = base.clone();
    malformed_name
        .tls_sni_certificates
        .push(rillcache::TlsSniCertificate {
            server_name: "*.example.com".into(),
            cert_file: fixture("alternate.crt"),
            key_file: fixture("alternate.key"),
            ocsp_file: None,
        });
    assert!(Server::bind(malformed_name).is_err());
    let mut duplicate_name = base.clone();
    duplicate_name.tls_sni_certificates = vec![
        rillcache::TlsSniCertificate {
            server_name: "alt.test".into(),
            cert_file: fixture("alternate.crt"),
            key_file: fixture("alternate.key"),
            ocsp_file: None,
        },
        rillcache::TlsSniCertificate {
            server_name: "ALT.TEST".into(),
            cert_file: fixture("alternate.crt"),
            key_file: fixture("alternate.key"),
            ocsp_file: None,
        },
    ];
    assert!(Server::bind(duplicate_name).is_err());
}
