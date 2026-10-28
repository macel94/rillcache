#![forbid(unsafe_code)]

/// The sole external dependency exception is rustls and the transitive crates
/// required to build it. Any other package in any target/feature graph fails.
const ALLOWED: &[&str] = &[
    "cc",
    "cfg-if",
    "find-msvc-tools",
    "getrandom",
    "libc",
    "once_cell",
    "rillcache",
    "ring",
    "rustls",
    "rustls-pemfile",
    "rustls-pki-types",
    "rustls-webpki",
    "shlex",
    "subtle",
    "untrusted",
    "wasi",
    "windows-sys",
    "windows-targets",
    "windows_aarch64_gnullvm",
    "windows_aarch64_msvc",
    "windows_i686_gnu",
    "windows_i686_gnullvm",
    "windows_i686_msvc",
    "windows_x86_64_gnu",
    "windows_x86_64_gnullvm",
    "windows_x86_64_msvc",
    "zeroize",
];

#[test]
fn only_rustls_and_its_required_transitive_crates_are_allowed() {
    let output = std::process::Command::new(env!("CARGO"))
        .args([
            "tree",
            "--locked",
            "--offline",
            "--all-features",
            "--target",
            "all",
            "--edges",
            "all",
            "--prefix",
            "none",
        ])
        .output()
        .expect("Cargo must be available to enforce the dependency policy");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let tree = String::from_utf8(output.stdout).unwrap();
    let packages: Vec<_> = tree
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    assert!(
        packages.iter().any(|line| line.starts_with("rillcache v")),
        "missing project package:\n{tree}"
    );
    for line in packages {
        let name = line.split_whitespace().next().unwrap_or_default();
        assert!(
            ALLOWED.contains(&name),
            "external dependency outside the rustls exception: {line}\n{tree}"
        );
    }
}
