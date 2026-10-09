//! Interface tests: they drive the real app through egui_kittest, finding widgets by their
//! accessible labels and clicking and typing as a person would, against temporary repositories.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;

use crate::app::NiceGitApp;

fn git(dir: &Path, args: &[&str]) -> String {
    let output =
        Command::new("git").args(args).current_dir(dir).env_remove("GIT_DIR").env_remove("GIT_WORK_TREE").output().expect("run git");
    assert!(output.status.success(), "git {args:?} failed: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// A repository with two commits on main, a `feature` branch, and local-only configuration.
fn repository() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temporary folder");
    let path = dir.path();
    git(path, &["init", "-q", "-b", "main"]);
    for (key, value) in
        [("user.name", "Test"), ("user.email", "test@example.invalid"), ("commit.gpgsign", "false"), ("core.autocrlf", "false")]
    {
        git(path, &["config", key, value]);
    }
    std::fs::write(path.join("notes.txt"), "first\n").unwrap();
    git(path, &["add", "notes.txt"]);
    git(path, &["commit", "-q", "-m", "Start the notes"]);
    git(path, &["branch", "feature"]);
    std::fs::write(path.join("notes.txt"), "first\nsecond\n").unwrap();
    git(path, &["commit", "-qam", "Add a second line"]);
    dir
}

fn open(path: &Path) -> Harness<'static, NiceGitApp> {
    let path: PathBuf = path.to_path_buf();
    Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_eframe(move |cc| NiceGitApp::new(cc, Some(path)))
}

/// Runs frames until `done` holds, failing after a generous timeout.
fn wait(harness: &mut Harness<'static, NiceGitApp>, what: &str, done: impl Fn(&Harness<'static, NiceGitApp>) -> bool) {
    let start = Instant::now();
    loop {
        harness.step();
        if done(harness) {
            harness.step();
            return;
        }
        assert!(start.elapsed() < Duration::from_secs(30), "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(15));
    }
}

fn loaded(harness: &mut Harness<'static, NiceGitApp>) {
    wait(harness, "the repository to load", |h| h.state().snapshot().is_some() && h.state().busy.is_none());
}

fn idle(harness: &mut Harness<'static, NiceGitApp>) {
    wait(harness, "the action to finish", |h| h.state().busy.is_none() && h.state().repo().is_some_and(|r| !r.loading));
}

/// New menus and popups spend their first frame measuring themselves, disabled.
fn settle(harness: &mut Harness<'static, NiceGitApp>) {
    harness.run_steps(3);
}

fn has_label(harness: &Harness<'static, NiceGitApp>, label: &str) -> bool {
    harness.query_by_label_contains(label).is_some()
}

#[test]
fn stage_commit_and_undo_through_the_interface() {
    let repo = repository();
    std::fs::write(repo.path().join("notes.txt"), "first\nsecond\nthird\n").unwrap();
    let mut harness = open(repo.path());
    loaded(&mut harness);

    harness.get_by_label("Stage All Changes").click();
    idle(&mut harness);
    assert_eq!(git(repo.path(), &["diff", "--cached", "--name-only"]), "notes.txt");

    let summary = harness.get_by_label("Commit summary");
    summary.focus();
    harness.step();
    harness.get_by_label("Commit summary").type_text("Add a third line");
    wait(&mut harness, "the commit button", |h| has_label(h, "Commit 1 file to main"));
    harness.get_by_label_contains("Commit 1 file to main").click();
    idle(&mut harness);
    assert_eq!(git(repo.path(), &["log", "-1", "--format=%s"]), "Add a third line");

    // Undo asks first, then moves the branch back and keeps the change staged.
    harness.get_by_label("Undo").click();
    wait(&mut harness, "the undo confirmation", |h| h.state().dialog.is_some());
    harness.get_all_by_label("Undo").last().expect("confirm button").click();
    idle(&mut harness);
    assert_eq!(git(repo.path(), &["log", "-1", "--format=%s"]), "Add a second line");
    assert_eq!(git(repo.path(), &["diff", "--cached", "--name-only"]), "notes.txt");

    // Redo puts the commit back.
    harness.get_by_label("Redo").click();
    wait(&mut harness, "the redo confirmation", |h| h.state().dialog.is_some());
    harness.get_all_by_label("Redo").last().expect("confirm button").click();
    idle(&mut harness);
    assert_eq!(git(repo.path(), &["log", "-1", "--format=%s"]), "Add a third line");
}

#[test]
fn discard_asks_first_and_can_be_undone() {
    let repo = repository();
    std::fs::write(repo.path().join("notes.txt"), "changed\n").unwrap();
    let mut harness = open(repo.path());
    loaded(&mut harness);

    harness.get_by_label("notes.txt").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Discard changes").click();
    wait(&mut harness, "the discard confirmation", |h| h.state().dialog.is_some());
    harness.get_by_label("Discard").click();
    idle(&mut harness);
    assert_eq!(std::fs::read_to_string(repo.path().join("notes.txt")).unwrap(), "first\nsecond\n");

    harness.get_by_label_contains("Undo discard").click();
    idle(&mut harness);
    assert_eq!(std::fs::read_to_string(repo.path().join("notes.txt")).unwrap(), "changed\n");
}

#[test]
fn check_out_a_branch_from_its_menu() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);

    harness.get_by_label("feature").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Check out").click();
    idle(&mut harness);
    assert_eq!(git(repo.path(), &["branch", "--show-current"]), "feature");
    assert_eq!(harness.state().snapshot().map(|s| s.current_branch.clone()).as_deref(), Some("feature"));
}

