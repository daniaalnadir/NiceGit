//! Integration tests for absorbing staged changes into earlier commits, run against real repositories
//! in temporary directories.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::{Command, Output};

use nicegit_core::absorb::{apply_absorb, plan_absorb, AbsorbPlan};
use nicegit_core::client::GitClient;
use tempfile::TempDir;

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
    String::from_utf8_lossy(&output.stdout).trim_end().to_string()
}

/// A repository on branch `main` with one commit, and the test's identity set.
fn repository() -> TempDir {
    let dir = tempfile::tempdir().expect("create temp dir");
    let root = dir.path();
    git(root, &["init", "-q", "-b", "main"]);
    for (key, value) in [("user.name", "Test"), ("user.email", "test@example.com"), ("commit.gpgsign", "false"), ("core.autocrlf", "false")]
    {
        git(root, &["config", key, value]);
    }
    write(root, "base.txt", "base\n");
    git(root, &["add", "base.txt"]);
    git(root, &["commit", "-q", "-m", "Base"]);
    dir
}

/// Writes `contents` to `name` in the repository.
fn write(root: &Path, name: &str, contents: &str) {
    std::fs::write(root.join(name), contents).unwrap();
}

/// Writes `name`, stages it, and commits it with `message`, leaving other staged changes as they are.
fn commit_only(root: &Path, name: &str, contents: &str, message: &str) {
    write(root, name, contents);
    git(root, &["add", name]);
    git(root, &["commit", "-q", "--only", "-m", message, "--", name]);
}

fn head(root: &Path) -> String {
    git(root, &["rev-parse", "HEAD"])
}

/// The hash of the commit on the current branch whose subject is `subject`.
fn commit_with_subject(root: &Path, subject: &str) -> String {
    git(root, &["log", "--format=%H%x09%s"])
        .lines()
        .find_map(|line| line.split_once('\t').filter(|(_, text)| *text == subject).map(|(hash, _)| hash.to_string()))
        .unwrap_or_else(|| panic!("no commit named {subject}"))
}

/// The subjects of the commits on the current branch, oldest first.
fn subjects(root: &Path) -> String {
    git(root, &["log", "--reverse", "--format=%s"])
}

/// The subjects of the commits `plan` offers, in order.
fn target_subjects(plan: &AbsorbPlan) -> Vec<String> {
    plan.targets.iter().map(|target| target.subject.clone()).collect()
}

/// Every commit `plan` offers.
fn all_targets(plan: &AbsorbPlan) -> BTreeSet<String> {
    plan.targets.iter().map(|target| target.commit.clone()).collect()
}

/// A repository with three commits after the base: `a.txt` gets its lines 1 to 5 in the first
/// commit, `b.txt` changes in the second, and line 2 of `a.txt` changes in the third.
fn three_commits() -> TempDir {
    let dir = repository();
    let root = dir.path();
    write(root, "a.txt", "one\ntwo\nthree\nfour\nfive\n");
    write(root, "b.txt", "b\n");
    git(root, &["add", "a.txt", "b.txt"]);
    git(root, &["commit", "-q", "-m", "Add a and b"]);
    commit_only(root, "b.txt", "b changed\n", "Change b");
    write(root, "a.txt", "one\nTWO\nthree\nfour\nfive\n");
    git(root, &["commit", "-q", "-am", "Change two in a"]);
    dir
}

#[test]
fn absorbs_each_staged_change_into_the_commit_that_last_touched_its_lines() {
    let repo = three_commits();
    let root = repo.path();
    let client = GitClient::new();
    // Edits to a line from the first commit and to the line from the third, and a change to b.txt
    // from the second. A new file has no earlier commit, so it stays staged.
    write(root, "a.txt", "one\nTwo\nthree\nfour\nFIVE\n");
    git(root, &["add", "a.txt"]);
    write(root, "b.txt", "b final\n");
    git(root, &["add", "b.txt"]);
    write(root, "new.txt", "new\n");
    git(root, &["add", "new.txt"]);
    let before_subjects = subjects(root);
    let before_head = head(root);

    let plan = plan_absorb(&client, root).unwrap();
    assert_eq!(plan.branch, "main");
    assert_eq!(plan.head, before_head);
    assert_eq!(target_subjects(&plan), vec!["Add a and b", "Change b", "Change two in a"]);
    assert_eq!(plan.unassigned.len(), 1);
    assert_eq!(plan.unassigned[0].0.path, "new.txt");
    assert_eq!(plan.unassigned[0].1, "New file");

    let absorbed = apply_absorb(&client, &plan, &all_targets(&plan), root).unwrap();
    assert_eq!(absorbed, 3);
    assert_eq!(subjects(root), before_subjects);
    assert_eq!(git(root, &["show", "HEAD~2:a.txt"]), "one\ntwo\nthree\nfour\nFIVE");
    assert_eq!(git(root, &["show", "HEAD~1:b.txt"]), "b final");
    assert_eq!(git(root, &["show", "HEAD:a.txt"]), "one\nTwo\nthree\nfour\nFIVE");
    assert_eq!(git(root, &["show", "HEAD:b.txt"]), "b final");
    // The new file is the only staged change, and the working tree is what it was.
    assert_eq!(git(root, &["diff", "--cached", "--name-only"]), "new.txt");
    assert_eq!(git(root, &["status", "--porcelain"]), "A  new.txt");
    assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "one\nTwo\nthree\nfour\nFIVE\n");
    assert_eq!(std::fs::read_to_string(root.join("new.txt")).unwrap(), "new\n");
}

