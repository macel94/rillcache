use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

const SHA: &str = "10d56eca165f14a2b5fb043e36d2c33cc52ed762";
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "rillcache-review-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn source() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("benchmarks/ci/10d56ec")
}
fn metadata(number: u64, attempt: u64) -> Manifest {
    Manifest {
        repo: "macel94/rillcache".into(),
        sha: SHA.into(),
        run_id: number + 1000,
        run_number: number,
        attempt,
        benchmark_attempt: 1,
        date: "2026-10-24T17:09:29Z".into(),
        status: "success".into(),
        complete: true,
        ai_status: "pending".into(),
    }
}
fn prepared(root: &Temp, number: u64, attempt: u64) -> PathBuf {
    let directory = root.0.join(format!("output-{number}-{attempt}"));
    generate(&source(), &directory, metadata(number, attempt)).unwrap();
    directory
}

#[test]
fn missing_credentials_preserve_verified_numbers_and_make_no_model_call() {
    let root = Temp::new();
    let output = prepared(&root, 1, 1);
    review_with(&output, None, |_| {
        panic!("must not invoke Copilot without credentials")
    })
    .unwrap();
    let text = fs::read_to_string(output.join("report.md")).unwrap();
    assert!(text.contains("configure the COPILOT"));
    assert!(text.contains("48952 | 58672 | 57526"));
    assert_ne!(manifest(&output).unwrap().ai_status, "completed");
    assert!(text.contains("no other model is substituted"));
}

#[test]
fn completed_review_is_separate_from_numbers_and_model_markup_is_inert() {
    let root = Temp::new();
    let output = prepared(&root, 1, 1);
    review_with(&output, Some("test-credential"), |_| Ok("## Tradeoff\n- Lower throughput; batch tails matter.\n<img src=x> ![tracking](https://invalid.example/)".into())).unwrap();
    let text = fs::read_to_string(output.join("report.md")).unwrap();
    assert!(text.contains("> \\#\\# Tradeoff"));
    assert!(text.contains("&lt;img src=x&gt;"));
    assert!(!text.contains("![tracking]"));
    assert_eq!(manifest(&output).unwrap().ai_status, "completed");
    assert!(text.contains("48952 | 58672 | 57526"));
}

#[test]
fn failure_empty_oversized_and_secret_output_never_reuse_a_previous_review() {
    let root = Temp::new();
    let output = prepared(&root, 1, 1);
    for response in [
        Err("timeout".into()),
        Ok(String::new()),
        Ok("x".repeat(16001)),
        Ok("An accidental leak: test-credential is private".into()),
        Ok("A token-shaped accidental leak: gho_example".into()),
        Ok("A control character \u{1b} in a long enough answer".into()),
    ] {
        review_with(&output, Some("test-credential"), |_| {
            Ok("A previous valid narrative that must disappear.".into())
        })
        .unwrap();
        review_with(&output, Some("test-credential"), |_| response).unwrap();
        let text = fs::read_to_string(output.join("report.md")).unwrap();
        assert!(!text.contains("A previous valid narrative"));
        assert!(!text.contains("test-credential"));
        assert!(text.contains("No AI assessment was accepted"));
    }
}

#[test]
fn incomplete_or_failed_benchmarks_skip_inference_but_still_render() {
    let root = Temp::new();
    for status in ["success", "failure"] {
        let mut meta = metadata(1, 1);
        meta.status = status.into();
        let output = root.0.join(status);
        generate(&source().join("missing"), &output, meta).unwrap();
        review_with(&output, Some("test-credential"), |_| {
            panic!("invalid data must not be sent to model")
        })
        .unwrap();
        let text = fs::read_to_string(output.join("report.md")).unwrap();
        assert!(text.contains("INCOMPLETE / INVALID"));
        assert!(!text.contains("### SET at a glance"));
        assert_eq!(
            fs::read_to_string(output.join("complete.txt")).unwrap(),
            "false\n"
        );
    }
    let output = root.0.join("failed-complete");
    let mut meta = metadata(2, 1);
    meta.status = "failure".into();
    generate(&source(), &output, meta).unwrap();
    review_with(&output, Some("test-credential"), |_| {
        panic!("failed benchmark must not get an AI victory narrative")
    })
    .unwrap();
}

