//! Integration tests for `GitClient`, run against real repositories in temporary directories.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use nicegit_core::client::GitClient;
use nicegit_core::diff::{DiffContext, DiffRequest};
use nicegit_core::models::{
    Branch, GitError, HistoryScope, MergeStyle, Operation, Snapshot, StashFile, StashOptions, StatusEntry, StatusKind,
};
use tempfile::TempDir;

/// The stash options the app used before they were chosen: untracked files are stashed too.
const DEFAULT_STASH: StashOptions = StashOptions { keep_index: false, include_untracked: true };

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

fn branch_exists(directory: &Path, name: &str) -> bool {
    git_output(directory, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{name}")]).status.success()
}

fn tag_exists(directory: &Path, name: &str) -> bool {
    git_output(directory, &["rev-parse", "--verify", "--quiet", &format!("refs/tags/{name}")]).status.success()
}

/// Sets the identity and signing options in the repository's own config only.
fn configure(directory: &Path) {
    for (key, value) in [
        ("user.name", "Test"),
        ("user.email", "test@example.com"),
        ("commit.gpgsign", "false"),
        ("tag.gpgsign", "false"),
        // Keeps line endings byte-for-byte on Windows, where Git may default to autocrlf.
        ("core.autocrlf", "false"),
    ] {
        git(directory, &["config", key, value]);
    }
}

/// A temporary repository on branch `main`.
struct Repo {
    dir: TempDir,
}

impl Repo {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("create temp dir");
        git(dir.path(), &["init", "-b", "main"]);
        configure(dir.path());
        Repo { dir }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn git(&self, arguments: &[&str]) -> String {
        git(self.path(), arguments)
    }

    fn write(&self, relative: &str, contents: &str) {
        let file = self.path().join(relative);
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent).expect("create parent folder");
        }
        fs::write(file, contents).expect("write file");
    }

    fn read(&self, relative: &str) -> String {
        fs::read_to_string(self.path().join(relative)).expect("read file")
    }

    fn head(&self) -> String {
        self.git(&["rev-parse", "HEAD"]).trim().to_string()
    }

    /// Writes, stages, and commits one file, returning the new HEAD.
    fn commit(&self, relative: &str, contents: &str, message: &str) -> String {
        self.write(relative, contents);
        self.git(&["add", "--", relative]);
        self.git(&["commit", "--quiet", "-m", message]);
        self.head()
    }
}

/// The message of an expected refusal, panicking if the call succeeded.
fn error_text<T: std::fmt::Debug>(result: Result<T, GitError>) -> String {
    match result {
        Ok(value) => panic!("expected a refusal, got Ok({value:?})"),
        Err(error) => error.to_string(),
    }
}

fn client() -> GitClient {
    GitClient::new()
}

fn snapshot(directory: &Path) -> Snapshot {
    client().load_snapshot(directory, 100).expect("load snapshot")
}

fn entry(directory: &Path, path: &str) -> StatusEntry {
    client()
        .load_status(directory)
        .expect("load status")
        .into_iter()
        .find(|entry| entry.path == path)
        .unwrap_or_else(|| panic!("no status entry for {path}"))
}

fn local_branch(snapshot: &Snapshot, name: &str) -> Branch {
    snapshot
        .branches
        .iter()
        .find(|branch| !branch.is_remote && branch.name == name)
        .cloned()
        .unwrap_or_else(|| panic!("no local branch {name}"))
}

/// Leaves `main` with a conflicted merge of `side` in progress. Returns the commit `main` was on.
fn start_conflicted_merge(repo: &Repo) -> String {
    let base = repo.commit("a.txt", "base\n", "Base");
    repo.git(&["switch", "-c", "side"]);
    repo.commit("a.txt", "side\n", "Side change");
    repo.git(&["switch", "main"]);
    repo.commit("a.txt", "main\n", "Main change");
    let merge = git_output(repo.path(), &["merge", "--no-edit", "side"]);
    assert!(!merge.status.success(), "the merge was expected to conflict");
    base
}

// MARK: Snapshot

#[test]
fn snapshot_of_one_commit_repository_is_clean() {
    let repo = Repo::new();
    let head = repo.commit("a.txt", "one\n", "First");

    let snap = snapshot(repo.path());

    assert_eq!(snap.current_branch, "main");
    assert_eq!(snap.head_hash.as_deref(), Some(head.as_str()));
    assert_eq!(snap.commits.len(), 1);
    assert_eq!(snap.commits[0].subject, "First");
    assert!(snap.status.is_empty());
    assert_eq!(snap.operation, None);
    assert!(snap.is_on_branch());
}

#[test]
fn snapshot_reports_modified_and_untracked_files() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.write("a.txt", "two\n");
    repo.write("new.txt", "fresh\n");

    let snap = snapshot(repo.path());

    assert_eq!(snap.status.len(), 2);
    let modified = snap.status.iter().find(|entry| entry.path == "a.txt").expect("modified entry");
    assert_eq!(modified.kind, StatusKind::Modified);
    assert!(modified.is_unstaged());
    assert!(!modified.is_staged());
    let untracked = snap.status.iter().find(|entry| entry.path == "new.txt").expect("untracked entry");
    assert_eq!(untracked.kind, StatusKind::Untracked);
}

#[test]
fn snapshot_of_repository_without_commits_loads() {
    let repo = Repo::new();
    repo.write("draft.txt", "not yet committed\n");

    let snap = snapshot(repo.path());

    assert_eq!(snap.current_branch, "main");
    assert_eq!(snap.head_hash, None);
    assert!(snap.commits.is_empty());
    assert_eq!(snap.status.len(), 1);
}

#[test]
fn snapshot_of_detached_head_names_detached_state() {
    let repo = Repo::new();
    let first = repo.commit("a.txt", "one\n", "First");
    repo.commit("a.txt", "two\n", "Second");
    repo.git(&["checkout", "--detach", &first]);

    let snap = snapshot(repo.path());

    assert!(snap.current_branch.starts_with("Detached HEAD"), "got {:?}", snap.current_branch);
    assert_eq!(snap.head_hash.as_deref(), Some(first.as_str()));
    assert!(!snap.is_on_branch());
}

#[test]
fn snapshot_reports_merge_operation_during_conflict() {
    let repo = Repo::new();
    start_conflicted_merge(&repo);

    let snap = snapshot(repo.path());

    assert_eq!(snap.operation, Some(Operation::Merge));
    assert!(snap.status.iter().any(|entry| entry.kind == StatusKind::Conflicted));
}

// MARK: Changes

#[test]
fn stage_and_unstage_one_file() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.write("b.txt", "bee\n");

    client().stage("b.txt", repo.path()).expect("stage");
    let staged = entry(repo.path(), "b.txt");
    assert!(staged.is_staged());
    assert_eq!(staged.kind, StatusKind::Added);

    client().unstage(&staged, repo.path()).expect("unstage");
    let after = entry(repo.path(), "b.txt");
    assert_eq!(after.kind, StatusKind::Untracked);
    assert!(repo.path().join("b.txt").exists());
}

#[test]
fn stage_all_and_unstage_all() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.write("x.txt", "x\n");
    repo.write("sub/y.txt", "y\n");

    client().stage_all(repo.path()).expect("stage all");
    assert_eq!(snapshot(repo.path()).staged_count(), 2);

    client().unstage_all(repo.path()).expect("unstage all");
    let snap = snapshot(repo.path());
    assert_eq!(snap.staged_count(), 0);
    assert_eq!(snap.status.len(), 2);
    assert!(snap.status.iter().all(|entry| entry.kind == StatusKind::Untracked));
}

#[test]
fn unstage_before_first_commit_keeps_working_file() {
    let repo = Repo::new();
    repo.write("first.txt", "draft\n");
    client().stage("first.txt", repo.path()).expect("stage");
    let staged = entry(repo.path(), "first.txt");
    assert!(staged.is_staged());

    client().unstage(&staged, repo.path()).expect("unstage before first commit");

    assert!(repo.path().join("first.txt").exists());
    assert_eq!(repo.read("first.txt"), "draft\n");
    assert_eq!(entry(repo.path(), "first.txt").kind, StatusKind::Untracked);
}

#[test]
fn discard_restores_modified_file() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.write("a.txt", "two\n");
    let selected = entry(repo.path(), "a.txt");

    client().discard(&selected, repo.path()).expect("discard");

    assert_eq!(repo.read("a.txt"), "one\n");
    assert!(client().load_status(repo.path()).unwrap().is_empty());
}

#[test]
fn discard_removes_untracked_file() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.write("scratch.log", "temporary\n");
    let selected = entry(repo.path(), "scratch.log");

    client().discard(&selected, repo.path()).expect("discard untracked");

    assert!(!repo.path().join("scratch.log").exists());
    assert!(client().load_status(repo.path()).unwrap().is_empty());
}

#[test]
fn discard_removes_newly_staged_file() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.write("added.txt", "new\n");
    client().stage("added.txt", repo.path()).expect("stage");
    let selected = entry(repo.path(), "added.txt");

    client().discard(&selected, repo.path()).expect("discard staged new file");

    assert!(!repo.path().join("added.txt").exists());
    assert!(client().load_status(repo.path()).unwrap().is_empty());
}

#[test]
fn discard_refuses_entry_that_changed_since_selection() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.write("a.txt", "two\n");
    // Captured while the change is unstaged, then staged by another action.
    let stale = entry(repo.path(), "a.txt");
    client().stage("a.txt", repo.path()).expect("stage");

    let result = client().discard(&stale, repo.path());

    assert!(error_text(result).contains("changed since it was selected"));
    assert_eq!(repo.read("a.txt"), "two\n");
    assert!(entry(repo.path(), "a.txt").is_staged());
}

#[cfg(not(windows))]
#[test]
fn discard_of_glob_named_file_leaves_other_untracked_files() {
    // `*` is not allowed in Windows file names, so this case only runs elsewhere.
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.write("*.txt", "literal name\n");
    repo.write("other.txt", "keep me\n");
    repo.write("notes.txt", "keep me too\n");
    let selected = entry(repo.path(), "*.txt");

    client().discard(&selected, repo.path()).expect("discard glob-named file");

    assert!(!repo.path().join("*.txt").exists());
    assert_eq!(repo.read("other.txt"), "keep me\n");
    assert_eq!(repo.read("notes.txt"), "keep me too\n");
}

// MARK: Commits

