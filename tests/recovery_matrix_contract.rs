#![forbid(unsafe_code)]

//! D11: the crash-recovery matrix is published with its measured rows and its
//! explicit limits, and the harness that produces it stays wired up.

use std::path::Path;

#[test]
fn crash_recovery_matrix_is_published_with_limits() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let doc = std::fs::read_to_string(manifest.join("docs/recovery-matrix.md"))
        .expect("recovery matrix page");
    for statement in [
        "Kill -9 at each boundary",
        "Filesystem matrix",
        "0 lost acknowledged writes",
        "power cut",
        "Retained limits",
    ] {
        assert!(
            doc.contains(statement),
            "the recovery matrix must keep the statement {statement:?}"
        );
    }
    assert!(
        doc.contains("host-directory") && doc.contains("ext4-loop"),
        "both filesystem rows must stay in the published table"
    );
    assert!(
        doc.contains("crc32") && doc.contains("hmac"),
        "both journal framings must stay in the published table"
    );

    let harness = manifest.join("scripts/recovery_matrix.py");
    assert!(harness.exists(), "the recovery matrix harness must exist");
    let harness = std::fs::read_to_string(&harness).expect("harness");
    for symbol in ["SIGKILL", "kill_after", "mkfs.ext4", "hmac"] {
        assert!(
            harness.contains(symbol),
            "the harness must still exercise {symbol:?}"
        );
    }
}
