#![forbid(unsafe_code)]

/// Reviewed exceptions: rustls, Mio (used internally by Tokio), Tokio and
/// core_affinity with their required transitive crates, plus the in-repo
/// `rillcache-io` crate that owns the only unsafe code (io_uring send batching)
/// behind a safe API. Any other package fails.
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
    "rillcache-io",
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

/// The unsafe boundary is exactly one in-repo crate with a safe API.
#[test]
fn unsafe_code_lives_only_in_the_reviewed_io_crate() {
    fn scan(root: &std::path::Path, findings: &mut Vec<String>) {
        for entry in std::fs::read_dir(root).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if path.is_dir() {
                if !["target", ".git", "vendor"].contains(&name.as_str()) {
                    scan(&path, findings);
                }
                continue;
            }
            if name == "policy.rs" {
                continue;
            }
            if path.extension().is_none_or(|extension| extension != "rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            let forbids = text
                .lines()
                .any(|line| line.trim().starts_with("#![forbid(unsafe_code)]"));
            let uses_unsafe = text.lines().any(|line| {
                let code = line.trim();
                !code.starts_with("//")
                    && [
                        "unsafe {",
                        "unsafe fn",
                        "unsafe impl",
                        "unsafe trait",
                        "unsafe extern",
                    ]
                    .iter()
                    .any(|pattern| code.contains(pattern))
            });
            let inside_io_crate = path
                .components()
                .any(|part| part.as_os_str() == "rillcache-io");
            if inside_io_crate {
                assert!(
                    !forbids,
                    "the unsafe crate must not forbid the code it exists to isolate: {}",
                    path.display()
                );
            } else if uses_unsafe {
                findings.push(path.display().to_string());
            }
        }
    }
    let mut findings = Vec::new();
    scan(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")),
        &mut findings,
    );
    assert!(
        findings.is_empty(),
        "unsafe outside the reviewed crate ({} files): {:?}",
        findings.len(),
        &findings[..findings.len().min(5)]
    );
}
