//! Interface tests for snapshots. A snapshot is taken through the app's own method rather than
//! by waiting for the timer, a file is deleted on disk, and the Snapshots window brings it back.

use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;

use crate::ui_tests::*;

/// The newest snapshot's ref, or nothing when the repository has none.
fn snapshot_refs(path: &std::path::Path) -> String {
    git(path, &["for-each-ref", "--format=%(refname)", "refs/nicegit/snapshots"])
}

#[test]
fn a_deleted_file_comes_back_from_a_snapshot() {
    let repo = repository();
    let path = repo.path();
    let edited = "first\nsecond\nthird\n";
    std::fs::write(path.join("notes.txt"), edited).unwrap();

    let mut harness = open(path);
    loaded(&mut harness);
    // The app takes the snapshot itself, as the timer would, without waiting for it.
    let ctx = harness.ctx.clone();
    harness.state_mut().start_snapshot(&ctx);
    wait(&mut harness, "the snapshot to be saved", |_| !snapshot_refs(path).is_empty());
    idle(&mut harness);

    // A file lost to an editor or a command.
    std::fs::remove_file(path.join("notes.txt")).unwrap();
    wait(&mut harness, "the deletion to show", |h| {
        h.state().repo().is_some_and(|repo| repo.snapshot.as_ref().is_some_and(|s| s.status.iter().any(|e| e.path == "notes.txt")))
    });
    idle(&mut harness);

    harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::P);
    wait(&mut harness, "the palette", |h| h.state().palette.is_some());
    harness.event(egui::Event::Text("Snapshots".into()));
    wait(&mut harness, "the Snapshots command", |h| h.query_all_by_label_contains("Snapshots").next().is_some());
    harness.key_press(Key::Enter);
    wait(&mut harness, "the snapshots window", |h| h.state().tools.iter().any(|t| t.id() == "snapshots"));
    wait(&mut harness, "the snapshot to be listed", |h| h.query_by_label_contains("1 file differs from now").is_some());
    wait(&mut harness, "the deleted file to be listed", |h| h.get_all_by_label("notes.txt").next().is_some());
    settle(&mut harness);

    // Tick the file, then restore it after confirming.
    wait(&mut harness, "the file's checkbox", |h| h.query_by_role_and_label(egui::accesskit::Role::CheckBox, "notes.txt").is_some());
    // Each click repeats until it shows, as a widget can still be settling under load.
    click_until(
        &mut harness,
        "the restore button to name the ticked file",
        |h| h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "notes.txt").click(),
        |h| h.query_by_label_contains("Restore 1 selected").is_some(),
    );
    click_until(
        &mut harness,
        "the restore confirmation",
        |h| h.get_by_label_contains("Restore 1 selected").click(),
        |h| h.query_by_label("Restore selected files").is_some(),
    );
    click_until(
        &mut harness,
        "the confirmation to close",
        |h| h.get_by_label("Restore").click(),
        |h| h.query_by_label("Restore selected files").is_none(),
    );
    idle(&mut harness);

    assert_eq!(std::fs::read_to_string(path.join("notes.txt")).unwrap(), edited, "the file is back with its snapshot content");
    assert_eq!(git(path, &["status", "--porcelain=v1"]).lines().count(), 1, "the file is modified against HEAD, and nothing else changed");
}
