//! Integration tests for identity, remotes, tags on remotes, upstreams, ignore rules, stashes of
//! selected files, worktrees, and submodules, run against real repositories in temporary folders.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use nicegit_core::client::GitClient;
use nicegit_core::models::{Branch, GitError, Snapshot, Worktree};
use nicegit_core::settings::{ignore_pattern, IgnoreRule, IgnoreScope};
use nicegit_core::submodule::SubmoduleState;
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

fn local_branch(snapshot: &Snapshot, name: &str) -> Branch {
    snapshot
        .branches
        .iter()
        .find(|branch| !branch.is_remote && branch.name == name)
        .cloned()
        .unwrap_or_else(|| panic!("no local branch {name}"))
}

fn branch_exists(directory: &Path, name: &str) -> bool {
    git_output(directory, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{name}")]).status.success()
}

fn tag_exists(directory: &Path, name: &str) -> bool {
    git_output(directory, &["rev-parse", "--verify", "--quiet", &format!("refs/tags/{name}")]).status.success()
}

fn path_string(path: &Path) -> String {
    path.to_str().expect("UTF-8 temp path").to_string()
}

/// Creates a bare repository on branch `main` to act as a remote.
fn bare_remote() -> TempDir {
    let remote = tempfile::tempdir().expect("create remote dir");
    git(remote.path(), &["init", "--bare", "-b", "main"]);
    remote
}

/// The linked worktree at `folder`, as Git lists it, comparing resolved locations.
fn listed_worktree(worktrees: &[Worktree], folder: &Path) -> Worktree {
    let wanted = fs::canonicalize(folder).expect("resolve worktree folder");
    worktrees
        .iter()
        .find(|worktree| fs::canonicalize(&worktree.path).map(|resolved| resolved == wanted).unwrap_or(false))
        .cloned()
        .unwrap_or_else(|| panic!("no worktree listed at {}", folder.display()))
}

// MARK: Identity

#[test]
fn set_identity_writes_repository_config() {
    let repo = Repo::new();
    client().set_identity("  Ada Lovelace ", " ada@example.com ", repo.path()).expect("set identity");

    assert_eq!(repo.git(&["config", "--local", "--get", "user.name"]).trim(), "Ada Lovelace");
    assert_eq!(repo.git(&["config", "--local", "--get", "user.email"]).trim(), "ada@example.com");
}

#[test]
fn set_identity_requires_name_and_email() {
    let repo = Repo::new();
    assert!(error_text(client().set_identity("   ", "ada@example.com", repo.path())).contains("Name and email are required"));
    assert!(error_text(client().set_identity("Ada", "", repo.path())).contains("Name and email are required"));
}

#[test]
fn apply_identity_with_signing_key_turns_on_signing() {
    let repo = Repo::new();
    client().apply_identity("Ada", "ada@example.com", Some(" ABC123 "), repo.path()).expect("apply identity");

    assert_eq!(repo.git(&["config", "--local", "--get", "user.signingkey"]).trim(), "ABC123");
    assert_eq!(repo.git(&["config", "--local", "--get", "commit.gpgsign"]).trim(), "true");
    assert_eq!(client().identity(repo.path()), ("Ada".to_string(), "ada@example.com".to_string()));
}

#[test]
fn apply_identity_without_key_leaves_signing_alone() {
    let repo = Repo::new();
    client().apply_identity("Ada", "ada@example.com", Some("  "), repo.path()).expect("apply identity");

    assert_eq!(repo.git(&["config", "--local", "--get", "commit.gpgsign"]).trim(), "false");
    assert!(!git_output(repo.path(), &["config", "--local", "--get", "user.signingkey"]).status.success());
}

// MARK: Remotes

#[test]
fn rename_remote_moves_it_when_address_matches() {
    let remote = bare_remote();
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.git(&["remote", "add", "origin", &path_string(remote.path())]);
    let expected = snapshot(repo.path()).remote_fetch_addresses["origin"].clone();

    client().rename_remote("origin", "upstream", &expected, repo.path()).expect("rename remote");

    assert_eq!(snapshot(repo.path()).remotes, vec!["upstream".to_string()]);
}

#[test]
fn rename_remote_refuses_when_address_changed() {
    let remote = bare_remote();
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.git(&["remote", "add", "origin", &path_string(remote.path())]);
    let stale = vec!["/somewhere/else".to_string()];

    let message = error_text(client().rename_remote("origin", "upstream", &stale, repo.path()));

    assert!(message.contains("remote address changed"), "{message}");
    assert_eq!(snapshot(repo.path()).remotes, vec!["origin".to_string()]);
}

#[test]
fn set_remote_address_changes_fetch_address_and_refuses_stale_view() {
    let first = bare_remote();
    let second = bare_remote();
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.git(&["remote", "add", "origin", &path_string(first.path())]);
    let expected = snapshot(repo.path()).remote_fetch_addresses["origin"].clone();

    assert!(error_text(client().set_remote_address("origin", "  ", &expected, repo.path())).contains("Enter the remote's new address"));

    let stale = vec!["/stale".to_string()];
    assert!(error_text(client().set_remote_address("origin", &path_string(second.path()), &stale, repo.path()))
        .contains("remote address changed"));
    assert_eq!(client().remote_address("origin", repo.path()).expect("address"), path_string(first.path()));

    client().set_remote_address("origin", &path_string(second.path()), &expected, repo.path()).expect("set address");
    assert_eq!(client().remote_address("origin", repo.path()).expect("address"), path_string(second.path()));
}

#[test]
fn remove_remote_forgets_it_and_refuses_stale_view() {
    let remote = bare_remote();
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.git(&["remote", "add", "origin", &path_string(remote.path())]);
    let expected = snapshot(repo.path()).remote_fetch_addresses["origin"].clone();

    assert!(error_text(client().remove_remote("origin", &["/stale".to_string()], repo.path())).contains("remote address changed"));
    assert_eq!(snapshot(repo.path()).remotes, vec!["origin".to_string()]);

    client().remove_remote("origin", &expected, repo.path()).expect("remove remote");
    assert!(snapshot(repo.path()).remotes.is_empty());
    assert!(remote.path().exists(), "the server must be left alone");
}

// MARK: Tags on remotes

#[test]
fn push_tag_publishes_shown_tag_and_refuses_moved_tag() {
    let remote = bare_remote();
    let repo = Repo::new();
    let first = repo.commit("a.txt", "one\n", "First");
    repo.git(&["remote", "add", "origin", &path_string(remote.path())]);
    client().create_tag("v1", &first, None, repo.path()).expect("create tag");
    let before = snapshot(repo.path());
    let tip = before.tag_tips["v1"].clone();
    let expected = before.remote_push_addresses["origin"].clone();

    client().push_tag("v1", "origin", &tip, &expected, repo.path()).expect("push tag");
    assert_eq!(git(remote.path(), &["rev-parse", "refs/tags/v1"]).trim(), tip);

    let second = repo.commit("a.txt", "two\n", "Second");
    repo.git(&["tag", "-f", "v1", &second]);
    let message = error_text(client().push_tag("v1", "origin", &tip, &expected, repo.path()));
    assert!(message.contains("tag changed"), "{message}");
}

#[test]
fn push_tag_never_moves_an_existing_remote_tag() {
    let remote = bare_remote();
    let repo = Repo::new();
    let first = repo.commit("a.txt", "one\n", "First");
    repo.git(&["remote", "add", "origin", &path_string(remote.path())]);
    repo.git(&["tag", "v1", &first]);
    let expected = snapshot(repo.path()).remote_push_addresses["origin"].clone();
    client().push_tag("v1", "origin", &first, &expected, repo.path()).expect("push first tag");

    let second = repo.commit("a.txt", "two\n", "Second");
    repo.git(&["tag", "-f", "v1", &second]);
    let tip = snapshot(repo.path()).tag_tips["v1"].clone();
    assert!(client().push_tag("v1", "origin", &tip, &expected, repo.path()).is_err());
    assert_eq!(git(remote.path(), &["rev-parse", "refs/tags/v1"]).trim(), first);
}

#[test]
fn delete_remote_tag_removes_it_from_the_remote() {
    let remote = bare_remote();
    let repo = Repo::new();
    let first = repo.commit("a.txt", "one\n", "First");
    repo.git(&["remote", "add", "origin", &path_string(remote.path())]);
    repo.git(&["tag", "v1", &first]);
    let before = snapshot(repo.path());
    let expected = before.remote_push_addresses["origin"].clone();
    client().push_tag("v1", "origin", &before.tag_tips["v1"], &expected, repo.path()).expect("push tag");

    client().delete_remote_tag("v1", "origin", &first, &expected, repo.path()).expect("delete remote tag");

    assert!(!tag_exists(remote.path(), "v1"));
    assert!(tag_exists(repo.path(), "v1"), "the local tag must be kept");
}

#[test]
fn delete_remote_tag_refuses_stale_local_tag() {
    let remote = bare_remote();
    let repo = Repo::new();
    let first = repo.commit("a.txt", "one\n", "First");
    repo.git(&["remote", "add", "origin", &path_string(remote.path())]);
    repo.git(&["tag", "v1", &first]);
    let expected = snapshot(repo.path()).remote_push_addresses["origin"].clone();
    client().push_tag("v1", "origin", &first, &expected, repo.path()).expect("push tag");
    let second = repo.commit("a.txt", "two\n", "Second");
    repo.git(&["tag", "-f", "v1", &second]);

    let message = error_text(client().delete_remote_tag("v1", "origin", &first, &expected, repo.path()));
    assert!(message.contains("tag changed"), "{message}");
    assert_eq!(git(remote.path(), &["rev-parse", "refs/tags/v1"]).trim(), first);
}

#[test]
fn delete_remote_tag_refuses_when_remote_tag_moved() {
    let remote = bare_remote();
    let repo = Repo::new();
    let first = repo.commit("a.txt", "one\n", "First");
    repo.git(&["remote", "add", "origin", &path_string(remote.path())]);
    repo.git(&["tag", "v1", &first]);
    let expected = snapshot(repo.path()).remote_push_addresses["origin"].clone();
    client().push_tag("v1", "origin", &first, &expected, repo.path()).expect("push tag");

    // Another client moves the remote tag to a different commit.
    let second = repo.commit("a.txt", "two\n", "Second");
    repo.git(&["tag", "v-second", &second]);
    repo.git(&["push", "--force", &path_string(remote.path()), "refs/tags/v-second:refs/tags/v1"]);

    assert!(client().delete_remote_tag("v1", "origin", &first, &expected, repo.path()).is_err());
    assert_eq!(git(remote.path(), &["rev-parse", "refs/tags/v1"]).trim(), second);
}

// MARK: Upstream

#[test]
fn set_and_unset_upstream_checks_branch_tip() {
    let remote = bare_remote();
    let repo = Repo::new();
    let head = repo.commit("a.txt", "one\n", "First");
    repo.git(&["remote", "add", "origin", &path_string(remote.path())]);
    repo.git(&["update-ref", "refs/remotes/origin/main", &head]);
    let tip = local_branch(&snapshot(repo.path()), "main").tip;

    client().set_upstream("main", Some("origin/main"), &tip, repo.path()).expect("set upstream");
    assert_eq!(snapshot(repo.path()).upstream.as_deref(), Some("origin/main"));

    let second = repo.commit("a.txt", "two\n", "Second");
    let message = error_text(client().set_upstream("main", None, &tip, repo.path()));
    assert!(message.contains("branch changed"), "{message}");
    assert_eq!(snapshot(repo.path()).upstream.as_deref(), Some("origin/main"));
    assert_eq!(snapshot(repo.path()).head_hash.as_deref(), Some(second.as_str()));

    client().set_upstream("main", None, &second, repo.path()).expect("unset upstream");
    assert_eq!(snapshot(repo.path()).upstream, None);
}

#[test]
fn set_upstream_refuses_missing_remote_branch() {
    let repo = Repo::new();
    let head = repo.commit("a.txt", "one\n", "First");
    let message = error_text(client().set_upstream("main", Some("origin/missing"), &head, repo.path()));
    assert!(message.contains("no longer exists"), "{message}");
}

// MARK: Ignore rules

#[test]
fn ignore_pattern_escapes_literal_names() {
    assert_eq!(ignore_pattern("notes/todo.txt", IgnoreRule::Path).as_deref(), Some("/notes/todo.txt"));
    assert_eq!(ignore_pattern("build/", IgnoreRule::Path).as_deref(), Some("/build/"));
    assert_eq!(ignore_pattern("a*b?[c].txt", IgnoreRule::Path).as_deref(), Some(r"/a\*b\?\[c].txt"));
    assert_eq!(ignore_pattern("name ", IgnoreRule::Path).as_deref(), Some("/name\\ "));
    assert_eq!(ignore_pattern("logs/app.log", IgnoreRule::FileExtension).as_deref(), Some("*.log"));
    assert_eq!(ignore_pattern("a/b.tar.gz", IgnoreRule::FileExtension).as_deref(), Some("*.gz"));
    assert_eq!(ignore_pattern(".gitignore", IgnoreRule::FileExtension), None);
    assert_eq!(ignore_pattern("README", IgnoreRule::FileExtension), None);
    assert_eq!(ignore_pattern("trailing.", IgnoreRule::FileExtension), None);
    assert_eq!(ignore_pattern("bad\nname", IgnoreRule::Path), None);
}

#[test]
fn ignore_exact_path_in_shared_gitignore() {
    let repo = Repo::new();
    repo.commit("tracked.txt", "x\n", "Base");
    repo.write("notes/todo.txt", "t\n");
    repo.write("other.txt", "o\n");

    client().ignore("notes/todo.txt", IgnoreRule::Path, IgnoreScope::Shared, repo.path()).expect("ignore path");

    assert_eq!(repo.read(".gitignore"), "/notes/todo.txt\n");
    let status = repo.git(&["status", "--porcelain"]);
    assert!(!status.contains("notes/todo.txt"), "{status}");
    assert!(status.contains("other.txt"), "{status}");
}

#[test]
fn ignore_exact_path_in_local_exclude_keeps_existing_rules() {
    let repo = Repo::new();
    repo.commit("tracked.txt", "x\n", "Base");
    fs::create_dir_all(repo.path().join(".git/info")).expect("create info folder");
    fs::write(repo.path().join(".git/info/exclude"), "existing-rule").expect("write exclude");
    repo.write("scratch.txt", "s\n");

    client().ignore("scratch.txt", IgnoreRule::Path, IgnoreScope::Local, repo.path()).expect("ignore path");

    assert_eq!(repo.read(".git/info/exclude"), "existing-rule\n/scratch.txt\n");
    assert!(!repo.git(&["status", "--porcelain"]).contains("scratch.txt"));
    assert!(!repo.path().join(".gitignore").exists());
}

#[test]
fn ignore_extension_in_shared_and_local_files() {
    let repo = Repo::new();
    repo.commit("tracked.txt", "x\n", "Base");
    repo.write("logs/app.log", "l\n");
    repo.write("other/debug.log", "d\n");

    client().ignore("logs/app.log", IgnoreRule::FileExtension, IgnoreScope::Shared, repo.path()).expect("ignore extension");
    assert_eq!(repo.read(".gitignore"), "*.log\n");
    assert!(git_output(repo.path(), &["check-ignore", "--quiet", "--no-index", "--", "other/debug.log"]).status.success());

    repo.write("notes.md", "n\n");
    repo.write("local/a.md", "a\n");
    client().ignore("notes.md", IgnoreRule::FileExtension, IgnoreScope::Local, repo.path()).expect("ignore local extension");
    // `git init` writes comment lines to info/exclude, so only the end of the file is checked.
    assert!(repo.read(".git/info/exclude").ends_with("\n*.md\n"));
    assert!(git_output(repo.path(), &["check-ignore", "--quiet", "--no-index", "--", "local/a.md"]).status.success());
}

#[test]
fn ignore_appends_to_gitignore_without_trailing_newline_and_skips_duplicates() {
    let repo = Repo::new();
    repo.commit("tracked.txt", "x\n", "Base");
    repo.write(".gitignore", "target/");
    repo.write("draft.txt", "d\n");

    client().ignore("draft.txt", IgnoreRule::Path, IgnoreScope::Shared, repo.path()).expect("ignore path");
    client().ignore("draft.txt", IgnoreRule::Path, IgnoreScope::Shared, repo.path()).expect("ignore again");

    assert_eq!(repo.read(".gitignore"), "target/\n/draft.txt\n");
}

#[test]
fn ignore_refuses_tracked_file() {
    let repo = Repo::new();
    repo.commit("tracked.txt", "x\n", "Base");

    let message = error_text(client().ignore("tracked.txt", IgnoreRule::Path, IgnoreScope::Shared, repo.path()));
    assert!(message.contains("Only untracked files can be ignored"), "{message}");
    assert!(!repo.path().join(".gitignore").exists());
}

#[test]
fn ignore_refuses_extension_rule_for_file_without_extension() {
    let repo = Repo::new();
    repo.commit("tracked.txt", "x\n", "Base");
    repo.write("LICENSE", "l\n");

    let message = error_text(client().ignore("LICENSE", IgnoreRule::FileExtension, IgnoreScope::Shared, repo.path()));
    assert!(message.contains("no extension"), "{message}");
}

#[cfg(not(windows))]
#[test]
fn ignore_glob_named_file_does_not_ignore_neighbours() {
    let repo = Repo::new();
    repo.commit("tracked.txt", "x\n", "Base");
    repo.write("a*.txt", "g\n");
    repo.write("ab.txt", "n\n");

    client().ignore("a*.txt", IgnoreRule::Path, IgnoreScope::Shared, repo.path()).expect("ignore glob name");

    assert!(git_output(repo.path(), &["check-ignore", "--quiet", "--no-index", "--", "a*.txt"]).status.success());
    assert!(!git_output(repo.path(), &["check-ignore", "--quiet", "--no-index", "--", "ab.txt"]).status.success());
}

// MARK: Stashes of selected files

#[cfg_attr(windows, ignore = "Windows file names cannot contain the glob characters this test uses")]
#[test]
fn stash_selected_glob_named_file_leaves_neighbours() {
    let repo = Repo::new();
    repo.commit("base.txt", "base\n", "Base");
    repo.write("base.txt", "changed\n");
    repo.write("a*.txt", "glob\n");
    repo.write("ab.txt", "neighbour\n");

    client().save_stash_paths(&["a*.txt".to_string()], "only the glob file", repo.path()).expect("stash selected file");

    assert!(!repo.path().join("a*.txt").exists());
    assert_eq!(repo.read("ab.txt"), "neighbour\n");
    assert_eq!(repo.read("base.txt"), "changed\n");
    let stashed = repo.git(&["stash", "show", "--include-untracked", "--name-only", "stash@{0}"]);
    assert_eq!(stashed.trim(), "a*.txt", "{stashed}");
    assert!(snapshot(repo.path()).stashes[0].message.contains("only the glob file"));
}

#[test]
fn stash_selected_tracked_file_keeps_other_changes() {
    let repo = Repo::new();
    repo.commit("one.txt", "one\n", "One");
    repo.commit("two.txt", "two\n", "Two");
    repo.write("one.txt", "one changed\n");
    repo.write("two.txt", "two changed\n");

    client().save_stash_paths(&["one.txt".to_string()], "", repo.path()).expect("stash one file");

    assert_eq!(repo.read("one.txt"), "one\n");
    assert_eq!(repo.read("two.txt"), "two changed\n");
    let stashed = repo.git(&["stash", "show", "--name-only", "stash@{0}"]);
    assert_eq!(stashed.trim(), "one.txt");
}

#[test]
fn stash_selected_paths_refuses_empty_or_unknown_selection() {
    let repo = Repo::new();
    repo.commit("one.txt", "one\n", "One");
    repo.write("one.txt", "changed\n");

    assert!(error_text(client().save_stash_paths(&[], "", repo.path())).contains("Select files to stash"));
    let message = error_text(client().save_stash_paths(&["missing.txt".to_string()], "", repo.path()));
    assert!(message.contains("changed since they were selected"), "{message}");
    assert_eq!(repo.read("one.txt"), "changed\n");
    assert!(snapshot(repo.path()).stashes.is_empty());
}

// MARK: Worktrees

#[test]
fn create_worktree_checks_out_branch_at_destination() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.git(&["branch", "feature"]);
    let parent = tempfile::tempdir().expect("create parent");
    let destination = parent.path().join("feature-checkout");
    let tip = local_branch(&snapshot(repo.path()), "feature").tip;

    client().create_worktree("feature", &tip, &destination, repo.path()).expect("create worktree");

    let worktrees = snapshot(repo.path()).worktrees;
    assert_eq!(worktrees.len(), 2);
    assert_eq!(listed_worktree(&worktrees, &destination).branch.as_deref(), Some("feature"));
    assert!(destination.join("a.txt").exists());
}

