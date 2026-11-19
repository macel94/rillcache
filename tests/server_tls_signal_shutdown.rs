#![forbid(unsafe_code)]

#[cfg(unix)]
mod unix {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::sync::{Arc, mpsc};
    use std::thread;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use rustls::pki_types::ServerName;
    use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

    struct ServerProcess {
        child: Child,
        stderr_lines: mpsc::Receiver<std::io::Result<String>>,
        stderr_reader: Option<thread::JoinHandle<()>>,
    }

    impl Drop for ServerProcess {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
            if let Some(reader) = self.stderr_reader.take() {
                let _ = reader.join();
            }
        }
    }

    type TlsStream = StreamOwned<ClientConnection, TcpStream>;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/tls")
            .join(name)
    }

    fn start(directory: &Path) -> (ServerProcess, SocketAddr) {
        let plain_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let tls_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let plain_address = plain_listener.local_addr().unwrap();
        let tls_address = tls_listener.local_addr().unwrap();
        drop(plain_listener);
        drop(tls_listener);

        let mut command = Command::new(env!("CARGO_BIN_EXE_rillcache"));
        command
            .args([
                "--bind",
                &plain_address.to_string(),
                "--tls-bind",
                &tls_address.to_string(),
                "--tls-cert-file",
                fixture("server.crt").to_str().unwrap(),
                "--tls-key-file",
                fixture("server.key").to_str().unwrap(),
                "--appendonly",
                "yes",
                "--appendfsync",
                "no",
                "--dir",
                directory.to_str().unwrap(),
                "--max-frame-bytes",
                "16777216",
                "--max-bulk-bytes",
                "8388608",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        if let Ok(mode) = std::env::var("RILLCACHE_TEST_THREADS")
            && mode != "0"
        {
            command.args(["--threads", &mode]);
        }
        let mut child = command.spawn().unwrap();
        let stderr = child.stderr.take().unwrap();
        let (stderr_sender, stderr_lines) = mpsc::channel();
        let stderr_reader = thread::spawn(move || {
            let mut stderr = BufReader::new(stderr);
            loop {
                let mut line = String::new();
                match stderr.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        if stderr_sender.send(Ok(line)).is_err() {
                            break;
                        }
                    }
                    Err(error) => {
                        let _ = stderr_sender.send(Err(error));
                        break;
                    }
                }
            }
        });
        let server = ServerProcess {
            child,
            stderr_lines,
            stderr_reader: Some(stderr_reader),
        };
        let ready = server
            .stderr_lines
            .recv_timeout(Duration::from_secs(10))
            .expect("TLS server did not announce readiness")
            .expect("TLS server stderr read failed");
        assert!(ready.contains(" listening on "), "{ready}");
        (server, tls_address)
    }

    fn wait_for_success(server: &mut ServerProcess) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = server.child.try_wait().unwrap() {
                assert!(status.success(), "server exited with {status}");
                return;
            }
            if Instant::now() >= deadline {
                let _ = server.child.kill();
                panic!("TLS server did not finish graceful shutdown within 10 seconds");
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn connect(address: SocketAddr) -> TlsStream {
        let tcp = TcpStream::connect(address).unwrap();
        tcp.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        tcp.set_write_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let mut roots = RootCertStore::empty();
        let mut reader = BufReader::new(std::fs::File::open(fixture("ca.crt")).unwrap());
        for certificate in rustls_pemfile::certs(&mut reader) {
            roots.add(certificate.unwrap()).unwrap();
        }
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = ClientConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let connection = ClientConnection::new(
            Arc::new(config),
            ServerName::try_from("localhost".to_owned()).unwrap(),
        )
        .unwrap();
        StreamOwned::new(connection, tcp)
    }

    fn command(stream: &mut TlsStream, request: &[u8]) -> Vec<u8> {
        stream.write_all(request).unwrap();
        read_bulk(stream)
    }

    fn expect_ok(stream: &mut TlsStream, request: &[u8]) {
        stream.write_all(request).unwrap();
        let mut response = [0; 5];
        stream.read_exact(&mut response).unwrap();
        assert_eq!(&response, b"+OK\r\n");
    }

    fn read_bulk(stream: &mut impl Read) -> Vec<u8> {
        let mut header = Vec::new();
        loop {
            let mut byte = [0; 1];
            stream.read_exact(&mut byte).unwrap();
            header.push(byte[0]);
            if header.ends_with(b"\r\n") {
                break;
            }
        }
        assert_eq!(header.first(), Some(&b'$'));
        let size: usize = std::str::from_utf8(&header[1..header.len() - 2])
            .unwrap()
            .parse()
            .unwrap();
        let mut value = vec![0; size];
        stream.read_exact(&mut value).unwrap();
        let mut crlf = [0; 2];
        stream.read_exact(&mut crlf).unwrap();
        assert_eq!(&crlf, b"\r\n");
        value
    }

    fn send_sigterm(server: &ServerProcess) {
        let status = Command::new("/bin/kill")
            .args(["-TERM", &server.child.id().to_string()])
            .status()
            .unwrap();
        assert!(status.success(), "kill command exited with {status}");
    }

    fn sigterm(server: &mut ServerProcess) {
        send_sigterm(server);
        wait_for_success(server);
    }

    #[test]
    fn tls_sigterm_drains_an_admitted_reply_and_flushes_aof_before_success_exit() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "rillcache-tls-signal-drain-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&directory).unwrap();
        let (mut server, address) = start(&directory);

        expect_ok(
            &mut connect(address),
            b"*4\r\n$6\r\nCONFIG\r\n$3\r\nSET\r\n$20\r\ncommandstats-enabled\r\n$3\r\nyes\r\n",
        );
        let value = vec![b'x'; 8 * 1024 * 1024];
        let mut set = format!("*3\r\n$3\r\nSET\r\n$3\r\nkey\r\n${}\r\n", value.len()).into_bytes();
        set.extend_from_slice(&value);
        set.extend_from_slice(b"\r\n");
        expect_ok(&mut connect(address), &set);

        let mut pending = connect(address);
        pending
            .write_all(b"*2\r\n$3\r\nGET\r\n$3\r\nkey\r\n")
            .unwrap();
        let mut admin = connect(address);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let stats = command(&mut admin, b"*2\r\n$4\r\nINFO\r\n$12\r\ncommandstats\r\n");
            if stats
                .windows(b"cmdstat_get:calls=1".len())
                .any(|window| window == b"cmdstat_get:calls=1")
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "TLS GET was not dispatched before SIGTERM"
            );
            thread::sleep(Duration::from_millis(10));
        }

        send_sigterm(&server);
        assert_eq!(read_bulk(&mut pending), value);
        wait_for_success(&mut server);
        drop(server);

        let (mut restarted, restarted_address) = start(&directory);
        let mut recovery = connect(restarted_address);
        assert_eq!(
            command(&mut recovery, b"*2\r\n$3\r\nGET\r\n$3\r\nkey\r\n"),
            value
        );
        sigterm(&mut restarted);
        drop(restarted);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