#[test]
fn commit_rejects_empty_message_and_adds_history() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.write("b.txt", "two\n");
    client().stage("b.txt", repo.path()).expect("stage");

    assert_eq!(client().commit("   ", repo.path()), Err(GitError::EmptyCommitMessage));
    assert_eq!(snapshot(repo.path()).commits.len(), 1);

    client().commit("Second", repo.path()).expect("commit");
    let snap = snapshot(repo.path());
    assert_eq!(snap.commits.len(), 2);
    assert_eq!(snap.commits[0].subject, "Second");
    assert!(snap.status.is_empty());
}

#[test]
fn undo_last_commit_keeps_changes_staged() {
    let repo = Repo::new();
    let first = repo.commit("a.txt", "one\n", "First");
    let second = repo.commit("a.txt", "two\n", "Second");

    let parent = client().undo_last_commit("main", &second, repo.path()).expect("undo");

    assert_eq!(parent, first);
    assert_eq!(repo.head(), first);
    let change = entry(repo.path(), "a.txt");
    assert!(change.is_staged());
    assert_eq!(change.index_status, 'M');
    assert_eq!(repo.read("a.txt"), "two\n");
}

#[test]
fn undo_last_commit_refuses_stale_head() {
    let repo = Repo::new();
    let first = repo.commit("a.txt", "one\n", "First");
    let second = repo.commit("a.txt", "two\n", "Second");

    let result = client().undo_last_commit("main", &first, repo.path());

    assert!(error_text(result).contains("changed since this action was selected"));
    assert_eq!(repo.head(), second);
}

// MARK: Checkout

#[test]
fn checkout_with_dirty_tree_stashes_and_switches() {
    let repo = Repo::new();
    let base = repo.commit("a.txt", "one\n", "First");
    repo.git(&["branch", "feature"]);
    repo.write("a.txt", "two\n");
    repo.write("untracked.txt", "new\n");
    let snap = snapshot(repo.path());

    let switched = client().checkout(&local_branch(&snap, "feature"), "main", Some(&base), repo.path()).expect("checkout");

    assert!(switched);
    let stashes = client().list_stashes(repo.path()).unwrap();
    assert_eq!(stashes.len(), 1);
    assert!(stashes[0].message.contains("NiceGit"), "stash message was {:?}", stashes[0].message);
    assert!(client().load_status(repo.path()).unwrap().is_empty());
    assert!(!repo.path().join("untracked.txt").exists());
    assert_eq!(repo.git(&["branch", "--show-current"]).trim(), "feature");
}

#[test]
fn checkout_with_clean_tree_reports_no_stash() {
    let repo = Repo::new();
    let base = repo.commit("a.txt", "one\n", "First");
    repo.git(&["branch", "feature"]);
    let snap = snapshot(repo.path());

    let switched = client().checkout(&local_branch(&snap, "feature"), "main", Some(&base), repo.path());

    assert_eq!(switched, Ok(false));
    assert!(client().list_stashes(repo.path()).unwrap().is_empty());
    assert_eq!(repo.git(&["branch", "--show-current"]).trim(), "feature");
}

#[test]
fn checkout_refuses_stale_head_without_stashing() {
    let repo = Repo::new();
    let first = repo.commit("a.txt", "one\n", "First");
    repo.commit("a.txt", "two\n", "Second");
    repo.git(&["branch", "feature"]);
    repo.write("a.txt", "dirty\n");
    let snap = snapshot(repo.path());

    let result = client().checkout(&local_branch(&snap, "feature"), "main", Some(&first), repo.path());

    assert!(error_text(result).contains("changed since this action was selected"));
    assert!(client().list_stashes(repo.path()).unwrap().is_empty());
    assert_eq!(repo.read("a.txt"), "dirty\n");
    assert_eq!(repo.git(&["branch", "--show-current"]).trim(), "main");
}

#[test]
fn checkout_refuses_during_merge_without_stashing() {
    let repo = Repo::new();
    start_conflicted_merge(&repo);
    repo.git(&["branch", "other"]);
    let snap = snapshot(repo.path());
    assert_eq!(snap.operation, Some(Operation::Merge));

    let result = client().checkout(&local_branch(&snap, "other"), "main", snap.head_hash.as_deref(), repo.path());

    assert!(error_text(result).contains("Finish or abort"));
    assert!(client().list_stashes(repo.path()).unwrap().is_empty());
    assert_eq!(repo.git(&["branch", "--show-current"]).trim(), "main");
    assert!(repo.path().join(".git").join("MERGE_HEAD").exists());
}

#[test]
fn create_branch_checks_out_new_branch_at_head() {
    let repo = Repo::new();
    let head = repo.commit("a.txt", "one\n", "First");

    client().create_branch("topic", "main", Some(&head), repo.path()).expect("create branch");

    assert_eq!(repo.git(&["branch", "--show-current"]).trim(), "topic");
    assert_eq!(repo.head(), head);
}

#[test]
fn create_branch_rejects_empty_name() {
    let repo = Repo::new();
    let head = repo.commit("a.txt", "one\n", "First");

    let result = client().create_branch("   ", "main", Some(&head), repo.path());

    assert_eq!(result, Err(GitError::EmptyBranchName));
    assert_eq!(repo.git(&["branch", "--show-current"]).trim(), "main");
}

// MARK: Branch deletion and merge

#[test]
fn delete_merged_branch_without_force() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.git(&["branch", "merged"]);
    let snap = snapshot(repo.path());

    client().delete_branch(&local_branch(&snap, "merged"), false, repo.path()).expect("delete merged branch");

    assert!(!branch_exists(repo.path(), "merged"));
}

#[test]
fn force_deletes_unmerged_branch() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.git(&["switch", "-c", "unmerged"]);
    repo.commit("b.txt", "work\n", "Unmerged work");
    repo.git(&["switch", "main"]);
    let snap = snapshot(repo.path());
    let branch = local_branch(&snap, "unmerged");

    assert!(client().delete_branch(&branch, false, repo.path()).is_err());
    assert!(branch_exists(repo.path(), "unmerged"));

    client().delete_branch(&branch, true, repo.path()).expect("force delete");
    assert!(!branch_exists(repo.path(), "unmerged"));
}

#[test]
fn delete_with_stale_tip_keeps_branch() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.git(&["branch", "moving"]);
    let shown = local_branch(&snapshot(repo.path()), "moving");
    repo.git(&["switch", "moving"]);
    repo.commit("b.txt", "moved\n", "Moved on");
    repo.git(&["switch", "main"]);

    assert!(error_text(client().delete_branch(&shown, true, repo.path())).contains("This branch changed"));
    assert!(client().delete_branch(&shown, false, repo.path()).is_err());
    assert!(branch_exists(repo.path(), "moving"));
}

#[test]
fn merge_refuses_branch_whose_tip_moved_after_selection() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.git(&["branch", "feature"]);
    let shown = local_branch(&snapshot(repo.path()), "feature");
    repo.git(&["switch", "feature"]);
    repo.commit("b.txt", "feature work\n", "Feature work");
    repo.git(&["switch", "main"]);
    let main_head = repo.head();

    let result = client().merge(&shown, "main", Some(&main_head), MergeStyle::Default, repo.path());

    assert!(error_text(result).contains("selected branch changed"));
    assert_eq!(repo.head(), main_head);

    // With the current tip shown again, the same merge succeeds.
    let fresh = local_branch(&snapshot(repo.path()), "feature");
    client().merge(&fresh, "main", Some(&main_head), MergeStyle::Default, repo.path()).expect("merge fresh tip");
    assert!(repo.path().join("b.txt").exists());
}

/// A `main` with one commit, and a `feature` one commit ahead of it. `main` is still checked out.
fn fast_forwardable_feature(repo: &Repo) {
    repo.commit("a.txt", "one\n", "First");
    repo.git(&["switch", "-c", "feature"]);
    repo.commit("b.txt", "feature work\n", "Feature work");
    repo.git(&["switch", "main"]);
}

/// A `main` and a `feature` that have each gained a commit since they split. `main` is checked out.
fn diverged_feature(repo: &Repo) {
    repo.commit("a.txt", "one\n", "First");
    repo.git(&["switch", "-c", "feature"]);
    repo.commit("b.txt", "feature work\n", "Feature work");
    repo.git(&["switch", "main"]);
    repo.commit("c.txt", "main work\n", "Main work");
}

fn has_merge_head(directory: &Path) -> bool {
    git_output(directory, &["rev-parse", "--verify", "--quiet", "MERGE_HEAD"]).status.success()
}

#[test]
fn merge_default_fast_forwards_a_branch_that_is_ahead() {
    let repo = Repo::new();
    fast_forwardable_feature(&repo);
    let shown = local_branch(&snapshot(repo.path()), "feature");

    client().merge(&shown, "main", Some(&repo.head()), MergeStyle::Default, repo.path()).expect("fast-forward merge");

    assert_eq!(repo.head(), shown.tip);
    assert!(!git_output(repo.path(), &["rev-parse", "--verify", "--quiet", "HEAD^2"]).status.success(), "fast-forward has one parent");
}

#[test]
fn merge_no_fast_forward_records_a_merge_commit() {
    let repo = Repo::new();
    fast_forwardable_feature(&repo);
    let shown = local_branch(&snapshot(repo.path()), "feature");
    let main_head = repo.head();

    client().merge(&shown, "main", Some(&main_head), MergeStyle::NoFastForward, repo.path()).expect("merge commit");

    assert_eq!(repo.git(&["rev-parse", "HEAD^1"]).trim(), main_head);
    assert_eq!(repo.git(&["rev-parse", "HEAD^2"]).trim(), shown.tip);
}

#[test]
fn squash_stages_a_diverged_branch_without_moving_head() {
    let repo = Repo::new();
    diverged_feature(&repo);
    let shown = local_branch(&snapshot(repo.path()), "feature");
    let main_head = repo.head();

    client().merge(&shown, "main", Some(&main_head), MergeStyle::Squash, repo.path()).expect("squash");

    assert_eq!(repo.head(), main_head);
    assert_eq!(repo.git(&["diff", "--cached", "--name-only"]).trim(), "b.txt");
    assert!(!has_merge_head(repo.path()));
    assert!(git_output(repo.path(), &["rev-parse", "--git-path", "SQUASH_MSG"]).status.success());
    // The working tree holds the merge result, with nothing left unstaged.
    assert_eq!(repo.read("b.txt"), "feature work\n");
    assert_eq!(repo.read("c.txt"), "main work\n");
    assert!(repo.git(&["diff", "--name-only"]).trim().is_empty());
}

