//! Integration tests for the external diff and merge tools, run against real repositories in
//! temporary directories. The tools themselves are small shell commands that record what they
//! were given.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

use nicegit_core::client::GitClient;
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

/// A temporary repository on branch `main`, configured in its own config only.
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
        Repo { dir }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn git(&self, arguments: &[&str]) -> String {
        git(self.path(), arguments)
    }

    fn write(&self, file: &str, contents: &str) {
        fs::write(self.path().join(file), contents).expect("write file");
    }

    fn commit(&self, file: &str, contents: &str, message: &str) {
        self.write(file, contents);
        self.git(&["add", file]);
        self.git(&["commit", "-q", "-m", message]);
    }

    /// Leaves `a.txt` conflicted between `main` and `side`.
    fn conflicted(&self) {
        self.commit("a.txt", "base\n", "Base");
        self.git(&["switch", "-q", "-c", "side"]);
        self.commit("a.txt", "side\n", "Side change");
        self.git(&["switch", "-q", "main"]);
        self.commit("a.txt", "main\n", "Main change");
        let merge = git_output(self.path(), &["merge", "--no-edit", "side"]);
        assert!(!merge.status.success(), "the merge should conflict");
    }

    /// Sets a key in the repository's own config.
    fn set(&self, key: &str, value: &str) {
        self.git(&["config", key, value]);
    }
}

/// Writes a shell command that is safe to embed in a config value: single quotes around `path`.
fn quoted(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
}

// MARK: Detection

#[test]
fn configured_tools_read_the_repository_config() {
    let repo = Repo::new();
    repo.set("diff.tool", "fake-diff");
    repo.set("merge.tool", "fake-merge");
    let client = GitClient::new();
    let tools = client.configured_tools(repo.path()).expect("read tools");
    assert_eq!(tools.diff.as_deref(), Some("fake-diff"));
    assert_eq!(tools.merge.as_deref(), Some("fake-merge"));
    assert_eq!(client.configured_diff_tool(repo.path()).as_deref(), Some("fake-diff"));
    assert_eq!(client.configured_merge_tool(repo.path()).as_deref(), Some("fake-merge"));
}

#[test]
fn an_empty_repository_value_overrides_a_tool_set_elsewhere() {
    let repo = Repo::new();
    // Git uses the last value it finds, so an empty value here means no tool, whatever the
    // global configuration names.
    repo.set("diff.tool", "");
    repo.set("merge.tool", "");
    let client = GitClient::new();
    assert_eq!(client.configured_diff_tool(repo.path()), None);
    assert_eq!(client.configured_merge_tool(repo.path()), None);
}

// MARK: Diff tool

#[cfg(unix)]
#[test]
fn open_in_difftool_starts_the_tool_and_returns_without_waiting() {
    let repo = Repo::new();
    repo.commit("notes.txt", "one\n", "Add notes");
    repo.write("notes.txt", "two\n");
    // The gate and the output live beside the repository, in their own temporary folder.
    let scratch = tempfile::tempdir().expect("create scratch dir");
    let gate = scratch.path().join("gate");
    let out = scratch.path().join("out.txt");
    // The tool does nothing until the test creates the gate, so opening must return before then.
    // The loop gives up after about ten seconds so a failing test cannot hang.
    let command = format!(
        "i=0; while [ ! -e {gate} ] && [ $i -lt 100 ]; do sleep 0.1; i=$((i+1)); done; printf '%s\\n' \"$REMOTE\" > {out}",
        gate = quoted(&gate),
        out = quoted(&out),
    );
    repo.set("diff.tool", "fake");
    repo.set("difftool.fake.cmd", &command);

    let started = Instant::now();
    GitClient::new().open_in_difftool("notes.txt", false, repo.path()).expect("open the diff tool");
    assert!(started.elapsed() < Duration::from_secs(5), "opening the diff tool waited for it");
    assert!(!out.exists(), "the tool ran before the test released it");

    fs::write(&gate, "").unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while !out.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    let recorded = fs::read_to_string(&out).expect("the diff tool recorded its arguments");
    assert!(recorded.contains("notes.txt"), "the tool was given the wrong path: {recorded}");
}

// MARK: Merge tool

#[cfg(unix)]
#[test]
fn open_in_mergetool_resolves_the_conflict_when_the_tool_succeeds() {
    let repo = Repo::new();
    repo.conflicted();
    repo.set("merge.tool", "fake");
    repo.set("mergetool.fake.cmd", "printf 'resolved\\n' > \"$MERGED\"");
    // Without trustExitCode for this tool, Git asks whether the merge succeeded, and --no-prompt would stop there.
    repo.set("mergetool.fake.trustExitCode", "true");
    repo.set("mergetool.keepBackup", "false");

    let (sender, receiver) = channel();
    GitClient::new()
        .open_in_mergetool("a.txt", repo.path(), move |status| {
            let _ = sender.send(status);
        })
        .expect("open the merge tool");
    let status = receiver.recv_timeout(Duration::from_secs(30)).expect("the merge tool exits").expect("wait for it");
    assert!(status.success(), "the merge tool should exit successfully");

    assert_eq!(repo.git(&["ls-files", "-u"]), "", "the conflict should be resolved");
    assert_eq!(fs::read_to_string(repo.path().join("a.txt")).unwrap(), "resolved\n");
}

#[test]
fn open_in_mergetool_refuses_a_file_that_is_not_conflicted() {
    let repo = Repo::new();
    repo.commit("a.txt", "base\n", "Base");
    repo.set("merge.tool", "fake");
    let result = GitClient::new().open_in_mergetool("a.txt", repo.path(), |_| {});
    assert!(result.is_err(), "a file without a conflict must not open the merge tool");
}

#[test]
fn open_in_mergetool_refuses_when_no_merge_tool_is_set() {
    let repo = Repo::new();
    repo.conflicted();
    repo.set("merge.tool", "");
    let result = GitClient::new().open_in_mergetool("a.txt", repo.path(), |_| {});
    assert!(result.is_err(), "without a configured merge tool nothing should start");
}
