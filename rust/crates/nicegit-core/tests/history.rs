//! Integration tests for history tools: blame, file history and versions, searching, and restoring
//! a file. Each test runs against a real repository in a temporary directory.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use nicegit_core::file_history::FileVersion;
use nicegit_core::models::GitError;
use nicegit_core::search::{CommitSearchField, ContentMatch};
use nicegit_core::GitClient;
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

    fn head(&self) -> String {
        self.git(&["rev-parse", "HEAD"]).trim().to_string()
    }

    fn write_bytes(&self, relative: &str, contents: &[u8]) {
        let file = self.path().join(relative);
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent).expect("create parent folder");
        }
        fs::write(file, contents).expect("write file");
    }

    fn write(&self, relative: &str, contents: &str) {
        self.write_bytes(relative, contents.as_bytes());
    }

    /// Writes one file, commits everything as the configured user, and returns the new commit.
    fn commit(&self, relative: &str, contents: &str, message: &str) -> String {
        self.write(relative, contents);
        self.git(&["add", "--all"]);
        self.git(&["commit", "--quiet", "-m", message]);
        self.head()
    }

    /// Commits everything as `name`, setting the author in this repository's own config.
    fn commit_as(&self, name: &str, email: &str, message: &str) -> String {
        self.git(&["config", "user.name", name]);
        self.git(&["config", "user.email", email]);
        self.git(&["add", "--all"]);
        self.git(&["commit", "--quiet", "-m", message]);
        self.git(&["config", "user.name", "Test"]);
        self.git(&["config", "user.email", "test@example.com"]);
        self.head()
    }
}

fn client() -> GitClient {
    GitClient::new()
}

/// The message of an expected refusal, panicking if the call succeeded.
fn error_text<T: std::fmt::Debug>(result: Result<T, GitError>) -> String {
    match result {
        Ok(value) => panic!("expected a refusal, got Ok({value:?})"),
        Err(error) => error.to_string(),
    }
}

// MARK: Blame

#[test]
fn blame_attributes_each_line_to_the_commit_that_wrote_it() {
    let repo = Repo::new();
    repo.write("notes.txt", "one\ntwo\nthree\n");
    repo.commit_as("Ann", "ann@example.com", "First");
    repo.write("notes.txt", "one\n2\nthree\n");
    let second = repo.commit_as("Bob", "bob@example.com", "Second");

    let lines = client().blame("notes.txt", None, false, repo.path()).expect("blame");
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0].number, 1);
    assert_eq!(lines[0].commit.author_name, "Ann");
    assert_eq!(lines[0].commit.author_email, "ann@example.com");
    assert_eq!(lines[1].content, "2");
    assert_eq!(lines[1].commit.hash, second);
    assert_eq!(lines[1].commit.author_name, "Bob");
    assert_eq!(lines[1].commit.summary, "Second");
    assert!(lines[1].commit.author_time.is_some());
    assert_eq!(lines[2].number, 3);
    assert_eq!(lines[2].commit.author_name, "Ann");
}

#[test]
fn blame_at_a_revision_ignores_later_changes() {
    let repo = Repo::new();
    repo.write("unrelated.txt", "x\n");
    let first = repo.commit_as("Ann", "ann@example.com", "Unrelated");
    // The file did not exist at the first commit, so blaming it there is an error.
    let missing = error_text(client().blame("notes.txt", Some(&first), false, repo.path()));
    assert!(!missing.is_empty());

    repo.write("notes.txt", "shared\n");
    let created = repo.commit_as("Ann", "ann@example.com", "Create");
    repo.write("notes.txt", "shared\nlater\n");
    repo.commit_as("Bob", "bob@example.com", "Append");

    let at_revision = client().blame("notes.txt", Some(&created), false, repo.path()).expect("blame at revision");
    assert_eq!(at_revision.len(), 1);
    assert_eq!(at_revision[0].commit.hash, created);
    assert_eq!(at_revision[0].content, "shared");
}

