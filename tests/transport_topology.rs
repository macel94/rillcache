#![forbid(unsafe_code)]
#![cfg(target_os = "linux")]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

struct Process {
    child: Child,
    logger: Option<JoinHandle<()>>,
    lines: Receiver<String>,
    addr: String,
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(logger) = self.logger.take() {
            logger.join().unwrap();
        }
    }
}

impl Process {
    fn start(core: Option<core_affinity::CoreId>) -> Self {
        Self::start_with_tls(core, false)
    }

    fn start_with_tls(core: Option<core_affinity::CoreId>, mixed: bool) -> Self {
        // Restrict a disposable launcher thread when requested; its child
        // inherits that mask. Never change the harness thread's affinity or
        // require an external taskset. None tests the full inherited mask.
        let mut child = thread::spawn(move || {
            if let Some(core) = core {
                assert!(core_affinity::set_for_current(core));
            }
            let mut command = Command::new(env!("CARGO_BIN_EXE_rillcache"));
            command.args(["--bind", "127.0.0.1:0"]);
            if mixed {
                command
                    .args(["--tls-bind", "127.0.0.2:0", "--tls-cert-file"])
                    .arg(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/fixtures/tls/server.crt"
                    ))
                    .arg("--tls-key-file")
                    .arg(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/fixtures/tls/server.key"
                    ));
            }
            command
                .stderr(Stdio::piped())
                .stdout(Stdio::null())
                .spawn()
                .unwrap()
        })
        .join()
        .unwrap();
        let stderr = child.stderr.take().unwrap();
        let (sender, lines) = mpsc::channel();
        let logger = thread::spawn(move || {
            for line in BufReader::new(stderr).lines() {
                if sender.send(line.unwrap()).is_err() {
                    break;
                }
            }
        });
        // Construct the owner before any assertion, so failure also reaps the
        // child and joins the stderr reader.
        let mut process = Self {
            child,
            logger: Some(logger),
            lines,
            addr: String::new(),
        };
        let line = process.lines.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(line.contains("listening on "), "server startup: {line}");
        process.addr = line
            .split("listening on ")
            .nth(1)
            .expect("listener address")
            .trim()
            .to_owned();
        process
    }

    fn connect(&self) -> TcpStream {
        let stream = TcpStream::connect(&self.addr).unwrap();
        stream.set_nodelay(true).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream
    }

    fn shutdown(&mut self, stream: &mut TcpStream) {
        stream.write_all(b"*1\r\n$8\r\nSHUTDOWN\r\n").unwrap();
        // Protocol SHUTDOWN closes without a success reply (existing contract).
        assert_eq!(stream.read(&mut [0]).unwrap(), 0);
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            assert!(
                Instant::now() < deadline,
                "shutdown did not join all threads"
            );
            thread::yield_now();
        }
        self.logger.take().unwrap().join().unwrap();
    }
}

fn affinity(status: &str) -> Vec<usize> {
    let list = status
        .lines()
        .find_map(|line| line.strip_prefix("Cpus_allowed_list:"))
        .expect("procfs affinity")
        .trim();
    list.split(',')
        .flat_map(|part| {
            let (first, last) = part.split_once('-').unwrap_or((part, part));
            first.parse::<usize>().unwrap()..=last.parse::<usize>().unwrap()
        })
        .collect()
}

fn process_affinity(pid: u32) -> Vec<usize> {
    affinity(&std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap())
}

fn ping(stream: &mut TcpStream) {
    stream.write_all(b"*1\r\n$4\r\nPING\r\n").unwrap();
    let mut pong = [0u8; 7];
    stream.read_exact(&mut pong).unwrap();
    assert_eq!(&pong, b"+PONG\r\n");
}