#[test]
fn command_palette_runs_a_command() {
    let repo = repository();
    std::fs::write(repo.path().join("extra.txt"), "new\n").unwrap();
    let mut harness = open(repo.path());
    loaded(&mut harness);

    harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::P);
    wait(&mut harness, "the palette", |h| h.state().palette.is_some());
    harness.event(egui::Event::Text("stage all".into()));
    harness.step();
    harness.key_press(Key::Enter);
    idle(&mut harness);
    assert_eq!(git(repo.path(), &["diff", "--cached", "--name-only"]), "extra.txt");
}

#[test]
fn every_repository_tool_opens_from_the_menu() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    let name = repo.path().file_name().unwrap().to_string_lossy().into_owned();
    let items = [
        ("Repository settings", "Repository Settings"),
        ("Stashes", "Stashes"),
        ("Search history", "Search commits"),
        ("Search file contents", "Find in working files"),
        ("Recover lost work", "Recover Lost Work"),
        ("Clean up branches", "Clean Up Branches"),
        ("Bisect", "Bisect"),
        ("Worktrees", "Worktrees"),
        ("Submodules", "Submodules"),
        ("GitFlow", "GitFlow"),
        ("Git LFS", "Git LFS"),
    ];
    for (item, title) in items {
        let before = harness.state().tools.len();
        let picker = format!("{name} {}", egui_phosphor::regular::CARET_DOWN);
        harness.get_all_by_label(&picker).next().expect("repository picker").click();
        settle(&mut harness);
        harness.get_by_label_contains(item).click();
        wait(&mut harness, item, |h| h.state().tools.len() > before || h.state().tools.iter().any(|t| t.title() == title));
        assert!(
            harness.state().tools.iter().any(|t| t.title() == title),
            "{item} opened {:?}",
            harness.state().tools.iter().map(|t| t.title()).collect::<Vec<_>>()
        );
        // Let the window load and draw a few frames without panicking, then close it.
        for _ in 0..5 {
            harness.step();
            std::thread::sleep(Duration::from_millis(20));
        }
        harness.state_mut().tools.clear();
        settle(&mut harness);
    }
}

