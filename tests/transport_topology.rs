#![forbid(unsafe_code)]
#![cfg(target_os = "linux")]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn one_effective_cpu_runs_one_network_runtime_plus_one_joined_cold_service() {
    let core = core_affinity::get_core_ids().unwrap()[0];
    // Pin a disposable launcher thread; its child inherits that mask. Never
    // change the test harness thread's affinity or require an external taskset.
    let child = thread::spawn(move || {
        assert!(core_affinity::set_for_current(core));
        Command::new(env!("CARGO_BIN_EXE_rillcache"))
            .args(["--bind", "127.0.0.1:0"])
            .stderr(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .unwrap()
    })
    .join()
    .unwrap();
    let mut child = OwnedChild(child);
    let stderr = child.0.stderr.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    let logger = thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            if sender.send(line.unwrap()).is_err() {
                break;
            }
        }
    });
    let line = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
    let addr = line
        .split("listening on ")
        .nth(1)
        .expect("listener address");
    let mut stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream.write_all(b"*1\r\n$4\r\nPING\r\n").unwrap();
    let mut pong = [0u8; 7];
    stream.read_exact(&mut pong).unwrap();
    assert_eq!(&pong, b"+PONG\r\n");
    let affinity = receiver.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(affinity.contains("Tokio runtime 0"), "{affinity}");

    let tasks = std::fs::read_dir(format!("/proc/{}/task", child.0.id())).unwrap();
    let mut names: Vec<_> = tasks
        .map(|task| {
            std::fs::read_to_string(task.unwrap().path().join("comm"))
                .unwrap()
                .trim()
                .to_owned()
        })
        .collect();
    names.sort();
    assert_eq!(names, ["rillcache", "rillcache-cold-"]);

    // Exercise the disk-capable command on the service, including mixed-case
    // dispatch and ordered pipeline continuation on the sole network runtime.
    stream
        .write_all(b"*3\r\n$3\r\naCl\r\n$7\r\nGENPASS\r\n$1\r\n8\r\n*1\r\n$4\r\nPING\r\n")
        .unwrap();
    let mut result = [0u8; 15];
    stream.read_exact(&mut result).unwrap();
    assert_eq!(&result[..4], b"$2\r\n");
    assert!(result[4..6].iter().all(u8::is_ascii_hexdigit));
    assert_eq!(&result[6..], b"\r\n+PONG\r\n");
    stream.write_all(b"*1\r\n$8\r\nSHUTDOWN\r\n").unwrap();
    // Protocol SHUTDOWN closes without a success reply (existing contract).
    assert_eq!(stream.read(&mut [0]).unwrap(), 0);
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(
            Instant::now() < deadline,
            "shutdown did not join all threads"
        );
        thread::yield_now();
    }
    logger.join().unwrap();
}