#[test]
fn squash_refuses_a_branch_that_is_already_contained() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.git(&["branch", "behind"]);
    repo.commit("b.txt", "two\n", "Second");
    let shown = local_branch(&snapshot(repo.path()), "behind");
    let main_head = repo.head();

    let result = client().merge(&shown, "main", Some(&main_head), MergeStyle::Squash, repo.path());

    assert!(error_text(result).contains("Already up to date: there is nothing to squash."));
    assert_eq!(repo.head(), main_head);
    assert!(repo.git(&["status", "--porcelain"]).is_empty());
}

#[test]
fn squash_of_a_branch_that_brings_no_change_is_refused_and_leaves_no_message() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.git(&["switch", "-c", "feature"]);
    repo.commit("b.txt", "two\n", "Feature");
    repo.git(&["switch", "main"]);
    // The same change, made again on main: the histories differ, but nothing is left to bring.
    repo.commit("b.txt", "two\n", "Same change on main");
    let shown = local_branch(&snapshot(repo.path()), "feature");
    let main_head = repo.head();

    let result = client().merge(&shown, "main", Some(&main_head), MergeStyle::Squash, repo.path());

    assert!(error_text(result).contains("brings no changes to squash"));
    assert_eq!(repo.head(), main_head);
    assert!(repo.git(&["status", "--porcelain"]).is_empty());
    let git_dir = repo.git(&["rev-parse", "--absolute-git-dir"]);
    assert!(!Path::new(git_dir.trim()).join("SQUASH_MSG").exists(), "Git's squash message is removed");
}

#[test]
fn squash_refuses_a_predicted_conflict_before_changing_anything() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.git(&["switch", "-c", "feature"]);
    repo.commit("a.txt", "feature\n", "Feature");
    repo.git(&["switch", "main"]);
    repo.commit("a.txt", "main\n", "Main");
    let shown = local_branch(&snapshot(repo.path()), "feature");
    let main_head = repo.head();

    let result = client().merge(&shown, "main", Some(&main_head), MergeStyle::Squash, repo.path());

    assert!(error_text(result).contains("This squash would conflict."));
    assert_eq!(repo.head(), main_head);
    assert!(repo.git(&["status", "--porcelain"]).is_empty());
    assert!(!has_merge_head(repo.path()));
}

#[test]
fn squash_refuses_a_moved_tip_and_a_dirty_tree() {
    let repo = Repo::new();
    diverged_feature(&repo);
    let shown = local_branch(&snapshot(repo.path()), "feature");
    let main_head = repo.head();
    repo.git(&["switch", "feature"]);
    repo.commit("d.txt", "moved\n", "Moved on");
    repo.git(&["switch", "main"]);

    let moved = client().merge(&shown, "main", Some(&main_head), MergeStyle::Squash, repo.path());
    assert!(error_text(moved).contains("selected branch changed"));
    assert!(repo.git(&["diff", "--cached", "--name-only"]).is_empty());

    let fresh = local_branch(&snapshot(repo.path()), "feature");
    repo.write("c.txt", "uncommitted\n");
    let dirty = client().merge(&fresh, "main", Some(&main_head), MergeStyle::Squash, repo.path());
    assert!(error_text(dirty).contains("Commit or stash changes"));
    assert_eq!(repo.head(), main_head);
    assert!(repo.git(&["diff", "--cached", "--name-only"]).is_empty());
}

// MARK: Stashes

#[test]
fn save_and_apply_stash_keeps_the_entry_until_dropped() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.write("a.txt", "two\n");
    repo.write("scratch.txt", "untracked\n");

    client().save_stash("my work", DEFAULT_STASH, repo.path()).expect("save stash");
    assert!(client().load_status(repo.path()).unwrap().is_empty());
    let stashes = client().list_stashes(repo.path()).unwrap();
    assert_eq!(stashes.len(), 1);
    assert!(stashes[0].message.contains("my work"), "stash message was {:?}", stashes[0].message);
    let stash = stashes[0].clone();

    client().apply_stash(&stash, repo.path()).expect("apply stash");
    assert_eq!(repo.read("a.txt"), "two\n");
    assert_eq!(repo.read("scratch.txt"), "untracked\n");
    assert_eq!(client().list_stashes(repo.path()).unwrap().len(), 1);

    client().drop_stash(&stash, repo.path()).expect("drop stash");
    assert!(client().list_stashes(repo.path()).unwrap().is_empty());
}

#[test]
fn pop_stash_restores_changes_and_removes_entry() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.write("a.txt", "two\n");
    client().save_stash("", DEFAULT_STASH, repo.path()).expect("save stash");
    assert_eq!(repo.read("a.txt"), "one\n");
    let stash = client().list_stashes(repo.path()).unwrap()[0].clone();

    client().pop_stash(&stash, repo.path()).expect("pop stash");

    assert_eq!(repo.read("a.txt"), "two\n");
    assert!(client().list_stashes(repo.path()).unwrap().is_empty());
}

#[test]
fn drop_of_stash_no_longer_listed_fails() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.write("a.txt", "two\n");
    client().save_stash("gone soon", DEFAULT_STASH, repo.path()).expect("save stash");
    let stash = client().list_stashes(repo.path()).unwrap()[0].clone();
    client().drop_stash(&stash, repo.path()).expect("first drop");

    assert!(error_text(client().drop_stash(&stash, repo.path())).contains("no longer exists"));
    assert!(client().apply_stash(&stash, repo.path()).is_err());
}

#[test]
fn save_stash_on_clean_tree_reports_error() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");

    assert!(client().save_stash("nothing", DEFAULT_STASH, repo.path()).is_err());
    assert!(client().list_stashes(repo.path()).unwrap().is_empty());
}

#[test]
fn stash_keeping_the_index_leaves_staged_changes() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.commit("b.txt", "one\n", "Second");
    repo.write("a.txt", "staged\n");
    repo.git(&["add", "--", "a.txt"]);
    repo.write("b.txt", "unstaged\n");
    let options = StashOptions { keep_index: true, include_untracked: true };

    client().save_stash("keep the index", options, repo.path()).expect("save stash");

    assert_eq!(repo.git(&["diff", "--cached", "--name-only"]).trim(), "a.txt", "a.txt is still staged");
    assert_eq!(repo.read("a.txt"), "staged\n", "its staged change is still in the working file");
    assert_eq!(repo.read("b.txt"), "one\n", "the unstaged change is stashed");
    assert!(repo.git(&["diff", "--name-only"]).trim().is_empty(), "nothing is unstaged");
    assert_eq!(client().list_stashes(repo.path()).unwrap().len(), 1, "a new stash exists");
}

#[test]
fn stash_without_untracked_leaves_new_files() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.write("a.txt", "two\n");
    repo.write("new.txt", "new\n");
    let options = StashOptions { keep_index: false, include_untracked: false };

    client().save_stash("tracked only", options, repo.path()).expect("save stash");

    assert_eq!(repo.read("new.txt"), "new\n", "the untracked file stays where it was");
    assert_eq!(repo.read("a.txt"), "one\n", "the tracked change is stashed");
    let status = client().load_status(repo.path()).unwrap();
    assert_eq!(status.iter().map(|entry| entry.path.as_str()).collect::<Vec<_>>(), vec!["new.txt"]);
    assert_eq!(client().list_stashes(repo.path()).unwrap().len(), 1, "a new stash exists");
}

#[test]
fn stash_with_a_partly_staged_file_and_keep_index() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.write("a.txt", "staged\n");
    repo.git(&["add", "--", "a.txt"]);
    repo.write("a.txt", "staged\nunstaged\n");
    let options = StashOptions { keep_index: true, include_untracked: true };

    client().save_stash("partly staged", options, repo.path()).expect("save stash");

    // Git keeps the staged content in the index and rewrites the working file to match it.
    assert_eq!(repo.git(&["show", ":a.txt"]), "staged\n", "the staged content stays staged");
    assert_eq!(repo.read("a.txt"), "staged\n", "the working file equals the index");
    let status = client().load_status(repo.path()).unwrap();
    assert_eq!(
        status.iter().map(|entry| (entry.path.as_str(), entry.index_status, entry.work_tree_status)).collect::<Vec<_>>(),
        vec![("a.txt", 'M', ' ')]
    );
    assert_eq!(client().list_stashes(repo.path()).unwrap().len(), 1, "a new stash exists");
}

/// Stashes the working changes, returning the stash entry.
fn stash_all(repo: &Repo, message: &str) -> nicegit_core::models::Stash {
    client().save_stash(message, DEFAULT_STASH, repo.path()).expect("save stash");
    client().list_stashes(repo.path()).unwrap()[0].clone()
}

fn stash_file(repo: &Repo, stash: &nicegit_core::models::Stash, path: &str) -> StashFile {
    client()
        .stash_files(stash, repo.path())
        .expect("list stash files")
        .into_iter()
        .find(|file| file.path == path)
        .unwrap_or_else(|| panic!("the stash has no file {path}"))
}

#[test]
fn stash_files_lists_tracked_and_untracked_changes() {
    let repo = Repo::new();
    repo.commit("kept.txt", "keep\n", "First");
    repo.commit("edited.txt", "one\n", "Second");
    repo.commit("gone.txt", "bye\n", "Third");
    repo.write("edited.txt", "two\n");
    fs::remove_file(repo.path().join("gone.txt")).unwrap();
    repo.write("new.txt", "fresh\n");
    let mut expected = vec![
        StashFile { status: "M".into(), path: "edited.txt".into(), untracked: false },
        StashFile { status: "D".into(), path: "gone.txt".into(), untracked: false },
        StashFile { status: "A".into(), path: "new.txt".into(), untracked: true },
    ];
    // Windows cannot have a newline in a file name, so this case runs elsewhere.
    #[cfg(not(windows))]
    {
        repo.write("line\nbreak.txt", "newline\n");
        expected.push(StashFile { status: "A".into(), path: "line\nbreak.txt".into(), untracked: true });
    }
    expected.sort_by(|a, b| a.path.cmp(&b.path));
    let stash = stash_all(&repo, "mixed");

    let files = client().stash_files(&stash, repo.path()).expect("list stash files");

    assert_eq!(files, expected);
}

