//! Interface tests for the remaining details of Mac app features, built on the helpers in
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

use crate::app::NiceGitApp;
use crate::ui_tests::*;

type App = Harness<'static, NiceGitApp>;

/// Whether any widget's label contains `text`.
fn shows(harness: &App, text: &str) -> bool {
    harness.query_all_by_label_contains(text).next().is_some()
}

/// Adds one commit to `feature` and leaves the checkout on main.
fn commit_on_feature(path: &Path) {
    git(path, &["switch", "-q", "feature"]);
    std::fs::write(path.join("side.txt"), "from feature\n").unwrap();
    git(path, &["add", "side.txt"]);
    git(path, &["commit", "-q", "-m", "Work on the feature"]);
    git(path, &["switch", "-q", "main"]);
}

/// Saved app state held in memory, standing in for the platform's storage between launches.
#[derive(Clone, Default)]
struct MemoryStorage(std::collections::HashMap<String, String>);

impl eframe::Storage for MemoryStorage {
    fn get_string(&self, key: &str) -> Option<String> {
        self.0.get(key).cloned()
    }

    fn set_string(&mut self, key: &str, value: String) {
        self.0.insert(key.to_string(), value);
    }

    fn remove_string(&mut self, key: &str) {
        self.0.remove(key);
    }

    fn flush(&mut self) {}
}

/// Opens the repository as a relaunched app, given what the previous run saved. The copy is
/// leaked so the harness can own it for the whole test, as `open` does for its folder.
fn relaunch(path: &Path, saved: &MemoryStorage) -> Harness<'static, NiceGitApp> {
    let storage: &'static MemoryStorage = Box::leak(Box::new(saved.clone()));
    let path = path.to_path_buf();
    Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_eframe(move |cc| {
        cc.storage = Some(storage as &dyn eframe::Storage);
        NiceGitApp::new(cc, Some(path))
    })
}

#[test]
fn a_commit_draft_and_the_side_by_side_choice_survive_a_relaunch() {
    let repo = repository();
    let path = repo.path();
    let mut saved = MemoryStorage::default();
    {
        let mut harness = open(path);
        loaded(&mut harness);
        type_into(&mut harness, "Commit summary", "Half-written message");
        harness.key_press_modifiers(Modifiers::COMMAND, Key::Comma);
        wait(&mut harness, "the Settings window", |h| h.query_by_role_and_label(Role::CheckBox, "Show diffs side by side").is_some());
        settle(&mut harness);
        harness.get_by_role_and_label(Role::CheckBox, "Show diffs side by side").click();
        harness.step();
        assert!(harness.state().settings.split_diff, "the choice is on before quitting");
        // Quitting saves the app state, as eframe does when the window closes.
        eframe::App::save(harness.state_mut(), &mut saved);
    }

    let mut harness = relaunch(path, &saved);
    loaded(&mut harness);
    assert_eq!(
        harness.state().repo().map(|r| r.draft.summary.as_str()),
        Some("Half-written message"),
        "the draft is restored for this checkout"
    );
    assert!(harness.state().settings.split_diff, "the side-by-side choice is restored");
    assert_eq!(
        harness.get_by_role_and_label(Role::TextInput, "Commit summary").accesskit_node().value().as_deref(),
        Some("Half-written message"),
        "the summary field shows the draft"
    );
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Comma);
    wait(&mut harness, "the Settings window", |h| h.query_by_role_and_label(Role::CheckBox, "Show diffs side by side").is_some());
    settle(&mut harness);
    let toggled = harness.get_by_role_and_label(Role::CheckBox, "Show diffs side by side").accesskit_node().toggled();
    assert_eq!(toggled, Some(egui::accesskit::Toggled::True), "the Settings window shows the choice as on");
}

