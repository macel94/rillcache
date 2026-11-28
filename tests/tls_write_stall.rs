#![forbid(unsafe_code)]

//! TLS slow-reader shedding, isolated from the timing-sensitive handshake
//! tests so the large reply transfers cannot starve their deadlines.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rillcache::protocol::{Limits, encode_request};
use rillcache::{Config, Server, ServerHandle};
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

const VALUE: usize = 16 * 1024 * 1024;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/tls")
        .join(name)
}

fn start(workers: Option<usize>, stall: Duration) -> ServerHandle {
    Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        tls_bind_addr: Some("127.0.0.1:0".parse().unwrap()),
        tls_cert_file: Some(fixture("server.crt")),
        tls_key_file: Some(fixture("server.key")),
        idle_timeout: Duration::from_secs(30),
        write_stall_timeout: Some(stall),
        limits: Limits {
            max_frame_bytes: 40 * 1024 * 1024,
            max_bulk_bytes: 24 * 1024 * 1024,
            ..Limits::default()
        },
        max_clients: 4,
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

fn client_config() -> Arc<ClientConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_root_certificates(roots())
        .with_no_client_auth();
    Arc::new(config)
}

fn connect(server: &ServerHandle) -> StreamOwned<ClientConnection, TcpStream> {
    let tcp = TcpStream::connect(server.tls_local_addr().unwrap()).unwrap();
    tcp.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    tcp.set_write_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let connection =
        ClientConnection::new(client_config(), ServerName::try_from("localhost").unwrap()).unwrap();
    StreamOwned::new(connection, tcp)
}

fn send(stream: &mut StreamOwned<ClientConnection, TcpStream>, arguments: &[&[u8]]) {
    let mut request = Vec::new();
    encode_request(arguments, &mut request);
    stream.write_all(&request).unwrap();
}

fn read_line(stream: &mut StreamOwned<ClientConnection, TcpStream>) -> Vec<u8> {
    let mut line = Vec::new();
    loop {
        let mut byte = [0u8; 1];
        stream.read_exact(&mut byte).unwrap();
        line.push(byte[0]);
        if line.ends_with(b"\r\n") {
            return line;
        }
    }
}

fn info_clients(stream: &mut StreamOwned<ClientConnection, TcpStream>) -> usize {
    send(stream, &[b"INFO", b"clients"]);
    let header = read_line(stream);
    let size: usize = std::str::from_utf8(&header[1..header.len() - 2])
        .unwrap()
        .parse()
        .unwrap();
    let mut body = vec![0; size + 2];
    stream.read_exact(&mut body).unwrap();
    let text = std::str::from_utf8(&body).unwrap();
    text.lines()
        .find_map(|line| line.strip_prefix("connected_clients:"))
        .unwrap()
        .parse()
        .unwrap()
}

fn ping(stream: &mut StreamOwned<ClientConnection, TcpStream>) {
    send(stream, &[b"PING"]);
    let mut reply = [0u8; 7];
    stream.read_exact(&mut reply).unwrap();
    assert_eq!(&reply, b"+PONG\r\n");
}

#[test]
fn tls_sheds_a_non_reading_client_after_the_write_stall_deadline() {
    for workers in [None, Some(4)] {
        let server = start(workers, Duration::from_millis(300));
        let mut admin = connect(&server);
        let value = vec![b'x'; VALUE];
        send(&mut admin, &[b"SET", b"tls-stall", &value]);
        let mut reply = [0u8; 5];
        admin.read_exact(&mut reply).unwrap();
        assert_eq!(&reply, b"+OK\r\n");

        let mut reader = connect(&server);
        send(&mut reader, &[b"GET", b"tls-stall"]);
        // Never drain. Poll the server's own registry so scheduling delays in
        // this process cannot hide or fake the deadline; the read-idle timeout
        // is 30 s, so only write-stall shedding can remove this client.
        let registered = Instant::now() + Duration::from_secs(5);
        while info_clients(&mut admin) != 2 {
            assert!(
                Instant::now() < registered,
                "the TLS reader was never registered in the client registry"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if info_clients(&mut admin) == 1 {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the non-reading TLS client was not shed"
            );
            std::thread::sleep(Duration::from_millis(50));
        }

        let mut received = 0usize;
        let mut buffer = [0u8; 64 * 1024];
        let shed = loop {
            match reader.read(&mut buffer) {
                Ok(0) => break true,
                Ok(read) => received += read,
                Err(_) => break true,
            }
        };
        assert!(
            shed && received < VALUE,
            "TLS must drop the unsent backlog of a non-reading client ({received} of {VALUE})"
        );
        ping(&mut admin);
        let mut fresh = connect(&server);
        ping(&mut fresh);
        server.shutdown().unwrap();
    }
}

#[test]
fn tls_keeps_a_slow_but_progressing_reader() {
    for workers in [None, Some(4)] {
        // One second, so a descheduled test process cannot turn a progressing
        // reader into a stalled one; the default deadline is 30 s.
        let server = start(workers, Duration::from_secs(1));
        let mut admin = connect(&server);
        let value = vec![b'x'; VALUE];
        send(&mut admin, &[b"SET", b"tls-slow", &value]);
        let mut reply = [0u8; 5];
        admin.read_exact(&mut reply).unwrap();
        assert_eq!(&reply, b"+OK\r\n");

        let mut reader = connect(&server);
        send(&mut reader, &[b"GET", b"tls-slow"]);
        let header = format!("${VALUE}\r\n");
        let expected = header.len() + VALUE + 2;
        let mut body = Vec::new();
        let mut buffer = [0u8; 64 * 1024];
        let mut reads = 0usize;
        while body.len() < expected {
            let read = reader.read(&mut buffer).unwrap();
            assert!(read > 0, "a progressing TLS reader must not be shed");
            body.extend_from_slice(&buffer[..read]);
            reads += 1;
            if reads.is_multiple_of(16) {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        assert!(body.starts_with(header.as_bytes()));
        ping(&mut reader);
        server.shutdown().unwrap();
    }
}