#[test]
fn unstaged_changes_survive_an_absorb() {
    let repo = three_commits();
    let root = repo.path();
    let client = GitClient::new();
    write(root, "a.txt", "one\nTWO\nthree\nfour\nFIVE\n");
    git(root, &["add", "a.txt"]);
    // An edit that is not staged, on another line of the same file.
    write(root, "a.txt", "one\nTWO\nTHREE\nfour\nFIVE\n");

    let plan = plan_absorb(&client, root).unwrap();
    assert_eq!(target_subjects(&plan), vec!["Add a and b"]);
    apply_absorb(&client, &plan, &all_targets(&plan), root).unwrap();

    assert_eq!(git(root, &["show", "HEAD~2:a.txt"]), "one\ntwo\nthree\nfour\nFIVE");
    assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "one\nTWO\nTHREE\nfour\nFIVE\n");
    assert_eq!(git(root, &["diff", "--name-only"]), "a.txt");
    assert_eq!(git(root, &["diff", "--cached", "--name-only"]), "");
}

#[test]
fn a_change_spanning_two_commits_stays_staged() {
    let repo = repository();
    let root = repo.path();
    let client = GitClient::new();
    write(root, "lines.txt", "first\nsecond\nthird\n");
    git(root, &["add", "lines.txt"]);
    git(root, &["commit", "-q", "-m", "Add lines"]);
    write(root, "lines.txt", "FIRST\nsecond\nthird\n");
    git(root, &["commit", "-q", "-am", "Change first"]);
    write(root, "lines.txt", "FIRST\nSECOND\nthird\n");
    git(root, &["commit", "-q", "-am", "Change second"]);
    // One staged change edits both lines, which came from two different commits.
    write(root, "lines.txt", "FIRST!\nSECOND!\nthird\n");
    git(root, &["add", "lines.txt"]);

    let plan = plan_absorb(&client, root).unwrap();
    assert!(plan.targets.is_empty());
    assert_eq!(plan.unassigned.len(), 1);
    assert_eq!(plan.unassigned[0].1, "Lines come from 2 commits");
    let error = apply_absorb(&client, &plan, &BTreeSet::new(), root).unwrap_err();
    assert!(error.to_string().contains("Choose at least one commit"), "{error}");
}

#[test]
fn a_commit_already_on_a_remote_is_not_rewritten() {
    let repo = three_commits();
    let root = repo.path();
    let remote = tempfile::tempdir().unwrap();
    git(remote.path(), &["init", "-q", "--bare", "-b", "main"]);
    git(root, &["remote", "add", "origin", &remote.path().to_string_lossy()]);
    git(root, &["push", "-q", "-u", "origin", "main"]);
    // A commit after the push, then staged edits to a line from it and to one from a pushed commit.
    commit_only(root, "c.txt", "c\n", "Add c");
    write(root, "c.txt", "C\n");
    git(root, &["add", "c.txt"]);
    write(root, "a.txt", "one\nTWO!\nthree\nfour\nfive\n");
    git(root, &["add", "a.txt"]);
    let client = GitClient::new();

    let plan = plan_absorb(&client, root).unwrap();
    assert_eq!(target_subjects(&plan), vec!["Add c"]);
    assert_eq!(plan.unassigned.len(), 1);
    assert_eq!(plan.unassigned[0].0.path, "a.txt");
    assert_eq!(plan.unassigned[0].1, "Commit already pushed");
}

#[test]
fn a_commit_before_a_merge_is_not_rewritten() {
    let repo = repository();
    let root = repo.path();
    let client = GitClient::new();
    write(root, "a.txt", "one\ntwo\n");
    git(root, &["add", "a.txt"]);
    git(root, &["commit", "-q", "-m", "Add a"]);
    git(root, &["branch", "feature"]);
    write(root, "b.txt", "b\n");
    git(root, &["add", "b.txt"]);
    git(root, &["commit", "-q", "-m", "Add b"]);
    git(root, &["switch", "-q", "feature"]);
    write(root, "c.txt", "c\n");
    git(root, &["add", "c.txt"]);
    git(root, &["commit", "-q", "-m", "Add c"]);
    git(root, &["switch", "-q", "main"]);
    git(root, &["merge", "-q", "--no-ff", "-m", "Merge feature", "feature"]);
    write(root, "a.txt", "ONE\ntwo\n");
    git(root, &["add", "a.txt"]);

    let plan = plan_absorb(&client, root).unwrap();
    assert!(plan.targets.is_empty());
    assert_eq!(plan.unassigned[0].1, "A merge commit comes after it");
}

