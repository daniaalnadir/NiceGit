//! Integration tests for Git LFS, GitFlow, branch clean-up, and the reflog, run against real
//! repositories in temporary directories.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::thread;
use std::time::Duration;

use nicegit_core::cleanup::{BranchCandidate, BranchDeletion};
use nicegit_core::client::GitClient;
use nicegit_core::gitflow::{GitFlowConfiguration, GitFlowKind};
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

fn client() -> GitClient {
    GitClient::new()
}

fn current_head(directory: &Path) -> String {
    git(directory, &["rev-parse", "HEAD"]).trim().to_string()
}

fn is_ancestor(directory: &Path, ancestor: &str, descendant: &str) -> bool {
    git_output(directory, &["merge-base", "--is-ancestor", ancestor, descendant]).status.success()
}

/// Whether Git LFS is installed. Tests that need it check this and otherwise assert the refusal.
fn git_lfs_installed(directory: &Path) -> bool {
    git_output(directory, &["lfs", "version"]).status.success()
}

/// Waits so that a commit made now is strictly older than a cut-off of "now".
fn let_clock_pass() {
    thread::sleep(Duration::from_millis(1100));
}

// MARK: Git LFS

const POINTER: &str = "version https://git-lfs.github.com/spec/v1\noid sha256:0000000000000000000000000000000000000000000000000000000000000000\nsize 3\n";

#[test]
fn lfs_status_reads_patterns_quoted_patterns_and_pointer_files() {
    let repo = Repo::new();
    repo.commit(
        ".gitattributes",
        "*.psd filter=lfs diff=lfs merge=lfs -text\n\"my big file.bin\" filter=lfs diff=lfs merge=lfs -text\n# *.old filter=lfs\n*.txt text\n",
        "Attributes",
    );
    repo.commit("scene.psd", POINTER, "Pointer");
    repo.commit("full.psd", "real image bytes\n", "Full content");
    repo.commit("readme.txt", "plain\n", "Plain");

    let status = client().lfs_status(repo.path()).expect("lfs status");

    assert_eq!(status.patterns, vec!["*.psd".to_string(), "my big file.bin".to_string()]);
    assert_eq!(status.version.is_some(), git_lfs_installed(repo.path()));
    let mut files: Vec<(String, bool)> = status.files.iter().map(|file| (file.path.clone(), file.is_pointer_only)).collect();
    files.sort();
    assert_eq!(files, vec![("full.psd".to_string(), false), ("scene.psd".to_string(), true)]);
}

#[test]
fn track_is_refused_without_git_lfs_and_leaves_attributes_alone() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    if git_lfs_installed(repo.path()) {
        return;
    }

    let error = client().track_lfs("*.psd", repo.path()).expect_err("tracking must be refused");

    assert!(error.to_string().contains("Git LFS is not installed"), "unexpected message: {error}");
    assert!(!repo.path().join(".gitattributes").exists(), "no attributes file may be written");
}

#[test]
fn track_rejects_empty_and_multiline_patterns() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");

    assert!(client().track_lfs("   ", repo.path()).is_err());
    assert!(client().track_lfs("*.psd\n*.bin", repo.path()).is_err());
    assert!(!repo.path().join(".gitattributes").exists());
}

#[test]
fn track_and_untrack_round_trip_when_git_lfs_is_installed() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    if !git_lfs_installed(repo.path()) {
        return;
    }

    client().track_lfs("*.psd", repo.path()).expect("track");
    client().track_lfs("*.psd", repo.path()).expect("tracking twice is harmless");
    client().track_lfs("my big file.bin", repo.path()).expect("track quoted pattern");
    let status = client().lfs_status(repo.path()).expect("status");
    assert_eq!(status.patterns, vec!["*.psd".to_string(), "my big file.bin".to_string()]);

    client().untrack_lfs("*.psd", repo.path()).expect("untrack");
    let status = client().lfs_status(repo.path()).expect("status");
    assert_eq!(status.patterns, vec!["my big file.bin".to_string()]);
}

