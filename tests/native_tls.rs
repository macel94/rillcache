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
            },
            rillcache::TlsSniCertificate {
                server_name: "localhost".into(),
                cert_file: fixture("server.crt"),
                key_file: fixture("server.key"),
            },
        ],
        ..Config::default()
    };
    config.idle_timeout = Duration::from_secs(30);
    Server::bind(config).unwrap().spawn().unwrap()
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
    std::fs::copy(fixture("server.crt"), &cert_file).unwrap();
    std::fs::copy(fixture("server.key"), &key_file).unwrap();
    std::fs::write(&trigger, b"generation-1").unwrap();

    let server = Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        tls_bind_addr: Some("127.0.0.1:0".parse().unwrap()),
        tls_cert_file: Some(cert_file.clone()),
        tls_key_file: Some(key_file.clone()),
        tls_reload_trigger: Some(trigger.clone()),
        idle_timeout: Duration::from_secs(30),
        ..Config::default()
    })
    .unwrap()
    .spawn()
    .unwrap();
    let mut established = connect(&server, false);
    ping(&mut established);

    let cert_next = directory.join("server.crt.next");
    let key_next = directory.join("server.key.next");
    std::fs::copy(fixture("alternate.crt"), &cert_next).unwrap();
    std::fs::copy(fixture("alternate.key"), &key_next).unwrap();
    std::fs::rename(cert_next, &cert_file).unwrap();
    std::fs::rename(key_next, &key_file).unwrap();
    let trigger_next = directory.join("generation.next");
    std::fs::write(&trigger_next, b"generation-2").unwrap();
    std::fs::rename(trigger_next, &trigger).unwrap();

    let mut rotated = connect_named(&server, false, "alt.test");
    ping(&mut rotated);
    ping(&mut established);

    std::fs::write(&cert_file, b"invalid replacement certificate").unwrap();
    std::fs::write(&trigger, b"generation-3").unwrap();
    let mut retained = connect_named(&server, false, "alt.test");
    ping(&mut retained);
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
    let trigger_without_bind = Config {
        tls_reload_trigger: Some(fixture("generation")),
        ..Config::default()
    };
    assert!(Server::bind(trigger_without_bind).is_err());
    let mut missing_trigger = base.clone();
    missing_trigger.tls_reload_trigger = Some(fixture("missing-generation"));
    assert!(Server::bind(missing_trigger).is_err());
    let mut missing_cert = base.clone();
    missing_cert.tls_cert_file = Some(fixture("missing.crt"));
    assert!(Server::bind(missing_cert).is_err());
    let mut malformed_name = base.clone();
    malformed_name
        .tls_sni_certificates
        .push(rillcache::TlsSniCertificate {
            server_name: "*.example.com".into(),
            cert_file: fixture("alternate.crt"),
            key_file: fixture("alternate.key"),
        });
    assert!(Server::bind(malformed_name).is_err());
    let mut duplicate_name = base.clone();
    duplicate_name.tls_sni_certificates = vec![
        rillcache::TlsSniCertificate {
            server_name: "alt.test".into(),
            cert_file: fixture("alternate.crt"),
            key_file: fixture("alternate.key"),
        },
        rillcache::TlsSniCertificate {
            server_name: "ALT.TEST".into(),
            cert_file: fixture("alternate.crt"),
            key_file: fixture("alternate.key"),
        },
    ];
    assert!(Server::bind(duplicate_name).is_err());
}
