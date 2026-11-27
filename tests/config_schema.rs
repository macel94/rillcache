#![forbid(unsafe_code)]

//! The versioned startup configuration schema (C13).
//!
//! The documented defaults must be the defaults the binary uses, the schema
//! marker must be versioned and ordered, and every invalid or out-of-version
//! startup file must fail before a listener binds. `--check-config` gives that
//! validation a side-effect-free, end-to-end entry point.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Duration;

use rillcache::Config;
use rillcache::engine::FsyncPolicy;

fn unique_dir() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rillcache-schema-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn check_config(path: &Path, extra: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rillcache"));
    command.arg("--check-config").arg("--config").arg(path);
    command.args(extra);
    command.output().unwrap()
}

#[test]
fn documented_defaults_match_the_binary_defaults() {
    let document = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/configuration.md"),
    )
    .unwrap();
    for row in [
        "| `max-clients` | `128` |",
        "| `max-data-bytes` | `67108864` (64 MiB) |",
        "| `appendfsync` | `everysec` |",
        "| `max-bulk-bytes` | `1048576` (1 MiB) |",
        "| `max-frame-bytes` | `2097152` (2 MiB) |",
        "| `idle-timeout-ms` | `30000` |",
        "| `frame-assembly-timeout-ms` | `60000` |",
        "| `shutdown-deadline-ms` | `25000` |",
        "| `auth-failure-limit` | `10` |",
        "| `auth-source-limit` | `0` (off) |",
        "| `auth-global-limit` | `0` (off) |",
        "| `auth-source-window-ms` | `60000` |",
        "| `appendonly` | `no` |",
        "| `storage-owners` | one owner |",
    ] {
        assert!(document.contains(row), "documented defaults drifted: {row}");
    }

    let declared = Config::default();
    assert_eq!(declared.max_clients, 128);
    assert_eq!(declared.max_data_bytes, 67_108_864);
    assert_eq!(declared.appendfsync, FsyncPolicy::EverySec);
    assert_eq!(declared.limits.max_bulk_bytes, 1_048_576);
    assert_eq!(declared.limits.max_frame_bytes, 2_097_152);
    assert_eq!(declared.idle_timeout, Duration::from_secs(30));
    assert_eq!(
        declared.frame_assembly_timeout,
        Some(Duration::from_secs(60))
    );
    assert_eq!(declared.shutdown_deadline, Duration::from_secs(25));
    assert_eq!(declared.auth_failure_limit, 10);
    assert_eq!(declared.auth_source_limit, 0);
    assert_eq!(declared.auth_global_limit, 0);
    assert_eq!(declared.auth_source_window_ms, 60_000);
    assert!(!declared.appendonly);
    assert_eq!(declared.storage_owners, None);
}

#[test]
fn the_schema_is_versioned_ordered_and_validated_before_bind() {
    let directory = unique_dir();
    let valid = directory.join("valid.conf");
    std::fs::write(
        &valid,
        "config-version 1\nbind 127.0.0.1:0\nmaxclients 128\nmax-data-bytes 268435456\n",
    )
    .unwrap();
    let accepted = check_config(&valid, &[]);
    assert!(accepted.status.success(), "{accepted:?}");

    // A later CLI option overrides the file value, and the merged options are
    // what `--check-config` validates.
    let overridden = check_config(&valid, &["--max-clients", "2048"]);
    assert!(overridden.status.success(), "{overridden:?}");

    let unversioned = directory.join("unversioned.conf");
    std::fs::write(&unversioned, "maxclients 16\n").unwrap();
    assert!(check_config(&unversioned, &[]).status.success());

    let future = directory.join("future.conf");
    std::fs::write(&future, "config-version 2\nmaxclients 16\n").unwrap();
    let rejected = check_config(&future, &[]);
    assert!(!rejected.status.success(), "{rejected:?}");
    // Failure output intentionally redacts paths and values, so the observable
    // contract is the nonzero exit plus the generic check message.
    let stderr = String::from_utf8_lossy(&rejected.stderr);
    assert!(
        stderr.contains("configuration check requires valid startup options"),
        "{stderr}"
    );

    let misplaced = directory.join("misplaced.conf");
    std::fs::write(&misplaced, "maxclients 16\nconfig-version 1\n").unwrap();
    assert!(!check_config(&misplaced, &[]).status.success());

    let unknown = directory.join("unknown.conf");
    std::fs::write(&unknown, "config-version 1\nnot-a-setting 1\n").unwrap();
    let rejected = check_config(&unknown, &[]);
    assert!(!rejected.status.success(), "{rejected:?}");

    let invalid = directory.join("invalid.conf");
    std::fs::write(&invalid, "config-version 1\nmaxclients 0\n").unwrap();
    assert!(!check_config(&invalid, &[]).status.success());

    let _ = std::fs::remove_dir_all(&directory);
}
