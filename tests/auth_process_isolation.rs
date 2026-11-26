#![forbid(unsafe_code)]

//! Real-binary evidence that authentication budgets are process-local.
//!
//! S4 requires either a shared limiter or an explicit requirement for an
//! external control plane. These tests prove the second case is not an
//! assumption: two independent server processes with identical per-source
//! limits do not share lockout state, so deployment-wide enforcement cannot be
//! obtained from the server's own budgets alone.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, ChildStderr, Command, Stdio};
use std::time::Duration;

use rillcache::protocol::encode_request;

const WRONGPASS: &[u8] = b"-WRONGPASS invalid username-password pair or user is disabled.\r\n";
const THROTTLED: &[u8] = b"-ERR too many authentication attempts, try later\r\n";

struct Server {
    child: Child,
    addr: String,
    // Keep the stderr pipe open for the server's lifetime so later diagnostics
    // cannot fail with a closed pipe.
    _stderr: BufReader<ChildStderr>,
}

impl Server {
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rillcache"))
            .args([
                "--bind",
                "127.0.0.1:0",
                "--requirepass",
                "secret",
                "--auth-source-limit",
                "2",
                "--auth-source-window-ms",
                "600000",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn rillcache");
        let mut stderr = BufReader::new(child.stderr.take().expect("stderr"));
        let mut line = String::new();
        stderr.read_line(&mut line).expect("startup line");
        assert!(line.contains(" listening on "), "startup: {line}");
        let addr = line
            .split(" listening on ")
            .nth(1)
            .expect("listener address")
            .trim()
            .to_owned();
        Self {
            child,
            addr,
            _stderr: stderr,
        }
    }

    fn connect(&self) -> TcpStream {
        let stream = TcpStream::connect(&self.addr).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream.set_nodelay(true).unwrap();
        stream
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn command(stream: &mut TcpStream, args: &[&[u8]], expected: &[u8]) {
    let mut request = Vec::new();
    encode_request(args, &mut request);
    stream.write_all(&request).unwrap();
    let mut reply = vec![0; expected.len()];
    stream.read_exact(&mut reply).unwrap();
    assert_eq!(reply, expected, "{args:?}");
}

#[test]
fn authentication_lockout_is_not_shared_between_processes() {
    let locked = Server::start();
    let independent = Server::start();

    let mut client = locked.connect();
    command(&mut client, &[b"AUTH", b"wrong"], WRONGPASS);
    command(&mut client, &[b"AUTH", b"wrong"], WRONGPASS);
    // The third attempt is refused before the password is even checked, and the
    // lockout survives reconnecting to the same process.
    command(&mut client, &[b"AUTH", b"secret"], THROTTLED);
    drop(client);
    let mut retry = locked.connect();
    command(&mut retry, &[b"AUTH", b"secret"], THROTTLED);

    // The second process has its own counters: a correct password still works,
    // a single failure is not a lockout, and a following success still works.
    let mut other = independent.connect();
    command(&mut other, &[b"AUTH", b"secret"], b"+OK\r\n");
    command(&mut other, &[b"PING"], b"+PONG\r\n");
    command(&mut other, &[b"AUTH", b"wrong"], WRONGPASS);
    command(&mut other, &[b"AUTH", b"secret"], b"+OK\r\n");
    command(&mut other, &[b"PING"], b"+PONG\r\n");

    // The locked process is still locked: no state crossed between processes.
    command(&mut retry, &[b"AUTH", b"secret"], THROTTLED);
}
