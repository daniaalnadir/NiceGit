//! Integration tests for the repository's local configuration: listing, setting, and removing keys,
//! run against real repositories in temporary directories.

use std::path::Path;
use std::process::Command;

use nicegit_core::client::GitClient;
use nicegit_core::models::GitError;
use tempfile::TempDir;

/// Runs Git in `directory` and returns its standard output, panicking on failure.
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

/// A temporary repository with local identity settings and no other configuration of note.
fn repository() -> TempDir {
    let dir = tempfile::tempdir().expect("create temp dir");
    git(dir.path(), &["init", "-q", "-b", "main"]);
    for (key, value) in [("user.name", "Test"), ("user.email", "test@example.com"), ("core.autocrlf", "false")] {
        git(dir.path(), &["config", key, value]);
    }
    dir
}

fn values(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| item.to_string()).collect()
}

fn error_text(result: Result<(), GitError>) -> String {
    match result {
        Ok(()) => panic!("expected a refusal"),
        Err(error) => error.to_string(),
    }
}

#[test]
fn a_newline_in_a_value_round_trips_through_the_list() {
    let dir = repository();
    let client = GitClient::new();
    client.set_config_value(dir.path(), "nicegit.note", &[], "first line\nsecond line").expect("set value");

    let entries = client.config_entries(dir.path()).expect("list config");
    let entry = entries.iter().find(|entry| entry.key == "nicegit.note").expect("the new key is listed");
    assert_eq!(entry.values, values(&["first line\nsecond line"]));
    assert!(!entry.is_multi_valued());
    assert_eq!(git(dir.path(), &["config", "--local", "--get", "nicegit.note"]).trim_end_matches('\n'), "first line\nsecond line");
}

#[test]
fn a_key_with_several_values_is_listed_and_never_written() {
    let dir = repository();
    git(dir.path(), &["config", "--add", "remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*"]);
    git(dir.path(), &["config", "--add", "remote.origin.fetch", "+refs/tags/*:refs/tags/*"]);
    let client = GitClient::new();

    let entries = client.config_entries(dir.path()).expect("list config");
    let entry = entries.iter().find(|entry| entry.key == "remote.origin.fetch").expect("multi-valued key is listed");
    assert!(entry.is_multi_valued());
    assert_eq!(entry.values.len(), 2);

    let shown = entry.values.clone();
    let refusal = error_text(client.set_config_value(dir.path(), "remote.origin.fetch", &shown, "+x:y"));
    assert!(refusal.contains("several values"), "unexpected refusal: {refusal}");
    let refusal = error_text(client.unset_config_value(dir.path(), "remote.origin.fetch", &shown));
    assert!(refusal.contains("several values"), "unexpected refusal: {refusal}");
    assert_eq!(git(dir.path(), &["config", "--local", "--get-all", "remote.origin.fetch"]).lines().count(), 2);
}

#[test]
fn a_stale_write_is_refused_and_leaves_the_value_alone() {
    let dir = repository();
    let client = GitClient::new();
    client.set_config_value(dir.path(), "nicegit.mode", &[], "one").expect("add key");

    // Another client changes the value after the window showed "one".
    git(dir.path(), &["config", "nicegit.mode", "two"]);

    let refusal = error_text(client.set_config_value(dir.path(), "nicegit.mode", &values(&["one"]), "three"));
    assert!(refusal.contains("changed since it was shown"), "unexpected refusal: {refusal}");
    assert_eq!(git(dir.path(), &["config", "--local", "--get", "nicegit.mode"]).trim(), "two");

    let refusal = error_text(client.unset_config_value(dir.path(), "nicegit.mode", &values(&["one"])));
    assert!(refusal.contains("changed since it was shown"), "unexpected refusal: {refusal}");
    assert_eq!(git(dir.path(), &["config", "--local", "--get", "nicegit.mode"]).trim(), "two");

    // Adding a key the window saw as absent is refused once it exists.
    let refusal = error_text(client.set_config_value(dir.path(), "nicegit.mode", &[], "three"));
    assert!(refusal.contains("changed since it was shown"), "unexpected refusal: {refusal}");
}

#[test]
fn an_invalid_key_is_rejected_before_git_runs() {
    let dir = repository();
    let client = GitClient::new();
    for key in ["nodot", "-x.name", "core.1name", "a..b"] {
        let refusal = error_text(client.set_config_value(dir.path(), key, &[], "value"));
        assert!(!refusal.is_empty(), "{key} should be refused");
        let refusal = error_text(client.unset_config_value(dir.path(), key, &values(&["value"])));
        assert!(!refusal.is_empty(), "{key} should be refused on removal");
    }
    assert!(client.config_entries(dir.path()).expect("list config").iter().all(|entry| entry.key != "nodot"));
}

#[test]
fn unset_removes_a_single_valued_key_and_matches_the_shown_value() {
    let dir = repository();
    let client = GitClient::new();
    client.set_config_value(dir.path(), "nicegit.test", &[], "1").expect("add key");
    let shown = client.config_entries(dir.path()).expect("list config");
    let entry = shown.iter().find(|entry| entry.key == "nicegit.test").expect("listed");

    client.unset_config_value(dir.path(), "nicegit.test", &entry.values).expect("remove key");

    let after = client.config_entries(dir.path()).expect("list config");
    assert!(after.iter().all(|entry| entry.key != "nicegit.test"));
    // Removing a key that is no longer set is refused, not reported as a Git exit status.
    let refusal = error_text(client.unset_config_value(dir.path(), "nicegit.test", &values(&["1"])));
    assert!(refusal.contains("changed since it was shown"), "unexpected refusal: {refusal}");
}

#[test]
fn keys_are_listed_the_way_git_prints_them() {
    let dir = repository();
    git(dir.path(), &["config", "Remote.Origin.URL", "https://example.invalid/repo.git"]);
    let client = GitClient::new();
    let entries = client.config_entries(dir.path()).expect("list config");
    assert!(entries.iter().any(|entry| entry.key == "remote.Origin.url"), "keys: {:?}", entries.iter().map(|e| &e.key).collect::<Vec<_>>());
}

#[test]
fn a_branch_key_with_a_slash_and_a_value_starting_with_a_dash_are_written_as_given() {
    let dir = repository();
    let client = GitClient::new();
    let key = "branch.feature/x y.description";
    client.set_config_value(dir.path(), key, &[], "-v").expect("set value");
    assert_eq!(git(dir.path(), &["config", "--local", "--get", key]).trim_end_matches('\n'), "-v");

    client.set_config_value(dir.path(), key, &values(&["-v"]), "--global").expect("edit value");
    assert_eq!(git(dir.path(), &["config", "--local", "--get", key]).trim_end_matches('\n'), "--global");

    client.unset_config_value(dir.path(), key, &values(&["--global"])).expect("remove value");
    let entries = client.config_entries(dir.path()).expect("list config");
    assert!(entries.iter().all(|entry| entry.key != key));
}
