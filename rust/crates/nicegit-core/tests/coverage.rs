//! Integration tests for `GitClient` functions that the other suites do not reach, run against
//! real repositories in temporary directories.

use std::collections::BTreeMap;
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

/// Local branch names, sorted.
fn local_branches(directory: &Path) -> Vec<String> {
    let mut names: Vec<String> =
        git(directory, &["for-each-ref", "--format=%(refname:short)", "refs/heads"]).lines().map(str::to_string).collect();
    names.sort();
    names
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

fn remote_branch(snapshot: &Snapshot, name: &str) -> Branch {
    snapshot
        .branches
        .iter()
        .find(|branch| branch.is_remote && branch.name == name)
        .cloned()
        .unwrap_or_else(|| panic!("no remote branch {name}"))
}

/// Creates a bare repository on branch `main` to act as a remote.
fn bare_remote() -> TempDir {
    let remote = tempfile::tempdir().expect("create remote dir");
    git(remote.path(), &["init", "--bare", "-b", "main"]);
    remote
}

fn path_string(path: &Path) -> String {
    path.to_str().expect("UTF-8 temp path").to_string()
}

/// Adds `remote` to `repo` under `name`.
fn connect(repo: &Repo, name: &str, remote: &TempDir) {
    repo.git(&["remote", "add", name, &path_string(remote.path())]);
}

/// Creates `branch` from the current HEAD with one commit, pushes it to `remote`, then removes
/// the local copy so only the remote-tracking branch remains. Returns the pushed commit.
fn push_branch_then_forget(repo: &Repo, remote: &str, branch: &str) -> String {
    repo.git(&["switch", "-c", branch]);
    let tip = repo.commit(&format!("{branch}.txt"), "work\n", &format!("{remote} {branch} work"));
    repo.git(&["push", remote, branch]);
    repo.git(&["switch", "main"]);
    repo.git(&["branch", "-D", branch]);
    tip
}

// MARK: Remote branches

#[test]
fn checkout_remote_creates_tracking_branch() {
    let remote = bare_remote();
    let repo = Repo::new();
    let main = repo.commit("a.txt", "one\n", "First");
    connect(&repo, "origin", &remote);
    repo.git(&["push", "origin", "main"]);
    let tip = push_branch_then_forget(&repo, "origin", "feature");
    let snap = snapshot(repo.path());
    assert!(!local_branches(repo.path()).contains(&"feature".to_string()));

    let switched = client()
        .checkout_remote(&remote_branch(&snap, "remotes/origin/feature"), &snap.remotes, "main", Some(&main), repo.path())
        .expect("checkout remote branch");

    assert!(!switched, "a clean tree needs no stash");
    assert_eq!(repo.git(&["branch", "--show-current"]).trim(), "feature");
    assert_eq!(repo.head(), tip);
    assert_eq!(repo.git(&["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"]).trim(), "origin/feature");
}

#[test]
fn checkout_remote_reuses_existing_tracking_branch() {
    let remote = bare_remote();
    let repo = Repo::new();
    let main = repo.commit("a.txt", "one\n", "First");
    connect(&repo, "origin", &remote);
    repo.git(&["push", "origin", "main"]);
    push_branch_then_forget(&repo, "origin", "feature");
    let snap = snapshot(repo.path());
    let remote_feature = remote_branch(&snap, "remotes/origin/feature");
    client().checkout_remote(&remote_feature, &snap.remotes, "main", Some(&main), repo.path()).expect("first checkout");
    repo.git(&["switch", "main"]);
    let before = local_branches(repo.path());

    let snap = snapshot(repo.path());
    client().checkout_remote(&remote_feature, &snap.remotes, "main", Some(&main), repo.path()).expect("second checkout");

    assert_eq!(local_branches(repo.path()), before, "no second tracking branch is created");
    assert_eq!(repo.git(&["branch", "--show-current"]).trim(), "feature");

    // Already on the tracking branch: nothing to switch.
    let snap = snapshot(repo.path());
    let switched = client()
        .checkout_remote(&remote_feature, &snap.remotes, "feature", snap.head_hash.as_deref(), repo.path())
        .expect("checkout while already on it");
    assert!(!switched);
}

#[test]
fn checkout_remote_reuses_renamed_tracking_branch() {
    let remote = bare_remote();
    let repo = Repo::new();
    let main = repo.commit("a.txt", "one\n", "First");
    connect(&repo, "origin", &remote);
    repo.git(&["push", "origin", "main"]);
    push_branch_then_forget(&repo, "origin", "feature");
    let snap = snapshot(repo.path());
    client()
        .checkout_remote(&remote_branch(&snap, "remotes/origin/feature"), &snap.remotes, "main", Some(&main), repo.path())
        .expect("first checkout");
    repo.git(&["switch", "main"]);
    repo.git(&["branch", "-m", "feature", "renamed"]);

    let snap = snapshot(repo.path());
    client()
        .checkout_remote(&remote_branch(&snap, "remotes/origin/feature"), &snap.remotes, "main", Some(&main), repo.path())
        .expect("checkout after rename");

    assert_eq!(repo.git(&["branch", "--show-current"]).trim(), "renamed");
    assert_eq!(local_branches(repo.path()), vec!["main".to_string(), "renamed".to_string()]);
}

#[test]
fn checkout_remote_refuses_when_several_local_branches_track_it() {
    let remote = bare_remote();
    let repo = Repo::new();
    let main = repo.commit("a.txt", "one\n", "First");
    connect(&repo, "origin", &remote);
    repo.git(&["push", "origin", "main"]);
    push_branch_then_forget(&repo, "origin", "feature");
    repo.git(&["branch", "--track", "first", "origin/feature"]);
    repo.git(&["branch", "--track", "second", "origin/feature"]);
    let snap = snapshot(repo.path());

    let result = client().checkout_remote(&remote_branch(&snap, "remotes/origin/feature"), &snap.remotes, "main", Some(&main), repo.path());

    assert!(error_text(result).contains("Several local branches track"));
    assert_eq!(repo.git(&["branch", "--show-current"]).trim(), "main");
    assert_eq!(local_branches(repo.path()), vec!["first", "main", "second"]);
}

#[test]
fn checkout_remote_names_second_remote_branch_distinctly() {
    let origin = bare_remote();
    let upstream = bare_remote();
    let repo = Repo::new();
    let main = repo.commit("a.txt", "one\n", "First");
    connect(&repo, "origin", &origin);
    connect(&repo, "upstream", &upstream);
    repo.git(&["push", "origin", "main"]);
    push_branch_then_forget(&repo, "origin", "feature");
    push_branch_then_forget(&repo, "upstream", "feature");
    let snap = snapshot(repo.path());
    client()
        .checkout_remote(&remote_branch(&snap, "remotes/origin/feature"), &snap.remotes, "main", Some(&main), repo.path())
        .expect("checkout origin feature");

    let snap = snapshot(repo.path());
    client()
        .checkout_remote(
            &remote_branch(&snap, "remotes/upstream/feature"),
            &snap.remotes,
            "feature",
            snap.head_hash.as_deref(),
            repo.path(),
        )
        .expect("checkout upstream feature");

    assert_eq!(repo.git(&["branch", "--show-current"]).trim(), "upstream-feature");
    assert_eq!(repo.git(&["rev-parse", "--abbrev-ref", "--symbolic-full-name", "upstream-feature@{upstream}"]).trim(), "upstream/feature");
    assert_eq!(repo.git(&["rev-parse", "--abbrev-ref", "--symbolic-full-name", "feature@{upstream}"]).trim(), "origin/feature");
}

#[test]
fn checkout_remote_refuses_stale_displayed_tip() {
    let remote = bare_remote();
    let repo = Repo::new();
    let main = repo.commit("a.txt", "one\n", "First");
    connect(&repo, "origin", &remote);
    repo.git(&["push", "origin", "main"]);
    push_branch_then_forget(&repo, "origin", "feature");
    let shown = remote_branch(&snapshot(repo.path()), "remotes/origin/feature");
    // The remote-tracking branch moves after it was displayed.
    repo.git(&["update-ref", "refs/remotes/origin/feature", &main]);
    let snap = snapshot(repo.path());

    let result = client().checkout_remote(&shown, &snap.remotes, "main", Some(&main), repo.path());

    assert!(error_text(result).contains("changed since it was selected"));
    assert_eq!(repo.git(&["branch", "--show-current"]).trim(), "main");
    assert!(!local_branches(repo.path()).contains(&"feature".to_string()));
}

// MARK: Branch rename

#[test]
fn rename_branch_moves_the_branch() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.git(&["branch", "old-name"]);
    let shown = local_branch(&snapshot(repo.path()), "old-name");

    client().rename_branch(&shown, "new-name", repo.path()).expect("rename");

    assert!(branch_exists(repo.path(), "new-name"));
    assert!(!branch_exists(repo.path(), "old-name"));
}