#[test]
fn publication_is_idempotent_and_cannot_clobber_a_newer_run() {
    let root = Temp::new();
    let destination = root.0.join("benchmarks");
    let newer = prepared(&root, 2, 1);
    let older = prepared(&root, 1, 2);
    publish(&newer, &destination).unwrap();
    let current = fs::read(destination.join("README.md")).unwrap();
    publish(&older, &destination).unwrap();
    publish(&newer, &destination).unwrap();
    assert_eq!(fs::read(destination.join("README.md")).unwrap(), current);
    assert!(destination.join("reports/1001-2.md").exists());
    assert!(destination.join("reports/1002-1.md").exists());
    let rerun = prepared(&root, 2, 2);
    publish(&rerun, &destination).unwrap();
    assert!(
        fs::read_to_string(destination.join("README.md"))
            .unwrap()
            .contains("review attempt: 2")
    );
}

#[test]
fn failed_latest_run_keeps_a_link_to_last_successful_numeric_report() {
    let root = Temp::new();
    let destination = root.0.join("benchmarks");
    publish(&prepared(&root, 1, 1), &destination).unwrap();
    let failed = root.0.join("failed");
    let mut meta = metadata(2, 1);
    meta.status = "failure".into();
    generate(&source().join("missing"), &failed, meta).unwrap();
    publish(&failed, &destination).unwrap();
    let current = fs::read_to_string(destination.join("README.md")).unwrap();
    assert!(current.starts_with("**Latest attempt is incomplete or failed."));
    assert!(current.contains("/1001-1.md"));
    assert_eq!(
        existing(&destination.join("reports/last-successful.json"))
            .unwrap()
            .unwrap()
            .run_number,
        1
    );
}

#[test]
fn existing_archive_and_invalid_publication_state_are_never_overwritten() {
    let root = Temp::new();
    let destination = root.0.join("benchmarks");
    let output = prepared(&root, 1, 1);
    publish(&output, &destination).unwrap();
    fs::write(output.join("report.md"), "changed archive").unwrap();
    assert!(publish(&output, &destination).is_err());
    let other = prepared(&root, 2, 1);
    fs::write(destination.join("reports/latest.json"), "corrupt").unwrap();
    assert!(publish(&other, &destination).is_err());
    assert!(!destination.join("reports/1002-1.md").exists());
}

fn copy_evidence(destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    fs::copy(
        source().join("reference-binaries.sha256"),
        destination.join("reference-binaries.sha256"),
    )
    .unwrap();
    for (name, _) in data::PROFILES {
        fs::create_dir_all(destination.join(name)).unwrap();
        for file in [
            "environment.txt",
            "rillcache.json",
            "redis.json",
            "valkey.json",
        ] {
            fs::copy(
                source().join(name).join(file),
                destination.join(name).join(file),
            )
            .unwrap();
        }
    }
}

#[test]
fn malformed_duplicate_mismatched_and_truncated_measurements_are_rejected() {
    let root = Temp::new();
    let evidence = root.0.join("input");
    copy_evidence(&evidence);
    let file = evidence.join("single-core-pipeline-1/rillcache.json");
    let original = fs::read_to_string(&file).unwrap();
    for replacement in [
        original.replacen("\"run\":2", "\"run\":1", 1),
        original.replacen("\"clients\":8", "\"clients\":7", 1),
        original.replacen("\"batch_p99_us\":318.805", "\"batch_p99_us\":0", 1),
        original.replacen("\"user_ticks\":11", "\"user_ticks\":99999", 1),
        original.replacen("\"ops_per_second\":50400.50", "\"ops_per_second\":1", 1),
        original.replacen("\"batches\":100000", "\"batches\":99999", 1),
        original.replacen("\"matched_threads\":9", "\"matched_threads\":0", 1),
        original.replacen(
            "\"workload\":\"ping\"",
            "\"workload\":\"ping\",\"workload\":\"get\"",
            1,
        ),
        original[..original.len() / 2].to_owned(),
    ] {
        assert_ne!(replacement, original);
        fs::write(&file, replacement).unwrap();
        let summary = data::summarize(&evidence, SHA);
        assert!(!summary.complete);
        assert_eq!(summary.records, 135);
        assert!(
            !summary
                .markdown
                .contains("| single-core-pipeline-1 | 48952")
        );
    }
}

#[test]
fn changed_binary_provenance_or_overlapping_cpu_budgets_are_rejected() {
    let root = Temp::new();
    let evidence = root.0.join("input");
    copy_evidence(&evidence);
    let environment = evidence.join("single-core-pipeline-1/environment.txt");
    let original = fs::read_to_string(&environment).unwrap();
    fs::write(
        &environment,
        original.replace(
            "61bd3f2f84977a6a9a1b8f6ccd9b6deeb88867289450a7be675ea7cb8ec3a582",
            &"0".repeat(64),
        ),
    )
    .unwrap();
    let summary = data::summarize(&evidence, SHA);
    assert!(!summary.complete);
    assert_eq!(summary.records, 0);
    fs::write(
        &environment,
        original.replace("Client CPU request: 1,2,3", "Client CPU request: 0,1"),
    )
    .unwrap();
    assert_eq!(data::summarize(&evidence, SHA).records, 135);
}

