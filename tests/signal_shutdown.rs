#![forbid(unsafe_code)]

#[cfg(unix)]
mod unix {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::process::{Child, Command, Stdio};
    use std::thread;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    struct ServerProcess {
        child: Child,
        _stderr: BufReader<std::process::ChildStderr>,
    }

    impl Drop for ServerProcess {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    fn start(directory: &std::path::Path) -> (ServerProcess, SocketAddr) {
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
        if let Ok(mode) = std::env::var("RILLCACHE_TEST_THREADS")
            && mode != "0"
        {
            command.args(["--threads", &mode]);
        }
        let mut child = command.spawn().unwrap();
        let mut stderr = BufReader::new(child.stderr.take().unwrap());
        let mut ready = String::new();
        stderr.read_line(&mut ready).unwrap();
        assert!(ready.contains(" listening on "), "{ready}");
        (
            ServerProcess {
                child,
                _stderr: stderr,
            },
            address,
        )
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

    fn sigterm(server: &mut ServerProcess) {
        let status = Command::new("/bin/kill")
            .args(["-TERM", &server.child.id().to_string()])
            .status()
            .unwrap();
        assert!(status.success(), "kill command exited with {status}");
        wait_for_success(server);
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