#[test]
fn amending_a_commit_already_on_a_remote_warns_before_amending() {
    let repo = repository();
    let path = repo.path();
    let remote = temp_dir();
    git(remote.path(), &["init", "-q", "--bare"]);
    git(path, &["remote", "add", "origin", &remote.path().to_string_lossy()]);
    let head = git(path, &["rev-parse", "HEAD"]);
    let mut harness = open(path);
    loaded(&mut harness);
    let tick_amend = |harness: &mut App| harness.get_by_role_and_label(Role::CheckBox, "Amend last commit").click();

    // Nothing has been pushed yet, so amending gives no warning.
    tick_amend(&mut harness);
    settle(&mut harness);
    assert!(!shows(&harness, "already on a remote"), "no warning for an unpublished commit");
    tick_amend(&mut harness);
    settle(&mut harness);

    git(path, &["push", "-q", "-u", "origin", "main"]);
    // Command-R reads the remotes again, as a deliberate refresh does.
    harness.key_press_modifiers(Modifiers::COMMAND, Key::R);
    wait(&mut harness, "the pushed branch to be known", |h| h.state().head_published());
    idle(&mut harness);
    tick_amend(&mut harness);
    wait(&mut harness, "the warning", |h| shows(h, "already on a remote"));
    assert_eq!(git(path, &["rev-parse", "HEAD"]), head, "ticking the box amends nothing yet");
}

#[test]
fn a_branch_or_tag_made_in_another_app_appears_without_refreshing() {
    let repo = repository();
    let path = repo.path();
    let mut harness = open(path);
    loaded(&mut harness);

    // Changes within moments of a load are taken to be NiceGit's own, so wait past that first.
    std::thread::sleep(Duration::from_millis(600));
    let start = Instant::now();
    git(path, &["branch", "topic/outside"]);
    git(path, &["tag", "v2.0"]);
    wait(&mut harness, "the new branch in the sidebar", |h| h.query_by_label("topic/outside").is_some());
    assert!(start.elapsed() < Duration::from_secs(10), "refreshed after {:?}", start.elapsed());

    // Tags sit in a section that starts closed; open it to find the new tag.
    wait(&mut harness, "the new tag in the snapshot", |h| h.state().snapshot().is_some_and(|s| s.tags.iter().any(|t| t == "v2.0")));
    harness.get_by_label_contains("TAGS").click();
    wait(&mut harness, "the new tag in the sidebar", |h| h.query_by_label("v2.0").is_some());
    assert!(start.elapsed() < Duration::from_secs(10), "refreshed after {:?}", start.elapsed());
}

#[test]
fn undo_a_cherry_pick_from_the_toolbar() {
    let repo = repository();
    let path = repo.path();
    commit_on_feature(path);
    let before = git(path, &["rev-parse", "HEAD"]);
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label("Work on the feature").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Cherry-pick").click();
    wait(&mut harness, "the cherry-pick confirmation", |h| h.state().dialog.is_some());
    settle(&mut harness);
    harness.get_by_label("Cherry-pick").click();
    idle(&mut harness);
    wait(&mut harness, "the cherry-picked commit", |_| git(path, &["log", "-1", "--format=%s"]) == "Work on the feature");
    assert!(path.join("side.txt").exists());

    toolbar_button(&mut harness, "Undo");
    wait(&mut harness, "the undo confirmation", |h| h.state().dialog.is_some());
    settle(&mut harness);
    harness.get_all_by_label("Undo").last().expect("confirm button").click();
    idle(&mut harness);
    wait(&mut harness, "HEAD back where it was", |_| git(path, &["rev-parse", "HEAD"]) == before);
    assert!(!path.join("side.txt").exists(), "the picked change is gone with the commit");
    assert!(git(path, &["status", "--porcelain"]).is_empty(), "nothing is left staged or changed");
}

#[test]
fn undo_a_revert_from_the_toolbar() {
    let repo = repository();
    let path = repo.path();
    let before = git(path, &["rev-parse", "HEAD"]);
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label("Add a second line").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Revert").click();
    wait(&mut harness, "the revert confirmation", |h| h.state().dialog.is_some());
    settle(&mut harness);
    harness.get_by_label("Revert").click();
    idle(&mut harness);
    wait(&mut harness, "the revert commit", |_| git(path, &["log", "-1", "--format=%s"]).starts_with("Revert"));
    assert_eq!(std::fs::read_to_string(path.join("notes.txt")).unwrap(), "first\n");

    toolbar_button(&mut harness, "Undo");
    wait(&mut harness, "the undo confirmation", |h| h.state().dialog.is_some());
    settle(&mut harness);
    harness.get_all_by_label("Undo").last().expect("confirm button").click();
    idle(&mut harness);
    wait(&mut harness, "HEAD back where it was", |_| git(path, &["rev-parse", "HEAD"]) == before);
    assert_eq!(std::fs::read_to_string(path.join("notes.txt")).unwrap(), "first\nsecond\n", "the reverted line is back");
    assert!(git(path, &["status", "--porcelain"]).is_empty(), "nothing is left staged or changed");
}

