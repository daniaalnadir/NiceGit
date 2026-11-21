//! Integration tests for the command log: commands run through `GitClient` against a real
//! repository are recorded with their arguments, exit status, and error output.

use std::fs;
use std::path::Path;

use nicegit_core::client::GitClient;
use nicegit_core::command_log::{self, Entry};
use tempfile::TempDir;

/// A repository with one commit, in a temporary folder.
fn repository() -> TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    let client = GitClient::new();
    let root = dir.path();
    client.run(&["init", "--initial-branch=main"], root).expect("init");
    client.run(&["config", "user.name", "Test"], root).expect("name");
    client.run(&["config", "user.email", "test@example.com"], root).expect("email");
    client.run(&["config", "commit.gpgsign", "false"], root).expect("signing");
    fs::write(root.join("base.txt"), "base\n").expect("write");
    client.run(&["add", "--all"], root).expect("add");
    client.run(&["commit", "-q", "-m", "Base"], root).expect("commit");
    dir
}

/// The entries recorded for `directory` since `before`, oldest first.
fn recorded_in(directory: &Path, before: u64) -> Vec<Entry> {
    command_log::entries_since(before).into_iter().filter(|entry| entry.directory == directory).collect()
}

#[test]
fn a_successful_command_is_recorded_with_its_arguments_and_exit_status() {
    let dir = repository();
    let root = dir.path();
    let client = GitClient::new();
    let before = command_log::entries_since(0).last().map_or(0, |entry| entry.id);

    client.run(&["rev-parse", "--verify", "HEAD"], root).expect("rev-parse");

    let entries = recorded_in(root, before);
    let entry = entries.iter().find(|entry| entry.arguments == ["rev-parse", "--verify", "HEAD"]).expect("the command is recorded");
    assert_eq!(entry.exit, Some(0));
    assert!(entry.stderr.is_empty());
    assert!(entry.stdout_bytes > 0, "the hash was printed");
    assert!(entry.background, "rev-parse only reads");
    assert_eq!(command_log::command_line(entry), "git rev-parse --verify HEAD");
}

#[test]
fn a_failing_command_is_recorded_with_its_exit_status_and_error_output() {
    let dir = repository();
    let root = dir.path();
    let client = GitClient::new();
    let before = command_log::entries_since(0).last().map_or(0, |entry| entry.id);

    let result = client.run(&["rev-parse", "--verify", "no-such-ref-for-command-log"], root);
    assert!(result.is_err());

    let entries = recorded_in(root, before);
    let entry = entries
        .iter()
        .find(|entry| entry.arguments.iter().any(|argument| argument == "no-such-ref-for-command-log"))
        .expect("the failure is recorded");
    assert_ne!(entry.exit, Some(0), "a failing command has a non-zero status");
    assert!(entry.exit.is_some(), "Git exited rather than being killed");
    assert!(!entry.stderr.is_empty(), "Git's error output is kept");
}

#[test]
fn a_change_is_recorded_as_a_change_and_a_secret_in_its_arguments_is_hidden() {
    let dir = repository();
    let root = dir.path();
    let client = GitClient::new();
    let before = command_log::entries_since(0).last().map_or(0, |entry| entry.id);

    client
        .run(&["-c", "http.extraheader=AUTHORIZATION: bearer secret-value", "config", "--local", "nicegit.marker", "yes"], root)
        .expect("config");

    let entries = recorded_in(root, before);
    let entry = entries.iter().find(|entry| entry.arguments.iter().any(|argument| argument == "nicegit.marker")).expect("recorded");
    assert!(!entry.background, "writing config changes the repository");
    assert_eq!(entry.arguments[1], "http.extraheader=<hidden>");
    assert!(!command_log::command_line(entry).contains("secret-value"));
}
