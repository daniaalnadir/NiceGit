//! Interface tests for absorbing staged changes. They open the window from the command palette,
//! click through it, then check the repository Git holds. They use the helpers in `ui_tests`.

use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;
use nicegit_core::models::short;

use crate::ui_tests::*;

#[test]
fn absorb_a_staged_fix_into_the_commit_that_wrote_the_line() {
    let repo = repository();
    let path = repo.path();
    // The fix changes line 2, which the HEAD commit wrote.
    std::fs::write(path.join("notes.txt"), "first\nSECOND\n").unwrap();
    git(path, &["add", "notes.txt"]);
    let head = git(path, &["rev-parse", "HEAD"]);
    let count_before = git(path, &["rev-list", "--count", "HEAD"]);
    let label = format!("{}  Add a second line", short(&head));

    let mut harness = open(path);
    loaded(&mut harness);
    harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::P);
    wait(&mut harness, "the palette", |h| h.state().palette.is_some());
    harness.event(egui::Event::Text("absorb staged".into()));
    harness.step();
    harness.key_press(Key::Enter);
    wait(&mut harness, "the absorb window", |h| h.query_by_role_and_label(Role::CheckBox, &label).is_some());

    harness.get_by_role_and_label(Role::Button, "Absorb").click();
    idle(&mut harness);

    assert_eq!(git(path, &["rev-list", "--count", "HEAD"]), count_before);
    assert_eq!(git(path, &["show", "HEAD:notes.txt"]), "first\nSECOND");
    assert_eq!(git(path, &["status", "--porcelain"]), "");
    let notice = harness.state().notice.as_ref().map(|n| n.text.clone()).expect("a message");
    assert_eq!(notice, "Absorbed 1 change into 1 commit.");
}