#[test]
fn a_pure_addition_goes_to_the_commit_of_the_line_above() {
    let repo = three_commits();
    let root = repo.path();
    let client = GitClient::new();
    // A line added below line 5, which the first commit wrote.
    write(root, "a.txt", "one\nTWO\nthree\nfour\nfive\nsix\n");
    git(root, &["add", "a.txt"]);

    let plan = plan_absorb(&client, root).unwrap();
    assert_eq!(target_subjects(&plan), vec!["Add a and b"]);
    apply_absorb(&client, &plan, &all_targets(&plan), root).unwrap();
    assert_eq!(git(root, &["show", "HEAD~2:a.txt"]), "one\ntwo\nthree\nfour\nfive\nsix");
    assert_eq!(git(root, &["show", "HEAD:a.txt"]), "one\nTWO\nthree\nfour\nfive\nsix");
    assert_eq!(git(root, &["diff", "--cached", "--name-only"]), "");
}

#[test]
fn absorbing_into_an_earlier_commit_keeps_the_later_commits_content() {
    let repo = three_commits();
    let root = repo.path();
    let client = GitClient::new();
    // A change to the first commit's line 5, and a change to b.txt that is not absorbed.
    write(root, "a.txt", "one\nTWO\nthree\nfour\nFIVE\n");
    git(root, &["add", "a.txt"]);
    write(root, "b.txt", "b\n");
    git(root, &["add", "b.txt"]);

    let plan = plan_absorb(&client, root).unwrap();
    let first = commit_with_subject(root, "Add a and b");
    let subset: BTreeSet<String> = [first].into_iter().collect();
    apply_absorb(&client, &plan, &subset, root).unwrap();

    assert_eq!(git(root, &["show", "HEAD~2:a.txt"]), "one\ntwo\nthree\nfour\nFIVE");
    assert_eq!(git(root, &["show", "HEAD:a.txt"]), "one\nTWO\nthree\nfour\nFIVE");
    assert_eq!(git(root, &["show", "HEAD:b.txt"]), "b changed");
    assert_eq!(git(root, &["diff", "--cached", "--name-only"]), "b.txt");
}

#[test]
fn refuses_when_the_branch_moved_after_the_plan() {
    let repo = three_commits();
    let root = repo.path();
    let client = GitClient::new();
    write(root, "a.txt", "one\nTWO\nthree\nfour\nFIVE\n");
    git(root, &["add", "a.txt"]);
    let plan = plan_absorb(&client, root).unwrap();

    commit_only(root, "d.txt", "d\n", "Add d");
    let moved_head = head(root);
    let before = subjects(root);
    let error = apply_absorb(&client, &plan, &all_targets(&plan), root).unwrap_err();

    assert!(error.to_string().contains("changed"), "{error}");
    assert_eq!(head(root), moved_head);
    assert_eq!(subjects(root), before);
}

#[test]
fn refuses_when_the_staged_changes_changed_after_the_plan() {
    let repo = three_commits();
    let root = repo.path();
    let client = GitClient::new();
    write(root, "a.txt", "one\nTWO\nthree\nfour\nFIVE\n");
    git(root, &["add", "a.txt"]);
    let plan = plan_absorb(&client, root).unwrap();

    write(root, "a.txt", "one\nTwo\nthree\nfour\nFIVE\n");
    git(root, &["add", "a.txt"]);
    let before_head = head(root);
    let error = apply_absorb(&client, &plan, &all_targets(&plan), root).unwrap_err();

    assert!(error.to_string().contains("Review them again"), "{error}");
    assert_eq!(head(root), before_head);
}

#[test]
fn refuses_when_nothing_is_staged() {
    let repo = three_commits();
    let client = GitClient::new();
    let error = plan_absorb(&client, repo.path()).unwrap_err();
    assert!(error.to_string().contains("Stage the changes"), "{error}");
}

#[test]
fn refuses_on_a_detached_head() {
    let repo = three_commits();
    let root = repo.path();
    let client = GitClient::new();
    write(root, "a.txt", "one\nTWO\nthree\nfour\nFIVE\n");
    git(root, &["add", "a.txt"]);
    git(root, &["checkout", "-q", "--detach"]);

    assert!(plan_absorb(&client, root).is_err());
}
