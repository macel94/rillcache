#![forbid(unsafe_code)]

//! P2: the stated SLOs and the capacity envelope are published and tied to the
//! machine-checked SLO file and the controlled-host runner.

use std::path::Path;

#[test]
fn stated_slos_and_capacity_plan_are_published_and_linked() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let doc = std::fs::read_to_string(manifest.join("docs/slo-and-capacity.md"))
        .expect("SLO and capacity page");
    for statement in [
        "Stated SLOs (v1)",
        "throughput",
        "p99",
        "Resident memory",
        "Capacity plan",
        "dedicated single-tenant",
        "not independently accepted",
    ] {
        assert!(
            doc.contains(statement),
            "the SLO page must keep {statement:?}"
        );
    }
    let slo = std::fs::read_to_string(manifest.join("ci/slo-v1.json")).expect("SLO file");
    for key in [
        "throughput_ops_per_second_min",
        "p50_ms_max",
        "p99_ms_max",
        "errors_max",
        "restart_seconds_max",
        "rss_bytes_max",
    ] {
        assert!(slo.contains(key), "ci/slo-v1.json must define {key}");
    }
    assert!(
        manifest.join("scripts/controlled_run.py").exists(),
        "the controlled-host runner must exist"
    );
    let readme = std::fs::read_to_string(manifest.join("docs/README.md")).expect("docs index");
    assert!(
        readme.contains("slo-and-capacity.md"),
        "the docs index must link the SLO page"
    );
}
