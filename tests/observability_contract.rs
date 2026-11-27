#![forbid(unsafe_code)]
#![cfg(target_os = "linux")]

//! The documented metrics/log/health contract under failure (O4).
//!
//! The bundled probe must fail closed when the server cannot accept a new
//! client, when the required journal state is absent, and when the declared
//! payload headroom is not met — and it must recover as soon as the condition
//! clears. Redaction, auth-failure and shutdown behaviour are covered by
//! `tests/cli.rs`, `tests/monitor.rs`, `tests/slowlog*` and the signal suites.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use rillcache::protocol::encode_request;

struct Server {
    child: Child,
    address: String,
}

impl Server {
    fn start(directory: &Path, arguments: &[&str]) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rillcache"));
        command
            .args(["--bind", "127.0.0.1:0", "--dir"])
            .arg(directory)
            .args(arguments)
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut child = command.spawn().expect("spawn rillcache");
        let mut stderr = BufReader::new(child.stderr.take().expect("stderr"));
        let mut line = String::new();
        stderr.read_line(&mut line).expect("startup line");
        assert!(line.contains(" listening on "), "startup: {line}");
        let address = line
            .split(" listening on ")
            .nth(1)
            .expect("listener address")
            .trim()
            .to_owned();
        Self { child, address }
    }

    fn connect(&self) -> TcpStream {
        let stream = TcpStream::connect(&self.address).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn unique_dir() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rillcache-observe-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn probe(address: &str, password: &Path, extra: &[&str]) -> bool {
    Command::new(env!("CARGO_BIN_EXE_rillcache"))
        .args(["--healthcheck", address, "--healthcheck-password-file"])
        .arg(password)
        .args(extra)
        .output()
        .unwrap()
        .status
        .success()
}

fn admit(stream: &mut TcpStream, password: &[u8]) {
    let mut request = Vec::new();
    encode_request(&[b"AUTH", password], &mut request);
    encode_request(&[b"PING"], &mut request);
    stream.write_all(&request).unwrap();
    let mut reply = vec![0u8; 12];
    stream.read_exact(&mut reply).unwrap();
    assert_eq!(&reply[..5], b"+OK\r\n", "AUTH failed");
    assert_eq!(&reply[5..], b"+PONG\r\n");
}

#[test]
fn the_probe_fails_closed_on_overload_persistence_state_and_headroom() {
    let directory = unique_dir();
    let password = directory.join("probe.pass");
    std::fs::write(&password, b"probe-secret\n").unwrap();

    // Overload: with both admission slots taken, the probe cannot connect and
    // must report failure; releasing a slot restores success.
    let busy = Server::start(
        &directory,
        &["--requirepass", "probe-secret", "--max-clients", "2"],
    );
    assert!(probe(&busy.address, &password, &[]), "idle server failed");
    let mut first = busy.connect();
    admit(&mut first, b"probe-secret");
    let mut second = busy.connect();
    admit(&mut second, b"probe-secret");
    assert!(
        !probe(&busy.address, &password, &[]),
        "saturated server passed the probe"
    );
    drop(first);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !probe(&busy.address, &password, &[]) {
        assert!(
            std::time::Instant::now() < deadline,
            "probe did not recover after capacity was released"
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    // Persistence failure: requiring a journal against a server that reports
    // `aof_enabled:0` must fail closed.
    let no_journal = Server::start(
        &directory,
        &["--requirepass", "probe-secret", "--appendonly", "no"],
    );
    assert!(
        !probe(
            &no_journal.address,
            &password,
            &["--healthcheck-require-aof"]
        ),
        "require-aof passed without a journal"
    );
    drop(no_journal);

    let journal = Server::start(
        &directory,
        &["--requirepass", "probe-secret", "--appendonly", "yes"],
    );
    assert!(
        probe(&journal.address, &password, &["--healthcheck-require-aof"]),
        "require-aof failed for a healthy journal"
    );
    assert!(
        !probe(
            &journal.address,
            &password,
            &["--healthcheck-min-payload-headroom", "1099511627776"],
        ),
        "impossible headroom was accepted"
    );
    let _ = std::fs::remove_dir_all(&directory);
}
