//! Integration tests for the practice repository the first-run tour opens.

use std::fs;
use std::path::Path;
use std::process::Command;

use nicegit_core::client::GitClient;

fn git(directory: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap_or_else(|error| panic!("could not run git {arguments:?}: {error}"));
    assert!(output.status.success(), "git {arguments:?} failed: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn practice_repository_has_commits_branch_and_an_uncommitted_edit() {
    let parent = tempfile::tempdir().expect("temp dir");
    let directory = GitClient::new().create_practice_repository(parent.path(), "Tester", "tester@example.com").expect("created");

    assert_eq!(directory, parent.path().join("NiceGit practice"));
    assert_eq!(git(&directory, &["rev-list", "--count", "main"]).trim(), "3");
    assert!(git(&directory, &["branch", "--list", "idea/greeting"]).contains("idea/greeting"));
    assert_eq!(git(&directory, &["rev-parse", "--abbrev-ref", "HEAD"]).trim(), "main");
    assert_eq!(git(&directory, &["status", "--porcelain"]), " M hello.txt\n");
    assert_eq!(git(&directory, &["config", "--local", "user.name"]).trim(), "Tester");
    assert_eq!(git(&directory, &["config", "--local", "user.email"]).trim(), "tester@example.com");
    assert!(git(&directory, &["log", "-1", "--format=%s", "main"]).contains("Add notes"));
}

#[test]
fn practice_repository_falls_back_to_default_identity_when_empty() {
    let parent = tempfile::tempdir().expect("temp dir");
    let directory = GitClient::new().create_practice_repository(parent.path(), "  ", "").expect("created");
    assert_eq!(git(&directory, &["config", "--local", "user.name"]).trim(), "NiceGit Tour");
    assert_eq!(git(&directory, &["config", "--local", "user.email"]).trim(), "tour@nicegit.invalid");
}

#[test]
fn rerun_replaces_the_marked_practice_folder() {
    let parent = tempfile::tempdir().expect("temp dir");
    let client = GitClient::new();
    let first = client.create_practice_repository(parent.path(), "Tester", "tester@example.com").expect("created");
    fs::write(first.join("extra.txt"), "left over\n").expect("written");
    let second = client.create_practice_repository(parent.path(), "Tester", "tester@example.com").expect("created again");

    assert_eq!(first, second);
    assert!(!second.join("extra.txt").exists(), "the marked folder is replaced");
    assert_eq!(git(&second, &["rev-list", "--count", "main"]).trim(), "3");
}

#[test]
fn unmarked_folder_is_kept_and_another_name_is_used() {
    let parent = tempfile::tempdir().expect("temp dir");
    let existing = parent.path().join("NiceGit practice");
    fs::create_dir_all(&existing).expect("created");
    fs::write(existing.join("mine.txt"), "keep me\n").expect("written");

    let directory = GitClient::new().create_practice_repository(parent.path(), "Tester", "tester@example.com").expect("created");

    assert_eq!(directory, parent.path().join("NiceGit practice 2"));
    assert_eq!(fs::read_to_string(existing.join("mine.txt")).expect("kept"), "keep me\n");
    assert!(!existing.join(".git").exists());
}
