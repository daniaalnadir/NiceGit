//! Interface tests for the remaining details of Mac app features (round two), built on the helpers in
//! `ui_tests`.

#![allow(unused_imports)]

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::kittest::{By, NodeT, Queryable};
use egui_kittest::Harness;
use egui_phosphor::regular as icon;
use nicegit_core::{Operation, StatusKind};

use crate::app::NiceGitApp;
use crate::ui_tests::*;

type App = Harness<'static, NiceGitApp>;

/// Commits a change to `file` with `message`.
fn commit_file(dir: &Path, file: &str, contents: &str, message: &str) {
    std::fs::write(dir.join(file), contents).unwrap();
    git(dir, &["add", "--", file]);
    git(dir, &["commit", "-q", "-m", message]);
}

/// Whether any widget's label contains `text`.
fn shows(harness: &App, text: &str) -> bool {
    harness.query_all_by_label_contains(text).next().is_some()
}

/// The operation Git has in progress, as the last snapshot read it.
fn operation(harness: &App) -> Option<Operation> {
    harness.state().snapshot().and_then(|s| s.operation)
}

/// Whether the last snapshot lists a conflicted file.
fn has_conflicts(harness: &App) -> bool {
    harness.state().snapshot().is_some_and(|s| s.status.iter().any(|entry| entry.kind == StatusKind::Conflicted))
}

/// Whether the last snapshot lists `path` among its changes.
fn changes_list(harness: &App, path: &str) -> bool {
    harness.state().snapshot().is_some_and(|s| s.status.iter().any(|entry| entry.path == path))
}

/// Runs frames until the button with this label stops moving, so a click lands where it was found.
fn settle_button(harness: &mut App, label: &str) {
    let mut last = harness.get_by_label_contains(label).rect();
    for _ in 0..30 {
        harness.step();
        let now = harness.get_by_label_contains(label).rect();
        if now == last {
            break;
        }
        last = now;
    }
}

// MARK: Ignoring untracked files

/// Opens the Changes panel menu for an untracked file and picks an Ignore item.
fn ignore_from_the_changes_menu(harness: &mut App, file: &str, item: &str) {
    harness.get_by_label(file).click_secondary();
    settle(harness);
    harness.get_by_label_contains("Ignore").click();
    settle(harness);
    harness.get_by_label_contains(item).click();
    idle(harness);
}

#[test]
fn ignore_an_untracked_file_in_the_shared_gitignore() {
    let repo = repository();
    let path = repo.path();
    std::fs::write(path.join("draft.txt"), "draft\n").unwrap();
    let mut harness = open(path);
    loaded(&mut harness);
    wait(&mut harness, "the untracked file in the changes", |h| h.query_by_label("draft.txt").is_some());

    ignore_from_the_changes_menu(&mut harness, "draft.txt", "This file in .gitignore");
    assert_eq!(std::fs::read_to_string(path.join(".gitignore")).unwrap(), "/draft.txt\n", "the exact path is written");
    assert_eq!(git(path, &["check-ignore", "draft.txt"]), "draft.txt", "Git now ignores the file");
    wait(&mut harness, "the file to leave the changes", |h| h.query_by_label("draft.txt").is_none());
    assert!(!changes_list(&harness, "draft.txt"), "the ignored file is no longer listed");
}

#[test]
fn ignore_every_file_with_an_extension_in_the_shared_gitignore() {
    let repo = repository();
    let path = repo.path();
    std::fs::write(path.join("server.log"), "started\n").unwrap();
    let mut harness = open(path);
    loaded(&mut harness);
    wait(&mut harness, "the untracked file in the changes", |h| h.query_by_label("server.log").is_some());

    ignore_from_the_changes_menu(&mut harness, "server.log", "All .log files in .gitignore");
    assert_eq!(std::fs::read_to_string(path.join(".gitignore")).unwrap(), "*.log\n", "the extension rule is written");
    assert_eq!(git(path, &["check-ignore", "server.log"]), "server.log", "Git now ignores the file");
    wait(&mut harness, "the file to leave the changes", |h| h.query_by_label("server.log").is_none());
}