#[test]
fn runtime_zero_preserves_launch_mask_before_and_after_traffic() {
    let expected = process_affinity(std::process::id());
    let mut process = Process::start(None);
    let mut stream = process.connect();
    ping(&mut stream); // proves runtime startup, not merely TCP handshake
    assert_eq!(
        process_affinity(process.child.id()),
        expected,
        "runtime zero must not narrow the launcher-owned process-leader mask"
    );
    for _ in 0..32 {
        stream
            .write_all(b"*2\r\n$3\r\nGET\r\n$7\r\nmissing\r\n")
            .unwrap();
        let mut missing = [0; 5];
        stream.read_exact(&mut missing).unwrap();
        assert_eq!(&missing, b"$-1\r\n");
    }
    assert_eq!(process_affinity(process.child.id()), expected);
    // Pinning remote workers must never escape the inherited cpuset; successful
    // singleton pins use distinct IDs. Failure may retain the launcher mask.
    let mut pinned = Vec::new();
    for task in std::fs::read_dir(format!("/proc/{}/task", process.child.id())).unwrap() {
        let task = task.unwrap().path();
        let status = std::fs::read_to_string(task.join("status")).unwrap();
        let mask = affinity(&status);
        assert!(mask.iter().all(|id| expected.contains(id)));
        let name = std::fs::read_to_string(task.join("comm")).unwrap();
        if name.starts_with("rillcache-tokio") && mask.len() == 1 {
            assert!(
                !pinned.contains(&mask[0]),
                "remote workers must not wrap CPU IDs"
            );
            pinned.push(mask[0]);
        }
    }
    process.shutdown(&mut stream);
}

#[test]
fn mixed_tls_compatibility_workers_preserve_launch_cpuset_and_are_counted() {
    let expected = process_affinity(std::process::id());
    let budget = thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .clamp(1, 64);
    let mut process = Process::start_with_tls(None, true);
    let mut stream = process.connect();
    ping(&mut stream);
    assert_eq!(process_affinity(process.child.id()), expected);
    let mut tasks = 0;
    let mut pinned = Vec::new();
    for task in std::fs::read_dir(format!("/proc/{}/task", process.child.id())).unwrap() {
        let task = task.unwrap().path();
        let status = std::fs::read_to_string(task.join("status")).unwrap();
        let mask = affinity(&status);
        assert!(mask.iter().all(|id| expected.contains(id)));
        let name = std::fs::read_to_string(task.join("comm")).unwrap();
        if name.starts_with("rillcache-tokio") && mask.len() == 1 {
            assert!(!pinned.contains(&mask[0]));
            pinned.push(mask[0]);
        }
        tasks += 1;
    }
    // N network runtimes + bounded cold service + shared data owner + TLS accept.
    assert_eq!(tasks, budget + 3);
    process.shutdown(&mut stream);
}

#[test]
fn one_effective_cpu_runs_one_network_runtime_plus_one_joined_cold_service() {
    let core = core_affinity::get_core_ids().unwrap()[0];
    let mut process = Process::start(Some(core));
    let mut stream = process.connect();
    ping(&mut stream);
    assert_eq!(process_affinity(process.child.id()), [core.id]);
    let affinity = process.lines.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(
        affinity.contains("Tokio runtime 0 preserves launcher affinity"),
        "{affinity}"
    );

    let tasks = std::fs::read_dir(format!("/proc/{}/task", process.child.id())).unwrap();
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

    // Exercise mixed-case ACL dispatch and ordered continuation on runtime zero.
    stream
        .write_all(b"*3\r\n$3\r\naCl\r\n$7\r\nGENPASS\r\n$1\r\n8\r\n*1\r\n$4\r\nPING\r\n")
        .unwrap();
    let mut result = [0u8; 15];
    stream.read_exact(&mut result).unwrap();
    assert_eq!(&result[..4], b"$2\r\n");
    assert!(result[4..6].iter().all(u8::is_ascii_hexdigit));
    assert_eq!(&result[6..], b"\r\n+PONG\r\n");
    assert_eq!(process_affinity(process.child.id()), [core.id]);
    process.shutdown(&mut stream);
}
