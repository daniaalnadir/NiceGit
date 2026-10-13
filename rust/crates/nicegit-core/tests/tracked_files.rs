//! Integration tests for listing a checkout's tracked files, run against real repositories in
//! temporary directories.

use std::fs;
use std::path::Path;
use std::process::Command;

use nicegit_core::client::GitClient;
use tempfile::TempDir;

/// Runs Git in `directory` and returns its standard output, panicking on failure.
fn git(directory: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap_or_else(|error| panic!("could not run git {arguments:?}: {error}"));
    assert!(output.status.success(), "git {arguments:?} failed: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// A temporary repository on branch `main` with local identity settings.
fn repository() -> TempDir {
    let dir = tempfile::tempdir().expect("create temp dir");
    git(dir.path(), &["init", "-q", "-b", "main"]);
    for (key, value) in [("user.name", "Test"), ("user.email", "test@example.com"), ("commit.gpgsign", "false"), ("core.autocrlf", "false")]
    {
        git(dir.path(), &["config", key, value]);
    }
    dir
}

#[test]
fn tracked_files_lists_paths_with_spaces_and_newlines() {
    let dir = repository();
    let root = dir.path();
    fs::write(root.join("plain.rs"), "fn main() {}\n").unwrap();
    fs::create_dir(root.join("docs folder")).unwrap();
    fs::write(root.join("docs folder").join("read me.md"), "text\n").unwrap();
    git(root, &["add", "plain.rs", "docs folder"]);
    #[cfg(unix)]
    {
        // A newline in a filename is legal on Unix; Git lists it with C-style quoting only in
        // porcelain output, so `-z` must hand back the raw name.
        fs::write(root.join("two\nlines.txt"), "x\n").unwrap();
        git(root, &["add", "two\nlines.txt"]);
    }
    git(root, &["commit", "-q", "-m", "Add files"]);

    let client = GitClient::new();
    let mut files = client.tracked_files(root).expect("list tracked files");
    files.sort();

    #[cfg(unix)]
    let expected = vec!["docs folder/read me.md".to_string(), "plain.rs".to_string(), "two\nlines.txt".to_string()];
    #[cfg(not(unix))]
    let expected = vec!["docs folder/read me.md".to_string(), "plain.rs".to_string()];
    assert_eq!(files, expected);

    // Run from a subfolder, the paths are still relative to the repository root.
    let nested = client.tracked_files(&root.join("docs folder")).expect("list from a subfolder");
    assert_eq!(nested.len(), expected.len());
}

#[test]
fn tracked_files_excludes_untracked_and_ignored_files() {
    let dir = repository();
    let root = dir.path();
    fs::write(root.join("tracked.txt"), "kept\n").unwrap();
    fs::write(root.join(".gitignore"), "ignored.log\n").unwrap();
    git(root, &["add", "tracked.txt", ".gitignore"]);
    git(root, &["commit", "-q", "-m", "Track a file"]);
    fs::write(root.join("untracked.txt"), "new\n").unwrap();
    fs::write(root.join("ignored.log"), "noise\n").unwrap();

    let files = GitClient::new().tracked_files(root).expect("list tracked files");
    assert!(files.contains(&"tracked.txt".to_string()));
    assert!(files.contains(&".gitignore".to_string()));
    assert!(!files.iter().any(|path| path == "untracked.txt"), "untracked files are not listed: {files:?}");
    assert!(!files.iter().any(|path| path == "ignored.log"), "ignored files are not listed: {files:?}");
}

#[test]
fn tracked_files_lists_a_conflicted_path_once() {
    let dir = repository();
    let root = dir.path();
    fs::write(root.join("shared.txt"), "base\n").unwrap();
    git(root, &["add", "--", "shared.txt"]);
    git(root, &["commit", "-q", "-m", "Base"]);
    git(root, &["switch", "-q", "-c", "side"]);
    fs::write(root.join("shared.txt"), "side\n").unwrap();
    git(root, &["commit", "-q", "-am", "Side"]);
    git(root, &["switch", "-q", "main"]);
    fs::write(root.join("shared.txt"), "main\n").unwrap();
    git(root, &["commit", "-q", "-am", "Main"]);
    // The merge stops with a conflict, so Git keeps three stages of the file.
    let merged = Command::new("git").args(["merge", "-q", "side"]).current_dir(root).output().unwrap();
    assert!(!merged.status.success(), "the merge conflicts");

    let files = GitClient::new().tracked_files(root).expect("list tracked files");
    assert_eq!(files, vec!["shared.txt".to_string()]);
}
