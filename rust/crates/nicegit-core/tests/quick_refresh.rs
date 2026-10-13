//! Integration tests for `GitClient::quick_snapshot`, the status-only refresh. Each test runs against
//! a real repository in a temporary directory, and checks the quick result against a full load.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use nicegit_core::models::{HistoryScope, Operation, Snapshot, StatusKind};
use nicegit_core::GitClient;
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

/// Runs raw Git in `directory` and returns its standard output, panicking on failure.
fn git(directory: &Path, arguments: &[&str]) -> String {
    let output = git_output(directory, arguments);
    assert!(output.status.success(), "git {arguments:?} failed: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// A repository with two commits on main, a `feature` branch, and the identity set locally.
fn repository() -> TempDir {
    let dir = TempDir::new().expect("temporary folder");
    let path = dir.path();
    git(path, &["init", "-q", "-b", "main"]);
    for (key, value) in [("user.name", "Test"), ("user.email", "test@example.com"), ("commit.gpgsign", "false"), ("core.autocrlf", "false")]
    {
        git(path, &["config", key, value]);
    }
    fs::write(path.join("notes.txt"), "first\n").unwrap();
    git(path, &["add", "notes.txt"]);
    git(path, &["commit", "-q", "-m", "Start the notes"]);
    git(path, &["branch", "feature"]);
    fs::write(path.join("notes.txt"), "first\nsecond\n").unwrap();
    git(path, &["commit", "-qam", "Add a second line"]);
    dir
}

/// A full load of the repository, with the history scope the app uses by default.
fn full(path: &Path) -> Snapshot {
    GitClient::new().load_snapshot_scoped(path, 100, HistoryScope::All).expect("full snapshot")
}

/// The quick refresh of `previous`, which must apply to the checkout.
fn quick(previous: &Snapshot, path: &Path) -> Snapshot {
    GitClient::new().quick_snapshot(previous, path).expect("quick snapshot").expect("a quick refresh applies")
}

// MARK: Working files

#[test]
fn quick_snapshot_updates_status_for_edited_files() {
    let dir = repository();
    let path = dir.path();
    let previous = full(path);
    assert!(previous.status.is_empty());

    fs::write(path.join("notes.txt"), "first\nsecond\nthird\n").unwrap();
    fs::write(path.join("new.txt"), "made since the last load\n").unwrap();
    let refreshed = quick(&previous, path);

    // Only the status differs, so the quick result is exactly what a full load now reads.
    assert_eq!(refreshed, full(path));
    assert!(refreshed.status.iter().any(|entry| entry.path == "notes.txt"));
    assert!(refreshed.status.iter().any(|entry| entry.path == "new.txt" && entry.kind == StatusKind::Untracked));
    assert_eq!(refreshed.commits, previous.commits, "the history is kept");
    assert_eq!(refreshed.branches, previous.branches, "the branches are kept");
}

#[test]
fn quick_snapshot_leaves_the_index_alone() {
    let dir = repository();
    let path = dir.path();
    let previous = full(path);
    // Rewriting the same bytes changes the file's stat data, which a status run would record in
    // the index. A quick refresh must leave the index as it found it.
    fs::write(path.join("notes.txt"), "first\nsecond\n").unwrap();
    let index = path.join(".git").join("index");
    let before = fs::read(&index).unwrap();

    quick(&previous, path);
    assert_eq!(fs::read(&index).unwrap(), before, "the index is not rewritten");
}

// MARK: Checkout changes

#[test]
fn quick_snapshot_needs_a_full_load_after_a_commit() {
    let dir = repository();
    let path = dir.path();
    let previous = full(path);

    fs::write(path.join("notes.txt"), "first\nsecond\nthird\n").unwrap();
    git(path, &["commit", "-qam", "Add a third line"]);
    assert!(GitClient::new().quick_snapshot(&previous, path).unwrap().is_none());
}

#[test]
fn quick_snapshot_needs_a_full_load_after_a_branch_switch() {
    let dir = repository();
    let path = dir.path();
    let previous = full(path);

    git(path, &["switch", "-q", "feature"]);
    assert!(GitClient::new().quick_snapshot(&previous, path).unwrap().is_none());
}

#[test]
fn quick_snapshot_needs_a_full_load_after_detaching_head() {
    let dir = repository();
    let path = dir.path();
    let previous = full(path);

    // The commit is the same, but the branch is not: the snapshot names a detached HEAD differently.
    git(path, &["checkout", "-q", "--detach"]);
    assert!(GitClient::new().quick_snapshot(&previous, path).unwrap().is_none());
}

#[test]
fn quick_snapshot_keeps_a_detached_head_when_only_files_change() {
    let dir = repository();
    let path = dir.path();
    git(path, &["checkout", "-q", "--detach"]);
    let previous = full(path);
    assert!(previous.current_branch.starts_with("Detached HEAD "), "the snapshot names the detached HEAD");

    fs::write(path.join("notes.txt"), "first\nsecond\nthird\n").unwrap();
    assert_eq!(quick(&previous, path), full(path));
}

// MARK: Unfinished operations

#[test]
fn quick_snapshot_needs_a_full_load_during_a_merge_conflict() {
    let dir = repository();
    let path = dir.path();
    git(path, &["switch", "-q", "feature"]);
    fs::write(path.join("notes.txt"), "first\nfrom feature\n").unwrap();
    git(path, &["commit", "-qam", "Change the notes on feature"]);
    git(path, &["switch", "-q", "main"]);
    let previous = full(path);
    assert_eq!(previous.operation, None);

    fs::write(path.join("notes.txt"), "first\nfrom main\n").unwrap();
    git(path, &["commit", "-qam", "Change the notes on main"]);
    let previous = full(path);
    let merge = git_output(path, &["merge", "--no-edit", "feature"]);
    assert!(!merge.status.success(), "the merge conflicts");
    assert_eq!(full(path).operation, Some(Operation::Merge));

    // The snapshot from before the merge has no operation, so the quick refresh must not apply.
    assert!(GitClient::new().quick_snapshot(&previous, path).unwrap().is_none());
}

#[test]
fn quick_snapshot_refreshes_status_within_the_same_operation() {
    let dir = repository();
    let path = dir.path();
    git(path, &["switch", "-q", "feature"]);
    fs::write(path.join("notes.txt"), "first\nfrom feature\n").unwrap();
    git(path, &["commit", "-qam", "Change the notes on feature"]);
    git(path, &["switch", "-q", "main"]);
    fs::write(path.join("notes.txt"), "first\nfrom main\n").unwrap();
    git(path, &["commit", "-qam", "Change the notes on main"]);
    let merge = git_output(path, &["merge", "--no-edit", "feature"]);
    assert!(!merge.status.success(), "the merge conflicts");
    let previous = full(path);
    assert_eq!(previous.operation, Some(Operation::Merge));

    // Resolving the file changes only the status, so the quick refresh applies and shows it.
    fs::write(path.join("notes.txt"), "first\nfrom both\n").unwrap();
    git(path, &["add", "notes.txt"]);
    let refreshed = quick(&previous, path);
    assert_eq!(refreshed, full(path));
    assert!(refreshed.status.iter().any(|entry| entry.path == "notes.txt" && entry.kind != StatusKind::Conflicted));
}

// MARK: Upstream

#[test]
fn quick_snapshot_updates_ahead_and_behind() {
    let dir = repository();
    let path = dir.path();
    let remote = TempDir::new().expect("temporary folder");
    git(remote.path(), &["init", "-q", "--bare", "-b", "main"]);
    git(path, &["remote", "add", "origin", &remote.path().to_string_lossy()]);
    git(path, &["push", "-q", "-u", "origin", "main"]);
    fs::write(path.join("notes.txt"), "first\nsecond\nthird\n").unwrap();
    git(path, &["commit", "-qam", "Add a third line"]);
    let previous = full(path);
    assert_eq!((previous.ahead, previous.behind), (Some(1), Some(0)));

    // Another commit on the upstream, made without touching HEAD or the working files: the
    // branch now differs from its upstream both ways.
    let parent = git(path, &["rev-parse", "HEAD~1"]);
    let other = git(path, &["commit-tree", "HEAD~1^{tree}", "-p", &parent, "-m", "Work from elsewhere"]);
    git(path, &["update-ref", "refs/remotes/origin/main", &other]);

    // The refs moved, so the branches and history are the previous ones; the counts are current,
    // and they match a full load's.
    let refreshed = quick(&previous, path);
    let expected = full(path);
    assert_eq!((refreshed.ahead, refreshed.behind), (Some(1), Some(1)));
    assert_eq!((refreshed.ahead, refreshed.behind), (expected.ahead, expected.behind));
    assert_eq!(refreshed.branches, previous.branches);
}

#[test]
fn quick_snapshot_needs_a_full_load_when_the_upstream_changes() {
    let dir = repository();
    let path = dir.path();
    let remote = TempDir::new().expect("temporary folder");
    git(remote.path(), &["init", "-q", "--bare", "-b", "main"]);
    git(path, &["remote", "add", "origin", &remote.path().to_string_lossy()]);
    git(path, &["push", "-q", "-u", "origin", "main"]);
    git(path, &["push", "-q", "origin", "main:side"]);
    git(path, &["fetch", "-q", "origin"]);
    let previous = full(path);
    assert_eq!(previous.upstream.as_deref(), Some("origin/main"));

    git(path, &["branch", "--set-upstream-to=origin/side", "main"]);
    assert_eq!(full(path).upstream.as_deref(), Some("origin/side"));
    assert!(GitClient::new().quick_snapshot(&previous, path).unwrap().is_none());
}

// MARK: Snapshots without a Git directory

#[test]
fn quick_snapshot_needs_a_full_load_without_a_git_directory() {
    let dir = repository();
    let path = dir.path();
    let previous = Snapshot::default();
    assert!(previous.absolute_git_dir.is_none());
    assert!(GitClient::new().quick_snapshot(&previous, path).unwrap().is_none());
    assert!(full(path).absolute_git_dir.is_some(), "a full load records the Git directory");
}

#[test]
fn quick_snapshot_notices_a_new_gitmodules_file() {
    let dir = repository();
    let path = dir.path();
    let previous = full(path);
    assert!(!previous.has_gitmodules);

    fs::write(path.join(".gitmodules"), "").unwrap();
    let refreshed = quick(&previous, path);

    // Creating the file is a working-file change, so the quick path must see it too.
    assert!(refreshed.has_gitmodules);
    assert_eq!(refreshed, full(path));
}
