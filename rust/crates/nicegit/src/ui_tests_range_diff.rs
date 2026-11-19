//! Interface tests for comparing versions of a branch: the palette command, the branch menu, and
//! the Force push confirmation's link. They click through the window and check what it shows.
//! They use the helpers in `ui_tests`.

use std::path::Path;

use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;

use crate::app::NiceGitApp;
use crate::ui_tests::*;
use crate::ui_tests_more::publish_to_bare;

/// Ten numbered lines, with line five set to `five`. Git pairs a small edit to a long file as a
/// change, where a rewrite of a one-line file reads as a removal and an addition.
fn ten_lines(five: &str) -> String {
    (1..=10).map(|n| if n == 5 { format!("{five}\n") } else { format!("line {n}\n") }).collect()
}

/// Writes `contents` to `file` and commits it with `message`.
fn commit_text(dir: &Path, file: &str, contents: &str, message: &str) {
    std::fs::write(dir.join(file), contents).unwrap();
    git(dir, &["add", "--", file]);
    git(dir, &["commit", "-q", "-m", message]);
}

/// Writes `contents` to `file` and folds it into the checkout's last commit.
fn amend_text(dir: &Path, file: &str, contents: &str) {
    std::fs::write(dir.join(file), contents).unwrap();
    git(dir, &["commit", "-q", "--amend", "-a", "--no-edit"]);
}

/// A repository on main whose last commit adds a ten-line file, then amends one of its lines.
fn amended_main() -> tempfile::TempDir {
    let repo = repository();
    let path = repo.path();
    commit_text(path, "lines.txt", &ten_lines("five"), "Add ten lines");
    amend_text(path, "lines.txt", &ten_lines("five, amended"));
    repo
}

/// Clicks Compare until the comparison's summary shows. Compare stays disabled while the
/// window reads the revisions, so the clicks begin once they are filled in.
fn compare_until(harness: &mut Harness<'static, NiceGitApp>, summary: &str) {
    click_until(harness, "the comparison summary", |h| h.get_by_role_and_label(Role::Button, "Compare").click(), |h| has_label(h, summary));
}

#[test]
fn the_palette_compares_the_checkouts_earlier_version() {
    let repo = amended_main();
    let mut harness = open(repo.path());
    loaded(&mut harness);

    harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::P);
    wait(&mut harness, "the palette", |h| h.state().palette.is_some());
    harness.event(egui::Event::Text("compare branch versions".into()));
    wait(&mut harness, "the compare command", |h| has_label(h, "Compare branch versions"));
    harness.key_press(Key::Enter);
    wait(&mut harness, "the compare button", |h| h.query_by_role_and_label(Role::Button, "Compare").is_some());

    compare_until(&mut harness, "1 changed");
    assert!(has_label(&harness, "0 unchanged, 1 changed, 0 removed, 0 added"));

    // The changed pair opens its comparison of the patches, with the new line marked added.
    harness.get_by_label("!").click();
    wait(&mut harness, "the comparison of the patches", |h| has_label(h, "five, amended"));
}

#[test]
fn the_branch_menu_compares_with_the_previous_version() {
    let repo = amended_main();
    let mut harness = open(repo.path());
    loaded(&mut harness);

    harness.get_all_by_label("main").next().expect("the main branch row").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Compare with previous version").click();
    wait(&mut harness, "the compare button", |h| h.query_by_role_and_label(Role::Button, "Compare").is_some());

    compare_until(&mut harness, "1 changed");
}

#[test]
fn force_push_confirmation_compares_with_the_remote_tip() {
    let repo = repository();
    let path = repo.path();
    let remote = publish_to_bare(path);
    commit_text(path, "lines.txt", &ten_lines("five"), "Add ten lines");
    git(path, &["push", "-q", "origin", "main"]);
    amend_text(path, "lines.txt", &ten_lines("five, amended"));
    let before = git(remote.path(), &["rev-parse", "main"]);

    let mut harness = open(path);
    loaded(&mut harness);
    harness.get_all_by_label("main").next().expect("the main branch row").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Force push").click();
    wait(&mut harness, "the force push confirmation", |h| h.state().dialog.is_some());
    settle(&mut harness);

    harness.get_by_label("Compare with remote").click();
    settle(&mut harness);
    assert!(harness.state().dialog.is_some(), "the link keeps the confirmation open");
    wait(&mut harness, "the compare button", |h| h.query_by_role_and_label(Role::Button, "Compare").is_some());
    compare_until(&mut harness, "1 changed");

    // Comparing pushes nothing.
    assert_eq!(git(remote.path(), &["rev-parse", "main"]), before, "comparing pushes nothing");
}
