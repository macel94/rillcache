#![forbid(unsafe_code)]

use std::net::{TcpListener, TcpStream};
use std::process::Command;

// These are functional checks, not a concurrent-load contract. Keep the live
// profiling fixture outside the health probe's actual one-second deadline.
static LIVE_SERVER_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn compiled_binaries_provide_help_and_server_version() {
    for binary in [
        env!("CARGO_BIN_EXE_rillcache"),
        env!("CARGO_BIN_EXE_rillcache-bench"),
    ] {
        let output = Command::new(binary).arg("--help").output().unwrap();
        assert!(output.status.success());
        assert!(String::from_utf8(output.stdout).unwrap().contains("Usage:"));
    }
    let output = Command::new(env!("CARGO_BIN_EXE_rillcache"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("rillcache {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn invalid_cli_arguments_fail_with_usage_status() {
    for binary in [
        env!("CARGO_BIN_EXE_rillcache"),
        env!("CARGO_BIN_EXE_rillcache-bench"),
    ] {
        let output = Command::new(binary)
            .arg("--not-an-option")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains("unknown option")
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn standalone_profiling_validates_all_workloads_and_fails_without_a_target() {
    let _fixture = LIVE_SERVER_TEST.lock().unwrap();
    use std::io::{BufRead, BufReader};
    use std::process::{Child, Stdio};

    struct ServerProcess(Child);
    impl Drop for ServerProcess {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut command = Command::new(env!("CARGO_BIN_EXE_rillcache"));
    command
        .args(["--bind", "127.0.0.1:0"])
        .stderr(Stdio::piped());
    let mode = std::env::var("RILLCACHE_TEST_THREADS").unwrap_or_else(|_| "0".into());
    if mode != "0" {
        command.args(["--threads", &mode]);
    }
    let mut server = ServerProcess(command.spawn().unwrap());
    let mut ready = String::new();
    BufReader::new(server.0.stderr.take().unwrap())
        .read_line(&mut ready)
        .unwrap();
    assert!(ready.contains(" listening on "), "{ready}");
    let addr = ready.split_whitespace().last().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rillcache-bench"))
        .args([
            "--addr",
            addr,
            "--server-pid",
            &server.0.id().to_string(),
            "--profile-idle-ms",
            "20",
            "--requests",
            "19",
            "--clients",
            "4",
            "--pipeline",
            "3",
            "--keyspace",
            "7",
            "--value-size",
            "0",
            "--warmup",
            "0",
            "--runs",
            "1",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = String::from_utf8(output.stdout).unwrap();
    assert_eq!(json.matches("\"requests\":19,").count(), 5);
    assert_eq!(json.matches("\"server_profile\":{").count(), 5);
    assert_eq!(json.matches("\"source\":\"linux-procfs\"").count(), 6);
    assert!(json.contains("\"idle_profile\":{\"source\":\"linux-procfs\""));
    assert!(!json.contains("NaN"));
    assert!(!json.contains("inf"));
    let named_workers = std::fs::read_dir(format!("/proc/{}/task", server.0.id()))
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|entry| std::fs::read_to_string(entry.path().join("comm")).ok())
        .filter(|name| name.trim().starts_with("rillcache-io-"))
        .count();
    assert_eq!(
        named_workers,
        mode.parse::<usize>().unwrap(),
        "actual fixed worker count differs from requested mode"
    );

    let failure = Command::new(env!("CARGO_BIN_EXE_rillcache-bench"))
        .args(["--addr", addr, "--server-pid", "4294967295"])
        .output()
        .unwrap();
    assert_eq!(failure.status.code(), Some(1));
    assert!(failure.stdout.is_empty());
    assert!(!failure.stderr.is_empty());
}

#[test]
fn authenticated_healthcheck_reads_a_bounded_password_file_without_leaking_secrets() {
    let _fixture = LIVE_SERVER_TEST.lock().unwrap();
    use rillcache::{Config, Server};
    use std::time::{SystemTime, UNIX_EPOCH};
    let mode = std::env::var("RILLCACHE_TEST_THREADS").unwrap_or_else(|_| "0".into());
    let secret = b"probe\0\r\n\xff";
    let server = Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        requirepass: Some(secret.to_vec()),
        worker_threads: match mode.as_str() {
            "0" => None,
            _ => Some(mode.parse().unwrap()),
        },
        ..Config::default()
    })
    .unwrap()
    .spawn()
    .unwrap();
    let path = std::env::temp_dir().join(format!(
        "rillcache-healthcheck-password-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let address = server.local_addr().to_string();
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    use std::io::Write;
    file.write_all(&[secret.as_slice(), b"\r\n"].concat())
        .unwrap();
    drop(file);
    let probe = || {
        Command::new(env!("CARGO_BIN_EXE_rillcache"))
            .args(["--healthcheck", &address, "--healthcheck-password-file"])
            .arg(&path)
            .output()
            .unwrap()
    };
    let success = probe();
    assert!(success.status.success(), "{:?}", success.stderr);
    assert!(success.stdout.is_empty());
    assert!(success.stderr.is_empty());
    let unauthenticated = Command::new(env!("CARGO_BIN_EXE_rillcache"))
        .args(["--healthcheck", &address])
        .output()
        .unwrap();
    assert_eq!(unauthenticated.status.code(), Some(1));
    for (content, status) in [
        (b"wrong-probe-secret".to_vec(), 1),
        (Vec::new(), 2),
        (b"\r\n".to_vec(), 2),
        (vec![b'x'; 1025], 2),
    ] {
        std::fs::write(&path, &content).unwrap();
        let failure = probe();
        assert_eq!(failure.status.code(), Some(status));
        assert!(failure.stdout.is_empty());
        assert!(
            !failure
                .stderr
                .windows(secret.len())
                .any(|part| part == secret)
        );
        assert!(
            !failure
                .stderr
                .windows(18)
                .any(|part| part == b"wrong-probe-secret")
        );
    }
    std::fs::remove_file(&path).unwrap();
    assert_eq!(probe().status.code(), Some(2));
    server.shutdown().unwrap();
}

#[test]
fn named_healthcheck_accepts_only_an_enabled_user_with_ping_permission() {
    let _fixture = LIVE_SERVER_TEST.lock().unwrap();
    use rillcache::{Config, Server};
    use std::time::{SystemTime, UNIX_EPOCH};
    let nonce = format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let acl = std::env::temp_dir().join(format!("rillcache-healthcheck-acl-{nonce}"));
    let password = std::env::temp_dir().join(format!("rillcache-healthcheck-secret-{nonce}"));
    std::fs::write(&acl, b"user default on >test-only-admin-secret -@all\nuser probe on >test-only-probe-secret -@all +ping\nuser denied on >test-only-probe-secret -@all\nuser disabled off >test-only-probe-secret -@all +ping\n").unwrap();
    std::fs::write(&password, b"test-only-probe-secret\n").unwrap();
    let mode = std::env::var("RILLCACHE_TEST_THREADS").unwrap_or_else(|_| "0".into());
    let server = Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        aclfile: Some(acl.clone()),
        worker_threads: match mode.as_str() {
            "0" => None,
            _ => Some(mode.parse().unwrap()),
        },
        ..Config::default()
    })
    .unwrap()
    .spawn()
    .unwrap();
    let address = server.local_addr().to_string();
    for user in ["probe", "denied", "disabled", "missing"] {
        for user_first in [false, true] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_rillcache"));
            command.args(["--healthcheck", &address]);
            if user_first {
                command.args(["--healthcheck-user", user]);
            }
            command.arg("--healthcheck-password-file").arg(&password);
            if !user_first {
                command.args(["--healthcheck-user", user]);
            }
            let output = command.output().unwrap();
            assert_eq!(
                output.status.code(),
                Some(if user == "probe" { 0 } else { 1 }),
                "user {user}"
            );
            assert!(output.stdout.is_empty());
            if user == "probe" {
                assert!(output.stderr.is_empty());
            }
            assert!(!String::from_utf8_lossy(&output.stderr).contains("test-only-probe-secret"));
        }
    }
    for extra in [
        vec!["--healthcheck-user", "probe"],
        vec!["--healthcheck-user", ""],
        vec!["--healthcheck-user", "bad user"],
        vec!["--healthcheck-user", "bad|user"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rillcache"))
            .args(["--healthcheck", &address])
            .args(extra)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
    }
    for duplicate in ["--healthcheck-user", "--healthcheck-password-file"] {
        let output = Command::new(env!("CARGO_BIN_EXE_rillcache"))
            .args([
                "--healthcheck",
                &address,
                "--healthcheck-user",
                "probe",
                "--healthcheck-password-file",
            ])
            .arg(&password)
            .args([duplicate, "ignored"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
    }
    server.shutdown().unwrap();
    std::fs::remove_file(acl).unwrap();
    std::fs::remove_file(password).unwrap();
}

#[test]
fn journal_status_healthcheck_requires_aof_and_info_permission_in_all_modes() {
    let _fixture = LIVE_SERVER_TEST.lock().unwrap();
    use rillcache::{Config, Server};
    use std::time::{SystemTime, UNIX_EPOCH};
    let directory = std::env::temp_dir().join(format!(
        "rillcache-journal-probe-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&directory).unwrap();
    let acl = directory.join("users.acl");
    let password = directory.join("password");
    std::fs::write(&acl, b"user default on nopass ~* &* +@all\nuser probe on >test-only-journal-secret -@all +ping +info\nuser pingonly on >test-only-journal-secret -@all +ping\n").unwrap();
    std::fs::write(&password, b"test-only-journal-secret\n").unwrap();
    let mode = std::env::var("RILLCACHE_TEST_THREADS").unwrap_or_else(|_| "0".into());
    for enabled in [false, true] {
        let server = Server::bind(Config {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            appendonly: enabled,
            dir: directory.clone(),
            aclfile: Some(acl.clone()),
            worker_threads: match mode.as_str() {
                "0" => None,
                _ => Some(mode.parse().unwrap()),
            },
            ..Config::default()
        })
        .unwrap()
        .spawn()
        .unwrap();
        let address = server.local_addr().to_string();
        for user in ["probe", "pingonly"] {
            for require_aof in [false, true] {
                let mut command = Command::new(env!("CARGO_BIN_EXE_rillcache"));
                command
                    .args([
                        "--healthcheck",
                        &address,
                        "--healthcheck-user",
                        user,
                        "--healthcheck-password-file",
                    ])
                    .arg(&password);
                if require_aof {
                    command.arg("--healthcheck-require-aof");
                }
                let output = command.output().unwrap();
                let healthy = !require_aof || (enabled && user == "probe");
                assert_eq!(
                    output.status.code(),
                    Some(if healthy { 0 } else { 1 }),
                    "aof={enabled}, user={user}, require={require_aof}"
                );
                assert!(output.stdout.is_empty());
                if healthy {
                    assert!(output.stderr.is_empty());
                }
                assert!(
                    !String::from_utf8_lossy(&output.stderr).contains("test-only-journal-secret")
                );
            }
        }
        if enabled {
            let writable = Command::new(env!("CARGO_BIN_EXE_rillcache"))
                .args([
                    "--healthcheck",
                    &address,
                    "--healthcheck-require-aof",
                    "--healthcheck-aof-write-probe",
                ])
                .arg(&directory)
                .output()
                .unwrap();
            assert!(writable.status.success(), "{:?}", writable.stderr);
            assert!(writable.stdout.is_empty());
            assert!(writable.stderr.is_empty());
            assert!(
                !std::fs::read_dir(&directory).unwrap().any(|entry| entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".rillcache-aof-healthcheck-")),
                "health probe left a temporary file"
            );

            let not_a_directory = Command::new(env!("CARGO_BIN_EXE_rillcache"))
                .args([
                    "--healthcheck",
                    &address,
                    "--healthcheck-require-aof",
                    "--healthcheck-aof-write-probe",
                ])
                .arg(&password)
                .output()
                .unwrap();
            assert_eq!(not_a_directory.status.code(), Some(1));
            assert!(not_a_directory.stdout.is_empty());
            assert!(
                !String::from_utf8_lossy(&not_a_directory.stderr)
                    .contains("test-only-journal-secret")
            );
        }

        let invalid_without_aof = Command::new(env!("CARGO_BIN_EXE_rillcache"))
            .args(["--healthcheck", &address, "--healthcheck-aof-write-probe"])
            .arg(&directory)
            .output()
            .unwrap();
        assert_eq!(invalid_without_aof.status.code(), Some(2));

        let plain = Command::new(env!("CARGO_BIN_EXE_rillcache"))
            .args(["--healthcheck", &address, "--healthcheck-require-aof"])
            .output()
            .unwrap();
        assert_eq!(plain.status.code(), Some(if enabled { 0 } else { 1 }));
        server.shutdown().unwrap();
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn tls_healthcheck_verifies_identity_and_supports_acl_and_journal_checks() {
    let _fixture = LIVE_SERVER_TEST.lock().unwrap();
    use rillcache::{Config, Server};
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};
    let fixture = |name: &str| {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/tls")
            .join(name)
    };
    let directory = std::env::temp_dir().join(format!(
        "rillcache-tls-probe-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&directory).unwrap();
    let password = directory.join("password");
    let acl = directory.join("users.acl");
    std::fs::write(&password, b"test-only-tls-probe-secret\n").unwrap();
    std::fs::write(&acl, b"user default on >test-only-admin-secret -@all\nuser probe on >test-only-tls-probe-secret -@all +ping +info\n").unwrap();
    let mode = std::env::var("RILLCACHE_TEST_THREADS").unwrap_or_else(|_| "0".into());
    let config = Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        tls_bind_addr: Some("127.0.0.1:0".parse().unwrap()),
        tls_cert_file: Some(fixture("server.crt")),
        tls_key_file: Some(fixture("server.key")),
        appendonly: true,
        dir: directory.clone(),
        aclfile: Some(acl),
        worker_threads: match mode.as_str() {
            "0" => None,
            _ => Some(mode.parse().unwrap()),
        },
        ..Config::default()
    };
    let server = Server::bind(config.clone()).unwrap().spawn().unwrap();
    let address = server.tls_local_addr().unwrap().to_string();
    let probe = |address: &str, ca: &Path, name: Option<&str>, require_aof: bool| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rillcache"));
        command
            .args([
                "--healthcheck",
                address,
                "--healthcheck-user",
                "probe",
                "--healthcheck-password-file",
            ])
            .arg(&password)
            .arg("--healthcheck-tls-ca-file")
            .arg(ca);
        if let Some(name) = name {
            command.args(["--healthcheck-server-name", name]);
        }
        if require_aof {
            command.arg("--healthcheck-require-aof");
        }
        command.output().unwrap()
    };
    for name in [None, Some("localhost")] {
        for require_aof in [false, true] {
            let output = probe(&address, &fixture("ca.crt"), name, require_aof);
            assert!(output.status.success(), "{:?}", output.stderr);
            assert!(output.stdout.is_empty());
            assert!(output.stderr.is_empty());
        }
    }
    let writable_tls_probe = Command::new(env!("CARGO_BIN_EXE_rillcache"))
        .args([
            "--healthcheck",
            &address,
            "--healthcheck-user",
            "probe",
            "--healthcheck-password-file",
        ])
        .arg(&password)
        .args([
            "--healthcheck-tls-ca-file",
            fixture("ca.crt").to_str().unwrap(),
            "--healthcheck-server-name",
            "localhost",
            "--healthcheck-require-aof",
            "--healthcheck-aof-write-probe",
        ])
        .arg(&directory)
        .output()
        .unwrap();
    assert!(
        writable_tls_probe.status.success(),
        "{:?}",
        writable_tls_probe.stderr
    );
    assert!(writable_tls_probe.stdout.is_empty());
    assert!(writable_tls_probe.stderr.is_empty());
    assert!(
        !std::fs::read_dir(&directory).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".rillcache-aof-healthcheck-")),
        "TLS health probe left a temporary file"
    );
    for output in [
        probe(&address, &fixture("ca.crt"), Some("wrong.example"), false),
        probe(&address, &fixture("client.crt"), None, false),
        probe(
            &server.local_addr().to_string(),
            &fixture("ca.crt"),
            None,
            false,
        ),
    ] {
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("test-only-tls-probe-secret"));
    }
    std::fs::write(&password, b"wrong-test-only-password").unwrap();
    let wrong_password = probe(&address, &fixture("ca.crt"), None, false);
    assert_eq!(wrong_password.status.code(), Some(1));
    assert!(!String::from_utf8_lossy(&wrong_password.stderr).contains("wrong-test-only-password"));
    let invalid_ca = directory.join("invalid-ca");
    for content in [
        Vec::new(),
        b"not-a-certificate".to_vec(),
        vec![b'x'; 1024 * 1024 + 1],
        std::fs::read(fixture("ca.crt")).unwrap().repeat(129),
    ] {
        std::fs::write(&invalid_ca, content).unwrap();
        assert_eq!(
            probe(&address, &invalid_ca, None, false).status.code(),
            Some(2)
        );
    }
    assert_eq!(
        probe(&address, &fixture("missing.crt"), None, false)
            .status
            .code(),
        Some(2)
    );
    assert_eq!(
        probe(&address, &fixture("ca.crt"), Some("invalid name"), false)
            .status
            .code(),
        Some(2)
    );
    server.shutdown().unwrap();

    // No implicit downgrade or unauthenticated client-certificate workaround.
    let mut mtls_config = config;
    mtls_config.tls_client_ca_file = Some(fixture("ca.crt"));
    let mtls = Server::bind(mtls_config).unwrap().spawn().unwrap();
    std::fs::write(&password, b"test-only-tls-probe-secret\n").unwrap();
    assert_eq!(
        probe(
            &mtls.tls_local_addr().unwrap().to_string(),
            &fixture("ca.crt"),
            None,
            false
        )
        .status
        .code(),
        Some(1)
    );
    mtls.shutdown().unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn healthcheck_is_quiet_on_success_and_fails_for_an_unavailable_endpoint() {
    let _fixture = LIVE_SERVER_TEST.lock().unwrap();
    use rillcache::{Config, Server};
    use std::io::{Read, Write};
    use std::time::Duration;
    let mode = std::env::var("RILLCACHE_TEST_THREADS").unwrap_or_else(|_| "0".into());
    let server = Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        worker_threads: match mode.as_str() {
            "0" => None,
            _ => Some(mode.parse().unwrap()),
        },
        ..Config::default()
    })
    .unwrap()
    .spawn()
    .unwrap();
    let address = server.local_addr().to_string();
    // spawn() does not promise that the serving thread has started. Establish
    // readiness once, separately from the unchanged CLI deadline; never retry a
    // failed health probe or increase its production timeout.
    {
        let mut ready =
            TcpStream::connect_timeout(&server.local_addr(), Duration::from_secs(3)).unwrap();
        ready
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        ready
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        ready.write_all(b"*1\r\n$4\r\nPING\r\n").unwrap();
        let mut reply = [0; 7];
        ready.read_exact(&mut reply).unwrap();
        assert_eq!(&reply, b"+PONG\r\n");
    }
    let output = Command::new(env!("CARGO_BIN_EXE_rillcache"))
        .args(["--healthcheck", &address])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    server.shutdown().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rillcache"))
        .args(["--healthcheck", &address])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("health check failed"));
}

#[test]
fn bind_failure_is_reported_with_a_nonzero_status() {
    let occupied = TcpListener::bind("127.0.0.1:0").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rillcache"))
        .args(["--bind", &occupied.local_addr().unwrap().to_string()])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty());
}