#[test]
fn blame_ignoring_whitespace_keeps_the_earlier_author() {
    let repo = Repo::new();
    repo.write("code.rs", "value = 1\n");
    let original = repo.commit_as("Ann", "ann@example.com", "Add value");
    repo.write("code.rs", "value  =  1\n");
    repo.commit_as("Bob", "bob@example.com", "Reformat");

    let plain = client().blame("code.rs", None, false, repo.path()).expect("blame");
    assert_eq!(plain[0].commit.author_name, "Bob");
    let ignoring = client().blame("code.rs", None, true, repo.path()).expect("blame -w");
    assert_eq!(ignoring[0].commit.hash, original);
    assert_eq!(ignoring[0].commit.author_name, "Ann");
}

#[test]
fn blame_of_crlf_file_splits_every_line() {
    let repo = Repo::new();
    repo.write_bytes("windows.txt", b"alpha\r\nbeta\r\ngamma\r\n");
    repo.commit_as("Ann", "ann@example.com", "Windows file");

    let lines = client().blame("windows.txt", None, false, repo.path()).expect("blame");
    assert_eq!(lines.len(), 3, "CRLF endings must not merge the lines");
    assert_eq!(lines.iter().map(|line| line.number).collect::<Vec<_>>(), vec![1, 2, 3]);
    assert_eq!(lines[0].content, "alpha\r");
    assert_eq!(lines[2].content, "gamma\r");
    assert!(lines.iter().all(|line| line.commit.author_name == "Ann"));
}

#[test]
fn blame_marks_unsaved_lines_as_uncommitted() {
    let repo = Repo::new();
    repo.commit("draft.txt", "committed\n", "Draft");
    repo.write("draft.txt", "committed\nlocal edit\n");

    let lines = client().blame("draft.txt", None, false, repo.path()).expect("blame working file");
    assert_eq!(lines.len(), 2);
    assert!(!lines[0].commit.is_uncommitted());
    assert!(lines[1].commit.is_uncommitted());
}

#[test]
fn blame_refuses_binary_files() {
    let repo = Repo::new();
    repo.write_bytes("data.bin", b"ab\0cd\n");
    repo.commit_as("Ann", "ann@example.com", "Binary");

    let message = error_text(client().blame("data.bin", None, false, repo.path()));
    assert!(message.contains("binary"), "unexpected message: {message}");
}

// MARK: File history and versions

#[test]
fn file_history_follows_a_rename() {
    let repo = Repo::new();
    repo.commit("old.txt", "alpha\nbeta\n", "Create old");
    repo.commit("old.txt", "alpha\nBETA\n", "Edit old");
    repo.git(&["mv", "old.txt", "new.txt"]);
    let renamed = repo.commit("new.txt", "alpha\nBETA\ngamma\n", "Rename to new");

    let history = client().file_history("new.txt", 200, repo.path()).expect("file history");
    assert_eq!(history.len(), 3);
    assert_eq!(history[0].commit.hash, renamed);
    assert_eq!(history[0].status, "R");
    assert_eq!(history[0].path, "new.txt");
    assert_eq!(history[1].status, "M");
    assert_eq!(history[1].path, "old.txt", "commits before the rename use the old name");
    assert_eq!(history[2].status, "A");
    assert_eq!(history[2].commit.subject, "Create old");
    assert!(!history[0].deletes_file());
    assert_eq!(history[2].commit.author_name, "Test");
}

#[test]
fn file_history_of_a_deleted_file_marks_the_deletion() {
    let repo = Repo::new();
    repo.commit("gone.txt", "bye\n", "Add");
    repo.git(&["rm", "--quiet", "gone.txt"]);
    repo.git(&["commit", "--quiet", "-m", "Delete"]);

    let history = client().file_history("gone.txt", 200, repo.path()).expect("file history");
    assert_eq!(history.len(), 2);
    assert!(history[0].deletes_file());
}

#[test]
fn file_history_of_repository_without_commits_is_empty() {
    let repo = Repo::new();
    assert!(client().file_history("anything.txt", 200, repo.path()).expect("history").is_empty());
}

