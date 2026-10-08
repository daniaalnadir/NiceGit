//! Integration tests for history rewriting and undo, run against real repositories in temporary
//! directories: interactive rebase, rebase, cherry-pick, revert, merge previews, and undo.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use nicegit_core::client::GitClient;
use nicegit_core::merge_preview::MergeOutcome;
use nicegit_core::models::{Branch, GitError, Operation, ResetMode, Snapshot, StatusEntry};
use nicegit_core::rebase::{Expectation, RebaseAction, RebasePlan, RebaseStep};
use nicegit_core::undo::{branch_move_step, BranchDeletion, UndoMode, UndoStep};
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

    /// Like `commit`, with the author and committer dates set to `time` (Unix seconds), so
    /// commits can be ordered by time.
    fn commit_at(&self, relative: &str, contents: &str, message: &str, time: i64) -> String {
        self.write(relative, contents);
        self.git(&["add", "--", relative]);
        let date = format!("@{time} +0000");
        let output = Command::new("git")
            .args(["commit", "--quiet", "-m", message])
            .current_dir(self.path())
            .env("GIT_AUTHOR_DATE", &date)
            .env("GIT_COMMITTER_DATE", &date)
            .output()
            .expect("run git commit");
        assert!(output.status.success(), "commit failed: {}", String::from_utf8_lossy(&output.stderr));
        self.head()
    }
}

fn client() -> GitClient {
    GitClient::new()
}

fn snapshot(directory: &Path) -> Snapshot {
    client().load_snapshot(directory, 100).expect("load snapshot")
}

fn local_branch(snapshot: &Snapshot, name: &str) -> Branch {
    snapshot
        .branches
        .iter()
        .find(|branch| !branch.is_remote && branch.name == name)
        .cloned()
        .unwrap_or_else(|| panic!("no local branch {name}"))
}

fn entry(directory: &Path, path: &str) -> StatusEntry {
    client()
        .load_status(directory)
        .expect("load status")
        .into_iter()
        .find(|entry| entry.path == path)
        .unwrap_or_else(|| panic!("no status entry for {path}"))
}

