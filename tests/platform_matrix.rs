#![forbid(unsafe_code)]

//! L5: the supported platform matrix must be explicit, match what CI actually
//! builds and tests, and exclude everything else rather than imply portability.

use std::path::Path;

fn read(relative: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

#[test]
fn matrix_documents_the_tested_platform_and_excludes_everything_else() {
    let matrix = read("docs/platforms.md");
    let profile = read("docs/production-profile.md");
    assert!(matrix.contains("Linux `x86_64`, glibc"));
    assert!(
        matrix.contains("**Supported and tested**"),
        "the tested platform must be marked supported, not implied"
    );
    for excluded in [
        "macOS, Windows, BSD, Solaris",
        "**Unsupported**",
        "network and shared filesystems",
        "tmpfs is a test/dev medium only",
    ] {
        assert!(matrix.contains(excluded), "matrix must state {excluded}");
    }
    assert!(
        matrix.contains("excluded from the support claim"),
        "partially built targets must not read as supported"
    );
    // The profile points at the authoritative matrix.
    assert!(
        profile.contains("platforms.md"),
        "the production profile must link the platform matrix"
    );
}

#[test]
fn matrix_matches_the_container_architectures_ci_actually_builds() {
    let matrix = read("docs/platforms.md");
    let container = read(".github/workflows/container.yml");
    let built: Vec<String> = container
        .lines()
        .filter_map(|line| line.trim().strip_prefix("- arch: "))
        .map(str::to_owned)
        .collect();
    assert!(
        !built.is_empty(),
        "container workflow must declare architectures"
    );
    for arch in &built {
        let documented = match arch.as_str() {
            "amd64" => matrix.contains("`x86_64`"),
            "arm64" => matrix.contains("`aarch64`"),
            other => panic!("undocumented container architecture in CI: {other}"),
        };
        assert!(
            documented,
            "CI builds {arch} but the matrix does not mention it"
        );
    }
    // Every built architecture beyond x86_64 must be excluded from support.
    if built.iter().any(|arch| arch == "arm64") {
        assert!(
            matrix.contains("behaviourally untested"),
            "arm64 is built but must be excluded from the support claim"
        );
    }
}

#[test]
fn matrix_is_linked_from_the_documentation_index() {
    let index = read("docs/README.md");
    assert!(
        index.contains("platforms.md"),
        "docs/README.md must link the platform matrix (documentation index contract)"
    );
}
