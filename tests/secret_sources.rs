#![forbid(unsafe_code)]
#![cfg(unix)]

//! The supported secret sources and their permission contract (S8).
//!
//! `--secure-secret-files` must refuse every documented secret source when it is
//! group/other-accessible, and accept owner-only files, before any listener
//! binds. `--check-config` runs the same validator without starting a server, so
//! this drives the real binary across the whole secret surface rather than one
//! unit-tested field.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

fn unique_dir() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rillcache-secrets-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn secret(directory: &Path, name: &str, mode: u32) -> PathBuf {
    let path = directory.join(name);
    std::fs::write(&path, b"0123456789abcdef0123456789abcdef").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
    path
}

fn check(arguments: &[String]) -> bool {
    Command::new(env!("CARGO_BIN_EXE_rillcache"))
        .arg("--check-config")
        .args(arguments)
        .output()
        .unwrap()
        .status
        .success()
}

fn flag(name: &str, path: &Path) -> Vec<String> {
    vec![name.to_string(), path.display().to_string()]
}

/// One secret source: label, the flag that points at the secret file, and the
/// arguments that make the surrounding options valid.
fn sources(directory: &Path) -> Vec<(&'static str, &'static str, Vec<String>)> {
    let acl = secret(directory, "users.acl", 0o600);
    let password = secret(directory, "password", 0o600);
    let journal = secret(directory, "journal.key", 0o600);
    let certificate = secret(directory, "server.crt", 0o600);
    let tls_key = secret(directory, "server.key", 0o600);
    let mut journal_arguments = vec!["--appendonly".to_string(), "yes".to_string()];
    journal_arguments.extend(flag("--aof-hmac-key-file", &journal));
    let mut tls_arguments = vec!["--tls-bind".to_string(), "127.0.0.1:0".to_string()];
    tls_arguments.extend(flag("--tls-cert-file", &certificate));
    tls_arguments.extend(flag("--tls-key-file", &tls_key));
    vec![
        (
            "requirepass-file",
            "--requirepass-file",
            flag("--requirepass-file", &password),
        ),
        ("aclfile", "--aclfile", flag("--aclfile", &acl)),
        (
            "aof-hmac-key-file",
            "--aof-hmac-key-file",
            journal_arguments,
        ),
        ("tls-key-file", "--tls-key-file", tls_arguments),
    ]
}

#[test]
fn every_secret_source_requires_owner_only_permissions_in_strict_mode() {
    let directory = unique_dir();
    for (name, secret_flag, arguments) in sources(&directory) {
        assert!(
            check(&arguments),
            "{name}: owner-only file was rejected in permissive mode"
        );
        let mut strict = vec!["--secure-secret-files".to_string()];
        strict.extend(arguments.clone());
        assert!(
            check(&strict),
            "{name}: owner-only file rejected in strict mode"
        );

        // Loosen the file that this source actually reads.
        let target = arguments
            .windows(2)
            .find(|pair| pair[0] == secret_flag)
            .map(|pair| PathBuf::from(&pair[1]))
            .expect("secret path");
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(
            !check(&strict),
            "{name}: group/other-readable file was accepted in strict mode"
        );
        assert!(
            check(&arguments),
            "{name}: default mode must stay permissive"
        );
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn strict_mode_enforces_a_long_random_password_on_the_command_line() {
    let short = vec![
        "--secure-secret-files".to_string(),
        "--requirepass".to_string(),
        "short".to_string(),
    ];
    assert!(!check(&short), "short requirepass was accepted");
    let long = vec![
        "--secure-secret-files".to_string(),
        "--requirepass".to_string(),
        "0123456789abcdef".to_string(),
    ];
    assert!(check(&long), "16-byte requirepass was rejected");
}
