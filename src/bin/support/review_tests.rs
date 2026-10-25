use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

const SHA: &str = "10d56eca165f14a2b5fb043e36d2c33cc52ed762";
// Shape verified against the pinned CLI's real pre-inference model rejection.
const ZERO_USAGE: &str = r#"{"totalPremiumRequestCost":0,"totalUserRequests":0,"totalNanoAiu":0,"totalApiDurationMs":0,"lastCallInputTokens":0,"lastCallOutputTokens":0,"modelMetrics":{},"agentMetrics":{},"providerModelMetrics":[]}"#;
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

fn assessment(text: impl Into<String>) -> Assessment {
    Assessment {
        text: text.into(),
        models: vec![MODEL.into()],
        selection: "preferred".into(),
        authentication: "workflow-token".into(),
    }
}

fn usage_json(model: &str) -> String {
    format!(
        "{{\"aiCreditsStatus\":\"complete\",\"providerModelMetrics\":[{{\"provider\":{{\"id\":\"copilot\",\"kind\":\"copilot\"}},\"modelId\":{},\"metrics\":{{\"requests\":{{\"count\":1}},\"usage\":{{\"outputTokens\":20}}}}}}]}}",
        quoted(model)
    )
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
        actual_models: Vec::new(),
        model_selection: "not-run".into(),
        authentication: "not-run".into(),
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
    assert!(text.contains("GITHUB\\_TOKEN needs copilot-requests"));
    assert!(text.contains("48952 | 58672 | 57526"));
    assert_ne!(manifest(&output).unwrap().ai_status, "completed");
    assert!(text.contains("none recorded"));
    assert!(manifest(&output).unwrap().actual_models.is_empty());
}

