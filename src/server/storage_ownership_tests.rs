//! Deterministic mixed-transport owner-lifetime regression, with real TLS I/O.
use super::*;
use crate::engine::Clock;
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use std::sync::mpsc;

struct PausedClock {
    armed: AtomicBool,
    entered: mpsc::SyncSender<()>,
    resume: std::sync::Mutex<mpsc::Receiver<()>>,
}
impl Clock for PausedClock {
    fn now_ms(&self) -> i64 {
        if self.armed.swap(false, Ordering::AcqRel) {
            self.entered.send(()).unwrap();
            self.resume.lock().unwrap().recv().unwrap();
        }
        1000
    }
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/tls")
        .join(name)
}

#[test]
fn plain_exit_cannot_destroy_owner_under_an_admitted_tls_request() {
    for workers in [None, Some(1)] {
        let (entered, observed) = mpsc::sync_channel(1);
        let (resume, resumed) = mpsc::sync_channel(1);
        let clock = Arc::new(PausedClock {
            armed: AtomicBool::new(false),
            entered,
            resume: std::sync::Mutex::new(resumed),
        });
        let config = Config {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            tls_bind_addr: Some("127.0.0.1:0".parse().unwrap()),
            tls_cert_file: Some(fixture("server.crt")),
            tls_key_file: Some(fixture("server.key")),
            worker_threads: workers,
            io_mode: IoMode::Tokio,
            ..Config::default()
        };
        let mut server = Server::bind(config).unwrap();
        server.store = Store::with_clock(1024, clock.clone());
        let services = server.store.test_services();
        let handle = server.spawn().unwrap();
        if workers.is_none() {
            assert!(handle.scheduling_policy().readiness_io);
            assert!(handle.scheduling_policy().tokio_compatibility_owner);
        }
        let mut plain = TcpStream::connect(handle.local_addr()).unwrap();
        plain
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        plain.write_all(b"PING\r\n").unwrap();
        let mut pong = [0; 7];
        plain.read_exact(&mut pong).unwrap();
        assert_eq!(&pong, b"+PONG\r\n");

        let mut roots = RootCertStore::empty();
        let file = std::fs::File::open(fixture("ca.crt")).unwrap();
        for cert in rustls_pemfile::certs(&mut io::BufReader::new(file)) {
            roots.add(cert.unwrap()).unwrap();
        }
        let client_config =
            ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_protocol_versions(&[&rustls::version::TLS13])
                .unwrap()
                .with_root_certificates(roots)
                .with_no_client_auth();
        let socket = TcpStream::connect(handle.tls_local_addr().unwrap()).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        socket
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let connection = ClientConnection::new(
            Arc::new(client_config),
            ServerName::try_from("localhost").unwrap(),
        )
        .unwrap();
        let mut tls = StreamOwned::new(connection, socket);
        tls.write_all(b"PING\r\n").unwrap();
        tls.flush().unwrap();
        tls.read_exact(&mut pong).unwrap();
        assert_eq!(&pong, b"+PONG\r\n");
        clock.armed.store(true, Ordering::Release);
        let tls_client = thread::spawn(move || {
            tls.write_all(b"SET tls admitted\r\n").unwrap();
            tls.flush().unwrap();
            let mut reply = [0; 5];
            // Stop may close the peer after admission but before reply output.
            if tls.read_exact(&mut reply).is_ok() {
                assert_eq!(&reply, b"+OK\r\n");
            }
        });
        observed.recv_timeout(Duration::from_secs(3)).unwrap();
        let (finished, result) = mpsc::sync_channel(1);
        let (stopped, stop_observed) = mpsc::sync_channel(1);
        let shutdown = thread::spawn(move || {
            handle.stopping.store(true, Ordering::Release);
            stopped.send(()).unwrap();
            finished.send(handle.shutdown()).unwrap();
        });
        // Stop is published while the TLS request is deterministically admitted
        // and paused. Joining must wait for it, whether plain has exited already
        // or is itself waiting for a compatibility-owner maintenance request.
        stop_observed.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(matches!(result.try_recv(), Err(mpsc::TryRecvError::Empty)));
        resume.send(()).unwrap();
        result
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .unwrap();
        shutdown.join().unwrap();
        tls_client.join().unwrap();
        assert_eq!(plain.read(&mut [0; 1]).unwrap(), 0);
        assert!(
            services.bridge_count() > 0,
            "mixed TLS must use external execution"
        );
        let trace = services.owner_trace();
        assert_eq!(trace.len(), 2);
        assert_eq!(trace[0].1, trace[1].1);
    }
}

#[cfg(unix)]
#[test]
fn local_owner_executes_complete_transactions_without_bridge_crossings() {
    // The published default engine and the opt-in async engine must both run
    // complete transactions without a data-bridge crossing.
    for io_mode in [IoMode::Mio, IoMode::Tokio] {
        let server = Server::bind(Config {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            worker_threads: None,
            io_mode,
            ..Config::default()
        })
        .unwrap();
        let services = server.store.test_services();
        let handle = server.spawn().unwrap();
        assert!(handle.scheduling_policy().readiness_io);
        if io_mode == IoMode::Tokio {
            assert!(handle.scheduling_policy().tokio_transport);
            assert_eq!(
                handle.scheduling_policy().tokio_worker_threads,
                thread::available_parallelism()
                    .map(usize::from)
                    .unwrap_or(1)
                    .clamp(1, 64)
            );
            assert!(!handle.scheduling_policy().tokio_compatibility_owner);
        } else {
            assert!(!handle.scheduling_policy().tokio_transport);
        }
        let mut peer = TcpStream::connect(handle.local_addr()).unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        peer.write_all(b"SET a old\r\nMULTI\r\nMSET a new b new\r\nINCR a\r\nEXEC\r\nMGET a b\r\n")
            .unwrap();
        let expected=b"+OK\r\n+OK\r\n+QUEUED\r\n+QUEUED\r\n*2\r\n+OK\r\n-ERR value is not an integer or out of range\r\n*2\r\n$3\r\nnew\r\n$3\r\nnew\r\n";
        let mut reply = vec![0; expected.len()];
        peer.read_exact(&mut reply).unwrap();
        assert_eq!(reply, expected);
        handle.shutdown().unwrap();
        assert_eq!(services.bridge_count(), 0);
        let trace = services.owner_trace();
        assert_eq!(trace.len(), 2);
        assert_eq!(trace[0].1, trace[1].1);
        assert_ne!(trace[0].1, std::thread::current().id());
    }
}

