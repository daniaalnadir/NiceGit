//! Interface tests for the Draft button in the commit box and the Claude section of Settings. A
//! stand-in replaces the drafting function, so no test reaches the network.

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;

use crate::app::NiceGitApp;
use crate::claude::Job;
use crate::ui_tests::*;
use nicegit_core::ai_message::Drafted;

const DRAFT: &str = "Draft a message with Claude";

/// The repository from `ui_tests::repository` with a change to `notes.txt` staged.
fn staged_repository() -> tempfile::TempDir {
    let repo = repository();
    std::fs::write(repo.path().join("notes.txt"), "first\nsecond\nthird\n").unwrap();
    git(repo.path(), &["add", "notes.txt"]);
    repo
}

fn field_text(harness: &Harness<'static, NiceGitApp>, label: &str) -> String {
    harness.get_by_role_and_label(Role::TextInput, label).accesskit_node().value().unwrap_or_default().to_string()
}

/// Scrolls the Settings window until the last control labelled `label` is in view.
fn scroll_settings_to(harness: &mut Harness<'static, NiceGitApp>, label: &str) {
    harness.get_all_by_label(label).last().unwrap_or_else(|| panic!("the Settings window has {label}")).scroll_to_me();
    harness.run_steps(3);
}

fn draft_button_shown(harness: &Harness<'static, NiceGitApp>) -> bool {
    harness.query_by_label(DRAFT).is_some()
}

/// Answers every draft the same way, as Claude would for the staged change.
fn answer(_job: Job) -> Result<Drafted, String> {
    Ok(Drafted { summary: "Add a third line".to_string(), body: "The notes gain a line.".to_string(), notes: Vec::new() })
}

/// Waits a while before answering, so a test can click the button again while the draft runs.
fn slow_answer(_job: Job) -> Result<Drafted, String> {
    std::thread::sleep(std::time::Duration::from_millis(400));
    Ok(Drafted { summary: "Too late".to_string(), body: String::new(), notes: Vec::new() })
}

fn refuse(_job: Job) -> Result<Drafted, String> {
    Err("The API key was not accepted.".to_string())
}

/// Opens the repository with the feature switched on and the stand-in drafter in place.
fn drafting_app(drafter: crate::claude::Drafter) -> (tempfile::TempDir, Harness<'static, NiceGitApp>) {
    let repo = staged_repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    harness.state_mut().settings.draft_with_claude = true;
    harness.state_mut().drafter = drafter;
    wait(&mut harness, "the Draft button", draft_button_shown);
    (repo, harness)
}

#[test]
fn the_draft_button_is_not_shown_unless_the_feature_is_on() {
    let repo = staged_repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    assert!(!harness.state().settings.draft_with_claude, "off by default");
    assert!(!draft_button_shown(&harness), "no Draft button while the feature is off");
    assert!(harness.query_by_label_contains("Commit summary").is_some(), "the commit box is still there");
}

#[test]
fn the_draft_button_needs_something_staged() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    harness.state_mut().settings.draft_with_claude = true;
    settle(&mut harness);
    assert!(!draft_button_shown(&harness), "nothing is staged, so there is nothing to draft");
}

#[test]
fn turning_the_feature_on_in_settings_shows_the_button_in_the_commit_box() {
    let repo = staged_repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    harness.state_mut().show_settings = true;
    wait(&mut harness, "the Claude switch", |h| h.query_by_label("Draft commit messages with Claude").is_some());
    harness.get_by_label("Draft commit messages with Claude").click();
    settle(&mut harness);
    assert!(harness.state().settings.draft_with_claude, "the switch is on");
    harness.state_mut().show_settings = false;
    wait(&mut harness, "the Draft button", draft_button_shown);
}

#[test]
fn a_draft_fills_the_summary_and_description() {
    let (_repo, mut harness) = drafting_app(answer);
    harness.get_by_label(DRAFT).click();
    wait(&mut harness, "the drafted summary", |h| field_text(h, "Commit summary") == "Add a third line");
    assert_eq!(field_text(&harness, "Commit description"), "The notes gain a line.");
    assert!(harness.state().draft_run.is_none(), "the draft has finished");
}

#[test]
fn a_draft_that_fails_shows_why_under_the_box() {
    let (_repo, mut harness) = drafting_app(refuse);
    harness.get_by_label(DRAFT).click();
    wait(&mut harness, "the reason", |h| has_label(h, "The API key was not accepted."));
    assert_eq!(field_text(&harness, "Commit summary"), "", "a failed draft changes nothing");
}

#[test]
fn a_draft_can_be_stopped_by_clicking_again_and_then_changes_nothing() {
    let (_repo, mut harness) = drafting_app(slow_answer);
    harness.get_by_label(DRAFT).click();
    wait(&mut harness, "the draft to be running", |h| h.state().draft_run.is_some());
    harness.get_by_label(DRAFT).click();
    settle(&mut harness);
    assert!(harness.state().draft_run.is_none(), "the draft was stopped");
    harness.run_steps(40);
    assert_eq!(field_text(&harness, "Commit summary"), "", "a stopped draft does not fill the message");
}

#[test]
fn a_finished_draft_asks_before_replacing_text_already_written() {
    let (_repo, mut harness) = drafting_app(answer);
    type_into(&mut harness, "Commit summary", "Keep this");
    harness.get_by_label(DRAFT).click();
    wait(&mut harness, "the replace question", |h| has_label(h, "Replace the current message?"));
    assert_eq!(field_text(&harness, "Commit summary"), "Keep this", "nothing is replaced before the answer");

    harness.get_by_label("Keep mine").click();
    settle(&mut harness);
    assert_eq!(field_text(&harness, "Commit summary"), "Keep this", "Keep mine leaves the message");
    assert!(!has_label(&harness, "Replace the current message?"), "the question is gone");

    harness.get_by_label(DRAFT).click();
    wait(&mut harness, "the replace question again", |h| has_label(h, "Replace the current message?"));
    harness.get_by_label("Replace").click();
    wait(&mut harness, "the replaced summary", |h| field_text(h, "Commit summary") == "Add a third line");
    assert_eq!(field_text(&harness, "Commit description"), "The notes gain a line.");
}

#[test]
fn the_saved_key_is_stored_in_the_chosen_file_and_never_shown_back() {
    let repo = staged_repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    let key_dir = crate::ui_tests::temp_dir();
    let key_file = key_dir.path().join("anthropic-key");
    harness.state_mut().key_file = Some(key_file.clone());
    harness.state_mut().show_settings = true;
    wait(&mut harness, "the key field", |h| h.query_by_label("Anthropic API key").is_some());

    type_into(&mut harness, "Anthropic API key", "sk-ant-ui-test-key");
    scroll_settings_to(&mut harness, "Save");
    harness.get_by_label("Save").click();
    wait(&mut harness, "the saved key", |_| key_file.exists());
    assert_eq!(std::fs::read_to_string(&key_file).unwrap(), "sk-ant-ui-test-key");
    settle(&mut harness);
    assert!(!has_label(&harness, "sk-ant-ui-test-key"), "the key is never shown back");
    assert!(harness.query_by_label("Remove").is_some(), "a saved key can be removed");

    scroll_settings_to(&mut harness, "Remove");
    harness.get_by_label("Remove").click();
    wait(&mut harness, "the key to be removed", |_| !key_file.exists());
    harness.state_mut().show_settings = false;
}