#[test]
fn stash_window_saves_selected_changes() {
    let repo = repository();
    std::fs::write(repo.path().join("notes.txt"), "changed\n").unwrap();
    let mut harness = open(repo.path());
    loaded(&mut harness);

    harness.get_by_label_contains("Manage stashes").click();
    wait(&mut harness, "the stash window", |h| h.state().tools.iter().any(|t| t.id() == "stashes"));
    harness.get_by_label_contains("Stash changes").click();
    idle(&mut harness);
    assert!(git(repo.path(), &["stash", "list"]).contains("notes") || !git(repo.path(), &["stash", "list"]).is_empty());
    assert_eq!(std::fs::read_to_string(repo.path().join("notes.txt")).unwrap(), "first\nsecond\n");
}

#[cfg(unix)]
#[test]
fn terminal_panel_opens_and_hides() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);

    settle(&mut harness);
    harness.get_by_label("Terminal").click();
    harness.step();
    assert!(harness.state().terminal.is_some());
    harness.key_press_modifiers(Modifiers::CTRL, Key::Backtick);
    harness.step();
    assert!(harness.state().terminal.is_none());
}

#[test]
fn revert_and_edit_message_from_the_graph() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);

    harness.get_by_label("Add a second line").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Revert").click();
    wait(&mut harness, "the revert confirmation", |h| h.state().dialog.is_some());
    harness.get_by_label("Revert").click();
    idle(&mut harness);
    assert!(git(repo.path(), &["log", "-1", "--format=%s"]).starts_with("Revert"));
    assert_eq!(std::fs::read_to_string(repo.path().join("notes.txt")).unwrap(), "first\n");

    // The new HEAD's message can be edited from its menu.
    let subject = git(repo.path(), &["log", "-1", "--format=%s"]);
    harness.get_by_label(&subject).click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Edit message").click();
    wait(&mut harness, "the message editor", |h| matches!(h.state().dialog, Some(crate::ui::dialogs::Dialog::EditMessage { .. })));
    if let Some(crate::ui::dialogs::Dialog::EditMessage { message, .. }) = harness.state_mut().dialog.as_mut() {
        *message = "Take the second line back out".into();
    }
    harness.step();
    harness.get_by_label("Save message").click();
    idle(&mut harness);
    assert_eq!(git(repo.path(), &["log", "-1", "--format=%s"]), "Take the second line back out");
}

#[test]
fn create_a_branch_from_the_toolbar() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    settle(&mut harness);

    harness.get_by_role_and_label(egui::accesskit::Role::Button, "Branch").click();
    wait(&mut harness, "the new branch dialog", |h| h.state().dialog.is_some());
    harness.event(egui::Event::Text("topic/ui-test".into()));
    harness.step();
    harness.get_by_label("OK").click();
    idle(&mut harness);
    assert_eq!(git(repo.path(), &["branch", "--show-current"]), "topic/ui-test");
}