#[test]
fn untrack_removes_only_the_matching_lfs_line() {
    let repo = Repo::new();
    repo.commit(
        ".gitattributes",
        "*.psd filter=lfs diff=lfs merge=lfs -text\n*.txt text\n\"a b.bin\" filter=lfs diff=lfs merge=lfs -text\n",
        "Attributes",
    );

    client().untrack_lfs("*.psd", repo.path()).expect("untrack");

    assert_eq!(repo.read(".gitattributes"), "*.txt text\n\"a b.bin\" filter=lfs diff=lfs merge=lfs -text\n");
    // An untracked pattern changes nothing.
    client().untrack_lfs("*.missing", repo.path()).expect("untrack of unknown pattern");
    assert_eq!(repo.read(".gitattributes"), "*.txt text\n\"a b.bin\" filter=lfs diff=lfs merge=lfs -text\n");
}

// MARK: GitFlow

/// A repository with GitFlow set up on its `main` branch. Returns the repository and its HEAD.
fn gitflow_repo(version_tag_prefix: &str) -> (Repo, String) {
    let repo = Repo::new();
    let head = repo.commit("a.txt", "one\n", "First");
    let configuration = GitFlowConfiguration { version_tag_prefix: version_tag_prefix.to_string(), ..Default::default() };
    client().initialize_gitflow(&configuration, repo.path()).expect("initialize gitflow");
    (repo, head)
}

#[test]
fn gitflow_init_creates_develop_and_saves_settings() {
    let repo = Repo::new();
    let head = repo.commit("a.txt", "one\n", "First");

    let configuration = GitFlowConfiguration { version_tag_prefix: "v".to_string(), ..Default::default() };
    client().initialize_gitflow(&configuration, repo.path()).expect("initialize");

    assert_eq!(git(repo.path(), &["rev-parse", "refs/heads/develop"]).trim(), head);
    let stored = client().gitflow_configuration(repo.path()).expect("read config").expect("configured");
    assert_eq!(stored, configuration);
    assert_eq!(git(repo.path(), &["config", "--get", "gitflow.prefix.feature"]).trim(), "feature/");
}

#[test]
fn gitflow_init_refuses_missing_production_branch_and_equal_names() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");

    let missing = GitFlowConfiguration { main_branch: "trunk".to_string(), ..Default::default() };
    assert!(client().initialize_gitflow(&missing, repo.path()).is_err());
    let same = GitFlowConfiguration { develop_branch: "main".to_string(), ..Default::default() };
    assert!(client().initialize_gitflow(&same, repo.path()).is_err());
    assert!(client().gitflow_configuration(repo.path()).expect("read").is_none());
}

#[test]
fn gitflow_feature_start_and_finish_merges_without_fast_forward() {
    let (repo, head) = gitflow_repo("");

    client().start_gitflow(GitFlowKind::Feature, "login", "main", Some(&head), repo.path()).expect("start");
    assert_eq!(repo.git(&["branch", "--show-current"]).trim(), "feature/login");
    repo.commit("login.txt", "login\n", "Add login");
    let feature_head = current_head(repo.path());

    client()
        .finish_gitflow("feature/login", Some(&feature_head), None, repo.path())
        .expect("finish");

    assert!(!branch_exists(repo.path(), "feature/login"), "the feature branch is deleted after finishing");
    assert_eq!(repo.git(&["branch", "--show-current"]).trim(), "develop");
    let develop = current_head(repo.path());
    assert_eq!(repo.git(&["rev-list", "--parents", "-n", "1", &develop]).split_whitespace().count(), 3, "merge commit expected");
    assert!(!tag_exists(repo.path(), "feature/login"));
}

#[test]
fn gitflow_start_refuses_uncommitted_changes_and_creates_nothing() {
    let (repo, head) = gitflow_repo("");
    repo.write("a.txt", "changed\n");

    let error = client().start_gitflow(GitFlowKind::Feature, "dirty", "main", Some(&head), repo.path()).expect_err("refused");

    assert!(error.to_string().contains("Commit or stash"), "unexpected message: {error}");
    assert!(!branch_exists(repo.path(), "feature/dirty"));
    assert_eq!(repo.read("a.txt"), "changed\n", "the change is kept");
}