#[test]
fn model_command_is_exact_readonly_bounded_and_not_auto_routed() {
    let args = copilot_args("Only numeric facts", Path::new("usage.json"));
    for pair in [
        ["--model", "gpt-6-luna"],
        ["--deny-tool", "shell"],
        ["--deny-tool", "write"],
        ["--deny-tool", "url"],
        ["--max-ai-credits", "30"],
    ] {
        assert!(args.windows(2).any(|w| w == pair));
    }
    for flag in [
        "--available-tools=",
        "--disallow-temp-dir",
        "--disable-builtin-mcps",
        "--no-custom-instructions",
        "--no-auto-update",
        "--no-ask-user",
    ] {
        assert!(args.iter().any(|s| s == flag));
    }
    for forbidden in ["auto", "--allow-all", "--allow-all-tools", "--allow-tool"] {
        assert!(!args.iter().any(|s| s == forbidden));
    }
}

#[cfg(target_os = "linux")]
#[test]
fn bounded_process_wrapper_uses_an_empty_home_and_exact_model_without_network() {
    use std::os::unix::fs::PermissionsExt;
    let root = Temp::new();
    let output = prepared(&root, 1, 1);
    let executable = root.0.join("fake-copilot");
    fs::write(&executable, "#!/bin/sh\nset -eu\n[ \"$COPILOT_GITHUB_TOKEN\" = test-credential ]\n[ -z \"${GITHUB_TOKEN:-}\" ]\n[ -z \"${GH_TOKEN:-}\" ]\n[ -z \"${COPILOT_PROVIDER_API_KEY:-}\" ]\n[ \"$1\" = --model ]\n[ \"$2\" = gpt-6-luna ]\n[ ! -e AGENTS.md ]\nprintf '%s\\n' 'A bounded fake assessment: compare equal CPU budgets and batch p99.'\n").unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let response = call_copilot(&output, &executable, "test-credential").unwrap();
    assert!(response.contains("bounded fake assessment"));
    assert_eq!(
        fs::metadata(output.join("copilot-private"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    fs::write(
        &executable,
        "#!/bin/sh\nprintf 'An error from the CLI' >&2\nexit 1\n",
    )
    .unwrap();
    assert!(call_copilot(&output, &executable, "test-credential").is_err());
}

#[cfg(unix)]
#[test]
fn symlinked_publication_outputs_are_rejected() {
    use std::os::unix::fs::symlink;
    let root = Temp::new();
    let output = prepared(&root, 1, 1);
    let destination = root.0.join("benchmarks");
    fs::create_dir_all(&destination).unwrap();
    let unrelated = root.0.join("source.rs");
    fs::write(&unrelated, "must not change").unwrap();
    symlink(&unrelated, destination.join("README.md")).unwrap();
    assert!(publish(&output, &destination).is_err());
    assert_eq!(fs::read_to_string(&unrelated).unwrap(), "must not change");
}

#[test]
fn manifests_and_cli_reject_path_injection_and_invalid_identity() {
    assert_eq!(
        Manifest::from_json(&json::parse(&metadata(1, 1).json()).unwrap())
            .unwrap()
            .sha,
        SHA
    );
    assert!(
        Manifest::from_json(
            &json::parse(&metadata(1, 1).json().replace(MODEL, "another-model")).unwrap()
        )
        .is_err()
    );
    for change in 0..6 {
        let mut meta = metadata(1, 1);
        match change {
            0 => meta.repo = "../../tmp".into(),
            1 => meta.sha = "\"\n".repeat(20),
            2 => meta.run_number = 0,
            3 => meta.attempt = 0,
            4 => meta.date = "<script>".into(),
            _ => meta.status = "unknown".into(),
        }
        assert!(meta.validate().is_err());
    }
    for args in [
        vec![],
        vec!["oops"],
        vec!["review"],
        vec!["review", "--out", "a", "--out", "b"],
        vec!["review", "--out", "a", "--copilot", "b", "--unsafe", "yes"],
    ] {
        assert!(arguments(args.into_iter().map(str::to_owned)).is_err());
    }
}
