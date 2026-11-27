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

/// L2: the release contract is published and mechanically enforced, and every
/// still-open P0 checklist item is accepted in the release notes rather than
/// disappearing between the checklist and the notes.
#[test]
fn release_lifecycle_contract_is_published_and_enforced() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let lifecycle = std::fs::read_to_string(manifest.join("docs/release-lifecycle.md"))
        .expect("published release contract");
    for section in [
        "Semantic Versioning",
        "Support window",
        "Deprecation process",
        "Upgrade expectations",
        "Release gates",
    ] {
        assert!(
            lifecycle.contains(section),
            "release contract is missing {section}"
        );
    }

    let script = manifest.join("scripts/cut-release.sh");
    let contents = std::fs::read_to_string(&script).expect("release gate script");
    assert!(
        contents.contains("refusing to release") && contents.contains("--publish"),
        "cut-release.sh must refuse failed gates and require an explicit publish flag"
    );

    let notes = std::fs::read_to_string(manifest.join("docs/releases/v0.1.0.md"))
        .expect("versioned release notes");
    for section in [
        "Candidate",
        "Compatibility profile",
        "Migration",
        "Rollback",
        "Support window",
    ] {
        assert!(
            notes.contains(section),
            "release notes are missing {section}"
        );
    }

    let checklist = std::fs::read_to_string(manifest.join("docs/production-readiness-gaps.md"))
        .expect("canonical checklist");
    let mut open_p0 = Vec::new();
    for line in checklist.lines() {
        if !line.starts_with("- [ ] **") || !line.contains("[P0]") {
            continue;
        }
        let rest = line.trim_start_matches("- [ ] **");
        if let Some((id, _)) = rest.split_once('.') {
            open_p0.push(id.to_string());
        }
    }
    assert!(
        !open_p0.is_empty(),
        "expected open P0 items while the project is not production ready"
    );
    for id in &open_p0 {
        assert!(
            notes.contains(&format!("`{id}`")),
            "open P0 item {id} is not accepted in the release notes"
        );
    }
}