#[test]
fn gitflow_release_finish_tags_merges_both_branches_and_deletes_the_release() {
    let (repo, head) = gitflow_repo("v");
    client().start_gitflow(GitFlowKind::Release, "1.0.0", "main", Some(&head), repo.path()).expect("start release");
    repo.commit("CHANGELOG.md", "1.0.0\n", "Release notes");
    let release_head = current_head(repo.path());

    client()
        .finish_gitflow("release/1.0.0", Some(&release_head), Some("  Shipped  "), repo.path())
        .expect("finish release");

    assert!(!branch_exists(repo.path(), "release/1.0.0"));
    assert!(tag_exists(repo.path(), "v1.0.0"), "the version tag is created with the configured prefix");
    assert_eq!(git(repo.path(), &["cat-file", "-t", "refs/tags/v1.0.0"]).trim(), "tag", "the tag is annotated");
    assert_eq!(git(repo.path(), &["tag", "-l", "--format=%(contents:subject)", "v1.0.0"]).trim(), "Shipped");
    assert!(is_ancestor(repo.path(), &release_head, "refs/heads/main"), "production includes the release");
    assert!(is_ancestor(repo.path(), &release_head, "refs/heads/develop"), "development includes the release");
    assert_eq!(repo.git(&["branch", "--show-current"]).trim(), "develop");
}

#[test]
fn gitflow_finish_refuses_plain_branches_and_dirty_checkouts() {
    let (repo, head) = gitflow_repo("");
    let error = client().finish_gitflow("main", Some(&head), None, repo.path()).expect_err("main is not a flow branch");
    assert!(error.to_string().contains("not a GitFlow"), "unexpected message: {error}");

    client().start_gitflow(GitFlowKind::Feature, "dirty", "main", Some(&head), repo.path()).expect("start");
    let feature_head = current_head(repo.path());
    repo.write("a.txt", "modified\n");
    let error = client().finish_gitflow("feature/dirty", Some(&feature_head), None, repo.path()).expect_err("dirty");
    assert!(error.to_string().contains("Commit or stash"), "unexpected message: {error}");
    assert!(branch_exists(repo.path(), "feature/dirty"), "the branch is kept");
}

#[test]
fn gitflow_release_finish_resumes_after_its_merge_and_tag_were_made_earlier() {
    let (repo, head) = gitflow_repo("v");
    client().start_gitflow(GitFlowKind::Release, "2.0.0", "main", Some(&head), repo.path()).expect("start");
    repo.commit("notes.txt", "2\n", "Notes");
    let release_head = current_head(repo.path());

    // Simulate an interrupted finish: main already has the release and its tag.
    repo.git(&["switch", "main"]);
    repo.git(&["merge", "--no-ff", "--no-edit", "release/2.0.0"]);
    repo.git(&["tag", "-a", "-m", "Earlier attempt", "v2.0.0"]);
    let tag_before = git(repo.path(), &["rev-parse", "refs/tags/v2.0.0"]);
    repo.git(&["switch", "release/2.0.0"]);

    client().finish_gitflow("release/2.0.0", Some(&release_head), None, repo.path()).expect("resume finish");

    assert!(!branch_exists(repo.path(), "release/2.0.0"));
    assert_eq!(git(repo.path(), &["rev-parse", "refs/tags/v2.0.0"]), tag_before, "the existing tag is reused");
    assert!(is_ancestor(repo.path(), &release_head, "refs/heads/develop"));
}

#[test]
fn gitflow_finish_stops_on_conflict_and_finishes_after_it_is_resolved() {
    let repo = Repo::new();
    repo.commit("a.txt", "base\n", "Base");
    let configuration = GitFlowConfiguration::default();
    client().initialize_gitflow(&configuration, repo.path()).expect("initialize");
    let head = current_head(repo.path());
    client().start_gitflow(GitFlowKind::Release, "3.0.0", "main", Some(&head), repo.path()).expect("start");
    repo.commit("a.txt", "release\n", "Release change");
    let release_head = current_head(repo.path());
    // Development moves on and changes the same line, so merging the release into it conflicts.
    repo.git(&["switch", "develop"]);
    repo.commit("a.txt", "develop\n", "Develop change");
    repo.git(&["switch", "release/3.0.0"]);

    let error = client()
        .finish_gitflow("release/3.0.0", Some(&release_head), None, repo.path())
        .expect_err("the develop merge conflicts");
    assert!(error.to_string().contains("stopped"), "unexpected message: {error}");
    assert!(tag_exists(repo.path(), "3.0.0"), "production was merged and tagged before the conflict");
    assert!(branch_exists(repo.path(), "release/3.0.0"), "the branch is kept for the retry");

    // Resolve the conflict and conclude the merge on develop, then finish again.
    repo.write("a.txt", "resolved\n");
    repo.git(&["add", "--", "a.txt"]);
    repo.git(&["commit", "--quiet", "--no-edit"]);
    repo.git(&["switch", "release/3.0.0"]);
    let tag_before = git(repo.path(), &["rev-parse", "refs/tags/3.0.0"]);

    client().finish_gitflow("release/3.0.0", Some(&release_head), None, repo.path()).expect("finish after resolving");

    assert!(!branch_exists(repo.path(), "release/3.0.0"));
    assert_eq!(git(repo.path(), &["rev-parse", "refs/tags/3.0.0"]), tag_before, "the tag is not created twice");
    assert_eq!(repo.git(&["branch", "--show-current"]).trim(), "develop");
}