#[test]
fn stash_file_diff_shows_one_file() {
    let repo = Repo::new();
    repo.commit("one.txt", "a\n", "First");
    repo.commit("two.txt", "b\n", "Second");
    repo.write("one.txt", "a changed\n");
    repo.write("two.txt", "b changed\n");
    let stash = stash_all(&repo, "two files");
    let file = stash_file(&repo, &stash, "one.txt");

    let diff = client().stash_file_diff(&stash, &file, DiffRequest::default(), repo.path()).expect("diff one file");

    assert!(diff.contains("+a changed"), "the file's change is shown: {diff}");
    assert!(!diff.contains("two.txt") && !diff.contains("b changed"), "the other file is not: {diff}");
}

#[test]
fn stash_file_diff_shows_an_untracked_file() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.write("scratch.txt", "fresh one\nfresh two\n");
    let stash = stash_all(&repo, "scratch");
    let file = stash_file(&repo, &stash, "scratch.txt");
    assert!(file.untracked);

    let diff = client().stash_file_diff(&stash, &file, DiffRequest::default(), repo.path()).expect("diff untracked file");

    assert!(diff.contains("+fresh one") && diff.contains("+fresh two"), "the content is added lines: {diff}");
    assert!(diff.contains("new file mode"), "the file is shown as new: {diff}");
}

#[cfg(not(windows))]
#[test]
fn stash_file_diff_with_glob_characters() {
    // `*` is not allowed in Windows file names, so this case only runs elsewhere.
    let repo = Repo::new();
    repo.write("a*.txt", "star one\n");
    repo.write("ab.txt", "ab one\n");
    repo.git(&["add", "--", ":(literal)a*.txt", "ab.txt"]);
    repo.git(&["commit", "-q", "-m", "First"]);
    repo.write("a*.txt", "star two\n");
    repo.write("ab.txt", "ab two\n");
    let stash = stash_all(&repo, "both");
    let file = stash_file(&repo, &stash, "a*.txt");

    let diff = client().stash_file_diff(&stash, &file, DiffRequest::default(), repo.path()).expect("diff glob-named file");

    assert!(diff.contains("+star two"), "the glob-named file's change is shown: {diff}");
    assert!(!diff.contains("ab.txt") && !diff.contains("ab two"), "the file it matches is not: {diff}");
}

#[test]
fn stash_diffs_can_ignore_whitespace() {
    let repo = Repo::new();
    repo.commit("a.txt", "a\nb\n", "First");
    repo.write("a.txt", "a  \nb changed\n");
    let stash = stash_all(&repo, "spaces");
    let file = stash_file(&repo, &stash, "a.txt");

    let plain = client().stash_diff_with(&stash, DiffRequest::default(), repo.path()).expect("whole stash");
    let quiet = client()
        .stash_diff_with(&stash, DiffRequest { ignore_whitespace: true, ..Default::default() }, repo.path())
        .expect("whole stash, ignoring whitespace");
    let file_quiet = client()
        .stash_file_diff(&stash, &file, DiffRequest { ignore_whitespace: true, ..Default::default() }, repo.path())
        .expect("one file, ignoring whitespace");

    assert!(plain.contains("+a  "), "the whitespace-only change is shown by default: {plain}");
    assert!(!quiet.contains("+a  ") && quiet.contains("+b changed"), "whitespace-only changes are left out: {quiet}");
    assert!(!file_quiet.contains("+a  ") && file_quiet.contains("+b changed"), "one file leaves them out too: {file_quiet}");
}

/// A 60-line file's content, with line 30 changed in the working copy when `changed` is set.
fn sixty_lines(changed: bool) -> String {
    (1..=60).map(|n| if changed && n == 30 { "changed 30\n".to_string() } else { format!("line {n}\n") }).collect()
}

/// How many unchanged lines a patch shows, counted by their leading space.
fn context_lines(patch: &str) -> usize {
    patch.lines().filter(|line| line.starts_with(' ')).count()
}

#[test]
fn diff_context_lines_widen_the_hunk() {
    let repo = Repo::new();
    repo.commit("long.txt", &sixty_lines(false), "Add a long file");
    repo.write("long.txt", &sixty_lines(true));
    let change = entry(repo.path(), "long.txt");
    let diff = |context: DiffContext| {
        client().diff_with(&change, false, DiffRequest { context, ..Default::default() }, repo.path()).expect("diff the change")
    };

    assert_eq!(context_lines(&diff(DiffContext::Default)), 6, "Git's default is three lines each side");
    assert_eq!(context_lines(&diff(DiffContext::Lines(3))), 6);
    assert_eq!(context_lines(&diff(DiffContext::Lines(10))), 20);
    assert_eq!(context_lines(&diff(DiffContext::WholeFile)), 59, "every unchanged line of the file");
}

#[test]
fn diff_context_applies_to_commits_and_stashes() {
    let repo = Repo::new();
    repo.commit("long.txt", &sixty_lines(false), "Add a long file");
    let changed = repo.commit("long.txt", &sixty_lines(true), "Change line 30");
    let request = |context: DiffContext| DiffRequest { context, ..Default::default() };

    let commit = |context| client().commit_diff_with(&changed, None, request(context), repo.path()).expect("commit diff");
    assert_eq!(context_lines(&commit(DiffContext::Lines(10))), 20);
    assert_eq!(context_lines(&commit(DiffContext::WholeFile)), 59);

    // The commit above already holds the change, so the stash needs its own edit to line 30.
    repo.write("long.txt", &sixty_lines(true).replace("changed 30\n", "stashed 30\n"));
    let stash = stash_all(&repo, "line 30");
    let whole = |context| client().stash_diff_with(&stash, request(context), repo.path()).expect("stash diff");
    assert_eq!(context_lines(&whole(DiffContext::Lines(10))), 20);
    assert_eq!(context_lines(&whole(DiffContext::WholeFile)), 59);

    let file = stash_file(&repo, &stash, "long.txt");
    let one = |context| client().stash_file_diff(&stash, &file, request(context), repo.path()).expect("stash file diff");
    assert_eq!(context_lines(&one(DiffContext::Lines(10))), 20);
    assert_eq!(context_lines(&one(DiffContext::WholeFile)), 59);
}

#[test]
fn restore_a_file_from_a_stash() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.commit("b.txt", "b one\n", "Second");
    repo.write("a.txt", "two\n");
    repo.write("b.txt", "b two\n");
    let stash = stash_all(&repo, "two files");
    // Other work the restore must not touch.
    repo.write("b.txt", "b other work\n");
    let (branch, head) = (snapshot(repo.path()).current_branch, repo.head());

    client().restore_from_stash("a.txt", &stash, false, &branch, Some(&head), repo.path()).expect("restore a.txt from the stash");

    assert_eq!(repo.read("a.txt"), "two\n", "the working file has the stash version");
    assert_eq!(repo.git(&["show", ":a.txt"]), "two\n", "the staged version has the stash version");
    assert_eq!(repo.read("b.txt"), "b other work\n", "other files are untouched");
    assert_eq!(client().list_stashes(repo.path()).unwrap().len(), 1, "the stash is kept");
}

#[test]
fn restore_an_untracked_file_from_a_stash() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.write("scratch.txt", "saved\n");
    let stash = stash_all(&repo, "scratch");
    assert!(!repo.path().join("scratch.txt").exists(), "stashing removed the untracked file");
    let (branch, head) = (snapshot(repo.path()).current_branch, repo.head());

    client()
        .restore_from_stash("scratch.txt", &stash, true, &branch, Some(&head), repo.path())
        .expect("restore the untracked file from the stash");

    assert_eq!(repo.read("scratch.txt"), "saved\n");
    assert_eq!(repo.git(&["show", ":scratch.txt"]), "saved\n", "the file is staged with its stash version");
}

#[test]
fn restore_from_a_dropped_stash_is_refused() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.write("a.txt", "two\n");
    let stash = stash_all(&repo, "gone soon");
    client().drop_stash(&stash, repo.path()).expect("drop stash");
    let (branch, head) = (snapshot(repo.path()).current_branch, repo.head());

    let result = client().restore_from_stash("a.txt", &stash, false, &branch, Some(&head), repo.path());

    assert!(error_text(result).contains("no longer exists"));
    assert_eq!(repo.read("a.txt"), "one\n", "the working file is untouched");
}

// MARK: Branch from stash

#[test]
fn branch_from_stash_restores_changes_on_the_stash_commit() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    let base = repo.commit("b.txt", "one\n", "Second");
    repo.write("a.txt", "staged\n");
    repo.git(&["add", "--", "a.txt"]);
    repo.write("b.txt", "unstaged\n");
    repo.write("scratch.txt", "new\n");
    let stash = stash_all(&repo, "work in progress");
    let later = repo.commit("c.txt", "later\n", "Later");
    assert_eq!(repo.head(), later);

    client().branch_from_stash("topic", &stash, "main", Some(&later), repo.path()).expect("branch from stash");

    assert_eq!(repo.git(&["branch", "--show-current"]).trim(), "topic");
    assert_eq!(repo.head(), base, "the branch is made at the commit the stash was made on");
    assert_eq!(repo.git(&["diff", "--cached", "--name-only"]).trim(), "a.txt", "the staged change is staged again");
    assert_eq!(repo.read("a.txt"), "staged\n");
    assert_eq!(repo.git(&["diff", "--name-only"]).trim(), "b.txt", "the unstaged change is unstaged");
    assert_eq!(repo.read("b.txt"), "unstaged\n");
    assert_eq!(repo.read("scratch.txt"), "new\n", "the untracked file is back");
    assert!(!repo.path().join("c.txt").exists(), "the later commit's file is not on the new branch");
    assert!(client().list_stashes(repo.path()).unwrap().is_empty(), "the applied stash is removed");
}

#[test]
fn branch_from_stash_refuses_a_dirty_tree_without_changing_anything() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.write("a.txt", "two\n");
    let stash = stash_all(&repo, "kept");
    let head = repo.head();
    repo.write("a.txt", "dirty\n");

    let result = client().branch_from_stash("topic", &stash, "main", Some(&head), repo.path());

    assert!(error_text(result).contains("Commit, stash, or discard"));
    assert_eq!(repo.git(&["branch", "--show-current"]).trim(), "main");
    assert_eq!(repo.read("a.txt"), "dirty\n", "the working change is untouched");
    assert_eq!(client().list_stashes(repo.path()).unwrap().len(), 1, "the stash is still listed");
}

