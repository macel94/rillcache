#![forbid(unsafe_code)]

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
