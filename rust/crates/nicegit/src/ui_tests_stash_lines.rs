//! Interface tests for stashing the selected lines of an unstaged file: from a change's hover
//! button, and from the selection bar's Stash selected lines button, and popping one over other
//! edits offers Apply into my changes.

use std::path::Path;

use egui_kittest::kittest::Queryable;

use crate::ui_tests::*;
use crate::ui_tests_more::{hover_change_until, wait_for_stageable_lines};

/// Commits a file with the given contents, as the repository's first commit for the test.
fn commit_text(dir: &Path, file: &str, contents: &str, message: &str) {
    std::fs::write(dir.join(file), contents).unwrap();
    git(dir, &["add", "--", file]);
    git(dir, &["commit", "-q", "-m", message]);
}

/// Selects the shown diff's line with this text, as clicking it would.
fn select_line(harness: &mut egui_kittest::Harness<'static, crate::app::NiceGitApp>, text: &str) {
    let app = harness.state_mut();
    let active = app.active;
    let repo = &mut app.repos[active];
    let index = repo.diff.as_ref().unwrap().lines.iter().position(|line| line.text == text).expect("the line is shown");
    repo.diff_options.selected.insert(index);
}

/// The stash list as Git prints it, one entry per line.
fn stash_list(path: &Path) -> Vec<String> {
    git(path, &["stash", "list"]).lines().map(str::to_string).collect()
}

#[test]
fn a_change_can_be_stashed_from_its_hover_button_and_the_rest_stays() {
    let repo = repository();
    let path = repo.path();
    let original: String = (1..=30).map(|n| format!("line {n}\n")).collect();
    commit_text(path, "long.txt", &original, "Add a long file");
    let edited = original.replace("line 3\n", "three\n").replace("line 28\n", "twenty-eight\n");
    std::fs::write(path.join("long.txt"), &edited).unwrap();
    let mut harness = open(path);
    loaded(&mut harness);
    harness.get_by_label("long.txt").click();
    wait_for_stageable_lines(&mut harness);

    hover_change_until(&mut harness, "Stash this change");
    harness.get_by_label("Stash this change").click();
    wait(&mut harness, "the stash dialog", |h| h.state().dialog.is_some());
    // The message is prefilled with the default, which the stash keeps when it is left as it is.
    let prefilled = matches!(
        &harness.state().dialog,
        Some(crate::ui::dialogs::Dialog::Input { value, .. }) if value == "Selected lines from long.txt"
    );
    assert!(prefilled, "the dialog offers the default message");
    harness.get_all_by_label("Stash").last().expect("the confirm button").click();
    idle(&mut harness);

    let expected_working = original.replace("line 28\n", "twenty-eight\n");
    wait(&mut harness, "the first change stashed", |_| std::fs::read_to_string(path.join("long.txt")).unwrap() == expected_working);
    let entries = stash_list(path);
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert!(entries[0].ends_with("On main: Selected lines from long.txt"), "{entries:?}");
    wait(&mut harness, "the stash in the sidebar", |h| h.query_by_label_contains("Selected lines from long.txt").is_some());
    // The stash holds only the change that was chosen.
    let shown = git(path, &["stash", "show", "-p", "stash@{0}"]);
    assert!(shown.contains("+three") && !shown.contains("+twenty-eight"), "{shown}");
}

#[test]
fn selected_lines_can_be_stashed_from_the_selection_bar() {
    let repo = repository();
    let path = repo.path();
    commit_text(path, "notes.txt", "first\nsecond\nfourth\n", "Add notes");
    std::fs::write(path.join("notes.txt"), "first\nsecond\nthird\nfourth\n").unwrap();
    let mut harness = open(path);
    loaded(&mut harness);
    harness.get_by_label("notes.txt").click();
    wait_for_stageable_lines(&mut harness);

    select_line(&mut harness, "+third");
    wait(&mut harness, "the selection bar", |h| h.query_by_label_contains("Stash selected lines").is_some());
    // Clicks again until the dialog opens, as the button can move while the diff settles.
    click_until(
        &mut harness,
        "the stash dialog",
        |h| {
            if let Some(button) = h.query_by_label_contains("Stash selected lines") {
                button.click();
            }
        },
        |h| h.state().dialog.is_some(),
    );
    harness.get_all_by_label("Stash").last().expect("the confirm button").click();
    idle(&mut harness);

    wait(&mut harness, "the line stashed", |_| std::fs::read_to_string(path.join("notes.txt")).unwrap() == "first\nsecond\nfourth\n");
    let entries = stash_list(path);
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert!(git(path, &["stash", "show", "-p", "stash@{0}"]).contains("+third"));
    assert_eq!(git(path, &["diff", "--cached", "--name-only"]), "", "nothing is staged");
}

#[test]
fn popping_a_stash_over_other_edits_offers_to_apply_it_into_the_changes() {
    let repo = repository();
    let path = repo.path();
    let original: String = (1..=30).map(|n| format!("line {n}\n")).collect();
    commit_text(path, "long.txt", &original, "Add a long file");
    let edited = original.replace("line 3\n", "three\n").replace("line 28\n", "twenty-eight\n");
    std::fs::write(path.join("long.txt"), &edited).unwrap();
    let mut harness = open(path);
    loaded(&mut harness);
    harness.get_by_label("long.txt").click();
    wait_for_stageable_lines(&mut harness);
    hover_change_until(&mut harness, "Stash this change");
    harness.get_by_label("Stash this change").click();
    wait(&mut harness, "the stash dialog", |h| h.state().dialog.is_some());
    harness.get_all_by_label("Stash").last().expect("the confirm button").click();
    idle(&mut harness);
    wait(&mut harness, "the change stashed", |_| stash_list(path).len() == 1);
    let expected_working = original.replace("line 28\n", "twenty-eight\n");
    wait(&mut harness, "the working file without it", |_| std::fs::read_to_string(path.join("long.txt")).unwrap() == expected_working);

    // Git's own pop refuses over the other unstaged change to the file, and the notice offers the rest.
    harness.get_by_label_contains("Selected lines from long.txt").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Pop (apply and delete)").click();
    wait(&mut harness, "the pop confirmation", |h| h.state().dialog.is_some());
    harness.get_all_by_label("Pop").last().expect("confirm button").click();
    idle(&mut harness);
    wait(&mut harness, "the notice offering to apply into the changes", |h| h.query_by_label_contains("Apply into my changes").is_some());
    assert_eq!(stash_list(path).len(), 1, "the stash is kept while the pop is refused");
    assert_eq!(std::fs::read_to_string(path.join("long.txt")).unwrap(), expected_working, "the refused pop changes nothing");

    harness.get_by_label_contains("Apply into my changes").click();
    idle(&mut harness);
    wait(&mut harness, "the lines back in the file", |_| std::fs::read_to_string(path.join("long.txt")).unwrap() == edited);
    assert!(stash_list(path).is_empty(), "the stash is removed once its lines are applied");
}
