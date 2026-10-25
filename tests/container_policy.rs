#![forbid(unsafe_code)]

fn file(path: &str) -> String {
    std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path)).unwrap()
}

#[test]
fn thin_image_keeps_runtime_and_build_context_boundaries_explicit() {
    let dockerfile = file("Dockerfile");
    let toolchain = file("rust-toolchain.toml");
    let channel = toolchain
        .lines()
        .find_map(|line| {
            line.strip_prefix("channel = \"")
                .and_then(|line| line.strip_suffix('"'))
        })
        .unwrap();
    assert!(dockerfile.starts_with("# The builder"));
    assert!(dockerfile.contains(&format!("FROM rust:{channel}-alpine3.23@sha256:")));
    let digest = dockerfile
        .split_once("@sha256:")
        .unwrap()
        .1
        .split_whitespace()
        .next()
        .unwrap();
    assert_eq!(digest.len(), 64);
    assert!(digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
    let runtime = dockerfile.split_once("\nFROM scratch\n").unwrap().1;
    assert!(!runtime.lines().any(|line| line.starts_with("RUN ")));
    assert_eq!(
        runtime
            .lines()
            .filter(|line| line.starts_with("COPY "))
            .count(),
        2
    );
    assert!(runtime.contains("USER 65532:65532"));
    assert!(runtime.contains("ENTRYPOINT [\"/rillcache\", \"--bind\", \"0.0.0.0:6379\"]"));
    assert!(runtime.contains("CMD [\"/rillcache\", \"--healthcheck\", \"127.0.0.1:6379\"]"));
    let ignore = file(".dockerignore");
    let rules: Vec<_> = ignore
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();
    assert_eq!(rules.first(), Some(&"**"));
    for disallowed in ["!.git", "!.env", "!target", "!.", "!**"] {
        assert!(
            !rules.iter().any(|rule| rule.starts_with(disallowed)),
            "unsafe build-context allowlist: {disallowed}"
        );
    }
    assert!(file("container/notices/Rust-MIT.txt").contains("Permission is hereby granted"));
    assert!(file("container/notices/musl-COPYRIGHT.txt").contains("Permission is hereby granted"));
}

#[test]
fn publishing_is_separate_from_unprivileged_builds_and_anonymous_checks() {
    let workflow = file(".github/workflows/container.yml");
    assert_eq!(workflow.matches("packages: write").count(), 1);
    assert!(!workflow.contains("contents: write"));
    assert!(!workflow.contains("pull_request_target"));
    assert!(!workflow.contains("secrets."));
    let (builds, rest) = workflow.split_once("\n  publish:\n").unwrap();
    let (publish, public) = rest.split_once("\n  public-pull:\n").unwrap();
    assert!(!builds.contains("docker/login-action"));
    assert!(builds.contains("runner: ubuntu-24.04-arm"));
    assert!(builds.contains("run: make ci"));
    assert!(publish.contains("needs: build"));
    assert!(publish.contains("github.event_name != 'pull_request'"));
    assert!(publish.contains("docker image load --input"));
    assert!(publish.contains("for arch in amd64 arm64"));
    assert!(publish.contains("\"$head\" == \"$GITHUB_SHA\""));
    assert!(public.contains("DOCKER_CONFIG=\"$config\" docker image pull"));
    assert!(!public.contains("docker/login-action"));
    assert!(!public.contains("packages: write"));
    for line in workflow
        .lines()
        .filter_map(|line| line.trim().strip_prefix("- uses: "))
    {
        let action = line.split_whitespace().next().unwrap();
        let (_, sha) = action.split_once('@').unwrap();
        assert_eq!(sha.len(), 40, "action must be commit-pinned: {action}");
        assert!(sha.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }
    let kube = file("deploy/kubernetes.yaml");
    for constraint in [
        "replicas: 1",
        "type: Recreate",
        "runAsNonRoot: true",
        "readOnlyRootFilesystem: true",
        "allowPrivilegeEscalation: false",
        "automountServiceAccountToken: false",
        "type: ClusterIP",
        "kind: NetworkPolicy",
    ] {
        assert!(
            kube.contains(constraint),
            "missing deployment boundary: {constraint}"
        );
    }
    assert!(!kube.contains("LoadBalancer"));
    assert!(!kube.contains("PersistentVolumeClaim"));
}