#[test]
fn ignore_an_untracked_file_only_on_this_computer() {
    let repo = repository();
    let path = repo.path();
    std::fs::write(path.join("local-notes.txt"), "mine\n").unwrap();
    let mut harness = open(path);
    loaded(&mut harness);
    wait(&mut harness, "the untracked file in the changes", |h| h.query_by_label("local-notes.txt").is_some());

    ignore_from_the_changes_menu(&mut harness, "local-notes.txt", "This file on this computer only");
    let exclude = std::fs::read_to_string(path.join(".git/info/exclude")).unwrap();
    assert!(exclude.lines().any(|line| line == "/local-notes.txt"), "the rule is in info/exclude: {exclude:?}");
    assert!(!path.join(".gitignore").exists(), "the shared .gitignore is not created");
    assert_eq!(git(path, &["check-ignore", "local-notes.txt"]), "local-notes.txt", "Git now ignores the file");
    wait(&mut harness, "the file to leave the changes", |h| h.query_by_label("local-notes.txt").is_none());
    assert!(!changes_list(&harness, "local-notes.txt"), "the ignored file is no longer listed");
}

// MARK: Tags

#[test]
fn delete_a_local_tag_from_the_sidebar_after_confirming() {
    let repo = repository();
    let path = repo.path();
    git(path, &["tag", "v1.0", "HEAD~1"]);
    git(path, &["tag", "beta"]);
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label_contains("TAGS").click();
    settle(&mut harness);
    harness.get_by_label("v1.0").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Delete local tag").click();
    wait(&mut harness, "the delete confirmation", |h| shows(h, "Delete tag v1.0?"));
    settle(&mut harness);
    assert_eq!(git(path, &["tag", "-l"]), "beta\nv1.0", "nothing is deleted before confirming");

    harness.get_by_role_and_label(Role::Button, "Delete").click();
    idle(&mut harness);
    assert_eq!(git(path, &["tag", "-l"]), "beta", "only the chosen tag is deleted");
    wait(&mut harness, "the tag to leave the sidebar", |h| h.query_by_label("v1.0").is_none());
    assert!(harness.query_by_label("beta").is_some(), "the other tag stays listed");
}

// MARK: Push

#[test]
fn push_the_current_branch_with_the_toolbar_button() {
    let repo = repository();
    let path = repo.path();
    let remote = temp_dir();
    git(remote.path(), &["init", "-q", "--bare", "-b", "main"]);
    git(path, &["remote", "add", "origin", &remote.path().display().to_string()]);
    git(path, &["push", "-q", "-u", "origin", "main"]);
    commit_file(path, "more.txt", "more\n", "Add a file to push");
    let local = git(path, &["rev-parse", "HEAD"]);
    let mut harness = open(path);
    loaded(&mut harness);

    toolbar_button(&mut harness, "Push 1");
    idle(&mut harness);
    assert_eq!(git(remote.path(), &["rev-parse", "main"]), local, "the remote's branch now points at the local commit");
    assert_eq!(git(path, &["rev-parse", "origin/main"]), local, "the remote-tracking branch follows");
}

// MARK: Stashes

#[test]
fn pop_a_stash_from_the_sidebar_menu_after_confirming() {
    let repo = repository();
    let path = repo.path();
    std::fs::write(path.join("notes.txt"), "changed\n").unwrap();
    git(path, &["stash", "push", "-q", "-m", "Draft changes"]);
    assert_eq!(git(path, &["stash", "list"]).lines().count(), 1, "one stash exists");
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label_contains("Draft changes").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Pop (apply and delete)").click();
    wait(&mut harness, "the pop confirmation", |h| shows(h, "Pop stash@{0}?"));
    settle(&mut harness);
    assert_eq!(git(path, &["stash", "list"]).lines().count(), 1, "nothing is popped before confirming");

    // The toolbar also has a Pop button, so the confirmation is the last one.
    harness.get_all_by_label("Pop").last().expect("confirm button").click();
    idle(&mut harness);
    assert!(git(path, &["stash", "list"]).is_empty(), "the stash is deleted once it applies");
    assert_eq!(std::fs::read_to_string(path.join("notes.txt")).unwrap(), "changed\n", "its change is back in the working file");
    wait(&mut harness, "the stash to leave the sidebar", |h| !shows(h, "Draft changes"));
}

