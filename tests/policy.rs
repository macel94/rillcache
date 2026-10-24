#![forbid(unsafe_code)]

#[test]
fn no_external_dependencies_in_any_target_or_feature() {
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
    assert_eq!(
        packages.len(),
        1,
        "external dependencies are prohibited:\n{tree}"
    );
    assert!(packages[0].starts_with("rillcache v"));
}
