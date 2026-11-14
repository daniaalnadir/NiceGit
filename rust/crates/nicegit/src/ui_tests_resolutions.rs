//! Interface tests for remembered conflict resolutions (Git's rerere): a resolution Git reuses is
//! marked in the Changes panel and the conflict editor, and the setting is changed from the editor
//! and the command palette. They use the helpers in `ui_tests`.

use std::path::Path;
use std::process::Command;

use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;

use crate::tools::conflict::remember_notice;
use crate::ui_tests::*;

/// Runs Git in `dir` and expects it to stop, as a merge does when it meets a conflict.
fn git_stops(dir: &Path, args: &[&str]) {
    let status =
        Command::new("git").args(args).current_dir(dir).env_remove("GIT_DIR").env_remove("GIT_WORK_TREE").status().expect("run git");
    assert!(!status.success(), "git {args:?} should stop at a conflict");
}

/// The shared repository with rerere on, where merging `feature` conflicts in notes.txt. A first
/// merge is resolved by hand, which records the resolution. Main is then reset to before it.
fn repository_with_recorded_resolution() -> tempfile::TempDir {
    let repo = repository();
    let path = repo.path();
    git(path, &["config", "rerere.enabled", "true"]);
    git(path, &["switch", "-q", "feature"]);
    std::fs::write(path.join("notes.txt"), "first\nfrom feature\n").unwrap();
    git(path, &["commit", "-qam", "Change notes on feature"]);
    git(path, &["switch", "-q", "main"]);
    let before = git(path, &["rev-parse", "HEAD"]);
    git_stops(path, &["merge", "--no-edit", "feature"]);
    std::fs::write(path.join("notes.txt"), "first\nsecond\nfrom feature\n").unwrap();
    git(path, &["add", "notes.txt"]);
    git(path, &["commit", "-q", "--no-edit"]);
    git(path, &["reset", "-q", "--hard", &before]);
    repo
}

#[test]
fn a_recorded_resolution_is_reused_marked_and_can_be_switched_off_in_the_editor() {
    let repo = repository_with_recorded_resolution();
    let path = repo.path();
    let mut harness = open(path);
    loaded(&mut harness);

    // Merge again from the app. Git reuses the recorded resolution: the file has no markers, but it
    // stays unmerged until it is staged.
    harness.get_by_label("feature").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Merge into main").click();
    wait(&mut harness, "the merge confirmation", |h| h.state().dialog.is_some());
    harness.get_by_label("Merge").click();
    idle(&mut harness);
    assert_eq!(harness.state().snapshot().and_then(|s| s.operation), Some(nicegit_core::Operation::Merge));
    assert_eq!(std::fs::read_to_string(path.join("notes.txt")).unwrap(), "first\nsecond\nfrom feature\n");
    assert_eq!(git(path, &["diff", "--name-only", "--diff-filter=U"]), "notes.txt");

    let notice = harness.state().notice.as_ref().map(|n| n.text.clone()).expect("a message");
    assert!(notice.ends_with("Git reused 1 recorded resolution. Check it before continuing."), "{notice}");
    wait(&mut harness, "the reused file to be marked in the Changes panel", |h| has_label(h, "Resolved from memory"));

    // The editor explains the reuse, and its checkbox reflects the repository's setting.
    harness.get_by_label_contains("  notes.txt").click();
    wait(&mut harness, "the conflict editor", |h| h.state().tools.iter().any(|t| t.id().starts_with("conflict:")));
    wait(&mut harness, "the reused resolution banner", |h| has_label(h, "Git reused a resolution you recorded earlier"));
    wait(&mut harness, "the memory checkbox", |h| h.query_by_label("Remember how I resolve conflicts").is_some());
    assert_eq!(git(path, &["config", "--local", "--get", "rerere.enabled"]), "true");

    // Turning the checkbox off writes the setting, and says so.
    harness.get_by_label("Remember how I resolve conflicts").click();
    idle(&mut harness);
    assert_eq!(git(path, &["config", "--local", "--get", "rerere.enabled"]), "false");
    let notice = harness.state().notice.as_ref().map(|n| n.text.clone()).expect("a message");
    assert_eq!(notice, remember_notice(false));
}

#[test]
fn the_palette_turns_conflict_memory_off() {
    let repo = repository();
    let path = repo.path();
    git(path, &["config", "rerere.enabled", "true"]);
    let mut harness = open(path);
    loaded(&mut harness);

    harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::P);
    wait(&mut harness, "the palette", |h| h.state().palette.is_some());
    harness.event(egui::Event::Text("remember conflict resolutions turn off".into()));
    harness.step();
    wait(&mut harness, "the palette entry that turns it off", |h| has_label(h, "Remember conflict resolutions: turn off"));
    harness.key_press(Key::Enter);
    idle(&mut harness);

    assert_eq!(git(path, &["config", "--local", "--get", "rerere.enabled"]), "false");
    let notice = harness.state().notice.as_ref().map(|n| n.text.clone()).expect("a message");
    assert_eq!(notice, remember_notice(false));
}
