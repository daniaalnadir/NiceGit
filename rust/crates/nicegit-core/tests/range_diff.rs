//! Integration tests for `GitClient::range_diff` and the reflog helpers that find an earlier
//! version of a branch, in real repositories.

use std::path::Path;
use std::process::{Command, Output};

use nicegit_core::range_diff::{RangeDiff, RangeDiffEntry, RangeDiffStatus};
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
    std::fs::write(path.join("start.txt"), "start\n").unwrap();
    git(path, &["add", "start.txt"]);
    git(path, &["commit", "-q", "-m", "Start"]);
    dir
}

/// Writes `contents` to `file`, stages it, and commits it with `subject`.
fn commit_file(path: &Path, file: &str, contents: &str, subject: &str) {
    std::fs::write(path.join(file), contents).unwrap();
    git(path, &["add", file]);
    git(path, &["commit", "-q", "-m", subject]);
}

/// Ten numbered lines, with line five set to `five`. Git pairs a small edit to a long file as a
/// change, where a rewrite of a one-line file reads as a removal and an addition.
fn lines(five: &str) -> String {
    (1..=10).map(|n| if n == 5 { format!("{five}\n") } else { format!("line {n}\n") }).collect()
}

/// The hash of the commit `revision` names.
fn hash(path: &Path, revision: &str) -> String {
    git(path, &["rev-parse", "--verify", revision])
}

/// The entries with `status`, in the order range-diff listed them.
fn with_status(diff: &RangeDiff, status: RangeDiffStatus) -> Vec<&RangeDiffEntry> {
    diff.entries.iter().filter(|entry| entry.status == status).collect()
}

/// The series of three commits on `feature`, which starts at `main`'s tip. Returns the base.
fn feature_series(path: &Path) -> String {
    let base = hash(path, "main");
    git(path, &["switch", "-q", "-c", "feature", &base]);
    commit_file(path, "a.txt", "one\n", "First step");
    commit_file(path, "b.txt", &lines("two"), "Second step");
    commit_file(path, "c.txt", &lines("three"), "Third step");
    git(path, &["switch", "-q", "main"]);
    base
}

/// Rewrites the feature series on `rewritten` from `base`: the first step is kept, the second
/// changes one line, and the third is replaced by a new fourth step.
fn rewrite_series(path: &Path, base: &str) {
    git(path, &["switch", "-q", "-c", "rewritten", base]);
    commit_file(path, "a.txt", "one\n", "First step");
    commit_file(path, "b.txt", &lines("two, changed"), "Second step");
    // Nothing in this file resembles the third step's, so range-diff cannot pair the two.
    let unrelated: String = (1..=10).map(|n| format!("fourth file, row {n}\n")).collect();
    commit_file(path, "d.txt", &unrelated, "Fourth step");
    git(path, &["switch", "-q", "main"]);
}

// MARK: Comparisons

#[test]
fn a_rewritten_series_pairs_each_commit_as_same_changed_removed_or_added() {
    let repo = repository();
    let path = repo.path();
    let base = feature_series(path);
    let old_tip = hash(path, "feature");
    rewrite_series(path, &base);
    let new_tip = hash(path, "rewritten");

    let diff = GitClient::new().range_diff(&base, &old_tip, &base, &new_tip, path).expect("range-diff");
    assert_eq!(diff.entries.len(), 4);
    assert_eq!(diff.count(RangeDiffStatus::Same), 1);
    assert_eq!(diff.count(RangeDiffStatus::Changed), 1);
    assert_eq!(diff.count(RangeDiffStatus::Removed), 1);
    assert_eq!(diff.count(RangeDiffStatus::Added), 1);

    let same = with_status(&diff, RangeDiffStatus::Same);
    assert_eq!(same[0].subject, "First step");
    assert_eq!(same[0].old.as_ref().map(|commit| commit.index), Some(1));
    assert_eq!(same[0].new.as_ref().map(|commit| commit.index), Some(1));
    assert!(same[0].diff_lines.is_empty(), "an unchanged pair has no comparison lines");

    let changed = with_status(&diff, RangeDiffStatus::Changed);
    assert_eq!(changed[0].subject, "Second step");
    assert_eq!(changed[0].old.as_ref().map(|commit| commit.index), Some(2));
    assert_eq!(changed[0].new.as_ref().map(|commit| commit.index), Some(2));
    assert!(changed[0].diff_lines.iter().any(|line| line.starts_with('-')), "{:?}", changed[0].diff_lines);
    assert!(changed[0].diff_lines.iter().any(|line| line.starts_with('+')), "{:?}", changed[0].diff_lines);

    let removed = with_status(&diff, RangeDiffStatus::Removed);
    assert_eq!(removed[0].subject, "Third step");
    assert_eq!(removed[0].old.as_ref().map(|commit| commit.index), Some(3));
    assert!(removed[0].new.is_none());

    let added = with_status(&diff, RangeDiffStatus::Added);
    assert_eq!(added[0].subject, "Fourth step");
    assert!(added[0].old.is_none());
    assert_eq!(added[0].new.as_ref().map(|commit| commit.index), Some(3));
}

