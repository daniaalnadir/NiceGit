//! Interface tests for the Words diff layout: changed words of a file are merged into one line,
//! Unified brings the whole lines back, and lines are not offered for staging while Words shows.

use egui_kittest::kittest::Queryable;

use crate::ui_tests::*;
use crate::ui_tests_more::wait_for_stageable_lines;

#[test]
fn words_layout_merges_changed_words_and_unified_brings_lines_back() {
    let repo = repository();
    let path = repo.path();
    std::fs::write(path.join("prose.txt"), "The quick brown fox jumps over the lazy dog.\n").unwrap();
    git(path, &["add", "--", "prose.txt"]);
    git(path, &["commit", "-q", "-m", "Add prose"]);
    std::fs::write(path.join("prose.txt"), "The slow brown fox jumps over the lazy cat.\n").unwrap();

    let mut harness = open(path);
    loaded(&mut harness);
    harness.get_by_label("prose.txt").click();
    wait_for_stageable_lines(&mut harness);

    harness.get_by_role_and_label(egui::accesskit::Role::Button, "Words").click();
    settle(&mut harness);
    // The changed words of the line sit in one row, each marked as removed or added.
    let merged = "The [removed: quick][added: slow] brown fox jumps over the lazy [removed: dog][added: cat].";
    wait(&mut harness, "the merged line", |h| h.query_by_label(merged).is_some());
    assert!(harness.state().settings.word_diff, "the Words choice is saved");
    // Lines of a working change cannot be staged one by one in this layout, and the view says so.
    assert!(harness.state().repo().is_some_and(|r| !r.diff_options.selectable), "no line selection in Words");
    assert!(has_label(&harness, "Switch to Unified to stage single lines."), "the staging note shows");

    harness.get_by_role_and_label(egui::accesskit::Role::Button, "Unified").click();
    wait(&mut harness, "the whole lines again", |h| h.query_by_label(merged).is_none());
    assert!(!harness.state().settings.word_diff, "Unified turns Words off");
    assert!(!has_label(&harness, "Switch to Unified to stage single lines."), "no note in Unified");
    // The file itself is untouched by changing the layout.
    assert_eq!(std::fs::read_to_string(path.join("prose.txt")).unwrap(), "The slow brown fox jumps over the lazy cat.\n");
    assert_eq!(git(path, &["status", "--porcelain"]), "M prose.txt");
}