#[test]
fn delete_one_stash_from_the_stashes_window_after_confirming() {
    let repo = repository();
    let path = repo.path();
    std::fs::write(path.join("notes.txt"), "older\n").unwrap();
    git(path, &["stash", "push", "-q", "-m", "Older work"]);
    std::fs::write(path.join("notes.txt"), "newer\n").unwrap();
    git(path, &["stash", "push", "-q", "-m", "Newer work"]);
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label_contains("Manage stashes").click();
    wait(&mut harness, "the stash window", |h| h.state().tools.iter().any(|t| t.id() == "stashes"));
    wait(&mut harness, "the saved stashes", |h| shows(h, "Older work") && shows(h, "Newer work"));
    settle(&mut harness);

    // The newest stash is listed first, so the second row's delete button belongs to "Older work".
    harness.get_all_by_label("Delete stash").nth(1).expect("delete button for the older stash").click();
    wait(&mut harness, "the delete confirmation", |h| shows(h, "Delete stash@{1} permanently?"));
    settle(&mut harness);
    assert_eq!(git(path, &["stash", "list"]).lines().count(), 2, "nothing is deleted before confirming");

    // The confirmation sits below the list, so its button is the last one.
    harness.get_all_by_label("Delete stash").last().expect("confirm button").click();
    idle(&mut harness);
    let remaining = git(path, &["stash", "list"]);
    assert_eq!(remaining.lines().count(), 1, "one stash is left: {remaining}");
    assert!(remaining.contains("Newer work") && !remaining.contains("Older work"), "the older stash was the one deleted");
    assert_eq!(std::fs::read_to_string(path.join("notes.txt")).unwrap(), "first\nsecond\n", "the working file is untouched");
}

// MARK: Conflict editor

/// Feature changes line two of `notes.txt`; main changes it too, so merging conflicts.
fn merge_conflict_repository() -> tempfile::TempDir {
    let repo = repository();
    let path = repo.path();
    git(path, &["switch", "-q", "feature"]);
    commit_file(path, "notes.txt", "first\nfrom feature\n", "Change notes on feature");
    git(path, &["switch", "-q", "main"]);
    repo
}

/// Merges `feature` into main from its menu, which Git stops with a conflict on `notes.txt`.
fn stop_a_merge_on_a_conflict(harness: &mut App) {
    harness.get_by_label("feature").click_secondary();
    settle(harness);
    harness.get_by_label_contains("Merge into main").click();
    wait(harness, "the merge confirmation", |h| h.state().dialog.is_some());
    harness.get_by_label("Merge").click();
    idle(harness);
    wait(harness, "the stopped merge", |h| operation(h) == Some(Operation::Merge));
    wait(harness, "the operation banner", |h| h.query_by_label_contains("  notes.txt").is_some());
    settle(harness);
}

/// Opens the conflict editor from the banner and waits until its versions have loaded.
fn open_the_conflict_editor(harness: &mut App) {
    harness.get_by_label_contains("  notes.txt").click();
    wait(harness, "the conflict editor", |h| h.state().tools.iter().any(|t| t.id().starts_with("conflict:")));
    wait(harness, "the conflict to load", |h| shows(h, "from feature"));
    settle_button(harness, "Delete file");
    settle(harness);
}

