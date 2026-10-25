#![forbid(unsafe_code)]

#[test]
fn local_and_github_checks_include_real_worker_mode_matrices() {
    let make = include_str!("../Makefile");
    assert!(
        include_str!("../.github/workflows/benchmarks.yml").contains("REQUIRE_THREAD_MODES: '1'")
    );
    assert!(make.contains("Thread-mode measurements: required"));
    assert!(make.contains("for workers in 0 1 4; do"));
    assert!(make.contains("check: fmt-check lint test test-constrained"));
    assert!(make.contains("taskset -c \"$$cpu\" env RILLCACHE_TEST_THREADS=0"));
    assert!(make.contains("RILLCACHE_TEST_THREADS=\"$$workers\""));
    assert!(make.contains("smoke-threads-$$workers.json"));
    assert!(make.contains("bench: release metadata bench-core bench-threads"));
    assert!(include_str!("../.github/workflows/ci.yml").contains("run: make ci"));
    assert!(
        include_str!("../.github/workflows/benchmarks.yml").contains("run: make bench-threads")
    );
}

#[test]
fn report_workflow_keeps_inference_separate_from_repository_write_access() {
    let workflow = include_str!("../.github/workflows/benchmarks.yml");
    let (before_review, rest) = workflow.split_once("\n  review:\n").unwrap();
    let (review, publish) = rest.split_once("\n  publish:\n").unwrap();
    assert!(before_review.contains("permissions:\n  contents: read"));
    assert!(!before_review.contains("contents: write"));
    assert!(!before_review.contains("copilot-requests: write"));
    assert!(review.contains("permissions:\n      contents: read"));
    assert!(!review.contains("contents: write"));
    assert!(review.contains("copilot-requests: write"));
    assert!(!review.contains("secrets.COPILOT_GITHUB_TOKEN"));
    assert!(!review.contains("HAS_COPILOT_TOKEN"));
    assert!(review.contains("rillcache-review require-ai --out target/report"));
    assert!(!review.contains("persist-credentials: true"));
    assert!(review.contains("--copilot \"$RUNNER_TEMP/copilot/copilot\""));
    assert!(review.contains("steps.prepare.outputs.complete == 'true'"));
    assert!(review.contains("github.event.repository.default_branch) && github.token || ''"));
    assert!(review.contains("artifact-ids: ${{ needs.compare.outputs.artifact-id }}"));
    assert!(publish.contains("contents: write"));
    assert!(publish.contains(
        "github.ref == format('refs/heads/{0}', github.event.repository.default_branch)"
    ));
    assert!(publish.contains("git add -- benchmarks/README.md benchmarks/reports"));
    assert!(publish.contains("artifact-ids: ${{ needs.review.outputs.artifact-id }}"));
    assert!(!publish.contains("COPILOT_GITHUB_TOKEN"));
    assert!(!publish.contains("copilot-requests:"));
    assert!(!publish.contains(" review --"));
    assert!(!publish.contains("--force"));
    assert!(!workflow.contains("pull_request_target"));
}
