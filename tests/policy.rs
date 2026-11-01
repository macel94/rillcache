#![forbid(unsafe_code)]

/// Reviewed exceptions: rustls, Mio (used internally by Tokio), Tokio and
/// core_affinity, plus explicitly authorized Linux Compio, with their required
/// transitive crates. Any other package still fails.
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

// Exact additional names in Compio's locked all-target graph. Linux runtime,
// net, io-uring and time only; cfg/target-specific packages must remain available
// to Cargo's offline graph even when they are not compiled on this host.
const COMPIO_TRANSITIVES: &[&str] = &[
    "arrayvec",
    "bitflags",
    "cfg_aliases",
    "compio",
    "compio-buf",
    "compio-driver",
    "compio-executor",
    "compio-io",
    "compio-log",
    "compio-net",
    "compio-runtime",
    "compio-send-wrapper",
    "concurrent-queue",
    "crossbeam-queue",
    "crossbeam-utils",
    "either",
    "errno",
    "event-listener",
    "flume",
    "futures-core",
    "futures-io",
    "futures-macro",
    "futures-sink",
    "futures-task",
    "futures-util",
    "generator",
    "io-uring",
    "itoa",
    "lazy_static",
    "linux-raw-sys",
    "local-event",
    "lock_api",
    "log",
    "loom",
    "matchers",
    "memchr",
    "mod_use",
    "nu-ansi-term",
    "pastey",
    "pin-utils",
    "polling",
    "proc-macro2",
    "quote",
    "regex-automata",
    "regex-syntax",
    "rustix",
    "rustversion",
    "scoped-tls",
    "scopeguard",
    "serde",
    "serde_core",
    "serde_derive",
    "serde_json",
    "sharded-slab",
    "slab",
    "slotmap",
    "smallvec",
    "spin",
    "syn",
    "synchrony",
    "thin-cell",
    "thread_local",
    "tracing",
    "tracing-core",
    "tracing-log",
    "tracing-subscriber",
    "unicode-ident",
    "version_check",
    "widestring",
    "windows-result",
    "zmij",
];

#[test]
fn completion_dependency_keeps_its_minimal_reviewed_linux_features() {
    let manifest = include_str!("../Cargo.toml");
    assert!(manifest.contains("[target.'cfg(target_os = \"linux\")'.dependencies]"));
    assert!(manifest.contains("compio = { version = \"=0.19.2\", default-features = false, features = [\"runtime\", \"net\", \"io-uring\", \"time\"] }"));
    assert!(manifest.contains("unsafe_code = \"forbid\""));
    assert!(manifest.contains("build = false"));
}

#[test]
fn only_reviewed_tls_runtime_affinity_completion_and_transitive_crates_are_allowed() {
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
            ALLOWED.contains(&name) || COMPIO_TRANSITIVES.contains(&name),
            "external dependency outside the reviewed runtime/TLS/affinity/completion exceptions: {line}\n{tree}"
        );
    }
}
