#![forbid(unsafe_code)]

#[cfg(unix)]
#[path = "support/signal_cancellation.rs"]
mod signal_cancellation;

#[cfg(unix)]
mod unix {
    use super::signal_cancellation;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{SocketAddr, TcpStream};
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
        // Let the server itself bind port zero. Reserving a port here and then
        // dropping the listener before spawn creates a TOCTOU race with other
        // parallel integration tests and can fail with EADDRINUSE.
        let mut command = Command::new(env!("CARGO_BIN_EXE_rillcache"));
        command
            .args([
                "--bind",
                "127.0.0.1:0",
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
        let address = ready
            .split_once(" listening on ")
            .expect("server readiness log did not include bound address")
            .1
            .trim()
            .parse::<SocketAddr>()
            .expect("server readiness log contained invalid bound address");
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

    fn wait_for_exit_by(server: &mut ServerProcess, deadline: Instant) -> std::process::ExitStatus {
        loop {
            if let Some(status) = server.child.try_wait().unwrap() {
                return status;
            }
            if Instant::now() >= deadline {
                let _ = server.child.kill();
                panic!("server exceeded the five-second shutdown test budget");
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

    enum ShutdownWithPendingReply {
        SecondSignal,
        DrainTimeout,
    }

    fn verify_shutdown_with_unread_reply(server_mode: &[&str], scenario: ShutdownWithPendingReply) {
        let directory = signal_cancellation::TestDirectory::new();
        let mut server_args = vec![
            "--appendfsync",
            "always",
            "--max-frame-bytes",
            "16777216",
            "--max-bulk-bytes",
            "8388608",
        ];
        server_args.extend_from_slice(server_mode);
        let (mut server, address) = start_with(&directory.0, &server_args);
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

        // Observe the reply start without relieving backpressure. In the timeout
        // case the client does not consume any bytes until after the process exits.
        if matches!(scenario, ShutdownWithPendingReply::DrainTimeout) {
            pending
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut first = [0];
            assert_eq!(pending.peek(&mut first).unwrap(), 1);
            assert_eq!(first, *b"$");
        }
        let started = Instant::now();
        send_sigterm(&server);
        let stopping = read_shutdown_log(&mut server);
        assert!(
            stopping.contains("received SIGTERM; stopping admission"),
            "unexpected shutdown log: {stopping:?}"
        );
        match scenario {
            ShutdownWithPendingReply::SecondSignal => {
                let forced_by = Instant::now() + Duration::from_secs(5);
                send_sigterm(&server);
                let forced = read_shutdown_log(&mut server);
                assert!(
                    forced.contains("forced termination: SIGTERM"),
                    "unexpected forced-shutdown log: {forced:?}"
                );
                let status = wait_for_exit_by(&mut server, forced_by);
                assert!(!status.success(), "forced shutdown exited successfully");
            }
            ShutdownWithPendingReply::DrainTimeout => {
                let budget = Duration::from_secs(5);
                let status = wait_for_exit_by(&mut server, started + budget);
                let elapsed = started.elapsed();
                assert!(status.success(), "bounded drain exited with {status}");
                // Allow scheduling slack around the one-second client drain, but
                // distinguish it from immediately dropping the admitted reply.
                assert!(
                    (Duration::from_millis(800)..budget).contains(&elapsed),
                    "unexpected drain duration: {elapsed:?}"
                );
                let complete = read_shutdown_log(&mut server);
                assert!(complete.contains("workers joined and enabled journal flushed"));
                let header = format!("${}\r\n", value.len());
                let reply_len = header.len() + value.len() + 2;
                let mut partial = Vec::new();
                if let Err(error) = (&mut pending)
                    .take(reply_len as u64)
                    .read_to_end(&mut partial)
                {
                    assert_eq!(error.kind(), std::io::ErrorKind::ConnectionReset, "{error}");
                }
                assert!(partial.starts_with(header.as_bytes()));
                assert!(
                    partial.len() < reply_len,
                    "the entire reply fit before exit"
                );
                let body = &partial[header.len()..];
                let body_len = value.len().min(body.len());
                assert!(
                    body[..body_len] == value[..body_len],
                    "reply prefix changed"
                );
                assert!(b"\r\n".starts_with(&body[body_len..]));
            }
        }

        drop(pending);
        drop(server);
        // Restart without altering the journal directory. Linux can also reclaim
        // the crash-stale AOF lock after the forced owner exits.
        #[cfg(target_os = "linux")]
        {
            let (mut restarted, address) = start_with(&directory.0, &server_args);
            let mut recovery = TcpStream::connect(address).unwrap();
            recovery
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            recovery
                .write_all(b"*2\r\n$3\r\nGET\r\n$3\r\nkey\r\n")
                .unwrap();
            assert_eq!(read_bulk(&mut recovery), value);
            sigterm(&mut restarted);
        }
    }

    #[test]
    fn second_sigterm_forces_nonzero_exit_with_an_admitted_reply_pending() {
        verify_shutdown_with_unread_reply(&[], ShutdownWithPendingReply::SecondSignal);
    }

    #[test]
    fn tokio_second_sigterm_forces_nonzero_exit_with_an_admitted_reply_pending() {
        verify_shutdown_with_unread_reply(
            &["--io-mode", "tokio"],
            ShutdownWithPendingReply::SecondSignal,
        );
    }

    #[test]
    fn sigterm_bounds_an_unread_reply_drain_and_preserves_acknowledged_aof() {
        verify_shutdown_with_unread_reply(&[], ShutdownWithPendingReply::DrainTimeout);
    }

    #[test]
    fn tokio_sigterm_bounds_an_unread_reply_drain_and_preserves_acknowledged_aof() {
        verify_shutdown_with_unread_reply(
            &["--io-mode", "tokio"],
            ShutdownWithPendingReply::DrainTimeout,
        );
    }

    fn verify_sigterm_cancels_pending_clients(server_mode: &[&str]) {
        let directory = signal_cancellation::TestDirectory::new();
        let (mut server, address) = start_with(&directory.0, server_mode);
        let connect = |address| {
            let stream = TcpStream::connect(address).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            stream
        };
        signal_cancellation::with_pending_clients(
            || connect(address),
            read_bulk,
            || sigterm(&mut server),
        );
        drop(server);

        let (mut restarted, address) = start_with(&directory.0, server_mode);
        signal_cancellation::verify_cancelled_mutations_were_not_replayed(connect(address));
        sigterm(&mut restarted);
    }

    #[test]
    fn sigterm_cancels_pending_clients_without_replaying_their_mutations() {
        verify_sigterm_cancels_pending_clients(&[]);
    }

    #[test]
    fn tokio_sigterm_cancels_pending_clients_without_replaying_their_mutations() {
        verify_sigterm_cancels_pending_clients(&["--io-mode", "tokio"]);
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