// MARK: Branch clean-up

/// `main` with one commit, a branch merged into it, and a branch with one commit of its own.
fn cleanup_repo() -> (Repo, String) {
    let repo = Repo::new();
    let base = repo.commit("a.txt", "one\n", "First");
    repo.git(&["branch", "merged-done"]);
    repo.git(&["switch", "-c", "unmerged-work"]);
    repo.commit("work.txt", "work\n", "Unmerged work");
    repo.git(&["switch", "main"]);
    (repo, base)
}

fn candidate<'a>(candidates: &'a [BranchCandidate], name: &str) -> Option<&'a BranchCandidate> {
    candidates.iter().find(|candidate| candidate.name == name)
}

#[test]
fn cleanup_lists_merged_branches_and_stale_ones_only() {
    let (repo, _) = cleanup_repo();
    repo.git(&["branch", "in-worktree"]);
    let worktree_dir = tempfile::tempdir().expect("worktree folder");
    let worktree = worktree_dir.path().join("checkout");
    repo.git(&["worktree", "add", worktree.to_str().expect("utf-8 path"), "in-worktree"]);

    let recent = client().branch_cleanup_candidates(90, repo.path()).expect("candidates");

    assert!(candidate(&recent, "merged-done").is_some_and(|candidate| candidate.is_merged));
    assert!(candidate(&recent, "unmerged-work").is_none(), "a recent unmerged branch is not listed");
    assert!(candidate(&recent, "main").is_none(), "the current branch is never listed");
    assert!(candidate(&recent, "in-worktree").is_none(), "a branch checked out elsewhere is never listed");

    let_clock_pass();
    let stale = client().branch_cleanup_candidates(0, repo.path()).expect("candidates");
    let unmerged = candidate(&stale, "unmerged-work").expect("stale unmerged branch is listed");
    assert!(!unmerged.is_merged);
    assert!(unmerged.last_commit_time.is_some());
    assert_eq!(unmerged.subject, "Unmerged work");
}

#[test]
fn cleanup_deletes_merged_branches_and_restore_brings_back_upstream() {
    let (repo, _) = cleanup_repo();
    repo.git(&["config", "branch.merged-done.remote", "origin"]);
    repo.git(&["config", "branch.merged-done.merge", "refs/heads/merged-done"]);
    let candidates = client().branch_cleanup_candidates(90, repo.path()).expect("candidates");
    let merged: Vec<BranchCandidate> = candidates.into_iter().filter(|candidate| candidate.is_merged).collect();
    let tip = git(repo.path(), &["rev-parse", "refs/heads/merged-done"]).trim().to_string();

    let report = client().delete_branches_keeping_undo(&merged, false, repo.path());

    assert!(report.failure.is_none(), "unexpected failure: {:?}", report.failure);
    assert!(!branch_exists(repo.path(), "merged-done"));
    assert_eq!(report.deleted.len(), 1);
    let deletion = report.deleted[0].clone();
    assert_eq!(deletion.tip, tip);
    assert_eq!(deletion.upstream_remote.as_deref(), Some("origin"));

    client().restore_deleted_branch(&deletion, repo.path()).expect("restore");
    assert_eq!(git(repo.path(), &["rev-parse", "refs/heads/merged-done"]).trim(), tip);
    assert_eq!(repo.git(&["config", "branch.merged-done.remote"]).trim(), "origin");
    assert_eq!(repo.git(&["config", "branch.merged-done.merge"]).trim(), "refs/heads/merged-done");

    let error = client().restore_deleted_branch(&deletion, repo.path()).expect_err("the name is taken again");
    assert!(error.to_string().contains("exists again"), "unexpected message: {error}");
}

