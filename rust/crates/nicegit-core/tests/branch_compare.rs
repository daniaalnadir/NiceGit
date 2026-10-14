//! Integration tests for `GitClient::compare_branch`, which compares the checked-out branch with
//! another branch's tip in a real repository.

use std::path::Path;
use std::process::{Command, Output};

use nicegit_core::branch_compare::{BranchComparison, ComparedCommit};
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

/// An empty repository on `main` with the identity set locally, and one commit "Start".
fn repository() -> TempDir {
    let dir = TempDir::new().expect("temporary folder");
    let path = dir.path();
    git(path, &["init", "-q", "-b", "main"]);
    for (key, value) in [("user.name", "Test"), ("user.email", "test@example.com"), ("commit.gpgsign", "false")] {
        git(path, &["config", key, value]);
    }
    commit(path, "Start");
    dir
}

/// Records an empty commit with `subject` on the checked-out branch.
fn commit(path: &Path, subject: &str) {
    git(path, &["commit", "-q", "--allow-empty", "-m", subject]);
}

/// The hash of the commit `revision` names.
fn hash(path: &Path, revision: &str) -> String {
    git(path, &["rev-parse", "--verify", revision])
}

fn subjects(commits: &[ComparedCommit]) -> Vec<&str> {
    commits.iter().map(|commit| commit.subject.as_str()).collect()
}

/// The comparison from the checked-out branch with the tip of `other`, with the default limit.
fn compare(path: &Path, other: &str) -> BranchComparison {
    let tip = hash(path, other);
    GitClient::new().compare_branch(&tip, path).expect("comparison")
}

// MARK: Comparisons

#[test]
fn each_side_lists_its_own_commits_newest_first_and_names_the_merge_base() {
    let repo = repository();
    let path = repo.path();
    let base = hash(path, "HEAD");
    commit(path, "Second on main");
    git(path, &["switch", "-q", "-c", "feature", &base]);
    commit(path, "First on feature");
    commit(path, "Second on feature");
    git(path, &["switch", "-q", "main"]);
    commit(path, "Third on main");

    let comparison = compare(path, "feature");
    assert_eq!(subjects(&comparison.only_in_other), ["Second on feature", "First on feature"]);
    assert_eq!(subjects(&comparison.only_in_current), ["Third on main", "Second on main"]);
    assert_eq!(comparison.base.as_deref(), Some(base.as_str()));
    assert!(!comparison.more_in_other && !comparison.more_in_current);
    assert_eq!(comparison.only_in_other[0].author, "Test");
    assert!(!comparison.only_in_other[0].relative_date.is_empty());
    assert_eq!(comparison.only_in_other[0].hash, hash(path, "feature"));
}

#[test]
fn a_branch_whose_commits_are_all_on_the_current_branch_has_nothing_on_its_side() {
    let repo = repository();
    let path = repo.path();
    let base = hash(path, "HEAD");
    git(path, &["branch", "merged", &base]);
    commit(path, "Only on main");

    let comparison = compare(path, "merged");
    assert!(comparison.only_in_other.is_empty(), "nothing on merged is missing from main");
    assert_eq!(subjects(&comparison.only_in_current), ["Only on main"]);
    assert_eq!(comparison.base.as_deref(), Some(base.as_str()));
}

#[test]
fn unrelated_histories_share_no_base_and_both_sides_are_listed() {
    let repo = repository();
    let path = repo.path();
    git(path, &["switch", "-q", "--orphan", "lonely"]);
    commit(path, "Lonely root");
    let lonely = hash(path, "HEAD");
    git(path, &["switch", "-q", "main"]);

    let comparison = GitClient::new().compare_branch(&lonely, path).expect("comparison");
    assert_eq!(comparison.base, None);
    assert_eq!(subjects(&comparison.only_in_other), ["Lonely root"]);
    assert_eq!(subjects(&comparison.only_in_current), ["Start"]);
}

#[test]
fn a_side_longer_than_the_limit_is_cut_to_its_newest_commits_and_flagged() {
    let repo = repository();
    let path = repo.path();
    let base = hash(path, "HEAD");
    git(path, &["switch", "-q", "-c", "feature", &base]);
    for subject in ["First on feature", "Second on feature", "Third on feature", "Fourth on feature"] {
        commit(path, subject);
    }
    git(path, &["switch", "-q", "main"]);
    for subject in ["First on main", "Second on main", "Third on main"] {
        commit(path, subject);
    }

    let comparison = GitClient::new().compare_branch_with_limit(&hash(path, "feature"), path, 2).expect("comparison");
    assert_eq!(subjects(&comparison.only_in_other), ["Fourth on feature", "Third on feature"]);
    assert!(comparison.more_in_other);
    assert_eq!(subjects(&comparison.only_in_current), ["Third on main", "Second on main"]);
    assert!(comparison.more_in_current);
}

#[test]
fn a_side_within_the_limit_is_not_flagged() {
    let repo = repository();
    let path = repo.path();
    let base = hash(path, "HEAD");
    git(path, &["switch", "-q", "-c", "feature", &base]);
    commit(path, "Only on feature");
    git(path, &["switch", "-q", "main"]);

    let comparison = GitClient::new().compare_branch_with_limit(&hash(path, "feature"), path, 2).expect("comparison");
    assert_eq!(subjects(&comparison.only_in_other), ["Only on feature"]);
    assert!(!comparison.more_in_other);
    assert!(comparison.only_in_current.is_empty());
    assert!(!comparison.more_in_current);
}

#[test]
fn comparing_fails_while_the_current_branch_has_no_commits() {
    let repo = repository();
    let path = repo.path();
    let other = hash(path, "HEAD");
    git(path, &["switch", "-q", "--orphan", "empty"]);

    let error = GitClient::new().compare_branch(&other, path).expect_err("an unborn branch cannot be compared");
    assert!(error.to_string().contains("There are no commits on the current branch yet."), "message was: {error}");
}