#[test]
fn branch_from_stash_refuses_a_dropped_stash() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.write("a.txt", "two\n");
    let stash = stash_all(&repo, "gone soon");
    client().drop_stash(&stash, repo.path()).expect("drop stash");
    let head = repo.head();

    let result = client().branch_from_stash("topic", &stash, "main", Some(&head), repo.path());

    assert!(error_text(result).contains("no longer exists"));
    assert!(!branch_exists(repo.path(), "topic"));
    assert_eq!(repo.read("a.txt"), "one\n");
}

#[test]
fn branch_from_stash_refuses_a_moved_head() {
    let repo = Repo::new();
    let base = repo.commit("a.txt", "one\n", "First");
    repo.write("a.txt", "two\n");
    let stash = stash_all(&repo, "stale");
    let later = repo.commit("b.txt", "later\n", "Later");
    assert_ne!(base, later);

    let result = client().branch_from_stash("topic", &stash, "main", Some(&base), repo.path());

    assert!(error_text(result).contains("changed since this action was selected"));
    assert!(!branch_exists(repo.path(), "topic"));
    assert_eq!(repo.head(), later);
    assert_eq!(client().list_stashes(repo.path()).unwrap().len(), 1, "the stash is still listed");
}

#[test]
fn branch_from_stash_refuses_bad_and_existing_names() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.write("a.txt", "two\n");
    let stash = stash_all(&repo, "named");
    let head = repo.head();

    assert_eq!(client().branch_from_stash("   ", &stash, "main", Some(&head), repo.path()), Err(GitError::EmptyBranchName));
    assert!(client().branch_from_stash("bad name..", &stash, "main", Some(&head), repo.path()).is_err());
    assert!(client().branch_from_stash("main", &stash, "main", Some(&head), repo.path()).is_err());

    assert_eq!(repo.git(&["branch", "--show-current"]).trim(), "main");
    assert_eq!(client().list_stashes(repo.path()).unwrap().len(), 1, "the stash is still listed");
    assert_eq!(repo.read("a.txt"), "one\n", "the working tree is untouched");
}

#[test]
fn branch_from_stash_does_not_overwrite_ignored_files() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    let base = repo.commit("build/out.txt", "tracked\n", "Add build output");
    repo.write("a.txt", "two\n");
    let stash = stash_all(&repo, "on base");
    repo.git(&["rm", "-q", "--", "build/out.txt"]);
    repo.write(".gitignore", "build/\n");
    repo.git(&["add", "--", ".gitignore"]);
    repo.git(&["commit", "--quiet", "-m", "Ignore build output"]);
    repo.write("build/out.txt", "local\n");
    let head = repo.head();
    assert_ne!(head, base);

    let result = client().branch_from_stash("topic", &stash, "main", Some(&head), repo.path());

    assert!(result.is_err(), "switching would overwrite the ignored file");
    assert!(!branch_exists(repo.path(), "topic"), "no branch is left behind");
    assert_eq!(repo.read("build/out.txt"), "local\n", "the ignored file keeps its contents");
    assert_eq!(client().list_stashes(repo.path()).unwrap().len(), 1, "the stash is still listed");
    assert_eq!(repo.git(&["branch", "--show-current"]).trim(), "main");
}

// MARK: Tags

#[test]
fn tag_tip_is_the_object_a_push_checks() {
    let repo = Repo::new();
    let head = repo.commit("a.txt", "one\n", "First");
    client().create_tag("light", "HEAD", None, false, repo.path()).expect("lightweight tag");
    client().create_tag("annotated", "HEAD", Some("Release"), false, repo.path()).expect("annotated tag");

    // A lightweight tag names the commit itself.
    assert_eq!(client().tag_tip("light", repo.path()).expect("tip of lightweight tag"), head);
    // An annotated tag names its tag object, the same object the snapshot and push_tag compare.
    let annotated = client().tag_tip("annotated", repo.path()).expect("tip of annotated tag");
    assert_ne!(annotated, head, "an annotated tag's tip is its tag object, not the commit");
    assert_eq!(annotated, snapshot(repo.path()).tag_tips["annotated"]);
    assert_eq!(annotated, repo.git(&["rev-parse", "refs/tags/annotated"]).trim());
    assert!(client().tag_tip("missing", repo.path()).is_err());
}

#[test]
fn delete_tag_requires_the_shown_tip() {
    let repo = Repo::new();
    let head = repo.commit("a.txt", "one\n", "First");
    client().create_tag("v1", "HEAD", None, false, repo.path()).expect("lightweight tag");
    client().create_tag("v2", "HEAD", Some("Release two"), false, repo.path()).expect("annotated tag");
    let snap = snapshot(repo.path());
    assert!(snap.tags.contains(&"v1".to_string()));
    assert!(snap.tags.contains(&"v2".to_string()));
    assert_eq!(snap.tag_tips["v1"], head);
    let annotated_tip = snap.tag_tips["v2"].clone();
    assert_ne!(annotated_tip, head, "an annotated tag points at its tag object");

    // The commit is not the shown tip of an annotated tag, so the delete must be refused.
    assert!(error_text(client().delete_tag("v2", &head, repo.path())).contains("changed since it was selected"));
    assert!(tag_exists(repo.path(), "v2"));

    client().delete_tag("v2", &annotated_tip, repo.path()).expect("delete annotated tag");
    assert!(!tag_exists(repo.path(), "v2"));

    client().delete_tag("v1", &head, repo.path()).expect("delete lightweight tag");
    assert!(!tag_exists(repo.path(), "v1"));
}

#[test]
fn tag_details_reads_an_annotated_tag() {
    let repo = Repo::new();
    let head = repo.commit("a.txt", "one\n", "First");
    client().create_tag("v1", "HEAD", Some("Release one\n\nWith detail."), false, repo.path()).expect("annotated tag");

    let details = client().tag_details("v1", repo.path()).expect("read tag").expect("an annotated tag has details");
    assert_eq!(details.tagger, "Test");
    assert!(details.date.is_some_and(|date| date > 0), "the tagger date is recorded");
    assert_eq!(details.message, "Release one\n\nWith detail.");
    assert_eq!(details.target, head, "the tag names the commit");
}

#[test]
fn tag_details_of_a_lightweight_tag_is_none() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    client().create_tag("light", "HEAD", None, false, repo.path()).expect("lightweight tag");

    assert_eq!(client().tag_details("light", repo.path()).expect("read tag"), None);
    assert!(client().tag_details("missing", repo.path()).is_err(), "a tag that does not exist is an error");
}

#[test]
fn tag_details_strips_a_signature_block() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    let message = "Signed release\n\n-----BEGIN PGP SIGNATURE-----\n\niQEzBAABCAAdFiEE\n-----END PGP SIGNATURE-----";
    client().create_tag("signed", "HEAD", Some(message), false, repo.path()).expect("annotated tag");

    let details = client().tag_details("signed", repo.path()).expect("read tag").expect("an annotated tag has details");
    assert_eq!(details.message, "Signed release\n\n(signed)");
    assert!(!details.message.contains("SIGNATURE"), "the signature block is not shown");
}

#[test]
fn create_tag_rejects_invalid_name() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");

    assert!(client().create_tag("bad name..", "HEAD", None, false, repo.path()).is_err());
    assert!(snapshot(repo.path()).tags.is_empty());
}

/// Whether Git can sign a tag in `directory` with its settings, checked by signing and deleting
/// a throwaway tag. A machine's global settings can stop Git signing, for example an empty
/// `gpg.format`, which a repository's own value does not override.
fn can_sign_tags(directory: &Path) -> bool {
    let signed = Command::new("git").args(["tag", "--sign", "--message", "probe", "nicegit-probe", "HEAD"]).current_dir(directory).output();
    let signed = signed.is_ok_and(|output| output.status.success());
    if signed {
        git(directory, &["tag", "--delete", "nicegit-probe"]);
    }
    signed
}

#[test]
fn signed_tag_is_signed_with_the_repository_signing_key() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    let keys = tempfile::tempdir().unwrap();
    let key = keys.path().join("signing");
    let made = Command::new("ssh-keygen").args(["-q", "-t", "ed25519", "-N", "", "-C", "test", "-f"]).arg(&key).output();
    let Ok(made) = made else {
        eprintln!("ssh-keygen is not installed; skipping");
        return;
    };
    assert!(made.status.success(), "{}", String::from_utf8_lossy(&made.stderr));
    // Repository-local settings, so a machine's own signing program is not used.
    let key_path = key.to_string_lossy().replace('\\', "/");
    repo.git(&["config", "gpg.format", "ssh"]);
    repo.git(&["config", "gpg.ssh.program", "ssh-keygen"]);
    repo.git(&["config", "user.signingkey", &key_path]);
    if !can_sign_tags(repo.path()) {
        eprintln!("Git cannot sign tags with this machine's settings; skipping");
        return;
    }

    client().create_tag("signed", "HEAD", Some("Release"), true, repo.path()).expect("signed tag");

    assert!(git(repo.path(), &["cat-file", "tag", "signed"]).contains("-----BEGIN SSH SIGNATURE-----"));
    let details = client().tag_details("signed", repo.path()).expect("read tag").expect("an annotated tag has details");
    assert!(details.message.ends_with("(signed)"), "{:?}", details.message);
}

#[test]
fn signed_tag_without_a_usable_key_is_not_created() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    let keys = tempfile::tempdir().unwrap();
    let missing = keys.path().join("no-such-key").to_string_lossy().replace('\\', "/");
    repo.git(&["config", "gpg.format", "ssh"]);
    repo.git(&["config", "gpg.ssh.program", "ssh-keygen"]);
    repo.git(&["config", "user.signingkey", &missing]);

    match client().create_tag("signed", "HEAD", Some("Release"), true, repo.path()) {
        Err(GitError::CommandFailed { message, .. }) => {
            assert!(message.starts_with("The tag could not be signed"), "{message}");
        }
        other => panic!("expected a signing failure, got {other:?}"),
    }
    assert!(!tag_exists(repo.path(), "signed"), "a tag that failed to sign is not left behind");
}

#[test]
fn signed_tag_needs_a_message() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");

    for message in [None, Some(""), Some("   ")] {
        let error = error_text(client().create_tag("signed", "HEAD", message, true, repo.path()));
        assert!(error.contains("A signed tag needs a message."), "{error}");
    }
    assert!(!tag_exists(repo.path(), "signed"));
}

// MARK: Remotes

