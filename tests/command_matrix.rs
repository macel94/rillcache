#![forbid(unsafe_code)]

use std::path::Path;
use std::process::Command;

use rillcache::{Config, Server};

/// Run the committed C5 differential matrix check against one execution mode.
///
/// `scripts/command_matrix.py` compares the advertised `COMMAND` surface and the
/// wire reply of every absent pinned upstream command with
/// `docs/production-readiness/command-matrix.md`. A mismatch means the command
/// surface changed without the profile evidence changing with it.
#[test]
fn command_matrix_matches_the_tested_head_in_every_mode() {
    for workers in [None, Some(1), Some(4)] {
        let server = Server::bind(Config {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            worker_threads: workers,
            ..Config::default()
        })
        .unwrap()
        .spawn()
        .unwrap();
        let output = Command::new("python3")
            .arg("scripts/command_matrix.py")
            .arg("--addr")
            .arg(server.local_addr().to_string())
            .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")))
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .output()
            .expect("Python 3 is required by the offline command matrix check");
        server.shutdown().unwrap();
        assert!(
            output.status.success(),
            "worker mode {workers:?} failed the command matrix check:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// The invariants themselves are covered by offline unit tests, so the matrix
/// cannot quietly start accepting an answered absent command or an undocumented
/// named exclusion.
#[test]
fn command_matrix_invariants_are_covered_offline() {
    let output = Command::new("python3")
        .arg("scripts/test_command_matrix.py")
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")))
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("PYTHONPATH", "scripts")
        .output()
        .expect("Python 3 is required by the offline command matrix check");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