#[test]
fn completed_review_is_separate_from_numbers_and_model_markup_is_inert() {
    let root = Temp::new();
    let output = prepared(&root, 1, 1);
    review_with(&output, Some("test-credential"), |_| Ok(assessment("## Tradeoff\n- Lower throughput; batch tails matter.\n<img src=x> ![tracking](https://invalid.example/)"))).unwrap();
    let text = fs::read_to_string(output.join("report.md")).unwrap();
    assert!(text.contains("> \\#\\# Tradeoff"));
    assert!(text.contains("&lt;img src=x&gt;"));
    assert!(!text.contains("![tracking]"));
    assert_eq!(manifest(&output).unwrap().ai_status, "completed");
    assert_eq!(manifest(&output).unwrap().actual_models, [MODEL]);
    assert!(text.contains("Authentication: workflow-token"));
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
            Ok(assessment(
                "A previous valid narrative that must disappear.",
            ))
        })
        .unwrap();
        review_with(&output, Some("test-credential"), |_| {
            response.map(assessment)
        })
        .unwrap();
        let text = fs::read_to_string(output.join("report.md")).unwrap();
        assert!(!text.contains("A previous valid narrative"));
        assert!(!text.contains("test-credential"));
        assert!(text.contains("No AI assessment was accepted"));
        assert!(manifest(&output).unwrap().actual_models.is_empty());
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
fn embedded_mode_tables_are_validated_separately_and_bad_modes_fail_quality() {
    let root = Temp::new();
    let evidence = root.0.join("input");
    copy_evidence(&evidence);
    let modes = evidence.join("thread-modes");
    fs::create_dir(&modes).unwrap();
    fs::copy(
        source().join("pipeline-1/environment.txt"),
        modes.join("environment.txt"),
    )
    .unwrap();
    let mut records = Vec::new();
    for run in 1..=3 {
        for workload in ["ping", "set", "get", "set-nx-existing", "set-xx-existing"] {
            records.push(format!("{{\"workload\":{},\"run\":{run},\"requests\":19,\"clients\":4,\"pipeline\":3,\"keyspace\":7,\"value_size\":0,\"warmup_per_client\":0,\"batches\":8,\"elapsed_seconds\":0.001,\"ops_per_second\":19000,\"batch_p50_us\":1,\"batch_p95_us\":2,\"batch_p99_us\":3,\"batch_max_us\":4,\"server_profile\":null}}", quoted(workload)));
        }
    }
    for workers in [1, 4] {
        fs::write(modes.join(format!("threads-{workers}.json")), format!("{{\"schema_version\":1,\"mode\":\"embedded\",\"server_configuration\":{{\"mode\":\"fixed-workers\",\"workers\":{workers}}},\"results\":[{}]}}", records.join(","))).unwrap();
    }
    let summary = data::summarize(&evidence, SHA);
    assert!(summary.complete, "{}", summary.markdown);
    assert_eq!(summary.records, 180);
    assert!(
        summary
            .markdown
            .contains("30/30 separate embedded records validated")
    );
    assert!(
        summary
            .markdown
            .contains("| set | 19000 | 19000–19000 | 19000 | 19000–19000 | 3.0 | 3.0 |")
    );
    let file = modes.join("threads-4.json");
    let valid = fs::read_to_string(&file).unwrap();
    for bad in [
        valid.replace("\"workers\":4", "\"workers\":1"),
        valid.replacen("\"clients\":4", "\"clients\":3", 1),
        valid.replacen("\"ops_per_second\":19000", "\"ops_per_second\":1", 1),
    ] {
        fs::write(&file, bad).unwrap();
        let summary = data::summarize(&evidence, SHA);
        assert!(!summary.complete);
        assert!(
            !summary
                .markdown
                .contains("### Fixed-worker mode measurements")
        );
        assert_eq!(summary.records, 180);
    }
    fs::write(&file, &valid).unwrap();
    let mode_environment = modes.join("environment.txt");
    let original_mode_environment = fs::read_to_string(&mode_environment).unwrap();
    fs::write(
        &mode_environment,
        format!("{original_mode_environment}\nThread-mode polling policy: required\n"),
    )
    .unwrap();
    assert!(
        !data::summarize(&evidence, SHA).complete,
        "new evidence must declare its policy"
    );
    for workers in [1, 4] {
        let file = modes.join(format!("threads-{workers}.json"));
        let text = fs::read_to_string(&file).unwrap().replace(
            &format!("\"workers\":{workers}"),
            &format!("\"workers\":{workers},\"active_poll_us\":50"),
        );
        fs::write(file, text).unwrap();
    }
    let active = data::summarize(&evidence, SHA);
    assert!(active.complete, "{}", active.markdown);
    assert!(active.markdown.contains("at most 50 µs after progress"));
    let active_file = modes.join("threads-4.json");
    let active_text = fs::read_to_string(&active_file).unwrap();
    for value in ["0", "51", "-1", "null"] {
        fs::write(
            &active_file,
            active_text.replace(
                "\"active_poll_us\":50",
                &format!("\"active_poll_us\":{value}"),
            ),
        )
        .unwrap();
        assert!(!data::summarize(&evidence, SHA).complete);
    }
    fs::write(&active_file, &active_text).unwrap();
    let env = fs::read_to_string(&mode_environment).unwrap();
    fs::write(
        &mode_environment,
        format!("{env}\nThread-mode run budget: cooperative-v1 (required)\n"),
    )
    .unwrap();
    assert!(!data::summarize(&evidence, SHA).complete);
    for workers in [1, 4] {
        let file = modes.join(format!("threads-{workers}.json"));
        let text = fs::read_to_string(&file).unwrap().replace(
            "\"active_poll_us\":50",
            &format!(
                "\"active_poll_us\":50,\"run_quantum_us\":{}",
                if workers == 1 { 0 } else { 100 }
            ),
        );
        fs::write(file, text).unwrap();
    }
    let bounded = data::summarize(&evidence, SHA);
    assert!(bounded.complete, "{}", bounded.markdown);
    assert!(
        bounded
            .markdown
            .contains("100 µs of continuous active passes")
    );
    let current = fs::read_to_string(&active_file).unwrap();
    for value in ["0", "101", "-1", "null"] {
        fs::write(
            &active_file,
            current.replace(
                "\"run_quantum_us\":100",
                &format!("\"run_quantum_us\":{value}"),
            ),
        )
        .unwrap();
        assert!(!data::summarize(&evidence, SHA).complete);
    }
    fs::write(&active_file, &current).unwrap();
    let known = fs::read_to_string(&mode_environment).unwrap();
    fs::write(
        &mode_environment,
        known.replace("cooperative-v1", "cooperative-v2"),
    )
    .unwrap();
    assert!(
        !data::summarize(&evidence, SHA).complete,
        "v2 must record its version"
    );
    for workers in [1, 4] {
        let file = modes.join(format!("threads-{workers}.json"));
        let text = fs::read_to_string(&file).unwrap().replace(
            "\"active_poll_us\":50",
            &format!(
                "\"active_poll_us\":50,\"run_policy_version\":{}",
                if workers == 1 { 0 } else { 2 }
            ),
        );
        fs::write(file, text).unwrap();
    }
    let v2 = data::summarize(&evidence, SHA);
    assert!(v2.complete, "{}", v2.markdown);
    assert!(v2.markdown.contains("after a yield/park returns"));
    let v2_text = fs::read_to_string(&active_file).unwrap();
    for version in ["0", "1", "3", "null"] {
        fs::write(
            &active_file,
            v2_text.replace(
                "\"run_policy_version\":2",
                &format!("\"run_policy_version\":{version}"),
            ),
        )
        .unwrap();
        assert!(!data::summarize(&evidence, SHA).complete);
    }
    fs::write(
        &mode_environment,
        known.replace("cooperative-v1", "unknown-v2"),
    )
    .unwrap();
    assert!(!data::summarize(&evidence, SHA).complete);
    fs::remove_dir_all(&modes).unwrap();
    let environment = evidence.join("pipeline-1/environment.txt");
    let original = fs::read_to_string(&environment).unwrap();
    fs::write(
        environment,
        format!("Thread-mode measurements: required\n{original}"),
    )
    .unwrap();
    let missing = data::summarize(&evidence, SHA);
    assert!(!missing.complete);
    assert!(
        missing
            .markdown
            .contains("required thread-mode evidence is missing")
    );
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
fn model_command_is_exact_readonly_bounded_with_explicit_fallback() {
    let args = copilot_args(MODEL, "Only numeric facts", Path::new("usage.json"));
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
    let fallback = copilot_args("auto", "Only numeric facts", Path::new("usage.json"));
    assert_eq!(&fallback[..2], ["--model", "auto"]);
    assert!(fallback.iter().any(|s| s == "--available-tools="));
}

#[cfg(target_os = "linux")]
#[test]
fn bounded_process_wrapper_uses_an_empty_home_and_exact_model_without_network() {
    use std::os::unix::fs::PermissionsExt;
    let root = Temp::new();
    let output = prepared(&root, 1, 1);
    let executable = root.0.join("fake-copilot");
    fs::write(&executable, format!("#!/bin/sh\nset -eu\n[ \"${{GITHUB_TOKEN:-}}${{COPILOT_GITHUB_TOKEN:-}}\" = test-credential ]\n[ -z \"${{GH_TOKEN:-}}\" ]\n[ -z \"${{COPILOT_PROVIDER_API_KEY:-}}\" ]\n[ \"$1\" = --model ]\n[ \"$2\" = gpt-6-luna ]\n[ ! -e AGENTS.md ]\nwhile [ \"$#\" -gt 0 ]; do if [ \"$1\" = --usage-output-file ]; then shift; usage=$1; fi; shift; done\nprintf '%s' '{}' > \"$usage\"\nprintf '%s\\n' 'A bounded fake assessment: compare equal CPU budgets and batch p99.'\n", usage_json(MODEL))).unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    for (token_env, method) in [
        ("GITHUB_TOKEN", "workflow-token"),
        ("COPILOT_GITHUB_TOKEN", "personal-token"),
    ] {
        let response = call_copilot(&output, &executable, "test-credential", token_env).unwrap();
        assert!(response.text.contains("bounded fake assessment"));
        assert_eq!(response.models, [MODEL]);
        assert_eq!(response.authentication, method);
    }
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
    assert!(call_copilot(&output, &executable, "test-credential", "GITHUB_TOKEN").is_err());
    fs::write(
        &executable,
        "#!/bin/sh\nprintf 'A plausible assessment without verified model usage.'\n",
    )
    .unwrap();
    assert!(call_copilot(&output, &executable, "test-credential", "GITHUB_TOKEN").is_err());
}

#[test]
fn model_usage_requires_real_tokens_concrete_models_and_copilot_provider() {
    let usage = usage_json(MODEL);
    assert_eq!(used_models(&usage).unwrap(), [MODEL]);
    for invalid in [
        usage.replace("\"complete\"", "\"partial\""),
        usage.replace("\"outputTokens\":20", "\"outputTokens\":0"),
        usage.replace("\"count\":1", "\"count\":0"),
        usage.replace("\"kind\":\"copilot\"", "\"kind\":\"byok\""),
        usage.replace(MODEL, "auto"),
        usage.replace(MODEL, "--bad-option"),
        usage.replace(MODEL, "ghs_accidentally_leaked"),
        "{}".into(),
    ] {
        assert!(used_models(&invalid).is_err(), "{invalid}");
    }
}

#[test]
fn fallback_requires_zero_or_absent_inference_not_incomplete_or_charged_usage() {
    assert!(zero_inference_usage(ZERO_USAGE).unwrap());
    for invalid in [
        ZERO_USAGE.replace("\"totalNanoAiu\":0", "\"totalNanoAiu\":1"),
        ZERO_USAGE.replace("\"totalUserRequests\":0", "\"totalUserRequests\":1"),
        ZERO_USAGE.replace(
            "\"modelMetrics\":{}",
            "\"modelMetrics\":{\"some-model\":{}}",
        ),
        ZERO_USAGE.replace(
            "\"totalNanoAiu\":0",
            "\"aiCreditsStatus\":\"partial\",\"totalNanoAiu\":0",
        ),
        "{}".into(),
    ] {
        assert!(!zero_inference_usage(&invalid).unwrap_or(false));
    }
}

#[test]
fn require_ai_gate_cannot_pass_missing_or_failed_assessments() {
    let root = Temp::new();
    let output = prepared(&root, 1, 1);
    let args = BTreeMap::from([("--out".into(), output.to_str().unwrap().into())]);
    assert!(execute("require-ai", &args).is_err());
    review_with(&output, Some("test-credential"), |_| {
        Ok(assessment("A validated offline assessment fixture."))
    })
    .unwrap();
    execute("require-ai", &args).unwrap();
    review_with(&output, Some("test-credential"), |_| {
        Err("provider failure".into())
    })
    .unwrap();
    assert!(execute("require-ai", &args).is_err());
    assert!(
        fs::read_to_string(output.join("report.md"))
            .unwrap()
            .contains("48952 | 58672 | 57526")
    );
}

#[test]
fn legacy_manifests_remain_byte_identical_on_publication_retries() {
    let root = Temp::new();
    let output = prepared(&root, 1, 1);
    let legacy = metadata(1, 1).json().replace(
        ",\"actual_models\":[],\"model_selection\":\"not-run\",\"authentication\":\"not-run\"",
        "",
    );
    assert!(!legacy.contains("actual_models"));
    fs::write(output.join("manifest.json"), &legacy).unwrap();
    let destination = root.0.join("benchmarks");
    publish(&output, &destination).unwrap();
    publish(&output, &destination).unwrap();
    assert_eq!(
        fs::read_to_string(destination.join("reports/1001-1.json")).unwrap(),
        legacy
    );
}

#[cfg(target_os = "linux")]
#[test]
fn unavailable_preferred_model_uses_auto_but_auth_or_charged_failures_never_retry() {
    use std::os::unix::fs::PermissionsExt;
    let root = Temp::new();
    let output = prepared(&root, 1, 1);
    let executable = root.0.join("fake-copilot");
    let script = format!(
        "#!/bin/sh\nset -eu\nmodel=$2\nprintf '%s\\n' \"$model\" >> attempts.txt\nwhile [ \"$#\" -gt 0 ]; do if [ \"$1\" = --usage-output-file ]; then shift; usage=$1; fi; shift; done\nif [ \"$model\" = gpt-6-luna ]; then printf '%s' 'Error: Model \"gpt-6-luna\" from --model flag is not available.' >&2; exit 1; fi\nprintf '%s' '{}' > \"$usage\"\nprintf '%s' 'A fake fallback assessment with explicit model provenance.'\n",
        usage_json("available-test-model")
    );
    fs::write(&executable, &script).unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let response = call_copilot(&output, &executable, "test-credential", "GITHUB_TOKEN").unwrap();
    assert_eq!(response.models, ["available-test-model"]);
    assert_eq!(response.selection, "automatic-fallback");
    let attempts = output.join("copilot-private/work/attempts.txt");
    assert_eq!(fs::read_to_string(&attempts).unwrap(), "gpt-6-luna\nauto\n");
    let zero_script = script.replace(
        "then printf '%s' 'Error:",
        &format!(
            "then printf '%s' '{}' > \"$usage\"; printf '%s' 'Error:",
            ZERO_USAGE
        ),
    );
    fs::write(&executable, zero_script).unwrap();
    fs::remove_file(&attempts).unwrap();
    let zero_response =
        call_copilot(&output, &executable, "test-credential", "GITHUB_TOKEN").unwrap();
    assert_eq!(zero_response.selection, "automatic-fallback");
    assert_eq!(fs::read_to_string(&attempts).unwrap(), "gpt-6-luna\nauto\n");
    for failed in [
        script.replace(
            "Error: Model \"gpt-6-luna\" from --model flag is not available.",
            "Authentication failed",
        ),
        script.replace(
            "then printf '%s' 'Error:",
            "then printf '{}' > \"$usage\"; printf '%s' 'Error:",
        ),
    ] {
        fs::write(&executable, failed).unwrap();
        fs::remove_file(&attempts).unwrap();
        assert!(call_copilot(&output, &executable, "test-credential", "GITHUB_TOKEN").is_err());
        assert_eq!(fs::read_to_string(&attempts).unwrap(), "gpt-6-luna\n");
    }
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
