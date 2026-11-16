//! Interface tests for dragging commits from the history graph onto branches to cherry-pick them.

use std::path::Path;
use std::process::Command;

use egui::Modifiers;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;

use crate::app::{NiceGitApp, Selection};
use crate::ui_tests::*;

type App = Harness<'static, NiceGitApp>;

/// Whether any widget's label contains `text`.
fn shows(harness: &App, text: &str) -> bool {
    harness.query_all_by_label_contains(text).next().is_some()
}

/// Writes a file and commits it with the given committer and author date, so the order commits
/// are applied in is decided by their dates rather than by the second they were made in.
fn commit_at(dir: &Path, file: &str, contents: &str, message: &str, date: &str) {
    std::fs::write(dir.join(file), contents).unwrap();
    git(dir, &["add", "--", file]);
    let output = Command::new("git")
        .args(["commit", "-q", "-m", message])
        .env("GIT_COMMITTER_DATE", date)
        .env("GIT_AUTHOR_DATE", date)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .output()
        .expect("run git");
    assert!(output.status.success(), "commit failed: {}", String::from_utf8_lossy(&output.stderr));
}

/// The test repository with `feature` carrying one commit of its own, and the checkout on main.
fn repository_with_feature_commit() -> tempfile::TempDir {
    let repo = repository();
    let path = repo.path();
    git(path, &["switch", "-q", "feature"]);
    commit_at(path, "side.txt", "from feature\n", "Work on the feature", "2001-01-01T10:00:00+0000");
    git(path, &["switch", "-q", "main"]);
    repo
}

/// Drags the row labelled `from` onto the one labelled `to`, as a pointer does: pressed, moved in
/// steps past the drag threshold, and released over the target.
fn drag_onto(harness: &mut App, from: &str, to: &str) {
    let start = harness.get_all_by_label(from).next().expect("the dragged row").rect().center();
    let target = harness.get_all_by_label(to).next().expect("the drop target").rect().center();
    harness.hover_at(start);
    harness.step();
    harness.drag_at(start);
    harness.step();
    for step in 1..=6 {
        harness.hover_at(start + (target - start) * (step as f32 / 6.0));
        harness.step();
    }
    harness.drop_at(target);
    harness.step();
}

/// The text of the notice on screen, if one is shown.
fn notice_text(harness: &App) -> Option<String> {
    harness.state().notice.as_ref().map(|notice| notice.text.clone())
}

#[test]
fn drag_a_commit_onto_the_current_branch_to_cherry_pick_it() {
    let repo = repository_with_feature_commit();
    let path = repo.path();
    let mut harness = open(path);
    loaded(&mut harness);

    drag_onto(&mut harness, "Work on the feature", "main");
    wait(&mut harness, "the cherry-pick confirmation", |h| shows(h, "Cherry-pick this commit?"));
    settle(&mut harness);
    harness.get_all_by_label("Cherry-pick").last().expect("the confirm button").click();
    idle(&mut harness);

    assert_eq!(git(path, &["log", "-1", "--format=%s"]), "Work on the feature", "the commit is applied to main");
    assert_eq!(std::fs::read_to_string(path.join("side.txt")).unwrap(), "from feature\n");
    assert_eq!(git(path, &["branch", "--show-current"]), "main", "the checkout stays on main");
}

#[test]
fn dropping_a_commit_on_the_current_branch_label_in_the_graph_asks_too() {
    let repo = repository_with_feature_commit();
    let path = repo.path();
    let mut harness = open(path);
    loaded(&mut harness);

    drag_onto(&mut harness, "Work on the feature", "Branch label main");
    wait(&mut harness, "the cherry-pick confirmation", |h| shows(h, "Cherry-pick this commit?"));
    settle(&mut harness);
    harness.get_all_by_label("Cherry-pick").last().expect("the confirm button").click();
    idle(&mut harness);

    assert_eq!(std::fs::read_to_string(path.join("side.txt")).unwrap(), "from feature\n");
}

