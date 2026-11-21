//! Integration tests for stacked branches: finding stacks, restacking branches after a lower one
//! changes, and pushing a stack, run against real repositories in temporary directories.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use nicegit_core::client::GitClient;
use nicegit_core::stack::{find_stacks, outdated_children, plan_stack_push, push_stack, restack};
use nicegit_core::undo::UndoStep;
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
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Sets the identity and signing options in the repository's own config only.
fn configure(directory: &Path) {
    for (key, value) in [
        ("user.name", "Test"),
        ("user.email", "test@example.com"),
        ("commit.gpgsign", "false"),
        ("tag.gpgsign", "false"),
        ("core.autocrlf", "false"),
    ] {
        git(directory, &["config", key, value]);
    }
}

/// A bare repository on branch `main` to act as a remote.
fn bare_remote() -> TempDir {
    let remote = tempfile::tempdir().expect("create temp dir");
    git(remote.path(), &["init", "--bare", "-b", "main"]);
    remote
}

/// Writes `files` (name and content), stages them, and commits them as `author` with `message`.
fn commit_as(directory: &Path, files: &[(&str, &str)], message: &str, author: &str) {
    for (name, content) in files {
        fs::write(directory.join(name), content).expect("write file");
        git(directory, &["add", "--", name]);
    }
    let output = Command::new("git")
        .args(["commit", "-q", "-m", message])
        .env("GIT_AUTHOR_NAME", author)
        .env("GIT_AUTHOR_EMAIL", "writer@example.com")
        .env("GIT_AUTHOR_DATE", "1600000000 +0000")
        .current_dir(directory)
        .output()
        .expect("run git commit");
    assert!(output.status.success(), "commit failed: {}", String::from_utf8_lossy(&output.stderr));
}

/// A repository on `main` with `base.txt`, and the stack main → a → b → c:
/// `a` has two commits (`a1.txt`, and `shared.txt` set to "a2"), `b` has two (`b1.txt`, and
/// `shared.txt` set to "b1"), and `c` has one (`c1.txt`). `shared.txt` is what conflicts can be built from.
fn stack_repository() -> TempDir {
    let dir = tempfile::tempdir().expect("create temp dir");
    let root = dir.path();
    git(root, &["init", "-q", "-b", "main"]);
    configure(root);
    commit_as(root, &[("base.txt", "base\n")], "Base", "Test");
    git(root, &["checkout", "-q", "-b", "a"]);
    commit_as(root, &[("a1.txt", "a1\n")], "A1", "Writer");
    commit_as(root, &[("shared.txt", "a2\n")], "A2", "Writer");
    git(root, &["checkout", "-q", "-b", "b"]);
    commit_as(root, &[("b1.txt", "b1\n")], "B1", "Writer");
    commit_as(root, &[("shared.txt", "b2\n")], "B2", "Writer");
    git(root, &["checkout", "-q", "-b", "c"]);
    commit_as(root, &[("c1.txt", "c1\n")], "C1", "Writer");
    dir
}

fn tip(directory: &Path, branch: &str) -> String {
    git(directory, &["rev-parse", &format!("refs/heads/{branch}")]).trim().to_string()
}

/// Amends the tip of `a` so that its `a2.txt`-style change is rewritten. `shared.txt` is left alone.
fn amend_a(root: &Path, content: &str) {
    git(root, &["checkout", "-q", "a"]);
    fs::write(root.join("a1.txt"), content).expect("write file");
    git(root, &["add", "--", "a1.txt"]);
    git(root, &["commit", "-q", "--amend", "--no-edit"]);
}

fn subjects(directory: &Path, revision: &str) -> String {
    git(directory, &["log", "--format=%s", "--reverse", revision])
}

// MARK: Finding stacks

#[test]
fn find_stacks_orders_a_chain_from_the_bottom() {
    let dir = stack_repository();
    let client = GitClient::new();
    let stacks = find_stacks(&client, dir.path()).unwrap();
    assert_eq!(stacks.len(), 1);
    let names: Vec<(&str, Option<&str>)> =
        stacks[0].branches.iter().map(|branch| (branch.name.as_str(), branch.parent.as_deref())).collect();
    assert_eq!(names, vec![("a", None), ("b", Some("a")), ("c", Some("b"))]);
    assert_eq!(stacks[0].branches[1].tip, tip(dir.path(), "b"));
}

#[test]
fn a_branch_on_the_base_is_not_a_stack() {
    let dir = stack_repository();
    git(dir.path(), &["checkout", "-q", "main"]);
    git(dir.path(), &["branch", "solo", "main"]);
    let client = GitClient::new();
    // The stack is still found; a lone branch on the base is not listed.
    let stacks = find_stacks(&client, dir.path()).unwrap();
    assert_eq!(stacks.len(), 1);
    assert!(stacks.iter().all(|stack| stack.branches.iter().all(|branch| branch.name != "solo")));
}

// MARK: Outdated children