/// Creates a bare repository on branch `main` to act as a remote.
fn bare_remote() -> TempDir {
    let remote = tempfile::tempdir().expect("create remote dir");
    git(remote.path(), &["init", "--bare", "-b", "main"]);
    remote
}

fn path_string(path: &Path) -> String {
    path.to_str().expect("UTF-8 temp path").to_string()
}

#[test]
fn publish_sets_upstream_and_push_updates_remote() {
    let remote = bare_remote();
    let repo = Repo::new();
    let first = repo.commit("a.txt", "one\n", "First");
    repo.git(&["remote", "add", "origin", &path_string(remote.path())]);

    client().publish("origin", &snapshot(repo.path()), repo.path()).expect("publish");

    let published = snapshot(repo.path());
    assert_eq!(published.upstream.as_deref(), Some("origin/main"));
    assert_eq!(git(remote.path(), &["rev-parse", "refs/heads/main"]).trim(), first);

    let second = repo.commit("a.txt", "two\n", "Second");
    client().push(&snapshot(repo.path()), repo.path()).expect("push");
    assert_eq!(git(remote.path(), &["rev-parse", "refs/heads/main"]).trim(), second);
}

#[test]
fn push_refuses_stale_snapshot() {
    let remote = bare_remote();
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.git(&["remote", "add", "origin", &path_string(remote.path())]);
    client().publish("origin", &snapshot(repo.path()), repo.path()).expect("publish");
    let pushed = repo.commit("a.txt", "two\n", "Second");
    client().push(&snapshot(repo.path()), repo.path()).expect("push second commit");
    let stale = snapshot(repo.path());
    repo.commit("a.txt", "three\n", "Third");

    assert!(error_text(client().push(&stale, repo.path())).contains("current branch changed"));
    assert_eq!(git(remote.path(), &["rev-parse", "refs/heads/main"]).trim(), pushed);
}

/// A repository whose `main` has been published and then rewritten locally, so a normal push
/// would be refused. Returns the repository, the remote, and the remote's tip the user last saw.
fn rewritten_after_publishing() -> (Repo, TempDir, String) {
    let remote = bare_remote();
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.git(&["remote", "add", "origin", &path_string(remote.path())]);
    client().publish("origin", &snapshot(repo.path()), repo.path()).expect("publish");
    repo.commit("a.txt", "two\n", "Second");
    client().push(&snapshot(repo.path()), repo.path()).expect("push second commit");
    repo.git(&["commit", "--quiet", "--amend", "--message", "Second, reworded"]);
    let seen = repo.git(&["rev-parse", "refs/remotes/origin/main"]).trim().to_string();
    (repo, remote, seen)
}

#[test]
fn force_push_replaces_a_rewritten_branch() {
    let (repo, remote, seen) = rewritten_after_publishing();
    let rewritten = repo.head();

    client().force_push(&snapshot(repo.path()), &seen, repo.path()).expect("force push");

    assert_eq!(git(remote.path(), &["rev-parse", "refs/heads/main"]).trim(), rewritten);
}

#[test]
fn force_push_is_refused_when_the_remote_moved() {
    let (repo, remote, seen) = rewritten_after_publishing();
    let shown = snapshot(repo.path());

    // Someone else pushes after the user last fetched.
    let other = tempfile::tempdir().expect("create clone dir");
    git(other.path(), &["clone", "-q", &path_string(remote.path()), "clone"]);
    let clone = other.path().join("clone");
    configure(&clone);
    fs::write(clone.join("b.txt"), "from elsewhere\n").expect("write file");
    git(&clone, &["add", "--", "b.txt"]);
    git(&clone, &["commit", "--quiet", "-m", "Work from elsewhere"]);
    git(&clone, &["push", "--quiet", "origin", "main"]);
    let theirs = git(&clone, &["rev-parse", "HEAD"]).trim().to_string();

    let error = error_text(client().force_push(&shown, &seen, repo.path()));
    assert!(error.contains("stale info") || error.contains("rejected"), "the lease refuses: {error}");
    assert_eq!(git(remote.path(), &["rev-parse", "refs/heads/main"]).trim(), theirs, "the other clone's commit is kept");
}

#[test]
fn force_push_pushes_no_tags_or_other_branches() {
    let (repo, remote, seen) = rewritten_after_publishing();
    // Settings that would mirror the remote and follow tags, if the push honoured them.
    repo.git(&["config", "remote.origin.mirror", "true"]);
    repo.git(&["config", "push.followTags", "true"]);
    repo.git(&["branch", "other"]);
    repo.git(&["tag", "-a", "v1", "-m", "Release one"]);

    client().force_push(&snapshot(repo.path()), &seen, repo.path()).expect("force push");

    assert_eq!(git(remote.path(), &["for-each-ref", "--format=%(refname)"]).trim(), "refs/heads/main", "only the branch is pushed");
    assert!(git(remote.path(), &["tag", "--list"]).trim().is_empty(), "no tags are pushed");
}

#[test]
fn force_push_preview_lists_the_commits_that_would_be_removed() {
    let remote = bare_remote();
    let repo = Repo::new();
    let first = repo.commit("a.txt", "one\n", "First");
    repo.git(&["remote", "add", "origin", &path_string(remote.path())]);
    client().publish("origin", &snapshot(repo.path()), repo.path()).expect("publish");
    repo.commit("a.txt", "two\n", "Second");
    repo.commit("a.txt", "three\n", "Third");
    client().push(&snapshot(repo.path()), repo.path()).expect("push");
    // Rewrite both pushed commits as one new commit on top of the first.
    repo.git(&["reset", "--hard", &first]);
    repo.commit("b.txt", "rewritten\n", "Rewritten");
    let tip = git(remote.path(), &["rev-parse", "refs/heads/main"]).trim().to_string();

    let preview = client().force_push_preview(&snapshot(repo.path()), repo.path()).expect("preview");

    assert_eq!(preview.remote_tip, tip);
    assert_eq!(preview.removed_count, 2);
    assert_eq!(preview.added_count, 1);
    let subjects: Vec<&str> = preview.removed.iter().map(|commit| commit.subject.as_str()).collect();
    assert_eq!(subjects, ["Third", "Second"], "the shared first commit is not listed");
}

#[test]
fn force_push_preview_asks_for_a_fetch_when_the_remote_branch_is_missing() {
    let remote = bare_remote();
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.git(&["remote", "add", "origin", &path_string(remote.path())]);
    client().publish("origin", &snapshot(repo.path()), repo.path()).expect("publish");
    let shown = snapshot(repo.path());
    repo.git(&["update-ref", "-d", "refs/remotes/origin/main"]);

    let error = error_text(client().force_push_preview(&shown, repo.path()));
    assert!(error.contains("Fetch first"), "{error}");
}

#[test]
fn force_push_rejects_a_changed_upstream() {
    let (repo, remote, seen) = rewritten_after_publishing();
    let shown = snapshot(repo.path());
    repo.git(&["config", "branch.main.merge", "refs/heads/other"]);

    let error = error_text(client().force_push(&shown, &seen, repo.path()));
    assert!(error.contains("upstream branch changed"), "{error}");
    assert_eq!(git(remote.path(), &["rev-parse", "refs/heads/main"]).trim(), seen, "the remote is untouched");
}

#[test]
fn force_push_rejects_a_malformed_expected_tip() {
    let (repo, remote, seen) = rewritten_after_publishing();
    let shown = snapshot(repo.path());
    let not_hex = "z".repeat(40);
    for bad in ["", "main", "origin/main", "HEAD", "abc123", "refs/remotes/origin/main", not_hex.as_str()] {
        assert!(client().force_push(&shown, bad, repo.path()).is_err(), "{bad:?} is refused");
    }
    assert_eq!(git(remote.path(), &["rev-parse", "refs/heads/main"]).trim(), seen, "the remote is untouched");
}

#[test]
fn commit_and_push_sends_the_new_commit_to_the_upstream() {
    let remote = bare_remote();
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.git(&["remote", "add", "origin", &path_string(remote.path())]);
    client().publish("origin", &snapshot(repo.path()), repo.path()).expect("publish");
    repo.write("a.txt", "two\n");
    repo.git(&["add", "a.txt"]);

    client().commit_and_push("Second", &snapshot(repo.path()), repo.path()).expect("commit and push");
    let head = git(repo.path(), &["rev-parse", "HEAD"]);
    assert_eq!(git(remote.path(), &["rev-parse", "refs/heads/main"]).trim(), head.trim());
    assert_eq!(git(repo.path(), &["log", "-1", "--format=%s"]).trim(), "Second");
}

#[test]
fn commit_and_push_refuses_a_stale_view_or_an_unpublished_branch_before_committing() {
    let remote = bare_remote();
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    // Not published yet: nothing to push to, so nothing is committed.
    repo.write("a.txt", "two\n");
    repo.git(&["add", "a.txt"]);
    let unpublished = snapshot(repo.path());
    assert!(error_text(client().commit_and_push("Second", &unpublished, repo.path())).contains("Publish"));
    assert_eq!(git(repo.path(), &["rev-list", "--count", "HEAD"]).trim(), "1");

    repo.git(&["remote", "add", "origin", &path_string(remote.path())]);
    client().publish("origin", &snapshot(repo.path()), repo.path()).expect("publish");
    let stale = snapshot(repo.path());
    repo.commit("b.txt", "elsewhere\n", "Made in another app");
    repo.write("a.txt", "three\n");
    repo.git(&["add", "a.txt"]);
    assert!(error_text(client().commit_and_push("Third", &stale, repo.path())).contains("changed"));
    assert_eq!(git(repo.path(), &["log", "-1", "--format=%s"]).trim(), "Made in another app", "nothing was committed");
}

#[test]
fn commit_and_push_keeps_the_commit_and_says_so_when_the_push_fails() {
    let remote = bare_remote();
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.git(&["remote", "add", "origin", &path_string(remote.path())]);
    client().publish("origin", &snapshot(repo.path()), repo.path()).expect("publish");
    // Someone else pushes first, so this push is rejected as not a fast-forward.
    let other = tempfile::tempdir().expect("temp dir");
    git(other.path(), &["clone", "-q", &path_string(remote.path()), "clone"]);
    let clone = other.path().join("clone");
    configure(&clone);
    fs::write(clone.join("theirs.txt"), "theirs\n").expect("write");
    git(&clone, &["add", "theirs.txt"]);
    git(&clone, &["commit", "-q", "-m", "Theirs"]);
    git(&clone, &["push", "-q", "origin", "main"]);
    repo.write("a.txt", "mine\n");
    repo.git(&["add", "a.txt"]);

    let error = error_text(client().commit_and_push("Mine", &snapshot(repo.path()), repo.path()));
    assert!(error.contains("The commit was made, but pushing it failed"), "{error}");
    assert_eq!(git(repo.path(), &["log", "-1", "--format=%s"]).trim(), "Mine");
}