#[cfg(unix)]
fn multi_owner_handle(owners: usize) -> ServerHandle {
    let server = Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        storage_owners: Some(owners),
        ..Config::default()
    })
    .unwrap();
    server.spawn().unwrap()
}

#[cfg(unix)]
fn pipeline(peer: &mut TcpStream, request: &[u8], expected: &[u8]) {
    peer.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    peer.write_all(request).unwrap();
    let mut reply = vec![0; expected.len()];
    peer.read_exact(&mut reply).unwrap();
    assert_eq!(reply, expected);
}

#[cfg(unix)]
#[test]
fn deferred_owner_batches_preserve_pipeline_order_across_owners() {
    for owners in [2, 4] {
        let handle = multi_owner_handle(owners);
        let mut peer = TcpStream::connect(handle.local_addr()).unwrap();
        // The keys deliberately interleave owners for the process seed. Even
        // if they do not, the ordering assertions exercise the same queue.
        pipeline(
            &mut peer,
            b"SET alpha 1\r\nSET beta 2\r\nGET alpha\r\nSET alpha 3\r\nGET beta\r\nGET alpha\r\n",
            b"+OK\r\n+OK\r\n$1\r\n1\r\n+OK\r\n$1\r\n2\r\n$1\r\n3\r\n",
        );
        // A local command is a barrier: earlier deferred replies must be emitted
        // before its reply, and later commands are parsed only afterwards.
        pipeline(
            &mut peer,
            b"SET order first\r\nPING\r\nGET order\r\n",
            b"+OK\r\n+PONG\r\n$5\r\nfirst\r\n",
        );
        // More commands than one deferred run: the buffered remainder is parsed
        // again after the run completes, without waiting for a second edge.
        let mut request = Vec::new();
        let mut expected = Vec::new();
        for index in 0..40 {
            request.extend_from_slice(format!("SET long:{index} {index}\r\n").as_bytes());
            expected.extend_from_slice(b"+OK\r\n");
        }
        pipeline(&mut peer, &request, &expected);
        // RESP arrays take the batching path, not the one-command inline path.
        let mut request = Vec::new();
        for index in 0..8 {
            let key = format!("resp:{index}");
            let value = format!("value:{index}");
            crate::protocol::encode_request(
                &[b"SET", key.as_bytes(), value.as_bytes()],
                &mut request,
            );
            crate::protocol::encode_request(&[b"GET", key.as_bytes()], &mut request);
        }
        let mut expected = Vec::new();
        for index in 0..8 {
            let value = format!("$7\r\nvalue:{index}\r\n");
            expected.extend_from_slice(b"+OK\r\n");
            expected.extend_from_slice(value.as_bytes());
        }
        pipeline(&mut peer, &request, &expected);
        handle.shutdown().unwrap();
    }
}

#[cfg(unix)]
#[test]
fn deferred_batches_keep_cross_owner_atomic_commands_ordered() {
    let handle = multi_owner_handle(4);
    let mut peer = TcpStream::connect(handle.local_addr()).unwrap();
    pipeline(
        &mut peer,
        b"SET a 1\r\nSET b 2\r\nMGET a b\r\nSET c 3\r\nMGET a b c\r\n",
        b"+OK\r\n+OK\r\n*2\r\n$1\r\n1\r\n$1\r\n2\r\n+OK\r\n*3\r\n$1\r\n1\r\n$1\r\n2\r\n$1\r\n3\r\n",
    );
    handle.shutdown().unwrap();
}

#[cfg(unix)]
fn read_line(peer: &mut TcpStream) -> Vec<u8> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        peer.read_exact(&mut byte).unwrap();
        line.push(byte[0]);
        if line.ends_with(b"\r\n") {
            return line;
        }
    }
}

#[cfg(unix)]
#[test]
fn deferred_completion_keeps_slowlog_and_error_accounting() {
    let handle = multi_owner_handle(2);
    let mut peer = TcpStream::connect(handle.local_addr()).unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    pipeline(
        &mut peer,
        b"CONFIG SET slowlog-log-slower-than 0\r\nCONFIG SET slowlog-max-len 128\r\n",
        b"+OK\r\n+OK\r\n",
    );
    pipeline(
        &mut peer,
        b"SET slow value\r\nGET slow\r\nLPUSH list x\r\nGET list\r\n",
        b"+OK\r\n$5\r\nvalue\r\n:1\r\n-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
    );
    peer.write_all(b"SLOWLOG LEN\r\n").unwrap();
    let length = read_line(&mut peer);
    let text = String::from_utf8_lossy(&length);
    let count: usize = text
        .strip_prefix(':')
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0);
    assert!(
        count >= 4,
        "slowlog should retain deferred commands: {text}"
    );
    handle.shutdown().unwrap();
}