#[test]
fn create_worktree_refuses_branch_that_moved_after_it_was_shown() {
    let repo = Repo::new();
    let first = repo.commit("a.txt", "one\n", "First");
    repo.git(&["branch", "feature"]);
    let second = repo.commit("a.txt", "two\n", "Second");
    repo.git(&["branch", "-f", "feature", &second]);
    let parent = tempfile::tempdir().expect("create parent");
    let destination = parent.path().join("feature-checkout");

    let message = error_text(client().create_worktree("feature", &first, &destination, repo.path()));
    assert!(message.contains("branch changed"), "{message}");
    assert!(!destination.exists());
    assert_eq!(snapshot(repo.path()).worktrees.len(), 1);
}

#[test]
fn remove_worktree_removes_linked_folder_and_registration() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.git(&["branch", "feature"]);
    let parent = tempfile::tempdir().expect("create parent");
    let destination = parent.path().join("feature-checkout");
    let tip = local_branch(&snapshot(repo.path()), "feature").tip;
    client().create_worktree("feature", &tip, &destination, repo.path()).expect("create worktree");
    let listed = listed_worktree(&snapshot(repo.path()).worktrees, &destination).path;

    client().remove_worktree(&listed, repo.path()).expect("remove worktree");

    assert_eq!(snapshot(repo.path()).worktrees.len(), 1);
    assert!(!destination.exists());
    assert!(branch_exists(repo.path(), "feature"));
}