#[test]
fn resolve_a_merge_conflict_in_the_editor() {
    let repo = repository();
    let path = repo.path();
    git(path, &["switch", "-q", "feature"]);
    std::fs::write(path.join("notes.txt"), "first\nfrom feature\n").unwrap();
    git(path, &["commit", "-qam", "Change notes on feature"]);
    git(path, &["switch", "-q", "main"]);
    let mut harness = open(path);
    loaded(&mut harness);

    // Merge from the branch menu; Git stops with a conflict.
    harness.get_by_label("feature").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Merge into main").click();
    wait(&mut harness, "the merge confirmation", |h| h.state().dialog.is_some());
    harness.get_by_label("Merge").click();
    idle(&mut harness);
    assert_eq!(harness.state().snapshot().and_then(|s| s.operation), Some(nicegit_core::Operation::Merge));

    // Open the conflict editor from the banner and keep the current version.
    harness.get_by_label_contains("  notes.txt").click();
    wait(&mut harness, "the conflict editor", |h| h.state().tools.iter().any(|t| t.id().starts_with("conflict:")));
    wait(&mut harness, "the conflict to load", |h| h.query_by_label_contains("Take current").is_some());
    // Wait until the window has finished sizing itself, so the button stays where it was found.
    let mut last = harness.get_by_label_contains("Take current").rect();
    for _ in 0..30 {
        harness.step();
        let now = harness.get_by_label_contains("Take current").rect();
        if now == last {
            break;
        }
        last = now;
    }
    let take = harness.get_by_label_contains("Take current").rect();
    assert!(take.max.y <= 900.0, "the conflict window's buttons fit on screen: {take:?}");
    harness.get_by_label_contains("Take current").click();
    settle(&mut harness);

    harness.get_by_label("Take version").click();
    idle(&mut harness);
    assert_eq!(std::fs::read_to_string(path.join("notes.txt")).unwrap(), "first\nsecond\n");
    assert!(git(path, &["diff", "--name-only", "--diff-filter=U"]).is_empty());

    // Continue the merge from the banner.
    harness.state_mut().tools.clear();
    settle(&mut harness);
    harness.get_by_label("Continue").click();
    idle(&mut harness);
    assert!(harness.state().snapshot().is_some_and(|s| s.operation.is_none()));
    assert_eq!(git(path, &["rev-list", "--parents", "-n", "1", "HEAD"]).split_whitespace().count(), 3, "a merge commit");
}

#[test]
fn restore_a_file_from_the_commit_inspector() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);

    harness.get_by_label("Add a second line").click();
    wait(&mut harness, "the commit's files", |h| h.state().repo().is_some_and(|r| !r.commit_files.is_empty()));
    settle(&mut harness);
    harness.get_by_label("notes.txt").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Restore version before this commit").click();
    wait(&mut harness, "the restore confirmation", |h| h.state().dialog.is_some());
    harness.get_by_label("Restore").click();
    idle(&mut harness);
    assert_eq!(std::fs::read_to_string(repo.path().join("notes.txt")).unwrap(), "first\n");
    assert_eq!(git(repo.path(), &["diff", "--cached", "--name-only"]), "notes.txt");
}

#[test]
fn arrow_keys_move_through_the_graph_and_escape_clears() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    let head = git(repo.path(), &["rev-parse", "HEAD"]);
    let parent = git(repo.path(), &["rev-parse", "HEAD~1"]);
    let selected = |h: &Harness<'static, NiceGitApp>| match h.state().repo().map(|r| r.selection.clone()) {
        Some(crate::app::Selection::Commit { hash, .. }) => Some(hash),
        _ => None,
    };

    harness.key_press(Key::ArrowDown);
    harness.step();
    assert_eq!(selected(&harness), Some(head));
    harness.key_press(Key::ArrowDown);
    harness.step();
    assert_eq!(selected(&harness), Some(parent));
    harness.key_press(Key::Escape);
    harness.step();
    assert_eq!(selected(&harness), None);
}

/// The smallest window the app allows still lays out every panel; debug builds check widths.
#[test]
fn smallest_window_lays_out_every_panel() {
    let repo = repository();
    std::fs::write(repo.path().join("notes.txt"), "changed\n").unwrap();
    let path = repo.path().to_path_buf();
    let mut harness = Harness::builder().with_size(egui::vec2(900.0, 560.0)).build_eframe(move |cc| NiceGitApp::new(cc, Some(path)));
    loaded(&mut harness);
    harness.state_mut().select_working_tree();
    settle(&mut harness);
    let head = git(repo.path(), &["rev-parse", "HEAD"]);
    harness.state_mut().select_commit(head);
    settle(&mut harness);
    harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::P);
    settle(&mut harness);
    harness.key_press(Key::Escape);
    harness.state_mut().show_settings = true;
    settle(&mut harness);
    let ctx = harness.ctx.clone();
    harness.state_mut().toggle_terminal(&ctx);
    settle(&mut harness);
}
