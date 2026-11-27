#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use rillcache::protocol::encode_request;
use rillcache::{Config, Server};

#[test]
fn onboarding_pages_links_and_logo_assets_pass_offline_checks() {
    let output = Command::new("python3")
        .arg("scripts/test_onboarding.py")
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")))
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .expect("Python 3 is required by the existing offline policy checks");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn readme_ping_set_get_example_works_without_configuration() {
    let server = Server::bind(Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        ..Config::default()
    })
    .unwrap()
    .spawn()
    .unwrap();
    let mut stream = TcpStream::connect(server.local_addr()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let cases: &[(&[&[u8]], &[u8])] = &[
        (&[b"PING"], b"+PONG\r\n"),
        (&[b"SET", b"greeting", b"hello"], b"+OK\r\n"),
        (&[b"GET", b"greeting"], b"$5\r\nhello\r\n"),
    ];
    for (args, expected) in cases {
        let mut request = Vec::new();
        encode_request(args, &mut request);
        stream.write_all(&request).unwrap();
        let mut actual = vec![0; expected.len()];
        stream.read_exact(&mut actual).unwrap();
        assert_eq!(actual.as_slice(), *expected);
    }
    drop(stream);
    server.shutdown().unwrap();
}

/// V5: the canonical readiness inventory, its status line and its entry-point
/// links must be checked by a command that runs in the required CI graph, not
/// only from an operator shell.
#[test]
fn canonical_readiness_inventory_and_links_are_checked_in_ci() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let check = Command::new("python3")
        .args(["scripts/production_readiness.py", "--check"])
        .current_dir(manifest)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .expect("Python 3 is required by the offline readiness check");
    assert!(
        check.status.success(),
        "readiness inventory check failed:\n{}\n{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    let output = String::from_utf8_lossy(&check.stdout);
    assert!(
        output.contains("Readiness documents valid"),
        "unexpected readiness output: {output}"
    );

    // The regression suite for missing links, invalid inventory data and status
    // drift must also run here, otherwise a broken check could pass itself.
    let unit = Command::new("python3")
        .arg("-m")
        .arg("unittest")
        .arg("test_production_readiness")
        .current_dir(manifest.join("scripts"))
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .expect("Python 3 is required by the readiness regression tests");
    assert!(
        unit.status.success(),
        "readiness regression tests failed:\n{}\n{}",
        String::from_utf8_lossy(&unit.stdout),
        String::from_utf8_lossy(&unit.stderr)
    );
}

/// V5: the canonical not-ready status and the required entry-point links must be
/// visible from the public entry points, not only from the checklist itself.
#[test]
fn public_entry_points_link_the_canonical_checklist_and_keep_the_not_ready_status() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let checklist = std::fs::read_to_string(manifest.join("docs/production-readiness-gaps.md"))
        .expect("canonical checklist");
    assert!(
        checklist.starts_with("# Production readiness gaps"),
        "canonical checklist heading changed"
    );
    assert!(
        checklist.contains("**Status: NOT PRODUCTION READY.**"),
        "the canonical file must keep the explicit not-ready status"
    );
    for entry in ["README.md", "docs/README.md"] {
        let text = std::fs::read_to_string(manifest.join(entry)).expect("entry point");
        assert!(
            text.contains("production-readiness-gaps.md"),
            "{entry} must link the canonical readiness checklist"
        );
    }
}

/// S1: the threat model, review scope, findings log and sign-off registry are
/// machine-checked in the required CI graph. A missing evidence path, a finding
/// closed without retest evidence, an open finding without a tracking gap, or an
/// independent sign-off claimed without a reviewer/signature fails the build.
#[test]
fn security_review_registry_scope_and_signoff_are_checked_in_ci() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let check = Command::new("python3")
        .args(["scripts/security_review.py", "--check"])
        .current_dir(manifest)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .expect("Python 3 is required by the offline security review check");
    assert!(
        check.status.success(),
        "security review registry check failed:\n{}\n{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    let output = String::from_utf8_lossy(&check.stdout);
    assert!(
        output.contains("Security review registry valid"),
        "unexpected security review output: {output}"
    );

    // The regression suite must run here as well, otherwise a broken checker
    // could pass itself and the registry would silently become an attestation.
    let unit = Command::new("python3")
        .args(["-m", "unittest", "test_security_review"])
        .current_dir(manifest.join("scripts"))
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .expect("Python 3 is required by the security review regression tests");
    assert!(
        unit.status.success(),
        "security review regression tests failed:\n{}\n{}",
        String::from_utf8_lossy(&unit.stdout),
        String::from_utf8_lossy(&unit.stderr)
    );
}