#[test]
fn remove_worktree_refuses_main_worktree_and_open_checkout() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.git(&["branch", "feature"]);
    let main = snapshot(repo.path()).worktrees[0].path.clone();
    let message = error_text(client().remove_worktree(&main, repo.path()));
    assert!(message.contains("cannot be removed"), "{message}");

    let parent = tempfile::tempdir().expect("create parent");
    let destination = parent.path().join("feature-checkout");
    let tip = local_branch(&snapshot(repo.path()), "feature").tip;
    client().create_worktree("feature", &tip, &destination, repo.path()).expect("create worktree");
    let linked = listed_worktree(&snapshot(repo.path()).worktrees, &destination).path;

    // Removing the checkout that is open in this session is refused.
    let message = error_text(client().remove_worktree(&linked, &destination));
    assert!(message.contains("cannot be removed"), "{message}");
    assert!(destination.exists());
}

#[test]
fn prune_forgets_worktree_whose_folder_was_deleted() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    repo.git(&["branch", "feature"]);
    let parent = tempfile::tempdir().expect("create parent");
    let destination = parent.path().join("feature-checkout");
    let tip = local_branch(&snapshot(repo.path()), "feature").tip;
    client().create_worktree("feature", &tip, &destination, repo.path()).expect("create worktree");
    fs::remove_dir_all(&destination).expect("delete worktree folder");

    let before = snapshot(repo.path()).worktrees;
    assert!(before.iter().any(|worktree| worktree.is_prunable), "{before:?}");

    client().prune_worktrees(repo.path()).expect("prune worktrees");
    assert_eq!(snapshot(repo.path()).worktrees.len(), 1);
}