#[test]
fn revisions_are_resolved_so_branch_names_work_and_option_like_text_is_refused() {
    let repo = repository();
    let path = repo.path();
    let base = feature_series(path);
    rewrite_series(path, &base);

    let diff = GitClient::new().range_diff("main", "feature", "main", "rewritten", path).expect("range-diff");
    assert_eq!(diff.entries.len(), 4);

    let error = GitClient::new().range_diff("--output=nowhere", "feature", "main", "rewritten", path);
    assert!(error.is_err(), "an option is not a revision");
}

#[test]
fn identical_series_have_only_same_entries_and_empty_series_have_none() {
    let repo = repository();
    let path = repo.path();
    let base = feature_series(path);
    let tip = hash(path, "feature");

    let same = GitClient::new().range_diff(&base, &tip, &base, &tip, path).expect("range-diff");
    assert_eq!(same.entries.len(), 3);
    assert_eq!(same.count(RangeDiffStatus::Same), 3);

    let empty = GitClient::new().range_diff(&tip, &tip, &tip, &tip, path).expect("range-diff");
    assert!(empty.entries.is_empty());
}

#[test]
fn a_series_with_no_commits_on_one_side_lists_the_other_side_as_added() {
    let repo = repository();
    let path = repo.path();
    let base = feature_series(path);
    let tip = hash(path, "feature");

    let diff = GitClient::new().range_diff(&base, &base, &base, &tip, path).expect("range-diff");
    assert_eq!(diff.entries.len(), 3);
    assert_eq!(diff.count(RangeDiffStatus::Added), 3);
    assert_eq!(diff.entries[0].subject, "First step");
    assert_eq!(diff.entries[0].new.as_ref().map(|commit| commit.index), Some(1));
    assert_eq!(diff.entries[2].new.as_ref().map(|commit| commit.index), Some(3));
    assert!(diff.entries.iter().all(|entry| entry.old.is_none()));

    let removed = GitClient::new().range_diff(&base, &tip, &base, &base, path).expect("range-diff");
    assert_eq!(removed.count(RangeDiffStatus::Removed), 3);
}

#[test]
fn a_series_longer_than_nine_commits_keeps_two_digit_indices() {
    let repo = repository();
    let path = repo.path();
    let base = hash(path, "main");
    git(path, &["switch", "-q", "-c", "long", &base]);
    for index in 1..=11 {
        commit_file(path, &format!("file{index}.txt"), &lines("first"), &format!("Step {index}"));
    }
    let old_tip = hash(path, "long");
    git(path, &["switch", "-q", "-c", "long-rewritten", &base]);
    for index in 1..=11 {
        let contents = if index == 10 { lines("second") } else { lines("first") };
        commit_file(path, &format!("file{index}.txt"), &contents, &format!("Step {index}"));
    }
    let new_tip = hash(path, "long-rewritten");

    let diff = GitClient::new().range_diff(&base, &old_tip, &base, &new_tip, path).expect("range-diff");
    assert_eq!(diff.entries.len(), 11, "{:?}", diff.entries);
    let changed = with_status(&diff, RangeDiffStatus::Changed);
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0].subject, "Step 10");
    assert_eq!(changed[0].old.as_ref().map(|commit| commit.index), Some(10));
    assert_eq!(changed[0].new.as_ref().map(|commit| commit.index), Some(10));
}

// MARK: Earlier versions