#[test]
fn an_amended_branch_reports_the_child_still_based_on_its_old_tip() {
    let dir = stack_repository();
    let root = dir.path();
    let old_a = tip(root, "a");
    amend_a(root, "a1 amended\n");
    let client = GitClient::new();
    let children = outdated_children(&client, "a", root).unwrap();
    // `c` is built on `b`, so only `b` is listed; `b` is based on the old tip of `a`.
    assert_eq!(children.len(), 1, "{children:?}");
    assert_eq!(children[0].name, "b");
    assert_eq!(children[0].old_base, old_a);
    assert_eq!(children[0].tip, tip(root, "b"));
    assert_eq!(children[0].commits, 2);
}

#[test]
fn a_branch_with_no_earlier_tip_has_no_outdated_children() {
    let dir = stack_repository();
    let client = GitClient::new();
    assert!(outdated_children(&client, "c", dir.path()).unwrap().is_empty());
}

// MARK: Restacking

#[test]
fn restack_moves_each_branch_and_keeps_its_commits() {
    let dir = stack_repository();
    let root = dir.path();
    let client = GitClient::new();
    amend_a(root, "a1 amended\n");
    let new_a = tip(root, "a");
    let children = outdated_children(&client, "a", root).unwrap();
    let old_b = tip(root, "b");
    let old_c = tip(root, "c");
    let moved = restack(&client, "a", &new_a, &children, root).unwrap();

    assert_eq!(moved.iter().map(|moved| moved.branch.as_str()).collect::<Vec<_>>(), vec!["b", "c"]);
    assert_eq!(tip(root, "b"), moved[0].after);
    assert_eq!(tip(root, "c"), moved[1].after);
    assert_ne!(tip(root, "b"), old_b);
    assert_ne!(tip(root, "c"), old_c);
    // History is linear and sits on the new tips, with the same messages and authors.
    assert_eq!(git(root, &["rev-list", "--merges", "b"]), "");
    assert_eq!(git(root, &["rev-parse", "b~2"]).trim(), new_a);
    assert_eq!(git(root, &["rev-parse", "c~1"]).trim(), tip(root, "b"));
    assert_eq!(subjects(root, &format!("{new_a}..c")), "B1\nB2\nC1\n");
    assert_eq!(
        git(root, &["log", "-1", "--format=%an <%ae> %ad", "--date=raw", "b"]).trim(),
        "Writer <writer@example.com> 1600000000 +0000"
    );
    assert_eq!(git(root, &["show", "b:shared.txt"]), "b2\n");
    assert_eq!(git(root, &["show", "c:c1.txt"]), "c1\n");
    // The working tree is on `a`, which did not move, so nothing in it changed.
    assert_eq!(git(root, &["status", "--porcelain=v1"]), "");
    assert_eq!(fs::read_to_string(root.join("a1.txt")).unwrap(), "a1 amended\n");
}

#[test]
fn a_checked_out_branch_moves_with_its_files() {
    let dir = stack_repository();
    let root = dir.path();
    let client = GitClient::new();
    amend_a(root, "a1 amended\n");
    let new_a = tip(root, "a");
    let children = outdated_children(&client, "a", root).unwrap();
    git(root, &["checkout", "-q", "c"]);
    restack(&client, "a", &new_a, &children, root).unwrap();

    assert_eq!(git(root, &["rev-parse", "--abbrev-ref", "HEAD"]).trim(), "c");
    assert_eq!(git(root, &["rev-parse", "c~1"]).trim(), tip(root, "b"));
    assert_eq!(fs::read_to_string(root.join("c1.txt")).unwrap(), "c1\n");
    assert_eq!(fs::read_to_string(root.join("shared.txt")).unwrap(), "b2\n");
    assert_eq!(git(root, &["status", "--porcelain=v1"]), "");
}

#[test]
fn a_dirty_checked_out_branch_stops_the_restack_and_moves_nothing() {
    let dir = stack_repository();
    let root = dir.path();
    let client = GitClient::new();
    amend_a(root, "a1 amended\n");
    let new_a = tip(root, "a");
    let children = outdated_children(&client, "a", root).unwrap();
    git(root, &["checkout", "-q", "c"]);
    fs::write(root.join("c1.txt"), "edited\n").unwrap();
    let before = (tip(root, "b"), tip(root, "c"));

    assert!(restack(&client, "a", &new_a, &children, root).is_err());
    assert_eq!((tip(root, "b"), tip(root, "c")), before);
    assert_eq!(fs::read_to_string(root.join("c1.txt")).unwrap(), "edited\n");
}

#[test]
fn a_conflicting_replay_changes_no_ref() {
    let dir = stack_repository();
    let root = dir.path();
    let client = GitClient::new();
    git(root, &["checkout", "-q", "a"]);
    fs::write(root.join("shared.txt"), "a2 amended\n").unwrap();
    git(root, &["commit", "-q", "-a", "--amend", "--no-edit"]);
    let new_a = tip(root, "a");
    let children = outdated_children(&client, "a", root).unwrap();
    assert_eq!(children.iter().map(|child| child.name.as_str()).collect::<Vec<_>>(), vec!["b"]);
    let before = (tip(root, "b"), tip(root, "c"));

    let error = restack(&client, "a", &new_a, &children, root).unwrap_err().to_string();
    assert!(error.contains("Restacking b would conflict at"), "{error}");
    assert_eq!((tip(root, "b"), tip(root, "c")), before);
}

