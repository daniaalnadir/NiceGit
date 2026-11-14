//! Interface tests for issue links in commit messages: a rule added in the Settings window, the links
//! the inspector shows, and a click on a link asking for its address to be opened. They use the
//! helpers in `ui_tests`.

use egui::accesskit::Role;
use egui::{Key, Modifiers, OutputCommand};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;

use crate::app::NiceGitApp;
use crate::ui_tests::*;

/// The addresses the app asked to open, from the last frame.
fn opened_urls(harness: &Harness<'static, NiceGitApp>) -> Vec<String> {
    harness
        .output()
        .platform_output
        .commands
        .iter()
        .filter_map(|command| match command {
            OutputCommand::OpenUrl(open) => Some(open.url.clone()),
            _ => None,
        })
        .collect()
}

/// Scrolls the Settings window until the widget labelled `label` is in view, wherever its section
/// sits among the others.
fn scroll_settings_to(harness: &mut Harness<'static, NiceGitApp>, label: &str) {
    harness.get_all_by_label(label).last().unwrap_or_else(|| panic!("the Settings window has {label}")).scroll_to_me();
    settle(harness);
}

/// A repository on a GitHub remote whose newest commit has a subject with `#42` and a body with a
/// Jira-style key that only a custom rule links.
fn repository_with_linked_message() -> tempfile::TempDir {
    let repo = repository();
    let path = repo.path();
    git(path, &["remote", "add", "origin", "https://github.com/acme/widgets.git"]);
    std::fs::write(path.join("notes.txt"), "first\nsecond\nthird\n").unwrap();
    git(path, &["commit", "-qam", "Fix crash (#42)\n\nSee JIRA-7"]);
    repo
}

#[test]
fn a_rule_added_in_settings_links_the_message_and_the_host_link_opens_its_issue() {
    let repo = repository_with_linked_message();
    let mut harness = open(repo.path());
    loaded(&mut harness);

    // Settings: add a Jira-style rule by typing into its fields, then save it.
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Comma);
    wait(&mut harness, "the Settings window", |h| {
        h.query_by_role_and_label(Role::CheckBox, "Link #123 to the repository's host").is_some()
    });
    settle(&mut harness);
    scroll_settings_to(&mut harness, "Add rule");
    harness.get_by_label("Add rule").click();
    settle(&mut harness);
    type_into(&mut harness, "Rule pattern", r"\b([A-Z][A-Z0-9]+-\d+)\b");
    type_into(&mut harness, "Rule address", "example.atlassian.net/browse/$1");
    scroll_settings_to(&mut harness, "Save rule");
    harness.get_by_label("Save rule").click();
    settle(&mut harness);
    assert!(harness.state().settings.issue_links.is_empty(), "an address without https:// is not saved");
    assert!(has_label(&harness, "must start with https://"), "the problem is shown inline");

    // Cancel and open a fresh form, so the corrected address is not typed after the bad one.
    scroll_settings_to(&mut harness, "Cancel");
    harness.get_by_label("Cancel").click();
    settle(&mut harness);
    scroll_settings_to(&mut harness, "Add rule");
    harness.get_by_label("Add rule").click();
    settle(&mut harness);
    type_into(&mut harness, "Rule pattern", r"\b([A-Z][A-Z0-9]+-\d+)\b");
    type_into(&mut harness, "Rule address", "https://example.atlassian.net/browse/$1");
    scroll_settings_to(&mut harness, "Save rule");
    harness.get_by_label("Save rule").click();
    settle(&mut harness);
    let rules = &harness.state().settings.issue_links;
    assert_eq!(rules.len(), 1, "the corrected rule is saved");
    assert_eq!(rules[0].pattern, r"\b([A-Z][A-Z0-9]+-\d+)\b");
    assert_eq!(rules[0].url, "https://example.atlassian.net/browse/$1");
    harness.key_press(Key::Escape);
    settle(&mut harness);

    // The commit's inspector shows both links, labelled by their text.
    harness.get_by_label("Fix crash (#42)").click();
    wait(&mut harness, "the commit's details", |h| {
        h.state().repo().is_some_and(|r| r.details.as_ref().is_some_and(|task| task.peek().is_some()))
    });
    settle(&mut harness);
    assert!(harness.query_by_label("#42").is_some(), "#42 is a link in the subject");
    assert!(harness.query_by_label("JIRA-7").is_some(), "JIRA-7 is a link in the description");

    // A click on #42 asks for its issue on the remote's host; a click on JIRA-7 asks for its tracker page.
    harness.get_by_label("#42").click();
    harness.step();
    assert_eq!(opened_urls(&harness), vec!["https://github.com/acme/widgets/issues/42".to_string()]);
    harness.get_by_label("JIRA-7").click();
    harness.step();
    assert_eq!(opened_urls(&harness), vec!["https://example.atlassian.net/browse/JIRA-7".to_string()]);
}
