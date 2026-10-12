//! Kept in a test binary of its own: the test sets an environment variable for the whole process,
//! and tests running alongside it would otherwise inherit that variable.

use std::fs;
use std::path::Path;
use std::process::Command;

use nicegit_core::client::GitClient;
use nicegit_core::diff::{DiffContext, DiffRequest};

fn git(directory: &Path, arguments: &[&str]) {
    let status = Command::new("git").args(arguments).current_dir(directory).status().expect("run git");
    assert!(status.success(), "git {arguments:?} failed");
}

fn context_lines(patch: &str) -> usize {
    patch.lines().filter(|line| line.starts_with(' ')).count()
}

#[test]
fn git_diff_opts_does_not_override_the_chosen_context() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path();
    git(path, &["init", "-q", "-b", "main"]);
    for (key, value) in [("user.name", "Test"), ("user.email", "test@example.com"), ("commit.gpgsign", "false")] {
        git(path, &["config", key, value]);
    }
    let original: String = (1..=60).map(|n| format!("line {n}\n")).collect();
    fs::write(path.join("long.txt"), &original).expect("write base");
    git(path, &["add", "--", "long.txt"]);
    git(path, &["commit", "-q", "-m", "Base"]);
    fs::write(path.join("long.txt"), original.replace("line 30\n", "changed 30\n")).expect("write change");

    let client = GitClient::new();
    let change = client.load_status(path).expect("status").into_iter().find(|entry| entry.path == "long.txt").expect("the change");
    let diff = |context: DiffContext| {
        client.diff_with(&change, false, DiffRequest { context, ..Default::default() }, path).expect("diff the change")
    };

    // Git reads GIT_DIFF_OPTS as a context option that overrides the flags NiceGit passes, so the
    // runner must remove it for the chosen context to apply.
    std::env::set_var("GIT_DIFF_OPTS", "--unified=0");
    let ten = diff(DiffContext::Lines(10));
    let whole = diff(DiffContext::WholeFile);
    std::env::remove_var("GIT_DIFF_OPTS");

    assert_eq!(context_lines(&ten), 20, "ten lines each side despite GIT_DIFF_OPTS");
    assert_eq!(context_lines(&whole), 59, "the whole file despite GIT_DIFF_OPTS");
}
