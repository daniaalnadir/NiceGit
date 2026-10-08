//! Integration tests for `GitClient`, run against real repositories in temporary directories.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use nicegit_core::client::GitClient;
use nicegit_core::models::{Branch, GitError, Operation, Snapshot, StatusEntry, StatusKind};
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

    let result = client().merge(&shown, "main", Some(&main_head), repo.path());

    assert!(error_text(result).contains("selected branch changed"));
    assert_eq!(repo.head(), main_head);

    // With the current tip shown again, the same merge succeeds.
    let fresh = local_branch(&snapshot(repo.path()), "feature");
    client().merge(&fresh, "main", Some(&main_head), repo.path()).expect("merge fresh tip");
    assert!(repo.path().join("b.txt").exists());
}

// MARK: Stashes

#[test]
fn save_and_apply_stash_keeps_the_entry_until_dropped() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.write("a.txt", "two\n");
    repo.write("scratch.txt", "untracked\n");

    client().save_stash("my work", repo.path()).expect("save stash");
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
    client().save_stash("", repo.path()).expect("save stash");
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
    client().save_stash("gone soon", repo.path()).expect("save stash");
    let stash = client().list_stashes(repo.path()).unwrap()[0].clone();
    client().drop_stash(&stash, repo.path()).expect("first drop");

    assert!(error_text(client().drop_stash(&stash, repo.path())).contains("no longer exists"));
    assert!(client().apply_stash(&stash, repo.path()).is_err());
}

#[test]
fn save_stash_on_clean_tree_reports_error() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");

    assert!(client().save_stash("nothing", repo.path()).is_err());
    assert!(client().list_stashes(repo.path()).unwrap().is_empty());
}

// MARK: Tags

#[test]
fn delete_tag_requires_the_shown_tip() {
    let repo = Repo::new();
    let head = repo.commit("a.txt", "one\n", "First");
    client().create_tag("v1", "HEAD", None, repo.path()).expect("lightweight tag");
    client().create_tag("v2", "HEAD", Some("Release two"), repo.path()).expect("annotated tag");
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
fn create_tag_rejects_invalid_name() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");

    assert!(client().create_tag("bad name..", "HEAD", None, repo.path()).is_err());
    assert!(snapshot(repo.path()).tags.is_empty());
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