#[test]
fn commit_file_diff_shows_only_that_file_in_that_commit() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n", "Add a");
    repo.commit("b.txt", "unrelated\n", "Add b");
    let hash = repo.commit("a.txt", "one\ntwo\n", "Extend a");
    let patch = client().commit_file_diff(&hash, "a.txt", false, repo.path()).expect("file diff");
    assert!(patch.contains("+two"));
    assert!(!patch.contains("unrelated"));
}

#[test]
fn file_bytes_reads_each_version() {
    let repo = Repo::new();
    let first = repo.commit("f.txt", "v1\n", "First");
    repo.commit("f.txt", "v2\n", "Second");
    repo.write("f.txt", "working\n");
    repo.git(&["add", "f.txt"]);
    repo.write("f.txt", "unstaged\n");

    let at_first = client().file_bytes("f.txt", &FileVersion::Revision(first.clone()), 1_000, repo.path()).expect("bytes");
    assert_eq!(at_first.as_deref(), Some(&b"v1\n"[..]));
    let staged = client().file_bytes("f.txt", &FileVersion::Index, 1_000, repo.path()).expect("bytes");
    assert_eq!(staged.as_deref(), Some(&b"working\n"[..]));
    let on_disk = client().file_bytes("f.txt", &FileVersion::WorkingFile, 1_000, repo.path()).expect("bytes");
    assert_eq!(on_disk.as_deref(), Some(&b"unstaged\n"[..]));
    let missing = client().file_bytes("missing.txt", &FileVersion::Revision(first), 1_000, repo.path()).expect("bytes");
    assert_eq!(missing, None);
}

#[test]
fn file_bytes_refuses_files_over_the_limit() {
    let repo = Repo::new();
    let first = repo.commit("big.txt", "0123456789\n", "Big");
    let message = error_text(client().file_bytes("big.txt", &FileVersion::Revision(first), 4, repo.path()));
    assert!(message.contains("too large"), "unexpected message: {message}");
}

// MARK: Searching history

#[test]
fn search_finds_commits_by_message_regardless_of_case() {
    let repo = Repo::new();
    let fix = repo.commit("a.txt", "1\n", "Fix login crash");
    repo.commit("b.txt", "2\n", "Add dashboard");

    let found = client().search_commits("LOGIN crash", CommitSearchField::Message, 200, repo.path()).expect("search");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].hash, fix);
    assert!(client().search_commits("nothing like this", CommitSearchField::Message, 200, repo.path()).expect("search").is_empty());
}

#[test]
fn search_finds_commits_by_author() {
    let repo = Repo::new();
    repo.write("docs.txt", "docs\n");
    let dana = repo.commit_as("Dana Writer", "dana@example.com", "Docs");
    repo.write("code.txt", "code\n");
    repo.commit_as("Eli Coder", "eli@example.com", "Code");

    let found = client().search_commits("dana", CommitSearchField::Author, 200, repo.path()).expect("search");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].hash, dana);
    assert_eq!(found[0].author_email, "dana@example.com");
}

#[test]
fn search_finds_commits_that_add_or_remove_text() {
    let repo = Repo::new();
    let adds = repo.commit("config.rs", "let retries = 3;\n", "Configure");
    repo.commit("config.rs", "let attempts = 5;\n", "Rename setting");

    let found = client().search_commits("retries = 3", CommitSearchField::CodeChange, 200, repo.path()).expect("search");
    assert_eq!(found.len(), 2, "the commit that removed the text also changed its occurrence");
    assert_eq!(found[1].hash, adds);
    let by_message = client().search_commits("retries", CommitSearchField::Message, 200, repo.path()).expect("search");
    assert!(by_message.is_empty());
}

#[test]
fn search_covers_commits_on_other_branches() {
    let repo = Repo::new();
    repo.commit("base.txt", "base\n", "Base");
    repo.git(&["switch", "-c", "feature"]);
    let hotfix = repo.commit("parser.txt", "fixed\n", "Hotfix for parser");
    repo.git(&["switch", "main"]);

    let found = client().search_commits("hotfix for parser", CommitSearchField::Message, 200, repo.path()).expect("search");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].hash, hotfix);
}

