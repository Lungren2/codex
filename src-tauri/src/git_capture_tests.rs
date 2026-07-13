use super::*;
use pretty_assertions::assert_eq;
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

const PATCH: &str = "diff --git a/src/old.rs b/src/new.rs\nsimilarity index 80%\nrename from src/old.rs\nrename to src/new.rs\n--- a/src/old.rs\n+++ b/src/new.rs\n@@ -1,2 +1,2 @@\n-old\n+new\n context\n";

#[test]
fn parses_rename_and_assigns_stable_hunk_id() {
    let first = parse_patch(PATCH.as_bytes(), DEFAULT_FILE_CAP, DEFAULT_PATCH_CAP).unwrap();
    let second = parse_patch(PATCH.as_bytes(), DEFAULT_FILE_CAP, DEFAULT_PATCH_CAP).unwrap();
    assert_eq!(first, second);
    assert_eq!(first[0].kind, DiffFileKind::Rename);
    assert_eq!(first[0].old_path.as_deref(), Some("src/old.rs"));
    assert_eq!(first[0].path, "src/new.rs");
    assert_eq!(first[0].hunks[0].lines[0].old_line, Some(1));
    assert_eq!(first[0].hunks[0].lines[1].new_line, Some(1));
}

#[test]
fn marks_secret_and_risk_sensitive_hunks() {
    let patch = "diff --git a/auth/config.rs b/auth/config.rs\n--- a/auth/config.rs\n+++ b/auth/config.rs\n@@ -0,0 +1 @@\n+api_key = \"abcdefghijklmnop\"\n";
    let files = parse_patch(patch.as_bytes(), DEFAULT_FILE_CAP, DEFAULT_PATCH_CAP).unwrap();
    assert!(files[0].hunks[0].contains_secret);
    assert_eq!(files[0].hunks[0].category, HunkCategory::RiskSensitive);
}

#[test]
fn retains_oversized_file_metadata_without_hunks() {
    let files = parse_patch(PATCH.as_bytes(), 10, DEFAULT_PATCH_CAP).unwrap();
    assert!(files[0].omitted);
    assert!(files[0].hunks.is_empty());
}

#[test]
fn binary_content_is_metadata_only() {
    let patch = "diff --git a/image.png b/image.png\nnew file mode 100644\nBinary files /dev/null and b/image.png differ\n";
    let files = parse_patch(patch.as_bytes(), DEFAULT_FILE_CAP, DEFAULT_PATCH_CAP).unwrap();
    assert_eq!(files[0].kind, DiffFileKind::Binary);
    assert!(files[0].hunks.is_empty());
}

#[test]
fn captures_tracked_untracked_renamed_and_binary_worktree_changes() {
    let repository = TempDir::new().unwrap();
    run_git(repository.path(), &["init", "-b", "main"]);
    run_git(
        repository.path(),
        &["config", "user.email", "codex-review@example.test"],
    );
    run_git(
        repository.path(),
        &["config", "user.name", "Codex Review Test"],
    );
    fs::write(repository.path().join("tracked.txt"), "original\n").unwrap();
    run_git(repository.path(), &["add", "tracked.txt"]);
    run_git(repository.path(), &["commit", "-m", "base"]);

    fs::rename(
        repository.path().join("tracked.txt"),
        repository.path().join("renamed.txt"),
    )
    .unwrap();
    fs::write(repository.path().join("untracked.txt"), "new text\n").unwrap();
    fs::write(repository.path().join("image.bin"), [0, 1, 2, 3]).unwrap();

    let blobs = repository.path().join("blobs");
    let result = capture(
        repository.path(),
        "thread-1",
        "turn-1",
        1,
        &blobs,
        &CaptureOptions {
            base_ref_override: Some("HEAD".to_string()),
            ..CaptureOptions::default()
        },
    )
    .unwrap();

    assert!(
        result
            .artifact
            .files
            .iter()
            .any(|file| file.path == "renamed.txt")
    );
    assert!(
        result
            .artifact
            .files
            .iter()
            .any(|file| file.path == "untracked.txt")
    );
    assert!(result.artifact.files.iter().any(|file| {
        file.path == "image.bin" && file.kind == DiffFileKind::Binary && file.hunks.is_empty()
    }));
    assert!(Path::new(&result.artifact.patch_set.blob_path).exists());
}

fn run_git(root: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {} failed: {}",
        arguments.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}