#[test]
fn git_directories_list_own_and_shared_directories() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "First");
    let main = client().git_directories(repo.path()).expect("main directories");
    assert_eq!(main.len(), 1);

    repo.git(&["branch", "feature"]);
    let parent = tempfile::tempdir().expect("create parent");
    let destination = parent.path().join("feature-checkout");
    let tip = local_branch(&snapshot(repo.path()), "feature").tip;
    client().create_worktree("feature", &tip, &destination, repo.path()).expect("create worktree");
    let directories = client().git_directories(&destination).expect("linked directories");
    assert_eq!(directories.len(), 2);
    assert!(directories.iter().any(|directory| directory.ends_with(".git")));
}

// MARK: Submodules

/// A repository with one submodule `sub`, added and committed. Returns the superproject and the
/// repository the submodule was cloned from.
fn superproject_with_submodule() -> (Repo, Repo) {
    let source = Repo::new();
    source.commit("sub.txt", "sub one\n", "Sub first");
    let repo = Repo::new();
    repo.commit("main.txt", "main\n", "Main");
    git(repo.path(), &["-c", "protocol.file.allow=always", "submodule", "add", &path_string(source.path()), "sub"]);
    repo.git(&["commit", "--quiet", "-m", "Add submodule"]);
    (repo, source)
}