#[test]
fn cleanup_keeps_unmerged_branches_unless_asked_and_refuses_moved_ones() {
    let (repo, _) = cleanup_repo();
    let_clock_pass();
    let stale = client().branch_cleanup_candidates(0, repo.path()).expect("candidates");
    let unmerged: Vec<BranchCandidate> = stale.iter().filter(|candidate| !candidate.is_merged).cloned().collect();
    assert_eq!(unmerged.len(), 1);

    let skipped = client().delete_branches_keeping_undo(&unmerged, false, repo.path());
    assert!(skipped.deleted.is_empty() && skipped.failure.is_none());
    assert!(branch_exists(repo.path(), "unmerged-work"));

    // The branch moves after it was listed; the deletion must be refused and the new commit kept.
    repo.git(&["switch", "unmerged-work"]);
    repo.commit("more.txt", "more\n", "More work");
    repo.git(&["switch", "main"]);
    let moved_tip = git(repo.path(), &["rev-parse", "refs/heads/unmerged-work"]).trim().to_string();

    let refused = client().delete_branches_keeping_undo(&unmerged, true, repo.path());
    assert!(refused.deleted.is_empty());
    assert!(refused.failure.is_some(), "a moved branch stops the clean-up");
    assert_eq!(git(repo.path(), &["rev-parse", "refs/heads/unmerged-work"]).trim(), moved_tip);
}

#[test]
fn cleanup_deletes_an_unmerged_branch_when_asked_and_restores_it() {
    let (repo, _) = cleanup_repo();
    let_clock_pass();
    let stale = client().branch_cleanup_candidates(0, repo.path()).expect("candidates");
    let unmerged: Vec<BranchCandidate> = stale.into_iter().filter(|candidate| !candidate.is_merged).collect();
    let tip = unmerged[0].tip.clone();

    let report = client().delete_branches_keeping_undo(&unmerged, true, repo.path());

    assert!(report.failure.is_none());
    assert!(!branch_exists(repo.path(), "unmerged-work"));
    let deletion: BranchDeletion = report.deleted[0].clone();
    client().restore_deleted_branch(&deletion, repo.path()).expect("restore");
    assert_eq!(git(repo.path(), &["rev-parse", "refs/heads/unmerged-work"]).trim(), tip);
}

// MARK: Reflog

#[test]
fn reflog_lists_entries_and_marks_commits_left_behind_by_a_reset() {
    let repo = Repo::new();
    let first = repo.commit("a.txt", "one\n", "First");
    let second = repo.commit("a.txt", "two\n", "Second");
    repo.git(&["reset", "--hard", &first]);

    let entries = client().reflog(50, repo.path()).expect("reflog");

    assert!(entries.len() >= 3, "every position HEAD had is listed: {entries:?}");
    assert_eq!(entries[0].selector, "HEAD@{0}");
    assert!(entries[0].action.starts_with("reset:"), "the newest entry is the reset: {:?}", entries[0]);
    assert_eq!(entries[0].hash, first);
    assert_eq!(entries[1].hash, second);
    assert_eq!(entries[1].subject, "Second");
    assert!(entries[1].time.is_some());

    let hashes: Vec<String> = entries.iter().map(|entry| entry.hash.clone()).collect();
    let unreachable = client().unreachable_commits(&hashes, repo.path()).expect("unreachable");
    assert!(unreachable.contains(&second), "the reset-away commit is on no branch");
    assert!(!unreachable.contains(&first), "the commit the branch still points to is reachable");
}

#[test]
fn reflog_is_empty_before_the_first_commit() {
    let repo = Repo::new();
    assert!(client().reflog(50, repo.path()).expect("reflog").is_empty());
}

#[test]
fn creating_a_branch_at_a_reflog_entry_keeps_the_commit_reachable() {
    let repo = Repo::new();
    let first = repo.commit("a.txt", "one\n", "First");
    let second = repo.commit("a.txt", "two\n", "Second");
    repo.git(&["reset", "--hard", &first]);
    let before = client().reflog(50, repo.path()).expect("reflog");
    let hashes: Vec<String> = before.iter().map(|entry| entry.hash.clone()).collect();
    assert!(client().unreachable_commits(&hashes, repo.path()).expect("unreachable").contains(&second));

    client().create_branch_at("rescued", &second, repo.path()).expect("create branch");

    assert_eq!(git(repo.path(), &["rev-parse", "refs/heads/rescued"]).trim(), second);
    assert_eq!(repo.git(&["branch", "--show-current"]).trim(), "main", "the checkout does not change");
    assert!(client().unreachable_commits(&hashes, repo.path()).expect("unreachable").is_empty());
    assert!(client().create_branch_at("rescued", &second, repo.path()).is_err(), "an existing name is refused");
}