#[test]
fn pull_fast_forwards_from_remote() {
    let remote = bare_remote();
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.git(&["remote", "add", "origin", &path_string(remote.path())]);
    client().publish("origin", &snapshot(repo.path()), repo.path()).expect("publish");

    // A second clone pushes a commit to the remote.
    let workspace = tempfile::tempdir().expect("create workspace");
    git(workspace.path(), &["clone", &path_string(remote.path()), "clone"]);
    let clone = workspace.path().join("clone");
    configure(&clone);
    fs::write(clone.join("remote.txt"), "from the other clone\n").expect("write file");
    git(&clone, &["add", "--", "remote.txt"]);
    git(&clone, &["commit", "--quiet", "-m", "Remote work"]);
    let remote_tip = git(&clone, &["rev-parse", "HEAD"]).trim().to_string();
    git(&clone, &["push", "origin", "main"]);

    client().pull(&snapshot(repo.path()), repo.path()).expect("pull");

    assert_eq!(repo.head(), remote_tip);
    assert_eq!(repo.read("remote.txt"), "from the other clone\n");
}

#[test]
fn pull_refuses_stale_snapshot() {
    let remote = bare_remote();
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.git(&["remote", "add", "origin", &path_string(remote.path())]);
    client().publish("origin", &snapshot(repo.path()), repo.path()).expect("publish");
    let stale = snapshot(repo.path());
    repo.commit("a.txt", "two\n", "Second");

    assert!(error_text(client().pull(&stale, repo.path())).contains("changed since this action was selected"));
}

#[test]
fn snapshot_counts_commits_ahead_of_and_behind_the_upstream() {
    let remote = bare_remote();
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.git(&["remote", "add", "origin", &path_string(remote.path())]);
    client().publish("origin", &snapshot(repo.path()), repo.path()).expect("publish");
    let published = snapshot(repo.path());
    assert_eq!(published.upstream.as_deref(), Some("origin/main"));
    assert_eq!((published.ahead, published.behind), (Some(0), Some(0)));

    // Another clone pushes one commit, and this checkout makes two of its own.
    let workspace = tempfile::tempdir().expect("create workspace");
    git(workspace.path(), &["clone", &path_string(remote.path()), "clone"]);
    let clone = workspace.path().join("clone");
    configure(&clone);
    fs::write(clone.join("remote.txt"), "from the other clone\n").expect("write file");
    git(&clone, &["add", "--", "remote.txt"]);
    git(&clone, &["commit", "--quiet", "-m", "Remote work"]);
    git(&clone, &["push", "origin", "main"]);
    repo.commit("b.txt", "two\n", "Second");
    repo.commit("c.txt", "three\n", "Third");
    client().fetch(repo.path()).expect("fetch");

    let diverged = snapshot(repo.path());
    assert_eq!((diverged.ahead, diverged.behind), (Some(2), Some(1)));
}

#[test]
fn pop_that_conflicts_keeps_the_stash() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.write("a.txt", "two\n");
    client().save_stash("keep me", DEFAULT_STASH, repo.path()).expect("save stash");
    repo.commit("a.txt", "three\n", "Change the same line");
    let stash = client().list_stashes(repo.path()).unwrap()[0].clone();

    let result = client().pop_stash(&stash, repo.path());

    assert!(result.is_err(), "the pop conflicts");
    assert_eq!(client().list_stashes(repo.path()).unwrap().len(), 1, "the stash is kept to try again");
}

/// A repository with `origin` and `upstream` remotes, both holding its `main` branch.
fn repo_with_two_remotes() -> (TempDir, TempDir, Repo, String) {
    let origin = bare_remote();
    let upstream = bare_remote();
    let repo = Repo::new();
    let first = repo.commit("a.txt", "one\n", "First");
    repo.git(&["remote", "add", "origin", &path_string(origin.path())]);
    repo.git(&["remote", "add", "upstream", &path_string(upstream.path())]);
    client().publish("origin", &snapshot(repo.path()), repo.path()).expect("publish");
    repo.git(&["push", "-q", "upstream", "main"]);
    (origin, upstream, repo, first)
}

#[test]
fn fetch_remote_updates_only_the_named_remote() {
    let (origin, upstream, repo, first) = repo_with_two_remotes();
    // Another clone pushes a new commit to each remote.
    let workspace = tempfile::tempdir().expect("create workspace");
    git(workspace.path(), &["clone", "--quiet", &path_string(origin.path()), "clone"]);
    let clone = workspace.path().join("clone");
    configure(&clone);
    fs::write(clone.join("origin.txt"), "on origin\n").expect("write file");
    git(&clone, &["add", "--", "origin.txt"]);
    git(&clone, &["commit", "--quiet", "-m", "On origin"]);
    git(&clone, &["push", "--quiet", "origin", "main"]);
    fs::write(clone.join("upstream.txt"), "on upstream\n").expect("write file");
    git(&clone, &["add", "--", "upstream.txt"]);
    git(&clone, &["commit", "--quiet", "-m", "On upstream"]);
    git(&clone, &["push", "--quiet", &path_string(upstream.path()), "main"]);
    let upstream_head = git(upstream.path(), &["rev-parse", "main"]).trim().to_string();
    let expected = snapshot(repo.path()).remote_fetch_addresses["upstream"].clone();

    client().fetch_remote("upstream", &expected, repo.path()).expect("fetch upstream");

    assert_eq!(repo.git(&["rev-parse", "refs/remotes/upstream/main"]).trim(), upstream_head, "upstream's branch moved");
    assert_eq!(repo.git(&["rev-parse", "refs/remotes/origin/main"]).trim(), first, "origin's tracking branch did not move");
}

#[test]
fn fetch_remote_prunes_a_branch_deleted_on_that_remote() {
    let (_origin, upstream, repo, _first) = repo_with_two_remotes();
    repo.git(&["push", "-q", "upstream", "main:refs/heads/gone"]);
    assert!(git_output(repo.path(), &["rev-parse", "--verify", "--quiet", "refs/remotes/upstream/gone"]).status.success());
    git(upstream.path(), &["update-ref", "-d", "refs/heads/gone"]);
    let expected = snapshot(repo.path()).remote_fetch_addresses["upstream"].clone();

    client().fetch_remote("upstream", &expected, repo.path()).expect("fetch upstream");

    assert!(
        !git_output(repo.path(), &["rev-parse", "--verify", "--quiet", "refs/remotes/upstream/gone"]).status.success(),
        "the deleted branch is pruned"
    );
}

#[test]
fn fetch_remote_refuses_changed_address_and_fetches_nothing() {
    let (_origin, _upstream, repo, first) = repo_with_two_remotes();
    let expected = snapshot(repo.path()).remote_fetch_addresses["upstream"].clone();
    // The remote now points at another repository, which has a commit this one does not.
    let moved = bare_remote();
    repo.commit("b.txt", "two\n", "Second");
    repo.git(&["push", "-q", &path_string(moved.path()), "main"]);
    repo.git(&["remote", "set-url", "upstream", &path_string(moved.path())]);

    let message = error_text(client().fetch_remote("upstream", &expected, repo.path()));

    assert!(message.contains("remote address changed"), "{message}");
    assert_eq!(repo.git(&["rev-parse", "refs/remotes/upstream/main"]).trim(), first, "nothing was fetched");
}

// MARK: History scope

/// The commit hashes in a snapshot's history, newest first.
fn history_hashes(snapshot: &Snapshot) -> Vec<String> {
    snapshot.commits.iter().map(|commit| commit.hash.clone()).collect()
}

#[test]
fn local_scope_leaves_out_remote_only_commits() {
    let remote = bare_remote();
    let repo = Repo::new();
    let first = repo.commit("a.txt", "one\n", "First");
    repo.git(&["remote", "add", "origin", &path_string(remote.path())]);
    client().publish("origin", &snapshot(repo.path()), repo.path()).expect("publish");
    // A branch that exists only on the remote: pushed, its local copy deleted, then fetched.
    repo.git(&["checkout", "-q", "-b", "other"]);
    let other = repo.commit("other.txt", "other\n", "Remote only");
    repo.git(&["push", "-q", "origin", "other"]);
    repo.git(&["checkout", "-q", "main"]);
    repo.git(&["branch", "-q", "-D", "other"]);
    client().fetch(repo.path()).expect("fetch");

    let all = client().load_snapshot_scoped(repo.path(), 100, HistoryScope::All).expect("all");
    let local = client().load_snapshot_scoped(repo.path(), 100, HistoryScope::Local).expect("local");

    assert!(history_hashes(&all).contains(&other), "all branches show the remote's branch");
    assert!(!history_hashes(&local).contains(&other), "local branches leave out the remote-only commit");
    assert!(history_hashes(&local).contains(&first), "local branches keep main's history");
}

#[test]
fn current_scope_shows_head_and_its_upstream_only() {
    let remote = bare_remote();
    let repo = Repo::new();
    let first = repo.commit("a.txt", "one\n", "First");
    repo.git(&["remote", "add", "origin", &path_string(remote.path())]);
    client().publish("origin", &snapshot(repo.path()), repo.path()).expect("publish");
    repo.git(&["checkout", "-q", "-b", "feature"]);
    let feature = repo.commit("feature.txt", "feature\n", "Feature only");
    repo.git(&["checkout", "-q", "main"]);

    // Another clone pushes to main, so the upstream is one commit ahead of the local branch.
    let workspace = tempfile::tempdir().expect("create workspace");
    git(workspace.path(), &["clone", "--quiet", &path_string(remote.path()), "clone"]);
    let clone = workspace.path().join("clone");
    configure(&clone);
    fs::write(clone.join("remote.txt"), "from the other clone\n").expect("write file");
    git(&clone, &["add", "--", "remote.txt"]);
    git(&clone, &["commit", "--quiet", "-m", "Remote work"]);
    git(&clone, &["push", "--quiet", "origin", "main"]);
    let upstream = git(&clone, &["rev-parse", "HEAD"]).trim().to_string();
    client().fetch(repo.path()).expect("fetch");

    let all = client().load_snapshot_scoped(repo.path(), 100, HistoryScope::All).expect("all");
    assert!(history_hashes(&all).contains(&feature), "all branches show the feature branch");

    let current = client().load_snapshot_scoped(repo.path(), 100, HistoryScope::Current).expect("current");
    assert_eq!(history_hashes(&current), vec![upstream, first], "HEAD and its upstream, and nothing else");
    assert!(!history_hashes(&current).contains(&feature));
}

