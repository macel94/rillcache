#![forbid(unsafe_code)]

use std::path::Path;

fn check(path: &Path) {
    if path.is_dir() {
        for entry in std::fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_symlink() {
                continue;
            }
            check(&entry.path());
        }
    } else if matches!(
        path.extension().and_then(|value| value.to_str()),
        Some("md" | "rs")
    ) {
        let text = std::fs::read_to_string(path).unwrap();
        for (line, text) in text.lines().enumerate() {
            assert!(
                !text.starts_with("<<<<<<< ") && !text.starts_with(">>>>>>> "),
                "unresolved merge marker at {}:{}",
                path.display(),
                line + 1
            );
        }
    }
}

#[test]
fn source_and_documentation_have_no_committed_conflict_markers() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for path in ["README.md", "docs", "src", "tests"] {
        check(&root.join(path));
    }
}
