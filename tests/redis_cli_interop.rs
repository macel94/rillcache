#![forbid(unsafe_code)]

//! Optional real-client interoperability check.
//!
//! The test runs when `RILLCACHE_REDIS_CLI` points at a pinned `redis-cli` or a
//! `redis-cli` is available on PATH. Set `RILLCACHE_REQUIRE_REDIS_CLI=1` to turn
//! a missing client into a failure instead of a visible skip. This is the first
//! step of the documented real-client validation; language-client suites remain a
//! separate increment.

use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use rillcache::{Config, Server, ServerHandle};

fn start(config: Config) -> ServerHandle {
    let workers = match env::var("RILLCACHE_TEST_THREADS").as_deref() {
        Ok("0") | Err(_) => None,
        Ok("1") => Some(1),
        Ok("4") => Some(4),
        Ok(value) => panic!("invalid test worker mode {value}"),
    };
    Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        worker_threads: workers,
        ..config
    })
    .unwrap()
    .spawn()
    .unwrap()
}

fn cli_path() -> Option<PathBuf> {
    if let Some(path) = env::var_os("RILLCACHE_REDIS_CLI") {
        return Some(path.into());
    }
    let available = Command::new("redis-cli")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success());
    available.then(|| PathBuf::from("redis-cli"))
}

fn cli(cli: &Path, server: &ServerHandle, args: &[&str]) -> Output {
    Command::new(cli)
        .arg("-p")
        .arg(server.local_addr().port().to_string())
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("failed to run {}: {error}", cli.display()))
}

fn text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn both(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn assert_stdout(output: &Output, expected: &str) {
    assert!(
        output.status.success(),
        "redis-cli exited {:?}: {}",
        output.status.code(),
        both(output)
    );
    assert_eq!(text(output), expected, "combined output: {}", both(output));
}

fn assert_contains(output: &Output, needle: &str) {
    let combined = both(output);
    assert!(
        combined.contains(needle),
        "expected {needle:?} in combined output: {combined}"
    );
}

#[test]
fn pinned_redis_cli_negotiates_and_exercises_the_supported_surface() {
    let Some(cli_path) = cli_path() else {
        if env::var_os("RILLCACHE_REQUIRE_REDIS_CLI").is_some() {
            panic!("RILLCACHE_REQUIRE_REDIS_CLI is set but redis-cli was not found");
        }
        eprintln!("skipping real-client interop: install redis-cli or set RILLCACHE_REDIS_CLI");
        return;
    };
    let version = Command::new(&cli_path)
        .arg("--version")
        .output()
        .expect("redis-cli --version");
    eprintln!(
        "redis-cli interop version: {}",
        String::from_utf8_lossy(&version.stdout).trim()
    );

    let server = start(Config::default());

    assert_stdout(&cli(&cli_path, &server, &["--raw", "PING"]), "PONG");
    assert_stdout(
        &cli(
            &cli_path,
            &server,
            &["--raw", "SET", "interop:key", "value"],
        ),
        "OK",
    );
    assert_stdout(
        &cli(&cli_path, &server, &["--raw", "GET", "interop:key"]),
        "value",
    );
    assert_stdout(
        &cli(
            &cli_path,
            &server,
            &["--raw", "APPEND", "interop:key", "-tail"],
        ),
        "10",
    );
    assert_stdout(
        &cli(&cli_path, &server, &["--raw", "GET", "interop:key"]),
        "value-tail",
    );
    assert_stdout(
        &cli(
            &cli_path,
            &server,
            &["--raw", "CLIENT", "SETNAME", "cli-interop"],
        ),
        "OK",
    );
    // redis-cli opens a fresh connection for the next invocation, so the name is
    // correctly absent. This also checks that client metadata is connection-local.
    assert_stdout(
        &cli(&cli_path, &server, &["--raw", "CLIENT", "GETNAME"]),
        "",
    );

    // RESP3 negotiation and a data command after the handshake.
    assert_stdout(&cli(&cli_path, &server, &["-3", "--raw", "PING"]), "PONG");
    assert_stdout(
        &cli(&cli_path, &server, &["-3", "--raw", "GET", "interop:key"]),
        "value-tail",
    );
    let hello = cli(&cli_path, &server, &["-3", "--raw", "HELLO", "3"]);
    assert!(hello.status.success(), "HELLO 3 failed: {}", both(&hello));
    assert_contains(&hello, "proto");

    server.shutdown().unwrap();

    // Real CLI authentication and a first-argument ACL user. redis-cli sends
    // CLIENT SETINFO during its handshake, so grant that subcommand explicitly.
    let server = start(Config {
        requirepass: Some(b"secret".to_vec()),
        ..Config::default()
    });
    let unauthenticated = cli(&cli_path, &server, &["--raw", "PING"]);
    assert_contains(&unauthenticated, "NOAUTH");
    let wrong = cli(
        &cli_path,
        &server,
        &["--no-auth-warning", "-a", "wrong", "--raw", "PING"],
    );
    assert_contains(&wrong, "WRONGPASS");
    assert_stdout(
        &cli(
            &cli_path,
            &server,
            &["--no-auth-warning", "-a", "secret", "--raw", "PING"],
        ),
        "PONG",
    );
    assert_stdout(
        &cli(
            &cli_path,
            &server,
            &[
                "--no-auth-warning",
                "--user",
                "default",
                "-a",
                "secret",
                "--raw",
                "PING",
            ],
        ),
        "PONG",
    );
    server.shutdown().unwrap();
}
