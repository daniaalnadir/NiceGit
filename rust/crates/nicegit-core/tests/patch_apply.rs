//! Integration tests for previewing and applying patches, run against real repositories in
//! temporary directories.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use nicegit_core::client::GitClient;
use nicegit_core::patch_apply::{ApplyMode, FileStatus, PatchKind};
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
fn configure(directory: &Path, name: &str) {
    for (key, value) in [
        ("user.name", name),
        ("user.email", "test@example.com"),
        ("commit.gpgsign", "false"),
        ("tag.gpgsign", "false"),
        // Keeps line endings byte-for-byte on Windows, where Git may default to autocrlf.
        ("core.autocrlf", "false"),
    ] {
        git(directory, &["config", key, value]);
    }
}

/// A temporary repository on branch `main` with a file of ten lines and one more committed file.
struct Repo {
    dir: TempDir,
}

impl Repo {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("create temp dir");
        git(dir.path(), &["init", "-q", "-b", "main"]);
        configure(dir.path(), "Test");
        fs::write(dir.path().join("notes.txt"), numbered(10)).unwrap();
        fs::write(dir.path().join("old.txt"), "one\n").unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-q", "-m", "Start"]);
        Self { dir }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn client(&self) -> GitClient {
        GitClient::new()
    }
}

/// The text of `count` numbered lines.
fn numbered(count: usize) -> String {
    (1..=count).map(|n| format!("line {n}\n")).collect()
}

/// The patch Git writes for the staged changes, with renames detected.
fn staged_patch(directory: &Path) -> String {
    git(directory, &["add", "-A"]);
    let patch = git(directory, &["diff", "--cached", "-M", "--full-index", "--binary"]);
    git(directory, &["reset", "-q", "--hard"]);
    patch
}

// MARK: Preview

#[test]
fn preview_lists_a_rename_a_new_file_and_a_change_with_its_counts() {
    let repo = Repo::new();
    let path = repo.path();
    fs::write(path.join("notes.txt"), numbered(10).replace("line 2\n", "second\n")).unwrap();
    git(path, &["mv", "old.txt", "new name.txt"]);
    fs::write(path.join("new name.txt"), "one\ntwo\n").unwrap();
    fs::write(path.join("sp ace.txt"), "hello\n").unwrap();
    let patch = staged_patch(path);

    let preview = repo.client().preview_patch(patch.as_bytes(), path).expect("preview");
    assert_eq!(preview.kind, PatchKind::Diff);
    assert!(preview.applies_cleanly, "{:?}", preview.problem);
    assert!(preview.commits.is_empty());

    let by_path = |name: &str| preview.files.iter().find(|file| file.path == name).unwrap_or_else(|| panic!("{name} listed")).clone();
    let rename = by_path("new name.txt");
    assert_eq!(rename.status, FileStatus::Renamed);
    assert_eq!(rename.old_path.as_deref(), Some("old.txt"));
    assert_eq!((rename.additions, rename.deletions), (1, 0));
    let added = by_path("sp ace.txt");
    assert_eq!(added.status, FileStatus::Added);
    assert_eq!((added.additions, added.deletions), (1, 0));
    let changed = by_path("notes.txt");
    assert_eq!(changed.status, FileStatus::Modified);
    assert_eq!((changed.additions, changed.deletions), (1, 1));
    assert_eq!(preview.files.len(), 3);
}

#[test]
fn a_patch_that_no_longer_applies_says_why_and_offers_three_way() {
    let repo = Repo::new();
    let path = repo.path();
    fs::write(path.join("notes.txt"), numbered(10).replace("line 2\n", "patched\n")).unwrap();
    let patch = staged_patch(path);
    // The branch changes the same line in another way, so the patch's context no longer matches.
    fs::write(path.join("notes.txt"), numbered(10).replace("line 2\n", "moved on\n")).unwrap();
    git(path, &["commit", "-qam", "Change line two"]);

    let preview = repo.client().preview_patch(patch.as_bytes(), path).expect("preview");
    assert!(!preview.applies_cleanly);
    assert!(preview.problem.as_deref().is_some_and(|text| !text.is_empty()));
    // The base blob is in the repository, so a three-way merge can try again.
    assert!(preview.three_way_possible);
}

#[test]
fn a_patch_for_another_file_does_not_apply_and_offers_no_three_way() {
    let repo = Repo::new();
    let path = repo.path();
    fs::write(path.join("notes.txt"), numbered(10).replace("line 2\n", "patched\n")).unwrap();
    let patch = staged_patch(path);
    fs::remove_file(path.join("notes.txt")).unwrap();
    git(path, &["commit", "-qam", "Remove notes"]);

    let preview = repo.client().preview_patch(patch.as_bytes(), path).expect("preview");
    assert!(!preview.applies_cleanly);
    assert!(!preview.three_way_possible);
    let error = repo.client().apply_patch_with(patch.as_bytes(), ApplyMode::WorkingTree, true, path).unwrap_err();
    assert!(error.to_string().contains("notes.txt"), "{error}");
}