#[test]
fn an_amend_leaves_the_earlier_tip_in_the_reflog_and_compares_the_amended_commit() {
    let repo = repository();
    let path = repo.path();
    feature_series(path);
    git(path, &["switch", "-q", "feature"]);
    let old_tip = hash(path, "feature");
    std::fs::write(path.join("c.txt"), lines("three, amended")).unwrap();
    git(path, &["commit", "-q", "--amend", "-a", "--no-edit"]);
    git(path, &["switch", "-q", "main"]);
    let new_tip = hash(path, "feature");

    let client = GitClient::new();
    assert_eq!(client.reflog_previous_tip("feature", path).expect("reflog"), Some(old_tip.clone()));

    let inputs = client.default_range_diff_inputs("feature", path).expect("inputs").expect("an earlier version");
    assert_eq!(inputs.old_tip, old_tip);
    assert_eq!(inputs.new_tip, new_tip);
    // Only the amended commit differs, so both series start at its parent.
    let parent = hash(path, &format!("{old_tip}~1"));
    assert_eq!(inputs.old_base, parent);
    assert_eq!(inputs.new_base, parent);

    let diff = client.range_diff(&inputs.old_base, &inputs.old_tip, &inputs.new_base, &inputs.new_tip, path).expect("range-diff");
    assert_eq!(diff.entries.len(), 1, "{:?}", diff.entries);
    assert_eq!(diff.count(RangeDiffStatus::Changed), 1);
}

#[test]
fn a_commit_added_on_top_is_listed_as_added_against_the_earlier_tip() {
    let repo = repository();
    let path = repo.path();
    feature_series(path);
    git(path, &["switch", "-q", "feature"]);
    let old_tip = hash(path, "feature");
    commit_file(path, "e.txt", "five\n", "Fifth step");
    git(path, &["switch", "-q", "main"]);

    let client = GitClient::new();
    let inputs = client.default_range_diff_inputs("feature", path).expect("inputs").expect("an earlier version");
    assert_eq!(inputs.old_tip, old_tip);
    let diff = client.range_diff(&inputs.old_base, &inputs.old_tip, &inputs.new_base, &inputs.new_tip, path).expect("range-diff");
    assert_eq!(diff.entries.len(), 1);
    assert_eq!(diff.entries[0].status, RangeDiffStatus::Added);
    assert_eq!(diff.entries[0].subject, "Fifth step");
}

#[test]
fn a_branch_with_only_one_reflog_entry_has_no_earlier_version() {
    let repo = repository();
    let path = repo.path();
    git(path, &["branch", "fresh", "main"]);

    let client = GitClient::new();
    assert_eq!(client.reflog_previous_tip("fresh", path).expect("reflog"), None);
    assert!(client.default_range_diff_inputs("fresh", path).expect("inputs").is_none());
}

#[test]
fn a_remote_tracking_branch_reports_its_earlier_tip() {
    let repo = repository();
    let path = repo.path();
    let first = hash(path, "main");
    commit_file(path, "more.txt", "more\n", "Second");
    let second = hash(path, "main");
    git(path, &["update-ref", "refs/remotes/origin/main", &first]);
    git(path, &["update-ref", "refs/remotes/origin/main", &second]);

    let previous = GitClient::new().remote_reflog_previous_tip("origin", "main", path).expect("reflog");
    assert_eq!(previous, Some(first));
}

#[test]
fn tips_compare_from_their_merge_base_and_unrelated_tips_have_no_inputs() {
    let repo = repository();
    let path = repo.path();
    let base = feature_series(path);
    let old_tip = hash(path, "feature");
    rewrite_series(path, &base);
    let new_tip = hash(path, "rewritten");

    let client = GitClient::new();
    let merge_base = git(path, &["merge-base", &old_tip, &new_tip]);
    let inputs = client.tip_range_diff_inputs(&old_tip, &new_tip, path).expect("inputs").expect("a merge base");
    assert_eq!(inputs.old_base, merge_base);
    assert_eq!(inputs.new_base, merge_base);
    assert_eq!(inputs.old_tip, old_tip);
    assert_eq!(inputs.new_tip, new_tip);

    git(path, &["switch", "-q", "--orphan", "lonely"]);
    commit_file(path, "lonely.txt", "lonely\n", "Lonely root");
    let lonely = hash(path, "HEAD");
    git(path, &["switch", "-q", "main"]);
    assert!(client.tip_range_diff_inputs(&lonely, &new_tip, path).expect("inputs").is_none());
}
