//! Interface tests for splitting a commit from the graph menu. They click and type through the
//! window, then check the repository Git holds. They use the helpers in `ui_tests`.

use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use egui_phosphor::regular as icon;

use crate::app::NiceGitApp;
use crate::ui_tests::*;

/// Replaces the text in a labelled message box with `text`.
fn replace_text(harness: &mut Harness<'static, NiceGitApp>, label: &str, text: &str) {
    let role = Role::MultilineTextInput;
    harness.get_by_role_and_label(role, label).focus();
    harness.step();
    harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
    harness.step();
    harness.get_by_role_and_label(role, label).type_text(text);
    harness.step();
}

#[test]
fn split_a_commit_that_changes_two_files_into_two_commits() {
    let repo = repository();
    let path = repo.path();
    // The HEAD commit changes notes.txt and adds extra.txt.
    std::fs::write(path.join("extra.txt"), "extra\n").unwrap();
    git(path, &["add", "extra.txt"]);
    git(path, &["commit", "-q", "--amend", "--no-edit"]);
    let subject = git(path, &["log", "-1", "--format=%s"]);
    let before_tree = git(path, &["rev-parse", "HEAD^{tree}"]);

    let mut harness = open(path);
    loaded(&mut harness);
    harness.get_by_label(&subject).click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Split commit").click();
    wait(&mut harness, "the split window", |h| h.query_by_role_and_label(Role::CheckBox, "extra.txt").is_some());

    // Everything starts in the first commit. Leaving extra.txt out moves it to the second.
    harness.get_by_role_and_label(Role::CheckBox, "extra.txt").click();
    settle(&mut harness);
    replace_text(&mut harness, "First commit message", "Change notes");
    replace_text(&mut harness, "Second commit message", "Add the extra file");
    harness.get_by_role_and_label(Role::Button, "Split").click();
    idle(&mut harness);

    assert_eq!(git(path, &["log", "-2", "--format=%s"]), "Add the extra file\nChange notes");
    assert_eq!(git(path, &["show", "--format=", "--name-only", "HEAD~1"]), "notes.txt");
    assert_eq!(git(path, &["show", "--format=", "--name-only", "HEAD"]), "extra.txt");
    assert_eq!(git(path, &["rev-parse", "HEAD^{tree}"]), before_tree);
    assert_eq!(std::fs::read_to_string(path.join("notes.txt")).unwrap(), "first\nsecond\n");
    assert_eq!(std::fs::read_to_string(path.join("extra.txt")).unwrap(), "extra\n");
    assert_eq!(git(path, &["status", "--porcelain"]), "");
    let notice = harness.state().notice.as_ref().map(|n| n.text.clone()).expect("a message");
    assert!(notice.ends_with("into two commits."), "{notice}");
}

#[test]
fn split_an_older_commit_by_one_hunk_of_a_file() {
    let repo = repository();
    let path = repo.path();
    let lines: String = (1..=12).map(|n| format!("line {n}\n")).collect();
    std::fs::write(path.join("notes.txt"), &lines).unwrap();
    git(path, &["commit", "-qam", "Add twelve lines"]);
    // The top and the bottom change, far enough apart to be two hunks.
    let changed = lines.replacen("line 1\n", "first line\n", 1).replacen("line 12\n", "last line\n", 1);
    std::fs::write(path.join("notes.txt"), &changed).unwrap();
    git(path, &["commit", "-qam", "Change the top and bottom"]);
    std::fs::write(path.join("later.txt"), "later\n").unwrap();
    git(path, &["add", "later.txt"]);
    git(path, &["commit", "-qm", "Add later"]);
    let before_tree = git(path, &["rev-parse", "HEAD^{tree}"]);

    let mut harness = open(path);
    loaded(&mut harness);
    harness.get_by_label("Change the top and bottom").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Split commit").click();
    let caret = format!("{}  2 changes", icon::CARET_RIGHT);
    wait(&mut harness, "the split window", |h| h.query_by_label(&caret).is_some());

    // Show notes.txt's two changes, and keep only the top one in the first commit.
    harness.get_by_label(&caret).click();
    settle(&mut harness);
    harness.get_by_role_and_label(Role::CheckBox, "Change 2").click();
    settle(&mut harness);
    replace_text(&mut harness, "First commit message", "Change the top line");
    replace_text(&mut harness, "Second commit message", "Change the bottom line");
    harness.get_by_role_and_label(Role::Button, "Split").click();
    idle(&mut harness);

    assert_eq!(git(path, &["log", "-3", "--format=%s"]), "Add later\nChange the bottom line\nChange the top line");
    assert_eq!(git(path, &["show", "HEAD~2:notes.txt"]), lines.replacen("line 1\n", "first line\n", 1).trim_end());
    assert_eq!(git(path, &["show", "HEAD~1:notes.txt"]), changed.trim_end());
    assert_eq!(git(path, &["rev-parse", "HEAD^{tree}"]), before_tree);
    assert_eq!(std::fs::read_to_string(path.join("notes.txt")).unwrap(), changed);
    assert_eq!(git(path, &["status", "--porcelain"]), "");
}
