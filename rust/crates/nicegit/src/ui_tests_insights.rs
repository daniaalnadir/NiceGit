//! Interface tests for the Insights window: it opens from the Repository menu, reads the history,
//! switches periods, and opens a changed file's history. They use the helpers in `ui_tests`.

use std::time::{SystemTime, UNIX_EPOCH};

use egui_kittest::kittest::Queryable;

use crate::ui_tests::*;

const DAY: i64 = 24 * 60 * 60;

/// A repository with two authors, all within the last week so the default one-year period shows
/// them: Ann Lee commits three times to `src/lib.rs`, and Bob Stone once to `notes.txt`, which is
/// the newest commit so the inspector does not list `src/lib.rs` as well.
fn two_authors() -> tempfile::TempDir {
    let dir = temp_dir();
    let path = dir.path();
    git(path, &["init", "-q", "-b", "main"]);
    for (key, value) in
        [("user.name", "Test"), ("user.email", "test@example.invalid"), ("commit.gpgsign", "false"), ("core.autocrlf", "false")]
    {
        git(path, &["config", key, value]);
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH).expect("clock").as_secs() as i64;
    let commit = |author: &str, email: &str, days_ago: i64, message: &str| {
        git(path, &["add", "-A"]);
        let author = format!("{author} <{email}>");
        let date = format!("@{} +0000", now - days_ago * DAY);
        git(path, &["commit", "-q", "--author", &author, "--date", &date, "-m", message]);
    };
    std::fs::create_dir_all(path.join("src")).unwrap();
    std::fs::write(path.join("src").join("lib.rs"), "one\ntwo\nthree\n").unwrap();
    commit("Ann Lee", "ann@example.com", 4, "Add the library");
    std::fs::write(path.join("src").join("lib.rs"), "one\n2\nthree\n").unwrap();
    commit("Ann Lee", "ann@example.com", 3, "Fix the second line");
    std::fs::write(path.join("src").join("lib.rs"), "one\n2\nthree\nfour\n").unwrap();
    commit("Ann Lee", "ann@example.com", 2, "Add a fourth line");
    std::fs::write(path.join("notes.txt"), "todo\n").unwrap();
    commit("Bob Stone", "bob@example.com", 1, "Add notes");
    dir
}

/// Whether a label is shown exactly as `label`. The commit graph also names authors, so a
/// contains-match would find those too.
fn shows(harness: &egui_kittest::Harness<'static, crate::app::NiceGitApp>, label: &str) -> bool {
    harness.query_all_by_label(label).next().is_some()
}

#[test]
fn insights_show_the_authors_and_the_most_changed_file_then_open_its_history() {
    let repo = two_authors();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    repository_menu(&mut harness, &folder_name(repo.path()), "Insights");
    wait(&mut harness, "the Insights summary", |h| shows(h, "MOST CHANGED FILES"));

    assert!(shows(&harness, "Ann Lee"), "the first author is listed");
    assert!(shows(&harness, "Bob Stone"), "the second author is listed");
    assert!(harness.get_all_by_label("3 commits").count() >= 1, "Ann Lee's three commits are counted");
    assert!(harness.get_all_by_label("1 commit").count() >= 1, "Bob Stone's one commit is counted");
    assert!(shows(&harness, "src/lib.rs"), "the most changed file is listed in full");
    assert!(shows(&harness, "COMMITS OVER TIME"), "the chart is shown");
    assert!(shows(&harness, "FOLDERS"), "the folders are shown");
    for period in ["30 days", "90 days", "1 year", "All time", "This branch", "All branches"] {
        assert!(harness.query_by_label(period).is_some(), "{period} is offered");
    }

    // The window redraws while it lays out, so a click can land on a stale frame; click until it opens.
    click_until(
        &mut harness,
        "the file history of src/lib.rs",
        |h| h.get_by_label("src/lib.rs").click(),
        |h| h.state().tools.iter().any(|tool| tool.title() == "File history: lib.rs"),
    );

    harness.get_by_label("All time").click();
    harness.run_steps(3);
    wait(&mut harness, "the history for all time", |h| shows(h, "MOST CHANGED FILES") && shows(h, "Ann Lee"));
    wait(&mut harness, "the history for all time", |h| shows(h, "MOST CHANGED FILES") && shows(h, "Ann Lee"));
    assert!(shows(&harness, "Bob Stone"), "the authors are still listed after switching to all time");
    assert!(shows(&harness, "src/lib.rs"), "the file is still listed after switching to all time");
    assert!(harness.state().tools.iter().any(|tool| tool.title() == "Insights"), "the Insights window stays open");
}