#[test]
fn text_that_is_not_a_patch_is_previewed_with_a_problem() {
    let repo = Repo::new();
    let preview = repo.client().preview_patch(b"hello there\n", repo.path()).expect("preview");
    assert!(preview.files.is_empty());
    assert!(!preview.applies_cleanly);
    assert!(preview.problem.is_some());
    let empty = repo.client().preview_patch(b"  \n", repo.path()).expect("preview");
    assert_eq!(empty.problem.as_deref(), Some("The patch is empty."));
}

#[test]
fn patches_over_fifty_megabytes_are_refused() {
    let repo = Repo::new();
    let big = vec![b'a'; nicegit_core::patch_apply::MAX_PATCH_BYTES + 1];
    let error = repo.client().preview_patch(&big, repo.path()).unwrap_err();
    assert!(error.to_string().contains("50 MB"), "{error}");
}

// MARK: Applying

#[test]
fn applying_to_the_index_stages_the_changes() {
    let repo = Repo::new();
    let path = repo.path();
    fs::write(path.join("notes.txt"), numbered(10).replace("line 2\n", "staged\n")).unwrap();
    let patch = staged_patch(path);

    let message = repo.client().apply_patch_with(patch.as_bytes(), ApplyMode::Index, false, path).expect("apply");
    assert_eq!(message, "Applied the patch and staged its changes.");
    assert_eq!(git(path, &["diff", "--cached", "--name-only"]), "notes.txt\n");
    assert!(fs::read_to_string(path.join("notes.txt")).unwrap().contains("staged\n"));
}

#[test]
fn applying_to_the_working_files_leaves_the_changes_unstaged() {
    let repo = Repo::new();
    let path = repo.path();
    fs::write(path.join("notes.txt"), numbered(10).replace("line 2\n", "unstaged\n")).unwrap();
    let patch = staged_patch(path);

    let message = repo.client().apply_patch_with(patch.as_bytes(), ApplyMode::WorkingTree, false, path).expect("apply");
    assert_eq!(message, "Applied the patch. Its changes are unstaged.");
    assert_eq!(git(path, &["diff", "--cached", "--name-only"]), "");
    assert_eq!(git(path, &["diff", "--name-only"]), "notes.txt\n");
}

#[test]
fn a_mailbox_patch_creates_a_commit_with_the_same_subject_and_author() {
    let repo = Repo::new();
    let path = repo.path();
    configure(path, "Alice");
    fs::write(path.join("greeting.txt"), "hello\n").unwrap();
    git(path, &["add", "greeting.txt"]);
    git(path, &["commit", "-q", "-m", "Add greeting"]);
    configure(path, "Test");
    let commit = git(path, &["rev-parse", "HEAD"]).trim().to_string();
    let mailbox = repo.client().export_commit_patch(&commit, path).expect("export");
    // The other branch starts before the commit, so the patch applies there.
    git(path, &["checkout", "-q", "-b", "other", "HEAD~1"]);

    let preview = repo.client().preview_patch(mailbox.as_bytes(), path).expect("preview");
    assert_eq!(preview.kind, PatchKind::Mailbox);
    assert_eq!(preview.commits, vec!["Add greeting".to_string()]);
    assert!(preview.applies_cleanly, "{:?}", preview.problem);

    let message = repo.client().apply_patch_with(mailbox.as_bytes(), ApplyMode::Commits, false, path).expect("apply");
    assert_eq!(message, "Created 1 commit from the patch.");
    assert_eq!(git(path, &["log", "-1", "--format=%s"]), "Add greeting\n");
    assert_eq!(git(path, &["log", "-1", "--format=%an"]), "Alice\n");
    assert_eq!(git(path, &["status", "--porcelain"]), "");
    assert!(!path.join(".git").join("rebase-apply").exists());
}

#[test]
fn commits_are_refused_for_a_diff_and_for_staged_changes() {
    let repo = Repo::new();
    let path = repo.path();
    fs::write(path.join("notes.txt"), numbered(10).replace("line 2\n", "diff\n")).unwrap();
    let patch = staged_patch(path);
    let error = repo.client().apply_patch_with(patch.as_bytes(), ApplyMode::Commits, false, path).unwrap_err();
    assert!(error.to_string().contains("mailbox patch"), "{error}");

    fs::write(path.join("greeting.txt"), "hello\n").unwrap();
    git(path, &["add", "greeting.txt"]);
    git(path, &["commit", "-q", "-m", "Add greeting"]);
    let commit = git(path, &["rev-parse", "HEAD"]).trim().to_string();
    let mailbox = repo.client().export_commit_patch(&commit, path).expect("export");
    git(path, &["checkout", "-q", "-b", "other", "HEAD~1"]);
    // Staged work would mix with the new commits, so the apply is refused before `git am` runs.
    fs::write(path.join("old.txt"), "two\n").unwrap();
    git(path, &["add", "old.txt"]);
    let error = repo.client().apply_patch_with(mailbox.as_bytes(), ApplyMode::Commits, false, path).unwrap_err();
    assert!(error.to_string().contains("Commit or unstage"), "{error}");
    assert_eq!(git(path, &["log", "-1", "--format=%s"]), "Start\n");
}

