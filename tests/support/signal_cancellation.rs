#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rillcache::protocol::encode_request;

pub(super) struct TestDirectory(pub PathBuf);

impl TestDirectory {
    pub(super) fn new() -> Self {
        // Parallel tests can observe the same wall-clock tick.
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "rillcache-signal-cancel-{}-{nonce}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn send(client: &mut impl Write, args: &[&[u8]]) {
    let mut request = Vec::new();
    encode_request(args, &mut request);
    client.write_all(&request).unwrap();
    client.flush().unwrap();
}

fn expect(client: &mut impl Read, expected: &[u8]) {
    let mut reply = vec![0; expected.len()];
    client.read_exact(&mut reply).unwrap();
    assert_eq!(reply, expected);
}

pub(super) fn with_pending_clients<T: Read + Write>(
    mut connect: impl FnMut() -> T,
    mut read_bulk: impl FnMut(&mut T) -> Vec<u8>,
    shutdown: impl FnOnce(),
) {
    let mut idle = connect();
    send(&mut idle, &[b"PING"]);
    expect(&mut idle, b"+PONG\r\n");

    let mut incomplete = connect();
    let mut input = Vec::new();
    encode_request(&[b"PING"], &mut input);
    input.extend_from_slice(b"*3\r\n$3\r\nSET\r\n$10\r\nincomplete\r\n$3\r\nv");
    incomplete.write_all(&input).unwrap();
    incomplete.flush().unwrap();
    expect(&mut incomplete, b"+PONG\r\n");

    let mut subscriber = connect();
    send(&mut subscriber, &[b"SUBSCRIBE", b"events"]);
    expect(
        &mut subscriber,
        b"*3\r\n$9\r\nsubscribe\r\n$6\r\nevents\r\n:1\r\n",
    );

    let mut blocked = connect();
    let mut pipeline = Vec::new();
    encode_request(&[b"BLPOP", b"queue", b"0"], &mut pipeline);
    encode_request(&[b"SET", b"behind-block", b"ran"], &mut pipeline);
    blocked.write_all(&pipeline).unwrap();
    blocked.flush().unwrap();

    let mut observer = connect();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        send(&mut observer, &[b"INFO", b"clients"]);
        let info = read_bulk(&mut observer);
        if String::from_utf8_lossy(&info)
            .lines()
            .any(|line| line == "blocked_clients:1")
        {
            break;
        }
        assert!(Instant::now() < deadline, "BLPOP did not register");
        std::thread::sleep(Duration::from_millis(10));
    }
    // None of these peers sends FIN or completes its pending request before
    // the server has exited successfully.
    shutdown();
    drop((idle, incomplete, subscriber, blocked, observer));
}

pub(super) fn verify_cancelled_mutations_were_not_replayed(mut reader: impl Read + Write) {
    for key in [b"incomplete".as_slice(), b"behind-block"] {
        send(&mut reader, &[b"GET", key]);
        expect(&mut reader, b"$-1\r\n");
    }
}