#[test]
fn rename_branch_refuses_invalid_name() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.git(&["branch", "old-name"]);
    let shown = local_branch(&snapshot(repo.path()), "old-name");

    assert!(client().rename_branch(&shown, "bad name..", repo.path()).is_err());
    assert!(branch_exists(repo.path(), "old-name"));
    assert!(!branch_exists(repo.path(), "bad name.."));
}

#[test]
fn rename_branch_refuses_when_tip_moved() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.git(&["branch", "feature"]);
    let shown = local_branch(&snapshot(repo.path()), "feature");
    repo.git(&["switch", "feature"]);
    repo.commit("b.txt", "moved\n", "Moved on");
    repo.git(&["switch", "main"]);

    assert!(error_text(client().rename_branch(&shown, "renamed", repo.path())).contains("This branch changed"));
    assert!(branch_exists(repo.path(), "feature"));
    assert!(!branch_exists(repo.path(), "renamed"));
}

// MARK: Push branch

#[test]
fn push_branch_pushes_only_the_named_branch() {
    let remote = bare_remote();
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    connect(&repo, "origin", &remote);
    repo.git(&["switch", "-c", "feature"]);
    let tip = repo.commit("b.txt", "feature\n", "Feature work");
    repo.git(&["tag", "-a", "v1", "-m", "Release one"]);
    repo.git(&["switch", "main"]);
    repo.git(&["branch", "other"]);
    // With follow-tags on, a plain push would also send the annotated tag.
    repo.git(&["config", "push.followTags", "true"]);
    let snap = snapshot(repo.path());

    client().push_branch(&local_branch(&snap, "feature"), "origin", &snap.remote_push_addresses, repo.path()).expect("push branch");

    assert_eq!(git(remote.path(), &["rev-parse", "refs/heads/feature"]).trim(), tip);
    let remote_refs = git(remote.path(), &["for-each-ref", "--format=%(refname)"]);
    assert_eq!(remote_refs.trim(), "refs/heads/feature", "only the named branch and no tags were pushed");
}