fn branch_exists(directory: &Path, name: &str) -> bool {
    git_output(directory, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{name}")]).status.success()
}

/// The message of the refusal, panicking if the call succeeded.
fn refusal<T: std::fmt::Debug>(result: Result<T, GitError>) -> String {
    match result {
        Ok(value) => panic!("expected a refusal, got Ok({value:?})"),
        Err(error) => error.to_string(),
    }
}

/// Commit subjects from `base` (exclusive) through HEAD, oldest first.
fn subjects_after(directory: &Path, base: &str) -> Vec<String> {
    git(directory, &["log", "--reverse", "--format=%s", &format!("{base}..HEAD")]).lines().map(str::to_string).collect()
}

/// Commit subjects of the whole history, oldest first.
fn all_subjects(directory: &Path) -> Vec<String> {
    git(directory, &["log", "--reverse", "--format=%s"]).lines().map(str::to_string).collect()
}

/// Commits from `base` (exclusive) through HEAD, oldest first, as full hashes.
fn commits_after(directory: &Path, base: &str) -> Vec<String> {
    git(directory, &["rev-list", "--reverse", &format!("{base}..HEAD")]).lines().map(str::to_string).collect()
}

fn full_message(directory: &Path, commit: &str) -> String {
    git(directory, &["show", "--no-patch", "--format=%B", commit])
}

fn files_at(directory: &Path, commit: &str) -> Vec<String> {
    git(directory, &["ls-tree", "--name-only", commit]).lines().map(str::to_string).collect()
}

/// Builds `base` followed by three commits adding one file each: "Add one", "Add two", "Add three".
/// Returns the base and the three commits, oldest first.
fn linear_history(repo: &Repo) -> (String, String, String, String) {
    let base = repo.commit("base.txt", "base\n", "Base");
    let one = repo.commit("one.txt", "one\n", "Add one");
    let two = repo.commit("two.txt", "two\n", "Add two");
    let three = repo.commit("three.txt", "three\n", "Add three");
    (base, one, two, three)
}

fn plan_after(directory: &Path, base: &str) -> RebasePlan {
    client().interactive_rebase_plan(Some(base), directory).expect("plan rebase")
}

fn step(plan: &RebasePlan, index: usize, action: RebaseAction) -> RebaseStep {
    RebaseStep { commit: plan.commits[index].clone(), action }
}

/// Applies `steps` to the current branch `main`, shown at HEAD.
fn rewrite(repo: &Repo, steps: &[RebaseStep], plan: &RebasePlan) -> Result<(), GitError> {
    let head = repo.head();
    client().interactive_rebase(steps, plan, "main", &head, repo.path())
}

// MARK: Interactive rebase

#[test]
fn plan_lists_commits_after_base_oldest_first() {
    let repo = Repo::new();
    let (base, one, two, three) = linear_history(&repo);
    let plan = plan_after(repo.path(), &base);
    let hashes: Vec<&str> = plan.commits.iter().map(|commit| commit.hash.as_str()).collect();
    assert_eq!(hashes, [one.as_str(), two.as_str(), three.as_str()]);
    assert_eq!(plan.base.as_deref(), Some(base.as_str()));
    assert_eq!(plan.messages.get(&one).map(String::as_str), Some("Add one"));
    assert!(plan.published_commits.is_empty(), "no commit is on a remote yet");
}

#[test]
fn plan_marks_commits_on_a_remote_as_published() {
    let repo = Repo::new();
    let (base, one, two, _) = linear_history(&repo);
    repo.git(&["update-ref", "refs/remotes/origin/main", &one]);
    let plan = plan_after(repo.path(), &base);
    assert!(plan.published_commits.contains(&one));
    assert!(!plan.published_commits.contains(&two));
}

#[test]
fn plan_refuses_commit_outside_current_history() {
    let repo = Repo::new();
    let (base, _, _, _) = linear_history(&repo);
    repo.git(&["switch", "-c", "other", &base]);
    let other = repo.commit("other.txt", "other\n", "Other");
    repo.git(&["switch", "main"]);
    let message = refusal(client().interactive_rebase_plan(Some(&other), repo.path()));
    assert!(message.contains("not part of the current branch"), "{message}");
}

#[test]
fn interactive_rebase_reorders_commits() {
    let repo = Repo::new();
    let (base, _, _, _) = linear_history(&repo);
    let plan = plan_after(repo.path(), &base);
    let steps = [step(&plan, 1, RebaseAction::Pick), step(&plan, 0, RebaseAction::Pick), step(&plan, 2, RebaseAction::Pick)];
    rewrite(&repo, &steps, &plan).expect("rewrite");
    assert_eq!(subjects_after(repo.path(), &base), ["Add two", "Add one", "Add three"]);
}

#[test]
fn interactive_rebase_rewords_message_keeping_hash_lines() {
    let repo = Repo::new();
    let (base, _, _, _) = linear_history(&repo);
    let plan = plan_after(repo.path(), &base);
    let message = "Reworded subject\n\nBody line.\n# A line that starts with a hash\n";
    let steps =
        [step(&plan, 0, RebaseAction::Reword(message.to_string())), step(&plan, 1, RebaseAction::Pick), step(&plan, 2, RebaseAction::Pick)];
    rewrite(&repo, &steps, &plan).expect("rewrite");
    let first = commits_after(repo.path(), &base)[0].clone();
    let stored = full_message(repo.path(), &first);
    assert!(stored.starts_with("Reworded subject\n"), "{stored:?}");
    assert!(stored.contains("# A line that starts with a hash"), "{stored:?}");
    assert_eq!(subjects_after(repo.path(), &base), ["Reworded subject", "Add two", "Add three"]);
}

#[test]
fn interactive_rebase_refuses_empty_reword_message() {
    let repo = Repo::new();
    let (base, _, _, _) = linear_history(&repo);
    let plan = plan_after(repo.path(), &base);
    let steps =
        [step(&plan, 0, RebaseAction::Reword("   \n".to_string())), step(&plan, 1, RebaseAction::Pick), step(&plan, 2, RebaseAction::Pick)];
    let error = rewrite(&repo, &steps, &plan).expect_err("empty message refused");
    assert_eq!(error, GitError::EmptyCommitMessage);
    assert_eq!(subjects_after(repo.path(), &base), ["Add one", "Add two", "Add three"], "nothing was rewritten");
}

#[test]
fn interactive_rebase_squash_combines_messages() {
    let repo = Repo::new();
    let (base, _, _, _) = linear_history(&repo);
    let plan = plan_after(repo.path(), &base);
    let steps = [step(&plan, 0, RebaseAction::Pick), step(&plan, 1, RebaseAction::Squash), step(&plan, 2, RebaseAction::Pick)];
    rewrite(&repo, &steps, &plan).expect("rewrite");
    assert_eq!(subjects_after(repo.path(), &base), ["Add one", "Add three"]);
    let first = commits_after(repo.path(), &base)[0].clone();
    let message = full_message(repo.path(), &first);
    assert!(message.contains("Add one") && message.contains("Add two"), "{message:?}");
    let files = files_at(repo.path(), &first);
    assert!(files.contains(&"one.txt".to_string()) && files.contains(&"two.txt".to_string()), "{files:?}");
}

#[test]
fn interactive_rebase_fixup_keeps_only_the_kept_message() {
    let repo = Repo::new();
    let (base, _, _, _) = linear_history(&repo);
    let plan = plan_after(repo.path(), &base);
    let steps = [step(&plan, 0, RebaseAction::Pick), step(&plan, 1, RebaseAction::Fixup), step(&plan, 2, RebaseAction::Pick)];
    rewrite(&repo, &steps, &plan).expect("rewrite");
    assert_eq!(subjects_after(repo.path(), &base), ["Add one", "Add three"]);
    let first = commits_after(repo.path(), &base)[0].clone();
    assert!(!full_message(repo.path(), &first).contains("Add two"), "fixup discards its message");
    assert!(files_at(repo.path(), &first).contains(&"two.txt".to_string()), "fixup keeps its changes");
}

#[test]
fn interactive_rebase_drop_removes_the_commit_and_its_changes() {
    let repo = Repo::new();
    let (base, _, _, _) = linear_history(&repo);
    let plan = plan_after(repo.path(), &base);
    let steps = [step(&plan, 0, RebaseAction::Pick), step(&plan, 1, RebaseAction::Drop), step(&plan, 2, RebaseAction::Pick)];
    rewrite(&repo, &steps, &plan).expect("rewrite");
    assert_eq!(subjects_after(repo.path(), &base), ["Add one", "Add three"]);
    assert!(!files_at(repo.path(), "HEAD").contains(&"two.txt".to_string()));
}

#[test]
fn interactive_rebase_refuses_squash_into_nothing() {
    let repo = Repo::new();
    let (base, _, _, _) = linear_history(&repo);
    let plan = plan_after(repo.path(), &base);
    let steps = [step(&plan, 0, RebaseAction::Squash), step(&plan, 1, RebaseAction::Pick), step(&plan, 2, RebaseAction::Pick)];
    let message = refusal(rewrite(&repo, &steps, &plan).map(|_| ()));
    assert!(message.contains("nothing to squash into"), "{message}");
}

#[test]
fn interactive_rebase_refuses_stale_head_and_dirty_tree() {
    let repo = Repo::new();
    let (base, _, _, _) = linear_history(&repo);
    let plan = plan_after(repo.path(), &base);
    let steps = [step(&plan, 0, RebaseAction::Pick), step(&plan, 1, RebaseAction::Pick), step(&plan, 2, RebaseAction::Pick)];

    let stale = refusal(client().interactive_rebase(&steps, &plan, "main", &base, repo.path()));
    assert!(stale.contains("changed since this action"), "{stale}");

    repo.write("base.txt", "edited\n");
    let dirty = refusal(rewrite(&repo, &steps, &plan).map(|_| ()));
    assert!(dirty.contains("Commit or stash"), "{dirty}");
    assert_eq!(subjects_after(repo.path(), &base), ["Add one", "Add two", "Add three"]);
}

#[test]
fn interactive_rebase_from_root_rewrites_whole_history() {
    let repo = Repo::new();
    repo.commit("root.txt", "root\n", "Add root");
    repo.commit("one.txt", "one\n", "Add one");
    repo.commit("two.txt", "two\n", "Add two");
    let plan = client().interactive_rebase_plan(None, repo.path()).expect("plan from root");
    assert_eq!(plan.base, None);
    assert_eq!(plan.commits.len(), 3);
    let steps = [
        step(&plan, 0, RebaseAction::Reword("Root reworded".to_string())),
        step(&plan, 1, RebaseAction::Pick),
        step(&plan, 2, RebaseAction::Drop),
    ];
    rewrite(&repo, &steps, &plan).expect("rewrite from root");
    assert_eq!(all_subjects(repo.path()), ["Root reworded", "Add one"]);
}

// MARK: Rebase, cherry-pick, and revert

#[test]
fn rebase_onto_branch_replays_the_current_branch() {
    let repo = Repo::new();
    let base = repo.commit("base.txt", "base\n", "Base");
    let main_tip = repo.commit("main.txt", "main\n", "Main change");
    repo.git(&["switch", "-c", "feature", &base]);
    let feature_tip = repo.commit("feature.txt", "feature\n", "Feature change");

    let snapshot = snapshot(repo.path());
    let main = local_branch(&snapshot, "main");
    client()
        .start(
            Operation::Rebase,
            "main",
            None,
            Expectation { branch: Some("feature"), head: Some(&feature_tip), source_branch: Some(&main) },
            repo.path(),
        )
        .expect("rebase onto main");

    assert!(git_output(repo.path(), &["merge-base", "--is-ancestor", &main_tip, "HEAD"]).status.success());
    assert!(files_at(repo.path(), "HEAD").contains(&"feature.txt".to_string()));
}

#[test]
fn rebase_refuses_source_branch_whose_tip_moved() {
    let repo = Repo::new();
    let base = repo.commit("base.txt", "base\n", "Base");
    repo.commit("main.txt", "main\n", "Main change");
    repo.git(&["switch", "-c", "feature", &base]);
    let feature_tip = repo.commit("feature.txt", "feature\n", "Feature change");

    let snapshot = snapshot(repo.path());
    let mut main = local_branch(&snapshot, "main");
    main.tip = base.clone();
    let message = refusal(client().start(
        Operation::Rebase,
        "main",
        None,
        Expectation { branch: Some("feature"), head: Some(&feature_tip), source_branch: Some(&main) },
        repo.path(),
    ));
    assert!(message.contains("selected branch changed"), "{message}");
    assert_eq!(repo.head(), feature_tip, "the checkout was not rebased");
}

#[test]
fn cherry_pick_applies_several_commits_oldest_first() {
    let repo = Repo::new();
    let base = repo.commit_at("base.txt", "base\n", "Base", 1_700_000_000);
    repo.git(&["switch", "-c", "side", &base]);
    let first = repo.commit_at("s1.txt", "one\n", "Side one", 1_700_000_100);
    let second = repo.commit_at("s2.txt", "two\n", "Side two", 1_700_000_200);
    let third = repo.commit_at("s3.txt", "three\n", "Side three", 1_700_000_300);
    repo.git(&["switch", "main"]);

    client().cherry_pick(&[third.clone(), first.clone(), second.clone()], Some(&base), "main", repo.path()).expect("cherry-pick");
    assert_eq!(subjects_after(repo.path(), &base), ["Side one", "Side two", "Side three"]);
}

#[test]
fn cherry_pick_refuses_stale_head() {
    let repo = Repo::new();
    let base = repo.commit_at("base.txt", "base\n", "Base", 1_700_000_000);
    repo.git(&["switch", "-c", "side", &base]);
    let side = repo.commit_at("s1.txt", "one\n", "Side one", 1_700_000_100);
    repo.git(&["switch", "main"]);
    let message = refusal(client().cherry_pick(std::slice::from_ref(&side), Some(&side), "main", repo.path()));
    assert!(message.contains("changed since this action"), "{message}");
    assert_eq!(repo.head(), base);
}

#[test]
fn revert_undoes_one_commit() {
    let repo = Repo::new();
    repo.commit("base.txt", "base\n", "Base");
    let change = repo.commit("one.txt", "one\n", "Add one");
    client()
        .start(
            Operation::Revert,
            &change,
            None,
            Expectation { branch: Some("main"), head: Some(&change), source_branch: None },
            repo.path(),
        )
        .expect("revert");
    assert!(!files_at(repo.path(), "HEAD").contains(&"one.txt".to_string()));
    assert!(git(repo.path(), &["log", "-1", "--format=%s"]).starts_with("Revert"));
}

#[test]
fn revert_of_merge_keeps_the_chosen_parent_side() {
    let repo = Repo::new();
    repo.commit("base.txt", "base\n", "Base");
    repo.git(&["switch", "-c", "side"]);
    repo.commit("side.txt", "side\n", "Side change");
    repo.git(&["switch", "main"]);
    repo.commit("main.txt", "main\n", "Main change");
    repo.git(&["merge", "--no-ff", "--no-edit", "side"]);
    let merge = repo.head();

    client()
        .start(
            Operation::Revert,
            &merge,
            Some(1),
            Expectation { branch: Some("main"), head: Some(&merge), source_branch: None },
            repo.path(),
        )
        .expect("revert merge with mainline 1");
    let files = files_at(repo.path(), "HEAD");
    assert!(files.contains(&"main.txt".to_string()), "{files:?}");
    assert!(!files.contains(&"side.txt".to_string()), "{files:?}");
}

#[test]
fn is_published_reports_commits_contained_by_a_remote_branch() {
    let repo = Repo::new();
    let (_, one, two, _) = linear_history(&repo);
    repo.git(&["update-ref", "refs/remotes/origin/main", &one]);
    assert!(client().is_published(&one, repo.path()).expect("check"));
    assert!(!client().is_published(&two, repo.path()).expect("check"));
}

// MARK: Merge preview

#[test]
fn merge_preview_reports_up_to_date_when_source_is_contained() {
    let repo = Repo::new();
    let base = repo.commit("base.txt", "base\n", "Base");
    repo.commit("main.txt", "main\n", "Main change");
    let preview = client().preview_merge(&base, repo.path()).expect("preview");
    assert_eq!(preview.outcome, MergeOutcome::UpToDate);
    assert_eq!(preview.changed_file_count, 0);
    assert!(!preview.blocked_by_ignored_files);
    assert!(!preview.is_estimate);
}

#[test]
fn merge_preview_reports_fast_forward_with_file_count() {
    let repo = Repo::new();
    let base = repo.commit("base.txt", "base\n", "Base");
    repo.git(&["switch", "-c", "feature"]);
    repo.commit("f1.txt", "one\n", "Feature one");
    repo.commit("f2.txt", "two\n", "Feature two");
    repo.git(&["switch", "main"]);
    assert_eq!(repo.head(), base);
    let preview = client().preview_merge("feature", repo.path()).expect("preview");
    assert_eq!(preview.outcome, MergeOutcome::FastForward);
    assert_eq!(preview.changed_file_count, 2);
}

#[test]
fn merge_preview_reports_clean_merge_with_changed_files() {
    let repo = Repo::new();
    let base = repo.commit("base.txt", "base\n", "Base");
    repo.commit("main.txt", "main\n", "Main change");
    repo.git(&["switch", "-c", "side", &base]);
    repo.commit("side.txt", "side\n", "Side change");
    repo.git(&["switch", "main"]);
    let preview = client().preview_merge("side", repo.path()).expect("preview");
    assert_eq!(preview.outcome, MergeOutcome::Clean);
    assert_eq!(preview.changed_file_count, 1, "only side.txt would be added to the checkout");
}

#[test]
fn merge_preview_reports_expected_conflicts() {
    let repo = Repo::new();
    let base = repo.commit("a.txt", "base\n", "Base");
    repo.commit("a.txt", "main\n", "Main change");
    repo.git(&["switch", "-c", "side", &base]);
    repo.commit("a.txt", "side\n", "Side change");
    repo.git(&["switch", "main"]);
    let preview = client().preview_merge("side", repo.path()).expect("preview");
    assert_eq!(preview.outcome, MergeOutcome::Conflicts(vec!["a.txt".to_string()]));
}

#[test]
fn rebase_preview_is_marked_as_an_estimate() {
    let repo = Repo::new();
    let base = repo.commit("base.txt", "base\n", "Base");
    repo.commit("main.txt", "main\n", "Main change");
    repo.git(&["switch", "-c", "side", &base]);
    repo.commit("side.txt", "side\n", "Side change");
    repo.git(&["switch", "main"]);
    let preview = client().preview_rebase("side", repo.path()).expect("preview");
    assert!(preview.is_estimate);
    assert_eq!(preview.outcome, MergeOutcome::Clean);
}

// MARK: Undo of branch moves

/// Moves `main` from `before` to `after` with a real commit, and returns the snapshots taken.
fn commit_and_snapshots(repo: &Repo, relative: &str, message: &str) -> (Snapshot, Snapshot) {
    let before = snapshot(repo.path());
    repo.commit(relative, "content\n", message);
    (before, snapshot(repo.path()))
}

#[test]
fn undo_commit_keeps_its_changes_staged_and_redo_restores_it() {
    let repo = Repo::new();
    repo.commit("base.txt", "base\n", "Base");
    let first = repo.commit("one.txt", "one\n", "Add one");
    let (before, after) = commit_and_snapshots(&repo, "two.txt", "Add two");
    let second = repo.head();
    let recorded = branch_move_step(&before, &after, UndoMode::Soft).expect("commit is recorded");
    assert_eq!(
        recorded,
        UndoStep::BranchMove { branch: "main".to_string(), before: first.clone(), after: second.clone(), mode: UndoMode::Soft }
    );

    let redo = client().undo_step(&recorded, repo.path()).expect("undo commit");
    assert_eq!(repo.head(), first);
    assert!(git(repo.path(), &["diff", "--cached", "--name-only"]).contains("two.txt"), "the commit's changes stay staged");

    let undone_again = client().undo_step(&redo, repo.path()).expect("redo commit");
    assert_eq!(repo.head(), second);
    assert_eq!(undone_again, recorded, "undoing the redo gives back the original step");
}

#[test]
fn undo_hard_reset_restores_files_and_redo_removes_them_again() {
    let repo = Repo::new();
    repo.commit("base.txt", "base\n", "Base");
    let first = repo.commit("one.txt", "one\n", "Add one");
    let second = repo.commit("two.txt", "two\n", "Add two");
    let before = snapshot(repo.path());
    client().reset(&first, ResetMode::Hard, "main", &second, repo.path()).expect("hard reset");
    let after = snapshot(repo.path());
    assert!(!repo.path().join("two.txt").exists());

    let recorded = branch_move_step(&before, &after, UndoMode::for_reset(ResetMode::Hard)).expect("reset is recorded");
    assert_eq!(
        recorded,
        UndoStep::BranchMove { branch: "main".to_string(), before: second.clone(), after: first.clone(), mode: UndoMode::Keep }
    );
    let redo = client().undo_step(&recorded, repo.path()).expect("undo hard reset");
    assert_eq!(repo.head(), second);
    assert_eq!(repo.read("two.txt"), "two\n");

    client().undo_step(&redo, repo.path()).expect("redo hard reset");
    assert_eq!(repo.head(), first);
    assert!(!repo.path().join("two.txt").exists());
}

#[test]
fn undo_soft_reset_keeps_changes_staged() {
    let repo = Repo::new();
    repo.commit("base.txt", "base\n", "Base");
    let first = repo.commit("one.txt", "one\n", "Add one");
    let second = repo.commit("two.txt", "two\n", "Add two");
    let before = snapshot(repo.path());
    client().reset(&first, ResetMode::Soft, "main", &second, repo.path()).expect("soft reset");
    let after = snapshot(repo.path());
    let recorded = branch_move_step(&before, &after, UndoMode::for_reset(ResetMode::Soft)).expect("reset is recorded");
    client().undo_step(&recorded, repo.path()).expect("undo soft reset");
    assert_eq!(repo.head(), second);
    assert!(git(repo.path(), &["diff", "--cached", "--name-only"]).is_empty());
}

#[test]
fn undo_mixed_reset_leaves_a_clean_tree() {
    let repo = Repo::new();
    repo.commit("base.txt", "base\n", "Base");
    let first = repo.commit("one.txt", "one\n", "Add one");
    let second = repo.commit("two.txt", "two\n", "Add two");
    let before = snapshot(repo.path());
    client().reset(&first, ResetMode::Mixed, "main", &second, repo.path()).expect("mixed reset");
    let after = snapshot(repo.path());
    let recorded = branch_move_step(&before, &after, UndoMode::for_reset(ResetMode::Mixed)).expect("reset is recorded");
    client().undo_step(&recorded, repo.path()).expect("undo mixed reset");
    assert_eq!(repo.head(), second);
    assert_eq!(git(repo.path(), &["status", "--porcelain"]), "");
}

#[test]
fn undo_refuses_when_the_branch_moved_since_it_was_recorded() {
    let repo = Repo::new();
    let base = repo.commit("base.txt", "base\n", "Base");
    let second = repo.commit("two.txt", "two\n", "Add two");
    let stale = UndoStep::BranchMove { branch: "main".to_string(), before: base.clone(), after: base.clone(), mode: UndoMode::Soft };
    let message = refusal(client().undo_step(&stale, repo.path()));
    assert!(message.contains("changed"), "{message}");
    assert_eq!(repo.head(), second, "the branch was not moved");
}

#[test]
fn no_undo_step_is_recorded_while_an_operation_is_in_progress() {
    let repo = Repo::new();
    repo.commit("a.txt", "base\n", "Base");
    repo.git(&["switch", "-c", "side"]);
    repo.commit("a.txt", "side\n", "Side change");
    repo.git(&["switch", "main"]);
    repo.commit("a.txt", "main\n", "Main change");
    let before = snapshot(repo.path());
    let merge = git_output(repo.path(), &["merge", "--no-edit", "side"]);
    assert!(!merge.status.success(), "the merge should conflict");
    let after = snapshot(repo.path());
    assert!(after.operation.is_some());
    assert_eq!(branch_move_step(&before, &after, UndoMode::Keep), None);
}

// MARK: Undo of branch deletions

fn deletion_of(repo: &Repo, name: &str) -> BranchDeletion {
    let snapshot = snapshot(repo.path());
    let branch = local_branch(&snapshot, name);
    client().delete_branch_keeping_undo(&branch, false, repo.path()).expect("delete branch")
}

#[test]
fn undo_branch_deletion_restores_branch_and_upstream_settings() {
    let repo = Repo::new();
    let base = repo.commit("base.txt", "base\n", "Base");
    repo.git(&["branch", "feature", &base]);
    repo.git(&["config", "branch.feature.remote", "origin"]);
    repo.git(&["config", "branch.feature.merge", "refs/heads/feature"]);

    let deletion = deletion_of(&repo, "feature");
    assert_eq!(deletion.upstream_remote.as_deref(), Some("origin"));
    assert_eq!(deletion.upstream_merge.as_deref(), Some("refs/heads/feature"));
    assert!(!branch_exists(repo.path(), "feature"));

    let recorded = UndoStep::BranchDeletions(vec![deletion]);
    let redo = client().undo_step(&recorded, repo.path()).expect("restore branch");
    assert_eq!(git(repo.path(), &["rev-parse", "refs/heads/feature"]).trim(), base);
    assert_eq!(git(repo.path(), &["config", "branch.feature.remote"]).trim(), "origin");
    assert_eq!(git(repo.path(), &["config", "branch.feature.merge"]).trim(), "refs/heads/feature");

    client().undo_step(&redo, repo.path()).expect("delete again");
    assert!(!branch_exists(repo.path(), "feature"));
}

#[test]
fn undo_branch_deletion_refuses_when_the_name_is_reused() {
    let repo = Repo::new();
    let base = repo.commit("base.txt", "base\n", "Base");
    repo.git(&["branch", "feature", &base]);
    let deletion = deletion_of(&repo, "feature");
    let later = repo.commit("two.txt", "two\n", "Add two");
    repo.git(&["branch", "feature", &later]);

    let message = refusal(client().undo_step(&UndoStep::BranchDeletions(vec![deletion]), repo.path()));
    assert!(message.contains("exists again"), "{message}");
    assert_eq!(git(repo.path(), &["rev-parse", "refs/heads/feature"]).trim(), later, "the reused branch is untouched");
}

#[test]
fn redo_branch_deletion_refuses_when_the_restored_branch_moved() {
    let repo = Repo::new();
    let base = repo.commit("base.txt", "base\n", "Base");
    repo.git(&["branch", "feature", &base]);
    let deletion = deletion_of(&repo, "feature");
    let redo = client().undo_step(&UndoStep::BranchDeletions(vec![deletion]), repo.path()).expect("restore");
    let moved = repo.commit("two.txt", "two\n", "Add two");
    repo.git(&["update-ref", "refs/heads/feature", &moved, &base]);

    let message = refusal(client().undo_step(&redo, repo.path()));
    assert!(message.contains("changed after it was restored"), "{message}");
    assert_eq!(git(repo.path(), &["rev-parse", "refs/heads/feature"]).trim(), moved);
}

#[test]
fn force_deleted_unmerged_branch_is_restored_at_its_tip() {
    let repo = Repo::new();
    let base = repo.commit("base.txt", "base\n", "Base");
    repo.git(&["switch", "-c", "feature", &base]);
    let tip = repo.commit("feature.txt", "feature\n", "Feature change");
    repo.git(&["switch", "main"]);

    let snapshot = snapshot(repo.path());
    let branch = local_branch(&snapshot, "feature");
    let deletion = client().delete_branch_keeping_undo(&branch, true, repo.path()).expect("force delete");
    assert!(!branch_exists(repo.path(), "feature"));
    client().undo_step(&UndoStep::BranchDeletions(vec![deletion]), repo.path()).expect("restore");
    assert_eq!(git(repo.path(), &["rev-parse", "refs/heads/feature"]).trim(), tip);
}

// MARK: Undo of discards

#[test]
fn discard_undo_restores_the_discarded_edit() {
    let repo = Repo::new();
    repo.commit("a.txt", "base\n", "Base");
    repo.write("a.txt", "changed\n");
    let undo = client().discard_keeping_undo(&entry(repo.path(), "a.txt"), repo.path()).expect("discard").expect("undo available");
    assert_eq!(repo.read("a.txt"), "base\n");

    client().undo_discard(&undo, repo.path()).expect("undo discard");
    assert_eq!(repo.read("a.txt"), "changed\n");
}

#[test]
fn discard_undo_restores_staged_and_unstaged_versions() {
    let repo = Repo::new();
    repo.commit("a.txt", "base\n", "Base");
    repo.write("a.txt", "staged\n");
    repo.git(&["add", "--", "a.txt"]);
    repo.write("a.txt", "working\n");
    let undo = client().discard_keeping_undo(&entry(repo.path(), "a.txt"), repo.path()).expect("discard").expect("undo available");
    assert_eq!(repo.read("a.txt"), "base\n");
    assert_eq!(git(repo.path(), &["show", ":a.txt"]), "base\n");

    client().undo_discard(&undo, repo.path()).expect("undo discard");
    assert_eq!(repo.read("a.txt"), "working\n");
    assert_eq!(git(repo.path(), &["show", ":a.txt"]), "staged\n");
}

#[test]
fn discard_undo_restores_an_untracked_file() {
    let repo = Repo::new();
    repo.commit("a.txt", "base\n", "Base");
    repo.write("new.txt", "new\n");
    let undo = client().discard_keeping_undo(&entry(repo.path(), "new.txt"), repo.path()).expect("discard").expect("undo available");
    assert!(!repo.path().join("new.txt").exists());

    client().undo_discard(&undo, repo.path()).expect("undo discard");
    assert_eq!(repo.read("new.txt"), "new\n");
    assert!(git(repo.path(), &["status", "--porcelain"]).contains("?? new.txt"));
}

#[test]
fn discard_undo_refuses_when_the_file_changed_again() {
    let repo = Repo::new();
    repo.commit("a.txt", "base\n", "Base");
    repo.write("a.txt", "changed\n");
    let undo = client().discard_keeping_undo(&entry(repo.path(), "a.txt"), repo.path()).expect("discard").expect("undo available");
    repo.write("a.txt", "newer\n");

    let message = refusal(client().undo_discard(&undo, repo.path()));
    assert!(message.contains("changed after it was discarded"), "{message}");
    assert_eq!(repo.read("a.txt"), "newer\n", "newer work is not overwritten");
}
