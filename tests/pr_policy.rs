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
fn generated_reports_use_prs_without_an_exception_for_main_or_policy_files() {
    let script = include_str!("../scripts/publish-report-pr.sh");
    assert!(script.contains("git switch --create \"$branch\""));
    assert!(script.contains("git push origin \"HEAD:refs/heads/$branch\""));
    assert!(script.contains("gh pr create"));
    assert!(script.contains("gh workflow run ci.yml"));
    assert!(script.contains("gh workflow run pr-quality.yml"));
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