#[test]
fn push_branch_refuses_changed_push_address() {
    let remote = bare_remote();
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    connect(&repo, "origin", &remote);
    repo.git(&["branch", "feature"]);
    let snap = snapshot(repo.path());
    let mut stale: BTreeMap<String, Vec<String>> = snap.remote_push_addresses.clone();
    stale.insert("origin".to_string(), vec!["/somewhere/else/repository.git".to_string()]);

    let result = client().push_branch(&local_branch(&snap, "feature"), "origin", &stale, repo.path());

    assert!(error_text(result).contains("remote address changed"));
    assert!(git(remote.path(), &["for-each-ref", "--format=%(refname)"]).trim().is_empty());
}

#[test]
fn push_branch_refuses_when_tip_moved() {
    let remote = bare_remote();
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    connect(&repo, "origin", &remote);
    repo.git(&["switch", "-c", "feature"]);
    repo.commit("b.txt", "feature\n", "Feature work");
    repo.git(&["switch", "main"]);
    let snap = snapshot(repo.path());
    let shown = local_branch(&snap, "feature");
    repo.git(&["switch", "feature"]);
    repo.commit("c.txt", "more\n", "More feature work");
    repo.git(&["switch", "main"]);

    let result = client().push_branch(&shown, "origin", &snap.remote_push_addresses, repo.path());

    assert!(error_text(result).contains("This branch changed"));
    assert!(git(remote.path(), &["for-each-ref", "--format=%(refname)"]).trim().is_empty());
}

// MARK: Patches