#[test]
fn undo_a_rebase_from_the_toolbar() {
    let repo = repository();
    let path = repo.path();
    commit_on_feature(path);
    let before = git(path, &["rev-parse", "HEAD"]);
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label("feature").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Rebase main onto this").click();
    wait(&mut harness, "the rebase confirmation", |h| shows(h, "Rebase main onto feature?"));
    settle(&mut harness);
    harness.get_by_role_and_label(Role::Button, "Rebase").click();
    idle(&mut harness);
    wait(&mut harness, "the rebase", |_| git(path, &["rev-parse", "HEAD"]) != before);
    assert_eq!(git(path, &["log", "-1", "--format=%s"]), "Add a second line", "main's commit is replayed");

    toolbar_button(&mut harness, "Undo");
    wait(&mut harness, "the undo confirmation", |h| h.state().dialog.is_some());
    settle(&mut harness);
    harness.get_all_by_label("Undo").last().expect("confirm button").click();
    idle(&mut harness);
    wait(&mut harness, "HEAD back where it was", |_| git(path, &["rev-parse", "HEAD"]) == before);
    assert_eq!(git(path, &["log", "-1", "--format=%s"]), "Add a second line");
    assert!(git(path, &["status", "--porcelain"]).is_empty(), "nothing is left staged or changed");
}

#[test]
fn drag_a_graph_label_onto_the_current_row_to_merge_it() {
    let repo = repository();
    let path = repo.path();
    commit_on_feature(path);
    let feature_tip = git(path, &["rev-parse", "feature"]);
    let mut harness = open(path);
    loaded(&mut harness);

    // The label on the feature commit's row in the graph, not the sidebar's entry for it.
    let from = harness.get_by_label("Branch label feature").rect().center();
    let to = harness.get_by_label("Add a second line").rect().center();
    harness.hover_at(from);
    harness.step();
    harness.drag_at(from);
    harness.step();
    for step in 1..=6 {
        harness.hover_at(from + (to - from) * (step as f32 / 6.0));
        harness.step();
    }
    harness.drop_at(to);
    harness.step();
    wait(&mut harness, "the merge or rebase choice", |h| shows(h, "Integrate feature"));
    settle(&mut harness);

    harness.get_by_label(&format!("{}  Merge…", icon::GIT_MERGE)).click();
    wait(&mut harness, "the merge confirmation", |h| shows(h, "Merge feature into main?"));
    settle(&mut harness);
    harness.get_by_role_and_label(Role::Button, "Merge").click();
    idle(&mut harness);
    wait(&mut harness, "the merge", |_| git(path, &["rev-list", "--parents", "-n", "1", "HEAD"]).split_whitespace().count() == 3);
    assert_eq!(git(path, &["rev-parse", "HEAD^2"]), feature_tip, "the dragged branch is the merge's second parent");
}

#[test]
fn returning_to_the_app_waits_for_an_open_dialog_to_close() {
    let repo = repository();
    let path = repo.path();
    let mut harness = open(path);
    loaded(&mut harness);
    // With automatic refresh off, only the refresh on return can bring the file in.
    harness.state_mut().settings.auto_refresh = false;
    let focus = |harness: &mut App, focused: bool| {
        harness.input_mut().viewports.entry(egui::ViewportId::ROOT).or_default().focused = Some(focused);
        harness.run_steps(3);
    };
    let listed = |h: &App| h.state().snapshot().is_some_and(|s| s.status.iter().any(|entry| entry.path == "while-dialog-open.txt"));

    toolbar_button(&mut harness, "Branch");
    wait(&mut harness, "the new branch dialog", |h| h.state().dialog.is_some());

    focus(&mut harness, false);
    std::thread::sleep(Duration::from_millis(600));
    std::fs::write(path.join("while-dialog-open.txt"), "written while a dialog is open\n").unwrap();
    std::thread::sleep(Duration::from_millis(600));
    focus(&mut harness, true);
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(1500) {
        harness.step();
        std::thread::sleep(Duration::from_millis(15));
    }
    assert!(harness.state().dialog.is_some(), "the dialog is still open");
    assert!(!listed(&harness), "returning to the app does not refresh while a dialog is open");

    harness.get_by_label("Cancel").click();
    wait(&mut harness, "the refresh once the dialog closes", listed);
}