#[test]
fn command_click_two_commits_then_drag_one_cherry_picks_both_oldest_first() {
    let repo = repository();
    let path = repo.path();
    git(path, &["switch", "-q", "feature"]);
    commit_at(path, "one.txt", "one\n", "Pick one", "2001-01-01T10:00:00+0000");
    commit_at(path, "two.txt", "two\n", "Pick two", "2001-01-01T10:01:00+0000");
    git(path, &["switch", "-q", "main"]);
    let mut harness = open(path);
    loaded(&mut harness);

    // Marked newest first, so the order of the clicks does not decide the order applied.
    harness.get_by_label("Pick two").click_modifiers(Modifiers::COMMAND);
    harness.step();
    harness.get_by_label("Pick one").click_modifiers(Modifiers::COMMAND);
    harness.step();
    wait(&mut harness, "the marked commits bar", |h| shows(h, "2 commits selected"));

    drag_onto(&mut harness, "Pick two", "main");
    wait(&mut harness, "the two-commit confirmation", |h| shows(h, "Cherry-pick 2 commits?"));
    settle(&mut harness);
    harness.get_all_by_label("Cherry-pick").last().expect("the confirm button").click();
    idle(&mut harness);

    assert_eq!(git(path, &["log", "--format=%s", "-2"]), "Pick two\nPick one", "applied oldest first");
    assert_eq!(std::fs::read_to_string(path.join("two.txt")).unwrap(), "two\n");
}

#[test]
fn dropping_a_commit_the_branch_already_has_says_so_and_asks_nothing() {
    let repo = repository();
    let path = repo.path();
    let mut harness = open(path);
    loaded(&mut harness);

    // "Start the notes" is the first commit of main, so main already has it.
    drag_onto(&mut harness, "Start the notes", "main");
    wait(&mut harness, "the notice", |h| notice_text(h).is_some_and(|text| text.contains("already on main")));
    assert!(harness.state().dialog.is_none(), "no cherry-pick confirmation opens");
    assert_eq!(git(path, &["rev-list", "--count", "HEAD"]), "2", "main is unchanged");
}

#[test]
fn dropping_on_another_branch_checks_it_out_then_asks_to_cherry_pick() {
    let repo = repository_with_feature_commit();
    let path = repo.path();
    // A branch at the first commit, which main has moved past, so the cherry-pick is not already there.
    git(path, &["branch", "release", "main~1"]);
    let mut harness = open(path);
    loaded(&mut harness);

    drag_onto(&mut harness, "Work on the feature", "release");
    wait(&mut harness, "the checkout confirmation", |h| shows(h, "Check out release and cherry-pick 1 commit?"));
    settle(&mut harness);
    harness.get_all_by_label("Check out").last().expect("the confirm button").click();
    wait(&mut harness, "the cherry-pick confirmation on release", |h| shows(h, "Cherry-pick this commit?"));
    settle(&mut harness);
    assert_eq!(git(path, &["branch", "--show-current"]), "release", "the checkout happened before the cherry-pick is asked");
    harness.get_all_by_label("Cherry-pick").last().expect("the confirm button").click();
    idle(&mut harness);

    assert_eq!(git(path, &["branch", "--show-current"]), "release");
    assert_eq!(git(path, &["log", "-1", "--format=%s"]), "Work on the feature", "release has the commit");
    assert_eq!(std::fs::read_to_string(path.join("side.txt")).unwrap(), "from feature\n");
}

#[test]
fn a_plain_click_on_a_commit_selects_it_and_starts_no_drag() {
    let repo = repository_with_feature_commit();
    let path = repo.path();
    let mut harness = open(path);
    loaded(&mut harness);

    let tip = git(path, &["rev-parse", "feature"]);
    harness.get_by_label("Work on the feature").click();
    wait(&mut harness, "the commit to be selected", |h| {
        h.state().repo().is_some_and(|r| matches!(&r.selection, Selection::Commit { hash, .. } if *hash == tip))
    });
    assert!(harness.state().dialog.is_none(), "a click opens no confirmation");
    assert!(harness.state().repo().is_some_and(|r| r.marked.is_empty()), "a click marks nothing");
    assert_eq!(git(path, &["branch", "--show-current"]), "main", "a click changes nothing");
}