#[test]
fn exported_commit_applies_to_another_branch_as_unstaged_changes() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.git(&["switch", "-c", "feature"]);
    repo.write("a.txt", "two\n");
    repo.write("b.txt", "bee\n");
    repo.git(&["add", "--", "a.txt", "b.txt"]);
    repo.git(&["commit", "--quiet", "-m", "Change a and add b"]);
    let patch = client().export_commit_patch(&repo.head(), repo.path()).expect("export patch");
    assert!(patch.contains("Change a and add b"));
    repo.git(&["switch", "main"]);

    client().apply_patch(patch.as_bytes(), repo.path()).expect("apply patch");

    assert_eq!(repo.read("a.txt"), "two\n");
    assert_eq!(repo.read("b.txt"), "bee\n");
    assert!(repo.git(&["diff", "--cached", "--name-only"]).trim().is_empty(), "the patch is not staged");
    let modified = entry(repo.path(), "a.txt");
    assert_eq!(modified.kind, StatusKind::Modified);
    assert!(modified.is_unstaged());
    assert_eq!(entry(repo.path(), "b.txt").kind, StatusKind::Untracked);
    assert_eq!(repo.git(&["rev-list", "--count", "HEAD"]).trim(), "1", "no commit was made");
}

#[test]
fn export_refuses_merge_commit() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.git(&["switch", "-c", "side"]);
    repo.commit("b.txt", "side\n", "Side work");
    repo.git(&["switch", "main"]);
    repo.commit("c.txt", "main\n", "Main work");
    repo.git(&["merge", "--no-ff", "--no-edit", "side"]);

    let result = client().export_commit_patch(&repo.head(), repo.path());

    assert!(error_text(result).contains("Merge commits cannot be exported"));
}

#[test]
fn apply_patch_that_does_not_apply_changes_nothing() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.git(&["switch", "-c", "feature"]);
    repo.write("a.txt", "two\n");
    repo.write("b.txt", "bee\n");
    repo.git(&["add", "--", "a.txt", "b.txt"]);
    repo.git(&["commit", "--quiet", "-m", "Change a and add b"]);
    let patch = client().export_commit_patch(&repo.head(), repo.path()).expect("export patch");
    repo.git(&["switch", "main"]);
    // The patch expects "one" in a.txt, so this divergence makes it fail.
    repo.commit("a.txt", "other\n", "Diverge");

    assert!(client().apply_patch(patch.as_bytes(), repo.path()).is_err());

    assert_eq!(repo.read("a.txt"), "other\n");
    assert!(!repo.path().join("b.txt").exists(), "a failed check must not create the new file");
    assert!(client().load_status(repo.path()).unwrap().is_empty());
}

#[test]
fn apply_patch_refuses_during_unfinished_merge() {
    let repo = Repo::new();
    repo.commit("a.txt", "base\n", "Base");
    repo.git(&["switch", "-c", "side"]);
    let side = repo.commit("a.txt", "side\n", "Side change");
    let patch = client().export_commit_patch(&side, repo.path()).expect("export patch");
    repo.git(&["switch", "main"]);
    repo.commit("a.txt", "main\n", "Main change");
    assert!(!git_output(repo.path(), &["merge", "--no-edit", "side"]).status.success());
    let during_merge = repo.read("a.txt");

    let result = client().apply_patch(patch.as_bytes(), repo.path());

    assert!(error_text(result).contains("Finish or abort"));
    assert_eq!(repo.read("a.txt"), during_merge);
    assert_eq!(snapshot(repo.path()).operation, Some(Operation::Merge));
}

// MARK: Stashes

#[test]
fn stash_diff_shows_tracked_and_untracked_changes() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.write("a.txt", "two\n");
    repo.write("scratch.txt", "untracked\n");
    client().save_stash("mixed", repo.path()).expect("save stash");
    let stash = client().list_stashes(repo.path()).unwrap()[0].clone();

    let diff = client().stash_diff(&stash, repo.path()).expect("stash diff");

    assert!(diff.contains("a/a.txt"), "diff was {diff}");
    assert!(diff.contains("-one") && diff.contains("+two"), "diff was {diff}");
    assert!(diff.contains("scratch.txt"), "diff was {diff}");
    assert!(diff.contains("+untracked"), "diff was {diff}");
}

// MARK: Remotes