#[cfg(unix)]
#[test]
fn a_failing_mailbox_apply_is_aborted_and_leaves_no_sequence() {
    use std::os::unix::fs::PermissionsExt;

    let repo = Repo::new();
    let path = repo.path();
    fs::write(path.join("greeting.txt"), "hello\n").unwrap();
    git(path, &["add", "greeting.txt"]);
    git(path, &["commit", "-q", "-m", "Add greeting"]);
    let commit = git(path, &["rev-parse", "HEAD"]).trim().to_string();
    let mailbox = repo.client().export_commit_patch(&commit, path).expect("export");
    git(path, &["checkout", "-q", "-b", "other", "HEAD~1"]);
    // A hook that refuses the message makes `git am` stop partway, which must be undone.
    let hook = path.join(".git").join("hooks").join("applypatch-msg");
    fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    let head_before = git(path, &["rev-parse", "HEAD"]);

    let result = repo.client().apply_patch_with(mailbox.as_bytes(), ApplyMode::Commits, false, path);
    assert!(result.is_err());
    assert_eq!(git(path, &["rev-parse", "HEAD"]), head_before);
    assert!(!path.join(".git").join("rebase-apply").exists());
    assert_eq!(git(path, &["status", "--porcelain"]), "");
    assert!(!path.join("greeting.txt").exists());
}

#[test]
fn a_new_file_where_an_untracked_file_is_refused() {
    let repo = Repo::new();
    let path = repo.path();
    fs::write(path.join("sp ace.txt"), "hello\n").unwrap();
    let patch = staged_patch(path);
    fs::write(path.join("sp ace.txt"), "mine, not tracked\n").unwrap();

    let error = repo.client().apply_patch_with(patch.as_bytes(), ApplyMode::WorkingTree, false, path).unwrap_err();
    assert!(error.to_string().contains("sp ace.txt"), "{error}");
    assert_eq!(fs::read_to_string(path.join("sp ace.txt")).unwrap(), "mine, not tracked\n");
}

#[test]
fn a_new_file_where_an_ignored_file_is_refused() {
    let repo = Repo::new();
    let path = repo.path();
    fs::write(path.join(".gitignore"), "*.local\n").unwrap();
    git(path, &["add", ".gitignore"]);
    git(path, &["commit", "-q", "-m", "Ignore local files"]);
    // The patch creates a file under the ignored name, which Git would not stage on its own.
    let patch = "diff --git a/settings.local b/settings.local\nnew file mode 100644\n--- /dev/null\n+++ b/settings.local\n@@ -0,0 +1 @@\n+from the patch\n";
    // The local file the patch would create is ignored, so Git would not warn about it.
    fs::write(path.join("settings.local"), "mine\n").unwrap();

    let error = repo.client().apply_patch_with(patch.as_bytes(), ApplyMode::WorkingTree, false, path).unwrap_err();
    assert!(error.to_string().contains("settings.local"), "{error}");
    assert_eq!(fs::read_to_string(path.join("settings.local")).unwrap(), "mine\n");
}

#[test]
fn paths_with_spaces_and_glob_characters_change_only_their_own_files() {
    let repo = Repo::new();
    let path = repo.path();
    fs::write(path.join("a[1].txt"), "a\n").unwrap();
    fs::write(path.join("a1.txt"), "decoy\n").unwrap();
    git(path, &["add", "."]);
    git(path, &["commit", "-q", "-m", "Add the glob names"]);
    fs::write(path.join("a[1].txt"), "changed\n").unwrap();
    fs::write(path.join("sp ace.txt"), "new\n").unwrap();
    let patch = staged_patch(path);

    let preview = repo.client().preview_patch(patch.as_bytes(), path).expect("preview");
    assert!(preview.files.iter().any(|file| file.path == "a[1].txt"));
    repo.client().apply_patch(patch.as_bytes(), path).expect("apply");
    assert_eq!(fs::read_to_string(path.join("a[1].txt")).unwrap(), "changed\n");
    assert_eq!(fs::read_to_string(path.join("a1.txt")).unwrap(), "decoy\n");
    assert_eq!(fs::read_to_string(path.join("sp ace.txt")).unwrap(), "new\n");
}

#[test]
fn a_three_way_apply_that_conflicts_is_reported_for_resolving() {
    let repo = Repo::new();
    let path = repo.path();
    fs::write(path.join("notes.txt"), numbered(10).replace("line 2\n", "patched\n")).unwrap();
    let patch = staged_patch(path);
    fs::write(path.join("notes.txt"), numbered(10).replace("line 2\n", "moved on\n")).unwrap();
    git(path, &["commit", "-qam", "Change line two"]);

    let message = repo.client().apply_patch_with(patch.as_bytes(), ApplyMode::WorkingTree, true, path).expect("apply");
    assert_eq!(message, "Applied with conflicts in 1 files; resolve them in Changes.");
    assert_eq!(git(path, &["diff", "--name-only", "--diff-filter=U"]), "notes.txt\n");
    assert!(fs::read_to_string(path.join("notes.txt")).unwrap().contains("<<<<<<<"));
}
