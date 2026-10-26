#![forbid(unsafe_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn generate(input: &Path, output: &Path, sha: &str) {
    let result = Command::new(env!("CARGO_BIN_EXE_rillcache-review"))
        .args(["generate", "--input"])
        .arg(input)
        .arg("--out")
        .arg(output)
        .args([
            "--repo",
            "macel94/rillcache",
            "--sha",
            sha,
            "--run-id",
            "40245955560",
            "--run-number",
            "11",
            "--attempt",
            "1",
            "--benchmark-attempt",
            "1",
            "--date",
            "2026-10-25T23:51:41Z",
            "--status",
            "success",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn permanent_handoff_cohort_revalidates_exact_tables_and_rejects_wrong_source() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let scratch = Scratch(std::env::temp_dir().join(format!(
        "rillcache-handoff-evidence-{}-{}", std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
    )));
    fs::create_dir(&scratch.0).unwrap();
    let input = root.join("benchmarks/studies/client-handoffs/reference-0157177");
    let valid = scratch.0.join("valid");
    generate(&input, &valid, "01571773e4aaca4c1765a226b1f7f1187e1fd33e");
    assert_eq!(
        fs::read_to_string(valid.join("complete.txt")).unwrap(),
        "true\n"
    );
    let published = fs::read_to_string(root.join("benchmarks/reports/40245955560-1.md")).unwrap();
    let measurements = &published[published.find("## Measurements\n").unwrap()..];
    assert_eq!(
        fs::read_to_string(valid.join("measurements.md")).unwrap(),
        measurements
    );
    let invalid = scratch.0.join("wrong-source");
    generate(&input, &invalid, &"0".repeat(40));
    assert_eq!(
        fs::read_to_string(invalid.join("complete.txt")).unwrap(),
        "false\n"
    );
    assert!(
        !fs::read_to_string(invalid.join("measurements.md"))
            .unwrap()
            .contains("### SET at a glance")
    );
}
