#![forbid(unsafe_code)]

//! L6: the declared deployment path must ship reviewable artifacts, a runbook,
//! and no implied HA or persistence guarantee the profile does not provide.

use std::path::Path;
use std::process::Command;

fn read(relative: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

#[test]
fn systemd_unit_uses_only_documented_flags_and_split_safe_lifecycle_settings() {
    let unit = read("deploy/systemd/rillcache.service");
    let help = Command::new(env!("CARGO_BIN_EXE_rillcache"))
        .arg("--help")
        .output()
        .expect("candidate --help must run");
    assert!(help.status.success(), "--help failed");
    let help = String::from_utf8_lossy(&help.stdout).into_owned();

    // Every long option the ExecStart command uses must exist in the candidate's
    // own help; comments are not scanned.
    let exec: Vec<&str> = unit
        .lines()
        .skip_while(|line| !line.starts_with("ExecStart="))
        .take_while(|line| line.ends_with('\\') || line.starts_with("ExecStart="))
        .collect();
    assert!(!exec.is_empty(), "unit must define ExecStart");
    for line in exec {
        let Some(option) = line
            .split_whitespace()
            .find(|part| part.starts_with("--") && part.len() > 2)
        else {
            continue;
        };
        let option = option.split('\\').next().unwrap_or(option);
        // Compare the flag name only: "--secure-secret-files=yes" is documented as
        // "--secure-secret-files[=yes|no]".
        let option = option.split('=').next().unwrap_or(option);
        if option.len() <= 2 {
            continue;
        }
        assert!(
            help.contains(option),
            "unit uses {option} which the candidate help does not document"
        );
    }
    for required in [
        "--appendonly yes",
        "--appendfsync always",
        "--dir /var/lib/rillcache",
        "--aclfile /etc/rillcache/users.acl",
        "--secure-secret-files=yes",
        "--shutdown-deadline-ms 25000",
    ] {
        assert!(unit.contains(required), "unit must set {required}");
    }
    for hardening in [
        "User=rillcache",
        "NoNewPrivileges=yes",
        "ProtectSystem=strict",
        "ReadWritePaths=/var/lib/rillcache",
        "PrivateDevices=yes",
        "RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX",
        "TimeoutStopSec=35s",
        "Restart=on-failure",
        "UMask=0077",
    ] {
        assert!(unit.contains(hardening), "unit must set {hardening}");
    }
    // The unit documents the single-node contract instead of implying HA.
    let lower = unit.to_lowercase();
    assert!(lower.contains("single-node") || lower.contains("single process"));
    assert!(lower.contains("no standby") || lower.contains("single writer"));
}

#[test]
fn shipped_examples_do_not_imply_unsupported_ha_or_persistence_guarantees() {
    let compose = read("compose.yaml");
    let kubernetes = read("deploy/kubernetes.yaml");
    let docs = read("docs/containers.md");

    for (name, text) in [
        ("compose.yaml", &compose),
        ("deploy/kubernetes.yaml", &kubernetes),
    ] {
        let lower = text.to_lowercase();
        assert!(
            lower.contains("single")
                || lower.contains("replicas: 1")
                || lower.contains("replicas: \"1\""),
            "{name} must state or encode a single instance"
        );
        if name == "deploy/kubernetes.yaml" {
            assert!(
                !lower.contains("replicas: 2"),
                "{name} must not ship a multi-replica example"
            );
        }
    }
    assert!(
        compose.contains("replication") || compose.to_lowercase().contains("no background"),
        "compose example must disclaim replication/backup support"
    );
    for required in ["single", "replication", "no standby"] {
        assert!(
            docs.to_lowercase().contains(required),
            "docs/containers.md must state the single-node boundary ({required})"
        );
    }

    // The deployment runbook exists and points at the verified recovery path.
    let runbook = read("docs/containers.md");
    for required in [
        "deploy/systemd/rillcache.service",
        "restore_drill",
        "restart",
    ] {
        assert!(
            runbook.contains(required),
            "runbook must reference {required}"
        );
    }
}

#[test]
fn unit_has_no_second_instance_escape_hatch() {
    let unit = read("deploy/systemd/rillcache.service");
    assert_eq!(
        1,
        unit.matches("ExecStart=").count(),
        "a single ExecStart keeps the single-writer journal contract explicit"
    );
    assert!(
        !unit.contains("ReplicateDirectories"),
        "no topology features are supported"
    );
}
