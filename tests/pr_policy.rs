#![forbid(unsafe_code)]

#[test]
fn every_main_pr_requires_distinct_tests_and_real_valkey_measurements() {
    let policy = include_str!("../.github/main-ruleset.json");
    for required in [
        "\"enforcement\": \"active\"",
        "\"bypass_actors\": []",
        "\"refs/heads/main\"",
        "\"type\": \"pull_request\"",
        "\"type\": \"non_fast_forward\"",
        "\"strict_required_status_checks_policy\": true",
        "\"require_code_owner_review\": true",
        "\"context\": \"Tests and offline CI\"",
        "\"context\": \"Valkey performance gate\"",
        "\"context\": \"Valkey 4-CPU performance gate\"",
    ] {
        assert!(policy.contains(required), "missing main rule: {required}");
    }
    let ci = include_str!("../.github/workflows/ci.yml");
    assert!(ci.contains("name: Tests and offline CI"));
    assert!(ci.contains("run: make ci"));
    let gate = include_str!("../.github/workflows/pr-quality.yml");
    for required in [
        "pull_request:",
        "branches: [main]",
        "name: Valkey performance gate",
        "base/scripts/pr-performance-gate.py",
        "python3 \"$HARNESS\" --check evidence/gate",
        "persist-credentials: false",
        "if: always()",
    ] {
        assert!(gate.contains(required), "missing gate boundary: {required}");
    }
    let multicpu = include_str!("../.github/workflows/pr-multicpu-quality.yml");
    for required in [
        "pull_request:",
        "name: Valkey 4-CPU performance gate",
        "base/scripts/pr-multicpu-performance.py",
        "valkey-benchmark",
        "--generator",
        "python3 \"$HARNESS\" --check evidence/gate",
        "persist-credentials: false",
        "if: always()",
    ] {
        assert!(
            multicpu.contains(required),
            "missing four-CPU gate boundary: {required}"
        );
    }
    let wrapper = include_str!("../scripts/pr-multicpu-performance.py");
    assert!(wrapper.contains("at least four distinct allowed logical CPUs required"));
    assert!(wrapper.contains("server_cpus, client_cpus = placement(cpus)"));
    for forbidden in [
        "pull_request_target:",
        "continue-on-error:",
        "paths:",
        "paths-ignore:",
        "contents: write",
        "secrets.",
    ] {
        assert!(
            !multicpu.contains(forbidden),
            "unsafe four-CPU gate exemption: {forbidden}"
        );
    }
    for forbidden in [
        "pull_request_target:",
        "continue-on-error:",
        "paths:",
        "paths-ignore:",
        "contents: write",
        "secrets.",
    ] {
        assert!(
            !gate.contains(forbidden),
            "unsafe gate exemption: {forbidden}"
        );
    }
}

#[test]
fn hosted_pr_updater_never_runs_its_writer_on_untrusted_pr_code() {
    let workflow = include_str!("../.github/workflows/pr-branch-updater.yml");
    for required in [
        "workflow_run:",
        "workflow_dispatch:",
        "schedule:",
        "if: github.event_name == 'pull_request'",
        "if: github.event_name != 'pull_request' && github.ref == 'refs/heads/main'",
        "ref: main",
        "persist-credentials: false",
        "queue: max",
        "cancel-in-progress: false",
        "contents: write",
        "pull-requests: write",
        "actions: write",
        "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1",
        "actions/upload-artifact@cf430e030ddbb5b0abf93d22962f4752f3646cd9",
    ] {
        assert!(
            workflow.contains(required),
            "missing updater boundary: {required}"
        );
    }
    for forbidden in ["pull_request_target:", "secrets.", "git push", "--force"] {
        assert!(
            !workflow.contains(forbidden),
            "unsafe updater workflow content: {forbidden}"
        );
    }
    let helper = include_str!("../scripts/update-pr-branches.py");
    for required in [
        "expected_head_sha",
        "ci.yml",
        "pr-quality.yml",
        "pr-multicpu-quality.yml",
        "workflow_dispatch",
    ] {
        assert!(
            helper.contains(required),
            "missing updater behavior: {required}"
        );
    }
    assert!(!helper.contains("gh pr merge"));
}

#[test]
fn generated_reports_use_prs_without_an_exception_for_main_or_policy_files() {
    let script = include_str!("../scripts/publish-report-pr.sh");
    assert!(script.contains("git switch --create \"$branch\""));
    assert!(script.contains("git push origin \"HEAD:refs/heads/$branch\""));
    assert!(script.contains("gh pr create"));
    assert!(script.contains("gh workflow run ci.yml"));
    assert!(script.contains("gh workflow run pr-quality.yml"));
    assert!(script.contains("gh workflow run pr-multicpu-quality.yml"));
    assert!(script.contains("gh workflow run upstream-compatibility.yml"));
    assert!(script.contains("--auto --squash --match-head-commit"));
    assert!(script.contains("if [[ \"$created\" == 1 ]]"));
    let delivery = include_str!("../scripts/report-pr-delivery.py");
    assert!(delivery.contains("refusing a retry"));
    assert!(delivery.contains("auto-merge was not actually enabled"));
    assert!(delivery.contains("preserving existing merge intent"));
    assert!(!script.contains("HEAD:$DEFAULT_BRANCH"));
    assert!(!script.contains("--force"));
    let owners = include_str!("../.github/CODEOWNERS");
    for path in [
        "/.github/CODEOWNERS @macel94",
        "/.github/workflows/ @macel94",
        "/scripts/pr-performance-gate.py @macel94",
        "/scripts/compare-builds.py @macel94",
        "/benchmarks/targets.mk @macel94",
    ] {
        assert!(owners.contains(path));
    }
}
