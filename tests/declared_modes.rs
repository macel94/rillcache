#![forbid(unsafe_code)]

//! P5: the profile's non-default modes are declared as ungated/experimental, and the
//! declaration cannot drift from the actual required-gate scope or the checklist.

use std::path::Path;

fn read(manifest: &Path, relative: &str) -> String {
    std::fs::read_to_string(manifest.join(relative))
        .unwrap_or_else(|_| panic!("missing {relative}"))
}

#[test]
fn non_default_modes_are_declared_ungated_and_match_the_gate_scope() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let profile = read(manifest, "docs/production-profile.md");
    let section = profile
        .split("## Non-production experiment modes")
        .nth(1)
        .expect("profile must declare the non-production experiment modes")
        .split("\n## ")
        .next()
        .expect("section body");

    for flag in [
        "--tls-bind",
        "--appendonly",
        "--io-mode tokio",
        "--storage-owners",
        "--threads",
    ] {
        assert!(section.contains(flag), "the mode table must name {flag}");
    }
    for statement in [
        "no regression budget",
        "no performance gate",
        "no latency SLO",
        "Opt-in experiment",
    ] {
        assert!(
            section.contains(statement),
            "the mode table must state {statement:?} for the ungated modes"
        );
    }

    // The gate inventory must not claim to measure these modes.
    let gates = read(manifest, ".github/pr-workflow-gates.json");
    for forbidden in ["--io-mode tokio", "--storage-owners", "--tls-bind"] {
        assert!(
            !gates.contains(forbidden),
            "the required gate inventory must not gate {forbidden}"
        );
    }

    // The checklist entry must keep pointing at the same settled position.
    let checklist = read(manifest, "docs/production-readiness-gaps.md");
    let entry = checklist
        .split("**P5.")
        .nth(1)
        .expect("P5 entry")
        .split("**P6.")
        .next()
        .expect("P5 body");
    assert!(
        entry.contains("exclusion branch"),
        "P5 must record the exclusion branch it closed on"
    );
    assert!(
        entry.starts_with(|c: char| c == ']' || c.is_whitespace()) || entry.contains("Complete"),
        "unexpected P5 entry shape"
    );

    // Gate scope documentation must agree that only the default configuration is gated.
    let scope = read(manifest, "docs/gate-scope.md");
    assert!(
        scope.contains("non-default modes"),
        "gate scope must state that non-default modes are outside the required gates"
    );
}
