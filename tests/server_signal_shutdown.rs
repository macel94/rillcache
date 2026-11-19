#![forbid(unsafe_code)]

#[cfg(unix)]
mod unix {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::process::{Child, Command, Stdio};
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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

    fn start(directory: &std::path::Path) -> (ServerProcess, SocketAddr) {
        start_with(directory, &[])
    }

    fn start_with(directory: &std::path::Path, extra: &[&str]) -> (ServerProcess, SocketAddr) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);

        let mut command = Command::new(env!("CARGO_BIN_EXE_rillcache"));
        command
            .args([
                "--bind",
                &address.to_string(),
                "--appendonly",
                "yes",
                "--appendfsync",
                "no",
                "--dir",
                directory.to_str().unwrap(),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        if !extra.contains(&"--io-mode")
            && let Ok(mode) = std::env::var("RILLCACHE_TEST_THREADS")
            && mode != "0"
        {
            command.args(["--threads", &mode]);
        }
        command.args(extra);
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
            .expect("server did not announce readiness")
            .expect("server stderr read failed");
        assert!(ready.contains(" listening on "), "{ready}");
        (server, address)
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
                panic!("server did not finish graceful shutdown within 10 seconds");
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn wait_for_forced_exit(server: &mut ServerProcess) -> std::process::ExitStatus {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = server.child.try_wait().unwrap() {
                return status;
            }
            if Instant::now() >= deadline {
                let _ = server.child.kill();
                panic!("server did not force shutdown within 5 seconds");
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn read_shutdown_log(server: &mut ServerProcess) -> String {
        server
            .stderr_lines
            .recv_timeout(Duration::from_secs(5))
            .expect("server did not emit shutdown log")
            .expect("server stderr read failed")
    }

    fn request(address: SocketAddr, bytes: &[u8], expected: &[u8]) {
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream.write_all(bytes).unwrap();
        let mut actual = vec![0; expected.len()];
        stream.read_exact(&mut actual).unwrap();
        assert_eq!(actual, expected);
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

    fn read_bulk(stream: &mut TcpStream) -> Vec<u8> {
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

    fn verify_sigterm_drains_an_admitted_reply_and_flushes_aof(server_mode: &[&str]) {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "rillcache-signal-drain-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&directory).unwrap();
        let mut server_args = vec![
            "--max-frame-bytes",
            "16777216",
            "--max-bulk-bytes",
            "8388608",
        ];
        server_args.extend_from_slice(server_mode);
        let (mut server, address) = start_with(&directory, &server_args);
        request(
            address,
            b"*4\r\n$6\r\nCONFIG\r\n$3\r\nSET\r\n$20\r\ncommandstats-enabled\r\n$3\r\nyes\r\n",
            b"+OK\r\n",
        );

        let value = vec![b'x'; 8 * 1024 * 1024];
        let mut set = format!("*3\r\n$3\r\nSET\r\n$3\r\nkey\r\n${}\r\n", value.len()).into_bytes();
        set.extend_from_slice(&value);
        set.extend_from_slice(b"\r\n");
        request(address, &set, b"+OK\r\n");

        let mut pending = TcpStream::connect(address).unwrap();
        pending
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        pending
            .write_all(b"*2\r\n$3\r\nGET\r\n$3\r\nkey\r\n")
            .unwrap();
        let mut admin = TcpStream::connect(address).unwrap();
        admin
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            admin
                .write_all(b"*2\r\n$4\r\nINFO\r\n$12\r\ncommandstats\r\n")
                .unwrap();
            let stats = read_bulk(&mut admin);
            if stats
                .windows(b"cmdstat_get:calls=1".len())
                .any(|window| window == b"cmdstat_get:calls=1")
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "GET was not dispatched before SIGTERM"
            );
            thread::sleep(Duration::from_millis(10));
        }

        send_sigterm(&server);
        assert_eq!(read_bulk(&mut pending), value);
        wait_for_success(&mut server);
        drop(server);

        let (mut restarted, restarted_address) = start_with(&directory, &server_args);
        let mut recovery = TcpStream::connect(restarted_address).unwrap();
        recovery
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        recovery
            .write_all(b"*2\r\n$3\r\nGET\r\n$3\r\nkey\r\n")
            .unwrap();
        assert_eq!(read_bulk(&mut recovery), value);
        sigterm(&mut restarted);
        drop(restarted);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn sigterm_drains_an_admitted_reply_and_flushes_aof_before_success_exit() {
        verify_sigterm_drains_an_admitted_reply_and_flushes_aof(&[]);
    }

    #[test]
    fn tokio_sigterm_drains_an_admitted_reply_and_flushes_aof_before_success_exit() {
        verify_sigterm_drains_an_admitted_reply_and_flushes_aof(&["--io-mode", "tokio"]);
    }

    #[test]
    fn second_sigterm_forces_nonzero_exit_with_an_admitted_reply_pending() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "rillcache-signal-forced-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&directory).unwrap();
        let (mut server, address) = start_with(
            &directory,
            &[
                "--max-frame-bytes",
                "16777216",
                "--max-bulk-bytes",
                "8388608",
            ],
        );
        request(
            address,
            b"*4\r\n$6\r\nCONFIG\r\n$3\r\nSET\r\n$20\r\ncommandstats-enabled\r\n$3\r\nyes\r\n",
            b"+OK\r\n",
        );
        let value = vec![b'x'; 8 * 1024 * 1024];
        let mut set = format!("*3\r\n$3\r\nSET\r\n$3\r\nkey\r\n${}\r\n", value.len()).into_bytes();
        set.extend_from_slice(&value);
        set.extend_from_slice(b"\r\n");
        request(address, &set, b"+OK\r\n");

        let mut pending = TcpStream::connect(address).unwrap();
        pending
            .write_all(b"*2\r\n$3\r\nGET\r\n$3\r\nkey\r\n")
            .unwrap();
        let mut admin = TcpStream::connect(address).unwrap();
        admin
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            admin
                .write_all(b"*2\r\n$4\r\nINFO\r\n$12\r\ncommandstats\r\n")
                .unwrap();
            let stats = read_bulk(&mut admin);
            if stats
                .windows(b"cmdstat_get:calls=1".len())
                .any(|window| window == b"cmdstat_get:calls=1")
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "GET was not dispatched before SIGTERM"
            );
            thread::sleep(Duration::from_millis(10));
        }

        send_sigterm(&server);
        let stopping = read_shutdown_log(&mut server);
        assert!(
            stopping.contains("received SIGTERM; stopping admission"),
            "unexpected shutdown log: {stopping:?}"
        );
        send_sigterm(&server);
        let forced = read_shutdown_log(&mut server);
        assert!(
            forced.contains("forced termination: SIGTERM"),
            "unexpected forced-shutdown log: {forced:?}"
        );
        let status = wait_for_forced_exit(&mut server);
        assert!(!status.success(), "forced shutdown exited successfully");

        drop(pending);
        drop(server);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn sigterm_joins_workers_and_flushes_aof_before_success_exit() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "rillcache-signal-shutdown-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&directory).unwrap();

        let (mut first, first_address) = start(&directory);
        request(
            first_address,
            b"*3\r\n$3\r\nSET\r\n$3\r\nkey\r\n$5\r\nvalue\r\n",
            b"+OK\r\n",
        );
        sigterm(&mut first);
        drop(first);

        let (mut restarted, address) = start(&directory);
        request(
            address,
            b"*2\r\n$3\r\nGET\r\n$3\r\nkey\r\n",
            b"$5\r\nvalue\r\n",
        );
        sigterm(&mut restarted);
        drop(restarted);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