#[test]
fn current_scope_without_upstream_shows_head_history() {
    let repo = Repo::new();
    let first = repo.commit("a.txt", "one\n", "First");
    let second = repo.commit("a.txt", "two\n", "Second");
    repo.git(&["checkout", "-q", "-b", "feature"]);
    let feature = repo.commit("feature.txt", "feature\n", "Feature only");
    repo.git(&["checkout", "-q", "main"]);

    let current = client().load_snapshot_scoped(repo.path(), 100, HistoryScope::Current).expect("current");
    assert_eq!(history_hashes(&current), vec![second, first]);
    assert!(!history_hashes(&current).contains(&feature));
    assert_eq!(current.upstream, None);
}

#[test]
fn local_scope_keeps_a_detached_head() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.git(&["checkout", "-q", "-b", "side"]);
    repo.commit("side.txt", "side one\n", "Side one");
    // Only the tag reaches this commit once the branch is gone; HEAD is detached on it.
    let tagged = repo.commit("side.txt", "side two\n", "Side two");
    repo.git(&["tag", "keep"]);
    repo.git(&["checkout", "-q", "main"]);
    repo.git(&["branch", "-q", "-D", "side"]);
    repo.git(&["checkout", "-q", "--detach", "keep"]);

    let local = client().load_snapshot_scoped(repo.path(), 100, HistoryScope::Local).expect("local");
    assert_eq!(local.head_hash.as_deref(), Some(tagged.as_str()));
    assert!(local.current_branch.starts_with("Detached HEAD"), "{}", local.current_branch);
    assert!(history_hashes(&local).contains(&tagged), "the detached HEAD's commit stays visible");
}

#[test]
fn history_scopes_load_a_repository_without_commits() {
    let repo = Repo::new();
    for scope in [HistoryScope::All, HistoryScope::Local, HistoryScope::Current] {
        let snapshot = client().load_snapshot_scoped(repo.path(), 100, scope).unwrap_or_else(|error| panic!("{scope:?}: {error}"));
        assert!(snapshot.commits.is_empty(), "{scope:?}");
        assert_eq!(snapshot.head_hash, None, "{scope:?}");
    }
}

// MARK: Hidden branches

/// A repository with `feature` (two commits of its own) and `other` (one commit), both from
/// `main`'s first commit. Returns the hashes of `first`, the feature's two commits, and `other`'s.
fn repository_with_feature_and_other() -> (Repo, [String; 4]) {
    let repo = Repo::new();
    let first = repo.commit("a.txt", "one\n", "First");
    repo.git(&["checkout", "-q", "-b", "feature"]);
    let feature_one = repo.commit("feature.txt", "feature one\n", "Feature one");
    let feature_two = repo.commit("feature.txt", "feature two\n", "Feature two");
    repo.git(&["checkout", "-q", "main"]);
    repo.git(&["checkout", "-q", "-b", "other"]);
    let other = repo.commit("other.txt", "other\n", "Other");
    repo.git(&["checkout", "-q", "main"]);
    (repo, [first, feature_one, feature_two, other])
}

fn hidden(references: &[&str]) -> Vec<String> {
    references.iter().map(|reference| reference.to_string()).collect()
}

#[test]
fn hiding_a_branch_removes_only_its_own_commits() {
    let (repo, [first, feature_one, feature_two, other]) = repository_with_feature_and_other();
    let snapshot =
        client().load_snapshot_filtered(repo.path(), 100, HistoryScope::All, &hidden(&["refs/heads/feature"])).expect("snapshot");
    let hashes = history_hashes(&snapshot);
    assert!(!hashes.contains(&feature_one) && !hashes.contains(&feature_two), "the hidden branch's commits are left out");
    assert!(hashes.contains(&first) && hashes.contains(&other), "other branches keep their commits");
}

#[test]
fn hiding_a_branch_leaves_out_only_the_named_branch() {
    let repo = Repo::new();
    let first = repo.commit("a.txt", "one\n", "First");
    repo.git(&["checkout", "-q", "-b", "feature"]);
    let feature = repo.commit("feature.txt", "feature\n", "Feature");
    repo.git(&["checkout", "-q", "main"]);
    repo.git(&["checkout", "-q", "-b", "features"]);
    let features = repo.commit("features.txt", "features\n", "Features");
    repo.git(&["checkout", "-q", "main"]);

    for scope in [HistoryScope::All, HistoryScope::Local] {
        let snapshot = client().load_snapshot_filtered(repo.path(), 100, scope, &hidden(&["refs/heads/feature"])).expect("snapshot");
        let hashes = history_hashes(&snapshot);
        assert!(!hashes.contains(&feature), "{scope:?}: the named branch is hidden");
        assert!(hashes.contains(&features), "{scope:?}: a branch whose name starts the same way is still shown");
        assert!(hashes.contains(&first));
    }
}

#[test]
fn a_commit_that_a_shown_branch_reaches_stays_visible() {
    let repo = Repo::new();
    let shared = repo.commit("a.txt", "one\n", "Shared");
    repo.git(&["checkout", "-q", "-b", "feature"]);
    let feature = repo.commit("feature.txt", "feature\n", "Feature only");
    repo.git(&["checkout", "-q", "main"]);
    repo.git(&["branch", "-q", "other", &shared]);

    let snapshot =
        client().load_snapshot_filtered(repo.path(), 100, HistoryScope::All, &hidden(&["refs/heads/feature"])).expect("snapshot");
    let hashes = history_hashes(&snapshot);
    assert!(!hashes.contains(&feature));
    assert!(hashes.contains(&shared), "main and other reach the shared commit, so it still shows");
}

#[test]
fn hiding_the_current_branch_still_shows_head() {
    let repo = Repo::new();
    let first = repo.commit("a.txt", "one\n", "First");
    let second = repo.commit("a.txt", "two\n", "Second");

    let snapshot = client().load_snapshot_filtered(repo.path(), 100, HistoryScope::All, &hidden(&["refs/heads/main"])).expect("snapshot");
    assert_eq!(history_hashes(&snapshot), vec![second.clone(), first.clone()]);
    assert_eq!(snapshot.head_hash.as_deref(), Some(second.as_str()));

    let local = client().load_snapshot_filtered(repo.path(), 100, HistoryScope::Local, &hidden(&["refs/heads/main"])).expect("local");
    assert_eq!(history_hashes(&local), vec![second, first], "HEAD is walked in the Local scope too");
}

#[test]
fn a_detached_head_on_a_hidden_branch_still_shows() {
    let (repo, [_, feature_one, feature_two, _]) = repository_with_feature_and_other();
    repo.git(&["checkout", "-q", "--detach", "feature"]);

    for scope in [HistoryScope::All, HistoryScope::Local] {
        let snapshot = client().load_snapshot_filtered(repo.path(), 100, scope, &hidden(&["refs/heads/feature"])).expect("snapshot");
        assert!(history_hashes(&snapshot).contains(&feature_two), "{scope:?}: HEAD's commit is shown");
        assert!(history_hashes(&snapshot).contains(&feature_one), "{scope:?}: HEAD's history is shown");
    }
}

#[test]
fn hiding_a_remote_tracking_branch_removes_its_own_commits() {
    let repo = Repo::new();
    let first = repo.commit("a.txt", "one\n", "First");
    repo.git(&["checkout", "-q", "-b", "remote-only"]);
    let remote_only = repo.commit("remote.txt", "remote\n", "Remote only");
    repo.git(&["checkout", "-q", "main"]);
    repo.git(&["branch", "-q", "-D", "remote-only"]);
    repo.git(&["update-ref", "refs/remotes/origin/x", &remote_only]);

    let shown = client().load_snapshot_filtered(repo.path(), 100, HistoryScope::All, &[]).expect("shown");
    assert!(history_hashes(&shown).contains(&remote_only));

    let hidden_snapshot =
        client().load_snapshot_filtered(repo.path(), 100, HistoryScope::All, &hidden(&["refs/remotes/origin/x"])).expect("hidden");
    let hashes = history_hashes(&hidden_snapshot);
    assert!(!hashes.contains(&remote_only), "the hidden remote-tracking branch's commit is left out");
    assert!(hashes.contains(&first));
}

#[test]
fn local_scope_leaves_out_a_hidden_local_branch() {
    let (repo, [first, feature_one, feature_two, other]) = repository_with_feature_and_other();
    let local = client().load_snapshot_filtered(repo.path(), 100, HistoryScope::Local, &hidden(&["refs/heads/feature"])).expect("local");
    let hashes = history_hashes(&local);
    assert!(!hashes.contains(&feature_one) && !hashes.contains(&feature_two));
    assert!(hashes.contains(&first) && hashes.contains(&other));
}

#[test]
fn entries_that_are_not_branches_are_ignored() {
    let (repo, _) = repository_with_feature_and_other();
    repo.git(&["tag", "keep", "feature"]);
    for scope in [HistoryScope::All, HistoryScope::Local, HistoryScope::Current] {
        let ignored = client()
            .load_snapshot_filtered(repo.path(), 100, scope, &hidden(&["refs/tags/keep", "HEAD", "refs/stash", "feature", ""]))
            .expect("ignored");
        let unhidden = client().load_snapshot_filtered(repo.path(), 100, scope, &[]).expect("unhidden");
        assert_eq!(history_hashes(&ignored), history_hashes(&unhidden), "{scope:?}");
    }
}

#[test]
fn current_scope_ignores_hidden_branches() {
    let (repo, _) = repository_with_feature_and_other();
    let plain = client().load_snapshot_scoped(repo.path(), 100, HistoryScope::Current).expect("plain");
    let with_hidden =
        client().load_snapshot_filtered(repo.path(), 100, HistoryScope::Current, &hidden(&["refs/heads/main"])).expect("hidden");
    assert_eq!(history_hashes(&with_hidden), history_hashes(&plain));
}