#[test]
fn a_moved_child_tip_stops_the_restack() {
    let dir = stack_repository();
    let root = dir.path();
    let client = GitClient::new();
    amend_a(root, "a1 amended\n");
    let new_a = tip(root, "a");
    let mut children = outdated_children(&client, "a", root).unwrap();
    children[0].tip = children[0].old_base.clone();
    assert!(restack(&client, "a", &new_a, &children, root).is_err());
}

#[test]
fn undo_puts_every_moved_branch_back() {
    let dir = stack_repository();
    let root = dir.path();
    let client = GitClient::new();
    let (old_b, old_c) = (tip(root, "b"), tip(root, "c"));
    amend_a(root, "a1 amended\n");
    let new_a = tip(root, "a");
    let children = outdated_children(&client, "a", root).unwrap();
    let moved = restack(&client, "a", &new_a, &children, root).unwrap();
    let redo = client.undo_step(&UndoStep::BranchMoves(moved.clone()), root).unwrap();
    assert_eq!((tip(root, "b"), tip(root, "c")), (old_b.clone(), old_c.clone()));
    // The step Undo returns redoes the restack.
    client.undo_step(&redo, root).unwrap();
    assert_eq!((tip(root, "b"), tip(root, "c")), (moved[0].after.clone(), moved[1].after.clone()));
}

// MARK: Pushing a stack

/// Pushes a, b, and c to a new bare remote with upstreams set, and returns the remote.
fn pushed_stack() -> (TempDir, TempDir) {
    let dir = stack_repository();
    let remote = bare_remote();
    let root = dir.path();
    git(root, &["remote", "add", "origin", remote.path().to_str().unwrap()]);
    for branch in ["a", "b", "c"] {
        git(root, &["push", "-q", "-u", "origin", &format!("refs/heads/{branch}:refs/heads/{branch}")]);
    }
    (dir, remote)
}

fn remote_tip(remote: &Path, branch: &str) -> String {
    git(remote, &["rev-parse", &format!("refs/heads/{branch}")]).trim().to_string()
}

#[test]
fn push_stack_sends_every_branch_and_leases_the_rewritten_ones() {
    let (dir, remote) = pushed_stack();
    let root = dir.path();
    let client = GitClient::new();
    amend_a(root, "a1 amended\n");
    let new_a = tip(root, "a");
    let children = outdated_children(&client, "a", root).unwrap();
    restack(&client, "a", &new_a, &children, root).unwrap();

    let names: Vec<String> = ["a", "b", "c"].iter().map(|name| name.to_string()).collect();
    let pushes = plan_stack_push(&client, &names, root).unwrap();
    assert_eq!(pushes.iter().map(|push| push.forced).collect::<Vec<_>>(), vec![true, true, true]);
    let expected = std::collections::BTreeMap::from([("origin".to_string(), vec![remote.path().to_string_lossy().into_owned()])]);
    push_stack(&client, &pushes, &expected, root).unwrap();

    for branch in ["a", "b", "c"] {
        assert_eq!(remote_tip(remote.path(), branch), tip(root, branch), "{branch} was pushed");
    }
}

#[test]
fn a_remote_branch_moved_by_someone_else_fails_the_whole_push() {
    let (dir, remote) = pushed_stack();
    let root = dir.path();
    let client = GitClient::new();
    amend_a(root, "a1 amended\n");
    let new_a = tip(root, "a");
    let children = outdated_children(&client, "a", root).unwrap();
    restack(&client, "a", &new_a, &children, root).unwrap();
    let names: Vec<String> = ["a", "b", "c"].iter().map(|name| name.to_string()).collect();
    let pushes = plan_stack_push(&client, &names, root).unwrap();

    // Someone else pushes a new commit to `c` on the remote.
    let other = tempfile::tempdir().unwrap();
    git(other.path(), &["clone", "-q", remote.path().to_str().unwrap(), "."]);
    configure(other.path());
    git(other.path(), &["checkout", "-q", "c"]);
    commit_as(other.path(), &[("other.txt", "other\n")], "Someone else", "Other");
    git(other.path(), &["push", "-q", "origin", "c"]);
    let c_on_remote = remote_tip(remote.path(), "c");
    let a_before = remote_tip(remote.path(), "a");
    let b_before = remote_tip(remote.path(), "b");

    let expected = std::collections::BTreeMap::from([("origin".to_string(), vec![remote.path().to_string_lossy().into_owned()])]);
    assert!(push_stack(&client, &pushes, &expected, root).is_err());
    assert_eq!(remote_tip(remote.path(), "c"), c_on_remote);
    assert_eq!(remote_tip(remote.path(), "a"), a_before, "the atomic push left a alone");
    assert_eq!(remote_tip(remote.path(), "b"), b_before, "the atomic push left b alone");
}