#[test]
fn fetch_moves_remote_tracking_branch_without_touching_head() {
    let remote = bare_remote();
    let repo = Repo::new();
    let first = repo.commit("a.txt", "one\n", "First");
    connect(&repo, "origin", &remote);
    repo.git(&["push", "origin", "main"]);

    // Another clone pushes a commit to the remote.
    let workspace = tempfile::tempdir().expect("create workspace");
    git(workspace.path(), &["clone", &path_string(remote.path()), "clone"]);
    let clone = workspace.path().join("clone");
    configure(&clone);
    fs::write(clone.join("remote.txt"), "from the other clone\n").expect("write file");
    git(&clone, &["add", "--", "remote.txt"]);
    git(&clone, &["commit", "--quiet", "-m", "Remote work"]);
    let remote_tip = git(&clone, &["rev-parse", "HEAD"]).trim().to_string();
    git(&clone, &["push", "origin", "main"]);
    assert_eq!(repo.git(&["rev-parse", "refs/remotes/origin/main"]).trim(), first);

    client().fetch(repo.path()).expect("fetch");

    assert_eq!(repo.git(&["rev-parse", "refs/remotes/origin/main"]).trim(), remote_tip);
    assert_eq!(repo.head(), first, "fetching does not move the checkout");
}

// MARK: Repository

#[test]
fn initialize_creates_repository_that_snapshot_can_open() {
    let dir = tempfile::tempdir().expect("create empty folder");

    client().initialize(dir.path()).expect("initialize");

    assert!(dir.path().join(".git").exists());
    let snap = client().load_snapshot(dir.path(), 10).expect("load snapshot of new repository");
    assert_eq!(snap.head_hash, None);
    assert!(snap.commits.is_empty());
    assert!(snap.status.is_empty());
}

// MARK: Branches from a displayed tip

#[test]
fn create_branch_from_uses_displayed_tip_without_checkout() {
    let repo = Repo::new();
    let main = repo.commit("a.txt", "one\n", "First");
    repo.git(&["switch", "-c", "feature"]);
    let feature = repo.commit("b.txt", "bee\n", "Feature");
    repo.git(&["switch", "main"]);
    let shown = local_branch(&snapshot(repo.path()), "feature");

    client().create_branch_from("topic", &shown, repo.path()).expect("create branch from selection");

    assert_eq!(repo.git(&["rev-parse", "refs/heads/topic"]).trim(), feature);
    assert_eq!(repo.git(&["branch", "--show-current"]).trim(), "main");
    assert_eq!(repo.head(), main);
}

#[test]
fn create_branch_from_refuses_when_tip_moved() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.git(&["switch", "-c", "feature"]);
    repo.commit("b.txt", "bee\n", "Feature");
    repo.git(&["switch", "main"]);
    let shown = local_branch(&snapshot(repo.path()), "feature");
    repo.git(&["switch", "feature"]);
    repo.commit("c.txt", "more\n", "More feature work");
    repo.git(&["switch", "main"]);

    let result = client().create_branch_from("topic", &shown, repo.path());

    assert!(error_text(result).contains("This branch changed"));
    assert!(!branch_exists(repo.path(), "topic"));
}

// MARK: Commit message

#[test]
fn amend_message_changes_only_the_head_message() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    let second = repo.commit("a.txt", "two\n", "Second");
    repo.write("staged.txt", "staged\n");
    client().stage("staged.txt", repo.path()).expect("stage");
    repo.write("a.txt", "three\n");

    client().amend_message("Reworded second", "main", &second, repo.path()).expect("amend message");

    assert_eq!(repo.git(&["log", "-1", "--format=%s"]).trim(), "Reworded second");
    assert_eq!(repo.git(&["rev-list", "--count", "HEAD"]).trim(), "2", "the commit was amended, not added to");
    assert_eq!(repo.git(&["show", "--name-only", "--format=", "HEAD"]).trim(), "a.txt");
    assert!(entry(repo.path(), "staged.txt").is_staged(), "the staged file stays staged");
    assert!(!repo.git(&["diff", "--cached", "--name-only"]).is_empty());
    let unstaged = entry(repo.path(), "a.txt");
    assert!(unstaged.is_unstaged() && !unstaged.is_staged());
    assert_eq!(repo.read("a.txt"), "three\n");
}

#[test]
fn amend_message_refuses_stale_head() {
    let repo = Repo::new();
    let first = repo.commit("a.txt", "one\n", "First");
    let second = repo.commit("a.txt", "two\n", "Second");

    let result = client().amend_message("Reworded", "main", &first, repo.path());

    assert!(error_text(result).contains("changed since this action was selected"));
    assert_eq!(repo.head(), second);
    assert_eq!(repo.git(&["log", "-1", "--format=%s"]).trim(), "Second");
}
