//! Integration tests for listing a commit's whole tree and reading one file from it, against real
//! repositories in temporary directories.

use std::fs;
use std::path::Path;
use std::process::Command;

use nicegit_core::client::GitClient;
use nicegit_core::commit_tree::{FileView, LARGE_FILE_BYTES};
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
fn the_tree_lists_nested_files_and_paths_with_newlines() {
    let dir = repository();
    let root = dir.path();
    fs::create_dir_all(root.join("src").join("deep")).unwrap();
    fs::write(root.join("src").join("deep").join("lib.rs"), "pub fn f() {}\n").unwrap();
    fs::write(root.join("README.md"), "# Title\n").unwrap();
    #[cfg(unix)]
    fs::write(root.join("two\nlines.txt"), "x\n").unwrap();
    git(root, &["add", "--all"]);
    git(root, &["commit", "-q", "-m", "Add files"]);
    let commit = git(root, &["rev-parse", "HEAD"]).trim().to_string();

    let listing = GitClient::new().commit_tree(&commit, root).expect("list the tree");
    let paths: Vec<&str> = listing.entries.iter().map(|entry| entry.path.as_str()).collect();
    assert!(paths.contains(&"README.md"));
    assert!(paths.contains(&"src/deep/lib.rs"), "nested files are listed by their full path: {paths:?}");
    #[cfg(unix)]
    assert!(paths.contains(&"two\nlines.txt"), "a newline in a name is kept: {paths:?}");
    assert!(!listing.truncated());
    assert_eq!(listing.total, listing.entries.len());
    assert!(listing.entries.iter().all(|entry| !entry.submodule && entry.object.len() == 40));

    // Listing from a subfolder still gives paths from the root.
    let from_subfolder = GitClient::new().commit_tree(&commit, &root.join("src")).expect("list from a subfolder");
    assert_eq!(from_subfolder.total, listing.total);
}

#[test]
fn the_tree_is_read_at_the_commit_not_the_checkout() {
    let dir = repository();
    let root = dir.path();
    fs::write(root.join("old.txt"), "old\n").unwrap();
    git(root, &["add", "--all"]);
    git(root, &["commit", "-q", "-m", "First"]);
    let first = git(root, &["rev-parse", "HEAD"]).trim().to_string();
    fs::write(root.join("new.txt"), "new\n").unwrap();
    git(root, &["add", "--all"]);
    git(root, &["commit", "-q", "-m", "Second"]);

    let listing = GitClient::new().commit_tree(&first, root).expect("list the first commit");
    let paths: Vec<&str> = listing.entries.iter().map(|entry| entry.path.as_str()).collect();
    assert_eq!(paths, vec!["old.txt"]);
}

#[test]
fn a_text_file_reads_as_its_lines_at_the_commit() {
    let dir = repository();
    let root = dir.path();
    fs::write(root.join("notes.txt"), "first\r\nsecond\n\nlast").unwrap();
    git(root, &["add", "--all"]);
    git(root, &["commit", "-q", "-m", "Notes"]);
    fs::write(root.join("notes.txt"), "changed later\n").unwrap();
    git(root, &["commit", "-q", "-am", "Change"]);
    let client = GitClient::new();
    let first = git(root, &["rev-parse", "HEAD~1"]).trim().to_string();

    let listing = client.commit_tree(&first, root).expect("list");
    let object = &listing.entries[0].object;
    let view = client.file_view(object, false, root).expect("read the file");
    assert_eq!(view, FileView::Text(vec!["first".into(), "second".into(), String::new(), "last".into()]));
}

#[test]
fn a_binary_file_is_reported_by_size_not_shown() {
    let dir = repository();
    let root = dir.path();
    let bytes: Vec<u8> = b"PNG\0\x01\x02binary".to_vec();
    fs::write(root.join("image.bin"), &bytes).unwrap();
    git(root, &["add", "--all"]);
    git(root, &["commit", "-q", "-m", "Binary"]);
    let client = GitClient::new();
    let listing = client.commit_tree("HEAD", root).expect("list");

    let view = client.file_view(&listing.entries[0].object, false, root).expect("read the file");
    assert_eq!(view, FileView::Binary(bytes.len() as u64));
}

#[test]
fn a_large_file_waits_until_it_is_opened_anyway() {
    let dir = repository();
    let root = dir.path();
    let size = LARGE_FILE_BYTES as usize + 10;
    fs::write(root.join("big.txt"), "line\n".repeat(size / 5 + 1)).unwrap();
    git(root, &["add", "--all"]);
    git(root, &["commit", "-q", "-m", "Big"]);
    let client = GitClient::new();
    let listing = client.commit_tree("HEAD", root).expect("list");
    let object = &listing.entries[0].object;

    let view = client.file_view(object, false, root).expect("size check");
    let FileView::TooLarge(reported) = view else { panic!("a large file is not read: {view:?}") };
    assert_eq!(reported, (size / 5 + 1) as u64 * 5);
    assert!(matches!(client.file_view(object, true, root).expect("open anyway"), FileView::Text(lines) if lines.len() > 1000));
}

#[test]
fn a_submodule_is_listed_as_a_commit_entry() {
    let dir = repository();
    let root = dir.path();
    let inner = repository();
    fs::write(inner.path().join("inner.txt"), "inner\n").unwrap();
    git(inner.path(), &["add", "--all"]);
    git(inner.path(), &["commit", "-q", "-m", "Inner"]);
    let inner_commit = git(inner.path(), &["rev-parse", "HEAD"]).trim().to_string();
    // A gitlink can be recorded without cloning, so the test needs no network or file transport.
    git(root, &["update-index", "--add", "--cacheinfo", &format!("160000,{inner_commit},vendor/lib")]);
    git(root, &["commit", "-q", "-m", "Add submodule"]);

    let listing = GitClient::new().commit_tree("HEAD", root).expect("list");
    let entry = listing.entries.iter().find(|entry| entry.path == "vendor/lib").expect("the submodule is listed");
    assert!(entry.submodule);
    assert_eq!(entry.object, inner_commit);
}

#[test]
fn an_object_id_that_is_not_a_full_id_is_refused() {
    let dir = repository();
    fs::write(dir.path().join("a.txt"), "a\n").unwrap();
    git(dir.path(), &["add", "--all"]);
    git(dir.path(), &["commit", "-q", "-m", "A"]);
    let error = GitClient::new().file_view("--output=/dev/null", false, dir.path()).expect_err("an option is not an object");
    assert!(error.to_string().contains("not valid"), "the refusal names the problem: {error}");
}
