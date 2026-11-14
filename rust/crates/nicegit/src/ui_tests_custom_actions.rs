//! Interface tests for custom actions: adding one in Settings, then running it from the commit,
//! file, and repository menus. They use `git` itself as the program, which every machine has.

use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;

use crate::app::NiceGitApp;
use crate::ui::dialogs::{Dialog, Pending};
use crate::ui_tests::*;

/// Clicks a control in Settings. Settings scrolls, and the controls below its viewport are not
/// clickable until they are scrolled into view, as a person would scroll to them. Settings is
/// drawn over the window, so where a label such as "Commit" or "Repository" also names a control
/// behind it, the last match is Settings' own.
fn press_in_settings(harness: &mut Harness<'static, NiceGitApp>, label: &str) {
    harness.get_all_by_label(label).last().expect("a control in Settings").scroll_to_me();
    settle(harness);
    harness.get_all_by_label(label).last().expect("a control in Settings").click();
    settle(harness);
}

/// Adds an action through the Settings window, as a person does, then closes Settings.
fn add_action(harness: &mut Harness<'static, NiceGitApp>, name: &str, program: &str, arguments: &str, scope: &str, options: &[&str]) {
    harness.state_mut().show_settings = true;
    settle(harness);
    press_in_settings(harness, &format!("{}  Add action", egui_phosphor::regular::PLUS));
    type_into(harness, "Name", name);
    type_into(harness, "Program", program);
    type_into(harness, "Arguments", arguments);
    press_in_settings(harness, scope);
    for option in options {
        press_in_settings(harness, option);
    }
    press_in_settings(harness, "Save action");
    assert!(harness.state().settings.custom_actions.iter().any(|action| action.name == name), "{name} was saved");
    harness.state_mut().show_settings = false;
    settle(harness);
}

/// The notice text the app shows now, if any.
fn notice_text(harness: &Harness<'static, NiceGitApp>) -> Option<String> {
    harness.state().notice.as_ref().map(|notice| notice.text.clone())
}

#[test]
fn a_commit_action_tags_the_clicked_commit_and_reports_it() {
    let repo = repository();
    let path = repo.path();
    let mut harness = open(path);
    loaded(&mut harness);
    add_action(&mut harness, "Tag it", "git", "tag custom-{short_sha} {sha}", "Commit", &[]);

    let sha = git(path, &["rev-parse", "HEAD"]);
    let short = git(path, &["rev-parse", "--short", "HEAD"]);
    harness.get_by_label("Add a second line").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Custom actions").click();
    settle(&mut harness);
    harness.get_by_label("Tag it").click();
    idle(&mut harness);

    let tag = format!("custom-{short}");
    assert_eq!(git(path, &["rev-parse", &format!("{tag}^{{commit}}")]), sha, "the tag names the clicked commit");
    assert_eq!(notice_text(&harness).as_deref(), Some("Tag it finished."));
}

#[test]
fn a_file_action_gets_the_file_name_as_one_argument() {
    let repo = repository();
    let path = repo.path();
    std::fs::write(path.join("a b;c.txt"), "spaced\n").unwrap();
    std::fs::write(path.join("other.txt"), "untouched\n").unwrap();
    let mut harness = open(path);
    loaded(&mut harness);
    add_action(&mut harness, "Stage it", "git", "add -- {file}", "File", &[]);

    harness.get_by_label("a b;c.txt").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Custom actions").click();
    settle(&mut harness);
    harness.get_by_label("Stage it").click();
    idle(&mut harness);

    assert_eq!(git(path, &["diff", "--cached", "--name-only"]), "a b;c.txt", "exactly the clicked file is staged");
    assert!(git(path, &["ls-files", "--others", "--exclude-standard"]).contains("other.txt"), "other files stay unstaged");
    assert_eq!(notice_text(&harness).as_deref(), Some("Stage it finished."));
}

#[test]
fn an_action_that_asks_first_shows_its_command_and_waits_for_confirmation() {
    let repo = repository();
    let path = repo.path();
    let mut harness = open(path);
    loaded(&mut harness);
    add_action(&mut harness, "Tag run", "git", "tag confirmed-{branch}", "Repository", &["Ask before running"]);

    repository_menu(&mut harness, &folder_name(path), "Custom actions");
    harness.get_by_label("Tag run").click();
    wait(&mut harness, "the confirmation", |h| {
        matches!(h.state().dialog, Some(Dialog::Confirm { pending: Pending::RunCustomAction { .. }, .. }))
    });
    settle(&mut harness);
    // The exact program and its expanded arguments are shown, one per line.
    assert!(has_label(&harness, "confirmed-main"), "the expanded argument is shown");
    assert!(git(path, &["tag", "--list"]).is_empty(), "nothing runs before the confirmation");

    harness.get_by_label("Run").click();
    idle(&mut harness);
    assert_eq!(git(path, &["tag", "--list"]), "confirmed-main", "the tag is made once confirmed");
    assert_eq!(notice_text(&harness).as_deref(), Some("Tag run finished."));
}

#[test]
fn an_action_that_shows_its_output_opens_a_window_with_it() {
    let repo = repository();
    let path = repo.path();
    std::fs::write(path.join("new.txt"), "new\n").unwrap();
    let mut harness = open(path);
    loaded(&mut harness);
    add_action(&mut harness, "Show status", "git", "status --short", "Repository", &["Show output"]);

    repository_menu(&mut harness, &folder_name(path), "Custom actions");
    harness.get_by_label("Show status").click();
    idle(&mut harness);
    wait(&mut harness, "the output window", |h| has_label(h, "?? new.txt"));
    assert!(has_label(&harness, "Show status output"), "the window is titled after the action");
    assert_eq!(notice_text(&harness).as_deref(), Some("Show status finished."));
}

#[test]
fn a_failing_action_reports_the_first_line_of_its_error_output() {
    let repo = repository();
    let path = repo.path();
    let mut harness = open(path);
    loaded(&mut harness);
    add_action(&mut harness, "Break it", "git", "checkout no-such-branch-here", "Repository", &[]);

    repository_menu(&mut harness, &folder_name(path), "Custom actions");
    harness.get_by_label("Break it").click();
    idle(&mut harness);
    let text = notice_text(&harness).expect("a notice");
    assert!(text.starts_with("Break it failed: "), "the notice names the action: {text}");
    assert!(!text.ends_with("finished."), "a failure is not reported as success: {text}");
    assert!(harness.state().notice.as_ref().is_some_and(|notice| notice.is_error), "the notice is an error");
}

#[test]
fn an_action_with_a_placeholder_its_scope_cannot_fill_is_not_saved() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    harness.state_mut().show_settings = true;
    settle(&mut harness);
    press_in_settings(&mut harness, &format!("{}  Add action", egui_phosphor::regular::PLUS));
    type_into(&mut harness, "Name", "Bad");
    type_into(&mut harness, "Program", "git");
    // Repository is the default scope, and {sha} only means something for a commit.
    type_into(&mut harness, "Arguments", "tag {sha}");
    press_in_settings(&mut harness, "Save action");

    assert!(harness.state().settings.custom_actions.is_empty(), "the invalid action is not saved");
    assert!(has_label(&harness, "only filled in for Commit and File actions"), "the form says why");
}