#[test]
fn search_by_commit_id_finds_that_commit_for_any_field() {
    let repo = Repo::new();
    repo.commit("a.txt", "1\n", "First");
    let second = repo.commit("a.txt", "2\n", "Second message");
    let short = &second[..10];

    let by_id = client().search_commits(short, CommitSearchField::CommitId, 200, repo.path()).expect("search");
    assert_eq!(by_id.len(), 1);
    assert_eq!(by_id[0].hash, second);
    let from_message_field = client().search_commits(short, CommitSearchField::Message, 200, repo.path()).expect("search");
    assert_eq!(from_message_field.first().map(|commit| commit.hash.as_str()), Some(second.as_str()));
    assert!(client().search_commits("not-an-id", CommitSearchField::CommitId, 200, repo.path()).expect("search").is_empty());
}

// MARK: Searching contents

fn match_for<'a>(matches: &'a [ContentMatch], path: &str) -> &'a ContentMatch {
    matches.iter().find(|found| found.path == path).unwrap_or_else(|| panic!("no match in {path}: {matches:?}"))
}

#[test]
fn content_search_reports_paths_with_spaces_and_line_numbers() {
    let repo = Repo::new();
    repo.write("my notes.txt", "alpha\nNeedle here\n");
    repo.write("other.txt", "needle again\n");
    repo.git(&["add", "--all"]);
    repo.git(&["commit", "--quiet", "-m", "Notes"]);

    let matches = client().search_contents("needle", None, true, 100, repo.path()).expect("search");
    assert_eq!(matches.len(), 2);
    let spaced = match_for(&matches, "my notes.txt");
    assert_eq!(spaced.line, 2);
    assert_eq!(spaced.text, "Needle here");
    assert_eq!(match_for(&matches, "other.txt").line, 1);

    let case_sensitive = client().search_contents("needle", None, false, 100, repo.path()).expect("search");
    assert_eq!(case_sensitive.len(), 1);
    assert_eq!(case_sensitive[0].path, "other.txt");
}

#[test]
fn content_search_at_a_revision_ignores_the_working_files() {
    let repo = Repo::new();
    let first = repo.commit("my notes.txt", "keep needle\n", "Notes");
    repo.commit("my notes.txt", "nothing here\n", "Remove needle");
    repo.write("my notes.txt", "needle in working copy\n");

    let at_first = client().search_contents("needle", Some(&first), true, 100, repo.path()).expect("search");
    assert_eq!(at_first, vec![ContentMatch { path: "my notes.txt".into(), line: 1, text: "keep needle".into() }]);
    let working = client().search_contents("needle", None, true, 100, repo.path()).expect("search");
    assert_eq!(working.len(), 1);
    assert_eq!(working[0].text, "needle in working copy");
}

#[test]
fn content_search_stops_at_the_limit_and_matches_literally() {
    let repo = Repo::new();
    repo.write("a.txt", "x.y\nx.y\nxzy\n");
    repo.git(&["add", "--all"]);
    repo.git(&["commit", "--quiet", "-m", "Lines"]);

    let literal = client().search_contents("x.y", None, true, 100, repo.path()).expect("search");
    assert_eq!(literal.len(), 2, "the dot matches only itself");
    let limited = client().search_contents("x", None, true, 1, repo.path()).expect("search");
    assert_eq!(limited.len(), 1);
    assert!(client().search_contents("line\nbreak", None, true, 100, repo.path()).expect("search").is_empty());
}

// MARK: Restoring a file

/// Restores on `main` expecting a refusal, and returns the refusal's message.
fn restore_error(repo: &Repo, path: &str, source: &str, expected_head: &str) -> String {
    error_text(client().restore(path, source, "main", Some(expected_head), repo.path()))
}

