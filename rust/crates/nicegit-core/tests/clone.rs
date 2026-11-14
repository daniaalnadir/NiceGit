//! Integration tests for `GitClient::clone_repository_with_options`: shallow history, file
//! contents on demand, folders limited by sparse checkout, and the defaults, run in temporary
//! folders against a local source repository.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use nicegit_core::client::{CloneOptions, GitClient};
use tempfile::TempDir;

// MARK: Helpers

/// Runs raw Git in `directory`, returning its output without checking the status.
fn git_output(directory: &Path, arguments: &[&str]) -> Output {
    let mut command = Command::new("git");
    command.args(arguments).current_dir(directory);
    // Inherited repository variables would redirect these commands to another repository.
    for key in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_OBJECT_DIRECTORY", "GIT_COMMON_DIR"] {
        command.env_remove(key);
    }
    command.output().unwrap_or_else(|error| panic!("could not run git {arguments:?}: {error}"))
}

/// Runs raw Git in `directory` and returns its trimmed standard output, panicking on failure.
fn git(directory: &Path, arguments: &[&str]) -> String {
    let output = git_output(directory, arguments);
    assert!(output.status.success(), "git {arguments:?} failed: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// A source repository on `main` with three commits. Its folders are `a/` and `b/`, and it has a
/// root file. Upload of any object is allowed, so a blobless clone can fetch objects from it.
fn source_repository() -> TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path();
    git(path, &["init", "-q", "-b", "main"]);
    for (key, value) in [("user.name", "Test"), ("user.email", "test@example.invalid"), ("commit.gpgsign", "false")] {
        git(path, &["config", key, value]);
    }
    git(path, &["config", "uploadpack.allowFilter", "true"]);
    git(path, &["config", "uploadpack.allowAnySHA1InWant", "true"]);
    fs::create_dir_all(path.join("a")).expect("folder a");
    fs::create_dir_all(path.join("b")).expect("folder b");
    fs::write(path.join("root.txt"), "root\n").expect("root file");
    fs::write(path.join("a/one.txt"), "one\n").expect("file in a");
    fs::write(path.join("b/two.txt"), "two\n").expect("file in b");
    git(path, &["add", "."]);
    git(path, &["commit", "-q", "-m", "one"]);
    fs::write(path.join("a/one.txt"), "one\nmore\n").expect("edit in a");
    git(path, &["commit", "-qam", "two"]);
    fs::write(path.join("b/two.txt"), "two\nmore\n").expect("edit in b");
    git(path, &["commit", "-qam", "three"]);
    dir
}

/// A destination `name` inside `parent`, which the clone creates.
fn destination(parent: &Path, name: &str) -> PathBuf {
    parent.join(name)
}

fn client() -> GitClient {
    GitClient::new()
}

fn options(depth: Option<u32>, blobless: bool, sparse_folders: &[&str]) -> CloneOptions {
    CloneOptions { depth, blobless, sparse_folders: sparse_folders.iter().map(|folder| folder.to_string()).collect() }
}

fn commit_count(directory: &Path) -> u32 {
    git(directory, &["rev-list", "--count", "HEAD"]).parse().expect("a commit count")
}

// MARK: Tests

#[test]
fn depth_one_clones_only_the_latest_commit() {
    let source = source_repository();
    let parent = tempfile::tempdir().expect("temp dir");
    let target = destination(parent.path(), "shallow");
    let source_text = source.path().display().to_string();

    client().clone_repository_with_options(&source_text, &target, &options(Some(1), false, &[])).expect("shallow clone");

    assert_eq!(commit_count(&target), 1, "only the latest commit is present");
    assert_eq!(git(&target, &["rev-parse", "--is-shallow-repository"]), "true");
    assert!(client().repository_shape(&target, Some(&target.join(".git"))).shallow, "the file check agrees with Git");
    assert_eq!(git(&target, &["log", "-1", "--format=%s"]), "three");
}

#[test]
fn a_shallow_clone_keeps_every_branch_as_asked() {
    let source = source_repository();
    git(source.path(), &["branch", "feature", "HEAD~1"]);
    let parent = tempfile::tempdir().expect("temp dir");
    let target = destination(parent.path(), "branches");

    client().clone_repository_with_options(&source.path().display().to_string(), &target, &options(Some(1), false, &[])).expect("clone");

    let remote_branches = git(&target, &["branch", "-r", "--format=%(refname:short)"]);
    assert!(remote_branches.contains("origin/feature"), "--no-single-branch keeps other branches: {remote_branches}");
}

#[test]
fn blobless_clone_records_the_partial_clone_filter() {
    let source = source_repository();
    let parent = tempfile::tempdir().expect("temp dir");
    let target = destination(parent.path(), "blobless");

    client()
        .clone_repository_with_options(&source.path().display().to_string(), &target, &options(None, true, &[]))
        .expect("blobless clone");

    assert_eq!(git(&target, &["config", "remote.origin.partialclonefilter"]), "blob:none");
    assert_eq!(commit_count(&target), 3, "the whole history is still present");
}

#[test]
fn sparse_clone_checks_out_only_the_chosen_folder() {
    let source = source_repository();
    let parent = tempfile::tempdir().expect("temp dir");
    let target = destination(parent.path(), "sparse");

    client()
        .clone_repository_with_options(&source.path().display().to_string(), &target, &options(None, false, &["a"]))
        .expect("sparse clone");

    assert!(target.join("a").join("one.txt").is_file(), "the chosen folder is checked out");
    assert!(!target.join("b").exists(), "the other folder is not checked out");
    assert!(target.join("root.txt").is_file(), "files at the root are always checked out");
    assert_eq!(commit_count(&target), 3, "sparse checkout keeps the full history");
    assert!(client().repository_shape(&target, Some(&target.join(".git"))).sparse);
    assert!(!client().repository_shape(&target, Some(&target.join(".git"))).shallow);
}

#[test]
fn defaults_make_the_same_full_clone_as_before() {
    let source = source_repository();
    let parent = tempfile::tempdir().expect("temp dir");
    let plain = destination(parent.path(), "plain");
    let defaulted = destination(parent.path(), "defaulted");
    let source_text = source.path().display().to_string();

    client().clone_repository(&source_text, &plain).expect("plain clone");
    client().clone_repository_with_options(&source_text, &defaulted, &CloneOptions::default()).expect("defaulted clone");

    for target in [&plain, &defaulted] {
        assert_eq!(commit_count(target), 3, "the full history is present");
        assert!(target.join("a").join("one.txt").is_file() && target.join("b").join("two.txt").is_file(), "every folder is checked out");
        // Git exits with failure for an unset key, so the output is checked rather than the status.
        let filter = git_output(target, &["config", "--get", "remote.origin.partialclonefilter"]);
        assert!(!filter.status.success(), "no partial clone filter is set");
        assert_eq!(git(target, &["rev-parse", "--is-shallow-repository"]), "false");
        assert!(!client().repository_shape(target, Some(&target.join(".git"))).sparse, "no sparse checkout");
    }
    assert_eq!(
        client().repository_shape(&defaulted, Some(&defaulted.join(".git"))),
        nicegit_core::client::RepositoryShape { shallow: false, sparse: false }
    );
}

#[test]
fn an_invalid_folder_is_refused_before_git_runs() {
    let source = source_repository();
    let parent = tempfile::tempdir().expect("temp dir");
    let target = destination(parent.path(), "refused");

    let error = client()
        .clone_repository_with_options(&source.path().display().to_string(), &target, &options(None, false, &["../x"]))
        .expect_err("a folder outside the repository is refused");

    assert!(error.to_string().contains("cannot contain"), "the problem is named: {error}");
    assert!(!target.exists(), "nothing was cloned");
}

#[test]
fn a_zero_commit_depth_is_refused() {
    let source = source_repository();
    let parent = tempfile::tempdir().expect("temp dir");
    let target = destination(parent.path(), "zero");

    assert!(client().clone_repository_with_options(&source.path().display().to_string(), &target, &options(Some(0), false, &[])).is_err());
    assert!(!target.exists(), "nothing was cloned");
}

#[test]
fn fetch_full_history_completes_a_shallow_clone() {
    let source = source_repository();
    let parent = tempfile::tempdir().expect("temp dir");
    let target = destination(parent.path(), "unshallow");
    client()
        .clone_repository_with_options(&source.path().display().to_string(), &target, &options(Some(1), false, &[]))
        .expect("shallow clone");
    assert_eq!(commit_count(&target), 1);

    client().fetch_full_history(&target).expect("fetch the full history");

    assert_eq!(commit_count(&target), 3, "the full history is now present");
    assert!(!client().repository_shape(&target, Some(&target.join(".git"))).shallow);
}

#[test]
fn show_all_folders_checks_out_every_folder_of_a_sparse_clone() {
    let source = source_repository();
    let parent = tempfile::tempdir().expect("temp dir");
    let target = destination(parent.path(), "everything");
    client()
        .clone_repository_with_options(&source.path().display().to_string(), &target, &options(None, false, &["a"]))
        .expect("sparse clone");
    assert!(!target.join("b").exists());

    client().show_all_folders(&target).expect("disable sparse checkout");

    assert!(target.join("b").join("two.txt").is_file(), "every folder is checked out");
    assert!(!client().repository_shape(&target, Some(&target.join(".git"))).sparse);
}
