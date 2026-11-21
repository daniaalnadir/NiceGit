//! Working-tree snapshots, run against real repositories in temporary directories.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use nicegit_core::client::GitClient;
use nicegit_core::snapshots::{
    delete_snapshot, differences_from_now, list_snapshots, prune_snapshots, restore_from_snapshot, snapshot_changes, take_snapshot,
    working_tree_now, SnapshotInfo, DEFAULT_MAX_FILE_BYTES,
};
use nicegit_core::StatusKind;
use tempfile::TempDir;

// MARK: Helpers

fn git_output(directory: &Path, arguments: &[&str]) -> Output {
    let mut command = Command::new("git");
    command.args(arguments).current_dir(directory);
    for key in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_OBJECT_DIRECTORY", "GIT_COMMON_DIR"] {
        command.env_remove(key);
    }
    command.output().unwrap_or_else(|error| panic!("could not run git {arguments:?}: {error}"))
}

fn git(directory: &Path, arguments: &[&str]) -> String {
    let output = git_output(directory, arguments);
    assert!(output.status.success(), "git {arguments:?} failed: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// A temporary repository on branch `main` with one commit of `tracked.txt`.
struct Repo {
    dir: TempDir,
}

impl Repo {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("create temp dir");
        git(dir.path(), &["init", "-b", "main"]);
        for (key, value) in
            [("user.name", "Test"), ("user.email", "test@example.com"), ("commit.gpgsign", "false"), ("core.autocrlf", "false")]
        {
            git(dir.path(), &["config", key, value]);
        }
        let repo = Repo { dir };
        repo.write("tracked.txt", "one\n");
        repo.git(&["add", "tracked.txt"]);
        repo.git(&["commit", "-m", "Initial"]);
        repo
    }

    /// A repository with no commits yet.
    fn unborn() -> Self {
        let dir = tempfile::tempdir().expect("create temp dir");
        git(dir.path(), &["init", "-b", "main"]);
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
}

fn client() -> GitClient {
    GitClient::new()
}

fn take(repo: &Repo) -> Option<SnapshotInfo> {
    take_snapshot(&client(), repo.path(), DEFAULT_MAX_FILE_BYTES).expect("take snapshot")
}

/// The contents of a snapshot's file, read from its tree.
fn in_snapshot(repo: &Repo, snapshot: &SnapshotInfo, path: &str) -> Option<String> {
    let output = git_output(repo.path(), &["cat-file", "blob", &format!("{}:{path}", snapshot.tree)]);
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

// MARK: Tests

#[test]
fn snapshot_holds_tracked_edits_and_untracked_files_but_not_ignored_ones() {
    let repo = Repo::new();
    repo.write(".gitignore", "*.log\n");
    repo.write("tracked.txt", "edited\n");
    repo.write("new.txt", "fresh\n");
    repo.write("build.log", "ignored\n");
    let snapshot = take(&repo).expect("a snapshot is taken");
    assert_eq!(in_snapshot(&repo, &snapshot, "tracked.txt").as_deref(), Some("edited\n"));
    assert_eq!(in_snapshot(&repo, &snapshot, "new.txt").as_deref(), Some("fresh\n"));
    assert_eq!(in_snapshot(&repo, &snapshot, "build.log"), None);
    assert!(snapshot.reference.starts_with("refs/nicegit/snapshots/"));
    // The snapshot is not a branch, and no branch moved.
    assert_eq!(repo.git(&["branch", "--format=%(refname)"]).trim(), "refs/heads/main");
}

#[test]
fn taking_a_snapshot_leaves_the_real_index_and_files_alone() {
    let repo = Repo::new();
    repo.write("tracked.txt", "edited\n");
    repo.write("new.txt", "fresh\n");
    repo.write("staged.txt", "staged\n");
    repo.git(&["add", "staged.txt"]);
    let index_path = repo.path().join(".git").join("index");
    let index_before = fs::read(&index_path).expect("index exists");
    let status_before = repo.git(&["status", "--porcelain=v1", "-uall"]);
    take(&repo).expect("a snapshot is taken");
    assert_eq!(fs::read(&index_path).expect("index exists"), index_before, "the real index is byte-for-byte unchanged");
    assert_eq!(repo.git(&["status", "--porcelain=v1", "-uall"]), status_before);
    assert_eq!(repo.read("tracked.txt"), "edited\n");
    assert_eq!(repo.read("new.txt"), "fresh\n");
    assert!(!repo.path().join(".git").join("index.lock").exists());
}

#[test]
fn an_unchanged_working_tree_takes_no_new_snapshot() {
    let repo = Repo::new();
    repo.write("tracked.txt", "edited\n");
    let first = take(&repo).expect("the first snapshot");
    assert_eq!(take(&repo), None, "nothing changed since the snapshot");
    repo.write("tracked.txt", "edited again\n");
    let second = take(&repo).expect("a change is saved");
    assert_ne!(first.tree, second.tree);
    assert_eq!(take(&repo), None);
}

#[test]
fn a_clean_working_tree_matching_head_is_not_saved() {
    let repo = Repo::new();
    assert_eq!(take(&repo), None, "the working tree equals HEAD");
    assert!(list_snapshots(&client(), repo.path()).unwrap().is_empty());
}

#[test]
fn large_files_are_skipped() {
    let repo = Repo::new();
    repo.write("small.txt", "small\n");
    repo.write("large.bin", &"x".repeat(4096));
    repo.write("tracked.txt", &"y".repeat(4096));
    repo.git(&["add", "tracked.txt"]);
    repo.git(&["commit", "-m", "Big tracked"]);
    repo.write("tracked.txt", &"z".repeat(4096));
    let tree = working_tree_now(&client(), repo.path(), 1024).expect("the tree is built");
    let listed = git(repo.path(), &["ls-tree", "-r", "--name-only", &tree]);
    assert!(listed.lines().any(|line| line == "small.txt"));
    assert!(!listed.lines().any(|line| line == "large.bin"), "an untracked large file is left out");
    assert!(!listed.lines().any(|line| line == "tracked.txt"), "a tracked large file is left out");
    // The large file stays on disk and in the real index.
    assert_eq!(repo.read("large.bin").len(), 4096);
    assert!(repo.git(&["ls-files"]).lines().any(|line| line == "tracked.txt"));
}

#[test]
fn pruning_keeps_the_newest_snapshots() {
    let repo = Repo::new();
    for index in 0..5 {
        repo.write("tracked.txt", &format!("version {index}\n"));
        take(&repo).expect("a snapshot per change");
    }
    assert_eq!(list_snapshots(&client(), repo.path()).unwrap().len(), 5);
    let removed = prune_snapshots(&client(), repo.path(), 2, 7).expect("prune");
    assert_eq!(removed, 3);
    let kept = list_snapshots(&client(), repo.path()).unwrap();
    assert_eq!(kept.len(), 2);
    assert!(kept[0].time > kept[1].time, "the newest come first");
    assert_eq!(in_snapshot(&repo, &kept[0], "tracked.txt").as_deref(), Some("version 4\n"));
}

#[test]
fn pruning_removes_snapshots_older_than_the_keep_period() {
    let repo = Repo::new();
    repo.write("tracked.txt", "old\n");
    let old = take(&repo).expect("a snapshot");
    // A snapshot ten days old, under the same name scheme, is removed by age.
    let ancient = old.time - 10 * 86_400_000;
    git(repo.path(), &["update-ref", &format!("refs/nicegit/snapshots/{ancient}"), &old.commit]);
    let removed = prune_snapshots(&client(), repo.path(), 100, 7).expect("prune");
    assert_eq!(removed, 1);
    assert_eq!(list_snapshots(&client(), repo.path()).unwrap(), vec![old]);
}

#[test]
fn deleting_a_snapshot_removes_only_its_ref() {
    let repo = Repo::new();
    repo.write("tracked.txt", "first edit\n");
    let first = take(&repo).expect("first");
    repo.write("tracked.txt", "second edit\n");
    let second = take(&repo).expect("second");
    delete_snapshot(&client(), repo.path(), &first).expect("delete");
    assert_eq!(list_snapshots(&client(), repo.path()).unwrap(), vec![second]);
}

#[test]
fn restore_writes_the_snapshot_version_of_a_modified_and_a_deleted_file() {
    let repo = Repo::new();
    repo.write("keep.txt", "kept version\n");
    repo.write("gone.txt", "deleted version\n");
    repo.git(&["add", "keep.txt", "gone.txt"]);
    repo.git(&["commit", "-m", "Two files"]);
    repo.write("keep.txt", "saved edit\n");
    repo.write("gone.txt", "saved gone\n");
    let snapshot = take(&repo).expect("snapshot");
    repo.write("keep.txt", "later mess\n");
    fs::remove_file(repo.path().join("gone.txt")).unwrap();
    let index_before = git(repo.path(), &["ls-files", "--stage"]);

    restore_from_snapshot(&client(), &snapshot, &["keep.txt".into(), "gone.txt".into()], repo.path()).expect("restore");

    assert_eq!(repo.read("keep.txt"), "saved edit\n");
    assert_eq!(repo.read("gone.txt"), "saved gone\n");
    assert_eq!(git(repo.path(), &["ls-files", "--stage"]), index_before, "the index is not changed");
}

#[test]
fn restore_first_saves_the_current_files_so_the_restore_can_be_undone() {
    let repo = Repo::new();
    repo.write("tracked.txt", "snapshot version\n");
    let snapshot = take(&repo).expect("snapshot");
    repo.write("tracked.txt", "work just before restore\n");
    restore_from_snapshot(&client(), &snapshot, &["tracked.txt".into()], repo.path()).expect("restore");
    assert_eq!(repo.read("tracked.txt"), "snapshot version\n");
    let newest = list_snapshots(&client(), repo.path()).unwrap().into_iter().next().unwrap();
    assert_eq!(in_snapshot(&repo, &newest, "tracked.txt").as_deref(), Some("work just before restore\n"));
}

#[test]
fn restore_refuses_absent_paths_and_paths_outside_the_repository() {
    let repo = Repo::new();
    repo.write("tracked.txt", "edited\n");
    let snapshot = take(&repo).expect("snapshot");
    repo.write("tracked.txt", "later\n");
    let client = client();
    assert!(restore_from_snapshot(&client, &snapshot, &["never-existed.txt".into()], repo.path()).is_err());
    assert!(restore_from_snapshot(&client, &snapshot, &["../outside.txt".into()], repo.path()).is_err());
    assert!(restore_from_snapshot(&client, &snapshot, &["a/../../outside.txt".into()], repo.path()).is_err());
    assert!(restore_from_snapshot(&client, &snapshot, &[".git/config".into()], repo.path()).is_err());
    assert!(restore_from_snapshot(&client, &snapshot, &["/etc/hosts".into()], repo.path()).is_err());
    // A refusal writes nothing, even when another path in the same request is valid.
    assert!(restore_from_snapshot(&client, &snapshot, &["tracked.txt".into(), "missing.txt".into()], repo.path()).is_err());
    assert_eq!(repo.read("tracked.txt"), "later\n");
}

#[test]
fn restore_brings_back_an_untracked_file() {
    let repo = Repo::new();
    repo.write("notes/draft.md", "draft text\n");
    let snapshot = take(&repo).expect("snapshot");
    fs::remove_dir_all(repo.path().join("notes")).unwrap();
    restore_from_snapshot(&client(), &snapshot, &["notes/draft.md".into()], repo.path()).expect("restore");
    assert_eq!(repo.read("notes/draft.md"), "draft text\n");
    assert!(repo.git(&["status", "--porcelain=v1"]).contains("?? notes/"), "the file is untracked, as it was");
}

#[test]
fn restore_keeps_the_executable_bit() {
    let repo = Repo::new();
    repo.write("run.sh", "#!/bin/sh\necho hi\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(repo.path().join("run.sh"), fs::Permissions::from_mode(0o755)).unwrap();
    }
    let snapshot = take(&repo).expect("snapshot");
    fs::remove_file(repo.path().join("run.sh")).unwrap();
    restore_from_snapshot(&client(), &snapshot, &["run.sh".into()], repo.path()).expect("restore");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(repo.path().join("run.sh")).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111);
    }
}

#[test]
fn an_unborn_repository_takes_snapshots() {
    let repo = Repo::unborn();
    repo.write("first.txt", "first\n");
    let snapshot = take(&repo).expect("a snapshot without HEAD");
    assert_eq!(in_snapshot(&repo, &snapshot, "first.txt").as_deref(), Some("first\n"));
    let parents = git(repo.path(), &["rev-list", "--parents", "-n", "1", &snapshot.commit]);
    assert_eq!(parents.split_whitespace().count(), 1, "the first snapshot has no parent");
    repo.write("second.txt", "second\n");
    assert!(take(&repo).is_some());
}

#[test]
fn a_linked_worktree_snapshots_its_own_files() {
    let repo = Repo::new();
    let worktree_parent = tempfile::tempdir().expect("temp dir");
    let worktree = worktree_parent.path().join("linked");
    repo.git(&["worktree", "add", "-b", "feature", worktree.to_str().unwrap()]);
    fs::write(worktree.join("linked.txt"), "linked work\n").unwrap();
    fs::write(repo.path().join("main-only.txt"), "main work\n").unwrap();
    let linked_snapshot = take_snapshot(&client(), &worktree, DEFAULT_MAX_FILE_BYTES).expect("snapshot").expect("saved");
    assert_eq!(in_snapshot(&repo, &linked_snapshot, "linked.txt").as_deref(), Some("linked work\n"));
    assert_eq!(in_snapshot(&repo, &linked_snapshot, "main-only.txt"), None);
    // Both worktrees share the snapshot refs.
    assert_eq!(list_snapshots(&client(), repo.path()).unwrap().len(), 1);
}

#[test]
fn filenames_with_spaces_newlines_and_glob_characters_are_kept_literally() {
    let repo = Repo::new();
    // "[bracket].txt" read as a glob would match "b.txt". Windows refuses newlines, "*" and "?" in
    // file names, so those are tried only elsewhere.
    let mut names = vec!["with space.txt", "[bracket].txt", "b.txt"];
    if !cfg!(windows) {
        names.extend(["new\nline.txt", "star*.txt", "question?.txt"]);
    }
    for name in &names {
        repo.write(name, &format!("content of {name}\n"));
    }
    let snapshot = take(&repo).expect("snapshot");
    for name in &names {
        assert_eq!(in_snapshot(&repo, &snapshot, name), Some(format!("content of {name}\n")), "{name:?} is saved");
    }
    // Only the named file changes; its glob-like neighbours keep their content.
    for name in &names {
        repo.write(name, "changed\n");
    }
    fs::remove_file(repo.path().join("[bracket].txt")).unwrap();
    restore_from_snapshot(&client(), &snapshot, &["[bracket].txt".into()], repo.path()).expect("restore one");
    assert_eq!(repo.read("[bracket].txt"), "content of [bracket].txt\n");
    assert_eq!(repo.read("b.txt"), "changed\n");
    assert_eq!(repo.read("with space.txt"), "changed\n");
}

#[test]
fn changes_list_what_differs_from_the_snapshot_now() {
    let repo = Repo::new();
    repo.write("tracked.txt", "snapshot\n");
    repo.write("removed.txt", "will go\n");
    repo.git(&["add", "removed.txt"]);
    repo.git(&["commit", "-m", "Add removed"]);
    repo.write("removed.txt", "will go, edited\n");
    let snapshot = take(&repo).expect("snapshot");
    repo.write("tracked.txt", "now\n");
    fs::remove_file(repo.path().join("removed.txt")).unwrap();
    repo.write("fresh.txt", "new\n");
    let current = working_tree_now(&client(), repo.path(), DEFAULT_MAX_FILE_BYTES).unwrap();
    let changes = snapshot_changes(&client(), repo.path(), &snapshot, &current).unwrap();
    let summary: Vec<(String, StatusKind)> = changes.into_iter().map(|change| (change.path, change.kind)).collect();
    assert_eq!(
        summary,
        vec![
            ("fresh.txt".to_string(), StatusKind::Added),
            ("removed.txt".to_string(), StatusKind::Deleted),
            ("tracked.txt".to_string(), StatusKind::Modified),
        ]
    );
    let counts = differences_from_now(&client(), repo.path(), &[snapshot]).unwrap();
    assert_eq!(counts, vec![3]);
}

#[test]
fn snapshots_do_not_appear_in_the_history_graph() {
    let repo = Repo::new();
    repo.write("tracked.txt", "edited\n");
    let snapshot = take(&repo).expect("snapshot");
    let loaded = client().load_snapshot(repo.path(), 100).expect("load");
    assert!(loaded.commits.iter().all(|commit| commit.hash != snapshot.commit), "the snapshot commit is not in the graph");
    assert_eq!(loaded.commits.len(), 1, "only the real commit is shown");
}

#[test]
fn a_bare_repository_is_reported_as_bare_and_a_normal_one_is_not() {
    let bare = tempfile::tempdir().expect("create temp dir");
    git(bare.path(), &["init", "--bare", "-b", "main"]);
    assert!(client().repository_shape(bare.path(), None).bare, "a bare repository has no working files");
    let repo = Repo::new();
    assert!(!client().repository_shape(repo.path(), None).bare);
}