#[test]
fn restore_to_a_commit_replaces_staged_and_working_edits() {
    let repo = Repo::new();
    let first = repo.commit("f.txt", "v1\n", "First");
    let second = repo.commit("f.txt", "v2\n", "Second");
    repo.write("f.txt", "staged edit\n");
    repo.git(&["add", "f.txt"]);
    repo.write("f.txt", "unstaged edit\n");

    client().restore("f.txt", &first, "main", Some(&second), repo.path()).expect("restore");

    assert_eq!(fs::read_to_string(repo.path().join("f.txt")).expect("read"), "v1\n");
    assert_eq!(repo.git(&["show", ":f.txt"]), "v1\n");
    assert!(git_output(repo.path(), &["diff", "--quiet", &first, "--", "f.txt"]).status.success());
}

#[test]
fn restore_to_before_a_commit_uses_its_parent() {
    let repo = Repo::new();
    repo.commit("f.txt", "before\n", "First");
    let second = repo.commit("f.txt", "after\n", "Second");
    repo.write("f.txt", "mess\n");

    client().restore("f.txt", "HEAD^", "main", Some(&second), repo.path()).expect("restore");

    assert_eq!(fs::read_to_string(repo.path().join("f.txt")).expect("read"), "before\n");
    assert_eq!(repo.git(&["show", ":f.txt"]), "before\n");
}

#[test]
fn restore_to_a_version_without_the_file_removes_it() {
    let repo = Repo::new();
    let base = repo.commit("base.txt", "base\n", "Base");
    let second = repo.commit("f.txt", "added\n", "Add f");

    client().restore("f.txt", &base, "main", Some(&second), repo.path()).expect("restore");

    assert!(!repo.path().join("f.txt").exists());
    assert!(repo.git(&["ls-files", "--", "f.txt"]).is_empty());
    assert!(repo.git(&["status", "--porcelain"]).contains("D  f.txt"), "the deletion is staged");
}

#[test]
fn restore_refuses_a_stale_head_without_touching_the_file() {
    let repo = Repo::new();
    let first = repo.commit("f.txt", "v1\n", "First");
    let stale = repo.commit("f.txt", "v2\n", "Second");
    repo.commit("g.txt", "later\n", "Third");
    repo.write("f.txt", "keep me\n");

    let message = restore_error(&repo, "f.txt", &first, &stale);
    assert!(message.contains("changed since this action was selected"), "unexpected message: {message}");
    assert_eq!(fs::read_to_string(repo.path().join("f.txt")).expect("read"), "keep me\n");
}

#[test]
fn restore_refuses_during_an_unfinished_merge() {
    let repo = Repo::new();
    let base = start_conflicted_merge(&repo);
    let head = repo.head();
    let message = restore_error(&repo, "a.txt", &base, &head);
    assert!(message.contains("Finish or abort"), "unexpected message: {message}");
}

/// Leaves `main` with a conflicted merge of `side` in progress. Returns the commit `main` started on.
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

#[test]
fn restore_refuses_to_overwrite_an_untracked_file() {
    let repo = Repo::new();
    repo.commit("base.txt", "base\n", "Base");
    let added = repo.commit("f.txt", "tracked\n", "Add f");
    repo.git(&["rm", "--cached", "--quiet", "f.txt"]);
    repo.git(&["commit", "--quiet", "-m", "Stop tracking f"]);
    let head = repo.head();
    repo.write("f.txt", "local work\n");

    let message = restore_error(&repo, "f.txt", &added, &head);
    assert!(message.contains("untracked file or a folder"), "unexpected message: {message}");
    assert_eq!(fs::read_to_string(repo.path().join("f.txt")).expect("read"), "local work\n");
}

#[test]
fn restore_refuses_a_folder_at_the_path() {
    let repo = Repo::new();
    let first = repo.commit("f.txt", "file\n", "File");
    repo.git(&["rm", "--quiet", "f.txt"]);
    repo.git(&["commit", "--quiet", "-m", "Remove file"]);
    let head = repo.head();
    repo.write("f.txt/inside.txt", "folder contents\n");

    let message = restore_error(&repo, "f.txt", &first, &head);
    assert!(message.contains("untracked file or a folder"), "unexpected message: {message}");
    assert!(repo.path().join("f.txt").is_dir());
}
