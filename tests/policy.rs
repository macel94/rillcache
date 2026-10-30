#![forbid(unsafe_code)]

/// Reviewed exceptions: rustls, Mio (used internally by Tokio), Tokio and
/// core_affinity with their required transitive crates. Any other package fails.
const ALLOWED: &[&str] = &[
    "bytes",
    "cc",
    "cfg-if",
    "core_affinity",
    "find-msvc-tools",
    "getrandom",
    "hermit-abi",
    "libc",
    "mio",
    "num_cpus",
    "once_cell",
    "pin-project-lite",
    "rillcache",
    "ring",
    "rustls",
    "rustls-pemfile",
    "rustls-pki-types",
    "rustls-webpki",
    "shlex",
    "socket2",
    "subtle",
    "tokio",
    "untrusted",
    "winapi",
    "winapi-i686-pc-windows-gnu",
    "winapi-x86_64-pc-windows-gnu",
    "wasi",
    "windows-link",
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
fn only_reviewed_tls_runtime_affinity_and_transitive_crates_are_allowed() {
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
            "external dependency outside the reviewed runtime/TLS/affinity exceptions: {line}\n{tree}"
        );
    }
}
