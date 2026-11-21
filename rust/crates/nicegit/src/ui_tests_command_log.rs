//! Interface tests for the command log. They stage a file through the window, open the log from
//! the command palette, and read it back. They use the helpers in `ui_tests`.

use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;

use crate::ui_tests::*;

#[test]
fn command_log_lists_a_staging_command_and_shows_reads_on_request() {
    let repo = repository();
    std::fs::write(repo.path().join("notes.txt"), "first\n").unwrap();
    let mut harness = open(repo.path());
    loaded(&mut harness);

    harness.get_by_label("Stage All Changes").click();
    idle(&mut harness);
    assert_eq!(git(repo.path(), &["diff", "--cached", "--name-only"]), "notes.txt");

    harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::P);
    wait(&mut harness, "the palette", |h| h.state().palette.is_some());
    harness.event(egui::Event::Text("command log".into()));
    harness.step();
    harness.key_press(Key::Enter);
    wait(&mut harness, "the command log", |h| h.state().tools.iter().any(|tool| tool.title() == "Command Log"));
    settle(&mut harness);

    assert!(has_label(&harness, "git add"), "the staging command is listed");
    assert!(!has_label(&harness, "git status"), "reads are hidden until asked for");

    harness.get_by_label("Show reads").click();
    wait(&mut harness, "the status read", |h| has_label(h, "git status"));

    // A row opens its output and configuration when clicked.
    harness.get_all_by_label_contains("git add").next().expect("the staging row").click();
    settle(&mut harness);
    assert!(has_label(&harness, "No error output."), "the expanded row shows its output");
}