#[test]
fn submodules_list_recorded_commit_at_recorded_state() {
    let (repo, _source) = superproject_with_submodule();

    let listed = client().submodules(repo.path()).expect("list submodules");

    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].path, "sub");
    assert_eq!(listed[0].state, SubmoduleState::AtRecordedCommit);
    assert!(!listed[0].has_local_changes);
    assert_eq!(listed[0].recorded_commit, git(&repo.path().join("sub"), &["rev-parse", "HEAD"]).trim());
}

#[test]
fn submodule_states_follow_local_changes_and_checkouts() {
    let (repo, _source) = superproject_with_submodule();
    let sub = repo.path().join("sub");
    let recorded = client().submodules(repo.path()).expect("list")[0].recorded_commit.clone();

    repo.write("sub/scratch.txt", "scratch\n");
    let dirty = client().submodules(repo.path()).expect("list dirty");
    assert!(dirty[0].has_local_changes);
    assert_eq!(dirty[0].state, SubmoduleState::AtRecordedCommit);
    fs::remove_file(sub.join("scratch.txt")).expect("remove scratch file");

    configure(&sub);
    git(&sub, &["commit", "--quiet", "--allow-empty", "-m", "Sub second"]);
    let moved_head = git(&sub, &["rev-parse", "HEAD"]).trim().to_string();
    let other = client().submodules(repo.path()).expect("list other commit");
    assert_eq!(other[0].state, SubmoduleState::OnAnotherCommit(moved_head));
    assert_eq!(other[0].recorded_commit, recorded);

    client().update_submodule("sub", repo.path()).expect("check out recorded commit");
    let restored = client().submodules(repo.path()).expect("list restored");
    assert_eq!(restored[0].state, SubmoduleState::AtRecordedCommit);
    assert_eq!(git(&sub, &["rev-parse", "HEAD"]).trim(), recorded);
}

#[test]
fn missing_submodule_folder_is_not_checked_out_and_can_be_restored() {
    let (repo, _source) = superproject_with_submodule();
    let recorded = client().submodules(repo.path()).expect("list")[0].recorded_commit.clone();
    fs::remove_dir_all(repo.path().join("sub")).expect("remove submodule folder");

    let missing = client().submodules(repo.path()).expect("list missing");
    assert_eq!(missing[0].state, SubmoduleState::NotCheckedOut);
    assert!(!missing[0].has_local_changes);

    client().update_submodule("sub", repo.path()).expect("initialise submodule");

    let restored = client().submodules(repo.path()).expect("list restored");
    assert_eq!(restored[0].state, SubmoduleState::AtRecordedCommit);
    assert_eq!(restored[0].recorded_commit, recorded);
    assert!(repo.path().join("sub/sub.txt").exists());
}

#[test]
fn update_submodule_refuses_unregistered_path() {
    let (repo, _source) = superproject_with_submodule();

    let message = error_text(client().update_submodule("not-a-submodule", repo.path()));
    assert!(message.contains("no longer registered"), "{message}");
}