#[test]
fn conflict_editor_shows_the_base_current_and_incoming_versions() {
    let repo = merge_conflict_repository();
    let path = repo.path();
    let mut harness = open(path);
    loaded(&mut harness);
    stop_a_merge_on_a_conflict(&mut harness);
    open_the_conflict_editor(&mut harness);

    // Each pane has a heading and its full text, which must match exactly so a pane showing the
    // wrong version fails. Base is the file before either branch changed line two, current is
    // main's version, and incoming is feature's version.
    assert!(label_is(&harness, "Base"), "the base pane is titled");
    assert!(label_is(&harness, "Current"), "the current pane is titled");
    assert!(label_is(&harness, "Incoming"), "the incoming pane is titled");
    assert!(label_is(&harness, "first\n"), "the base pane shows the base text");
    assert!(label_is(&harness, "first\nsecond\n"), "the current pane shows main's text");
    assert!(label_is(&harness, "first\nfrom feature\n"), "the incoming pane shows feature's text");
    assert!(!label_is(&harness, "Not available for this file."), "every version exists for this file");
}

/// Whether a text label reads exactly `text`. Text is stored as the value of a label node.
fn label_is(harness: &App, text: &str) -> bool {
    harness.query_all(By::new().role(Role::Label).value(text)).next().is_some()
}

#[test]
fn delete_file_resolves_a_conflict_by_deleting_the_file() {
    let repo = merge_conflict_repository();
    let path = repo.path();
    let mut harness = open(path);
    loaded(&mut harness);
    stop_a_merge_on_a_conflict(&mut harness);
    open_the_conflict_editor(&mut harness);

    harness.get_by_role_and_label(Role::Button, "Delete file").click();
    wait(&mut harness, "the delete confirmation", |h| shows(h, "Delete this file?"));
    settle(&mut harness);
    assert!(path.join("notes.txt").exists(), "nothing is deleted before confirming");

    // The confirmation sits above the whole-file row, so its button comes first.
    harness.get_all_by_label("Delete file").next().expect("confirm button").click();
    idle(&mut harness);
    assert!(!path.join("notes.txt").exists(), "the conflicted file is deleted");
    assert!(git(path, &["ls-files", "--", "notes.txt"]).is_empty(), "the file is removed from the index");
    assert!(!has_conflicts(&harness), "no conflict is left");
}

// MARK: File history from the Changes panel

#[test]
fn file_history_opens_from_the_changes_menu() {
    let repo = repository();
    let path = repo.path();
    std::fs::write(path.join("notes.txt"), "first\nsecond\nthird\n").unwrap();
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label("notes.txt").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("File history").click();
    wait(&mut harness, "the file history window", |h| h.state().tools.iter().any(|t| t.id().starts_with("file-history")));
    wait(&mut harness, "the file's commits", |h| shows(h, "Add a second line · Test") && shows(h, "Start the notes · Test"));
    settle(&mut harness);
    assert!(shows(&harness, "2 commits"), "the window counts the two commits that changed the file");
}

// MARK: Submodules

#[test]
fn open_a_submodule_as_a_repository_tab_from_its_window() {
    let library = repository();
    let repo = repository();
    let path = repo.path();
    let add = Command::new("git")
        .args(["-c", "protocol.file.allow=always", "submodule", "add", "-q"])
        .arg(library.path())
        .arg("library")
        .current_dir(path)
        .output()
        .unwrap();
    assert!(add.status.success(), "{}", String::from_utf8_lossy(&add.stderr));
    git(path, &["commit", "-qm", "Add the library"]);
    let submodule = path.join("library").canonicalize().unwrap();
    let mut harness = open(path);
    loaded(&mut harness);

    repository_menu(&mut harness, &folder_name(path), "Submodules");
    let open_button = format!("{}  Open", icon::ARROW_SQUARE_OUT);
    wait(&mut harness, "the submodule card", |h| {
        h.query_by_role_and_label(Role::Button, &open_button).is_some_and(|b| !b.accesskit_node().is_disabled())
    });
    harness.get_by_role_and_label(Role::Button, &open_button).click_accesskit();
    wait(&mut harness, "the submodule's tab", |h| h.state().repos.len() == 2 && h.state().busy.is_none());
    loaded(&mut harness);
    // Both sides canonical: Windows spells a canonical path with a \\?\ prefix.
    assert_eq!(harness.state().repo().map(|r| r.path.canonicalize().unwrap()), Some(submodule), "the submodule's tab is active");
    assert_eq!(harness.state().snapshot().map(|s| s.name.clone()).as_deref(), Some("library"));
}
