//! Interface tests for browsing every file in a commit's tree: the inspector's All files view, and
//! the read-only file view it opens. Built on the helpers in `ui_tests`.

#![allow(unused_imports)]

use std::path::Path;

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use egui_phosphor::regular as icon;

use crate::app::{NiceGitApp, Selection};
use crate::ui_tests::*;

type App = Harness<'static, NiceGitApp>;

/// Whether any widget's label contains `text`.
fn shows(harness: &App, text: &str) -> bool {
    harness.query_all_by_label_contains(text).next().is_some()
}

/// The folder row's label in the All files view, as the tree draws it.
fn folder_row(name: &str) -> String {
    format!("{}  {name}", icon::FOLDER)
}

/// The repository with a `docs` folder added in "Add the guide", then edited in "Edit the guide".
/// The guide reads differently at the newer commit, so the file view must show the older one.
fn repository_with_guide() -> tempfile::TempDir {
    let repo = repository();
    let path = repo.path();
    std::fs::create_dir(path.join("docs")).unwrap();
    std::fs::write(path.join("docs").join("guide.md"), "guide\n").unwrap();
    git(path, &["add", "--all"]);
    git(path, &["commit", "-q", "-m", "Add the guide"]);
    std::fs::write(path.join("docs").join("guide.md"), "edited\n").unwrap();
    git(path, &["commit", "-qam", "Edit the guide"]);
    repo
}

/// Whether the selection is this commit's file view for `path`.
fn viewing(harness: &App, path: &str) -> bool {
    matches!(
        harness.state().repo().map(|r| r.selection.clone()),
        Some(Selection::CommitFile { path: shown, .. }) if shown == path
    )
}

/// Selects the "Add the guide" commit and waits for its files, then switches to All files.
fn open_all_files_of_the_guide_commit(harness: &mut App) {
    harness.get_by_label("Add the guide").click();
    wait(harness, "the commit's files", |h| h.state().repo().is_some_and(|r| !r.commit_files.is_empty()));
    settle(harness);
    harness.get_by_label("All files").click();
    wait(harness, "the commit's tree", |h| shows(h, &folder_row("docs")));
    settle(harness);
}

#[test]
fn all_files_lists_the_tree_and_a_file_opens_as_it_was_at_its_commit() {
    let repo = repository_with_guide();
    let mut harness = open(repo.path());
    loaded(&mut harness);

    open_all_files_of_the_guide_commit(&mut harness);
    assert!(harness.query_by_label("guide.md").is_none(), "a folder's files are hidden until it opens");

    harness.get_by_label(&folder_row("docs")).click();
    wait(&mut harness, "the folder's files", |h| h.query_by_label("guide.md").is_some());
    settle(&mut harness);
    harness.get_by_label("guide.md").click();
    wait(&mut harness, "the file view", |h| viewing(h, "docs/guide.md"));
    wait(&mut harness, "the file's first line", |h| h.query_by_label("guide").is_some());
    assert!(!shows(&harness, "edited"), "the file shows its version at the commit, not the newer one");
}

#[test]
fn switching_back_to_changed_files_restores_the_change_list() {
    let repo = repository_with_guide();
    let mut harness = open(repo.path());
    loaded(&mut harness);

    open_all_files_of_the_guide_commit(&mut harness);
    assert!(harness.query_by_label(&format!("{}  All changes", icon::FILES)).is_none(), "All files hides the commit's own diff row");

    harness.get_by_label_contains("Changed files").click();
    wait(&mut harness, "the change list", |h| shows(h, "All changes"));
    assert!(harness.query_by_label(&folder_row("docs")).is_none(), "the tree is hidden in Changed files");
    assert!(harness.query_by_label("docs/guide.md").is_some(), "the commit's changed file is listed");
}

#[test]
fn a_large_file_is_read_only_after_open_anyway() {
    let repo = repository();
    let path = repo.path();
    // Just over the two-megabyte limit, in lines, so the view has many rows to draw.
    std::fs::write(path.join("big.txt"), "line\n".repeat(430_000)).unwrap();
    git(path, &["add", "--all"]);
    git(path, &["commit", "-q", "-m", "Add a large file"]);
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label("Add a large file").click();
    wait(&mut harness, "the commit's files", |h| h.state().repo().is_some_and(|r| !r.commit_files.is_empty()));
    settle(&mut harness);
    harness.get_by_label("All files").click();
    // The Changed files row also reads "big.txt", so the wait needs the tree itself to have arrived.
    // The commit's large data loads in the background, so the app is busy until it is read; the
    // row is clicked only once it is idle. The tree can still be drawn again while the file list
    // settles, which drops a click, so the row is clicked until the file starts to open.
    wait(&mut harness, "the big file in the tree", |h| !shows(h, "Listing files") && h.query_by_label("big.txt").is_some());
    idle(&mut harness);
    let start = std::time::Instant::now();
    loop {
        wait(&mut harness, "the big file in the tree", |h| !shows(h, "Listing files") && h.query_by_label("big.txt").is_some());
        if let Some(row) = harness.query_by_label("big.txt") {
            row.click();
        }
        harness.run_steps(5);
        if shows(&harness, "Large files are not shown") || shows(&harness, "Reading file") {
            break;
        }
        assert!(start.elapsed() < test_timeout(), "the big file never opened");
    }
    wait(&mut harness, "the size note", |h| shows(h, "Large files are not shown"));
    assert!(!shows(&harness, "Reading file"), "nothing is still loading");

    harness.get_by_label("Open anyway").click();
    wait(&mut harness, "the file's lines", |h| h.query_all_by_label("line").next().is_some());
    assert!(!shows(&harness, "Open anyway"), "the note goes once the file is open");
}

#[test]
fn save_a_copy_writes_the_file_as_it_was_at_the_commit() {
    let repo = repository_with_guide();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    open_all_files_of_the_guide_commit(&mut harness);
    harness.get_by_label(&folder_row("docs")).click();
    wait(&mut harness, "the folder's files", |h| h.query_by_label("guide.md").is_some());
    settle(&mut harness);

    let saved = temp_dir();
    let destination = saved.path().join("saved-guide.md");
    crate::file_dialog::answer_next(&destination);
    harness.get_by_label("guide.md").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Save a copy").click();
    wait(&mut harness, "the saved copy", |_| destination.exists());
    assert_eq!(std::fs::read_to_string(&destination).unwrap(), "guide\n", "the copy is the version at the commit");
}
