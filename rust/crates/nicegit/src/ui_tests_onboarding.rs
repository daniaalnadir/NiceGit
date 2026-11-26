//! Interface tests for the first-run welcome, the identity page, and the tour. The identity is
//! read and saved in a temporary file, never in the person's own Git settings.

use std::path::Path;

use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;

use crate::app::NiceGitApp;
use crate::ui::onboarding::{at_start, Onboarding, TourStep};
use crate::ui_tests::*;

/// The app as a first run starts it: the tour not yet finished, with the identity and the
/// practice repository kept in the given folders.
fn onboarding_app(size: egui::Vec2, identity: &Path, practice_parent: Option<&Path>) -> Harness<'static, NiceGitApp> {
    arm_watchdog();
    let identity = identity.to_path_buf();
    let practice_parent = practice_parent.map(Path::to_path_buf);
    let mut harness = Harness::builder().with_size(size).build_eframe(move |cc| {
        let mut app = NiceGitApp::new(cc, None);
        app.settings.onboarding_done = false;
        app.onboarding = Some(Onboarding::Intro);
        app.identity_file = Some(identity.clone());
        app.practice_parent = practice_parent.clone();
        app
    });
    settle(&mut harness);
    harness
}

/// A temporary Git settings file holding `contents`.
fn settings_file(contents: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = temp_dir();
    let file = dir.path().join("gitconfig");
    std::fs::write(&file, contents).expect("settings file written");
    (dir, file)
}

/// Types into a text field. The focus takes two frames to reach the field, so the text is typed
/// after them; sooner, it goes to the field that had focus before.
fn type_into_field(harness: &mut Harness<'static, NiceGitApp>, label: &str, text: &str) {
    let role = egui::accesskit::Role::TextInput;
    harness.get_by_role_and_label(role, label).focus();
    harness.run_steps(2);
    harness.get_by_role_and_label(role, label).type_text(text);
    harness.run_steps(2);
}

/// The value the settings file holds for a Git key, or empty when it is unset.
fn config_value(file: &Path, key: &str) -> String {
    let output = std::process::Command::new("git").arg("config").arg("--file").arg(file).args(["--get", key]).output().expect("git runs");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn tour_step(harness: &Harness<'static, NiceGitApp>) -> Option<TourStep> {
    match harness.state().onboarding {
        Some(Onboarding::Tour { step, .. }) => Some(step),
        _ => None,
    }
}

#[test]
fn first_run_shows_sprig_then_skipping_twice_finishes_onboarding() {
    let (_dir, identity) = settings_file("");
    let mut harness = onboarding_app(egui::vec2(1440.0, 900.0), &identity, None);

    assert!(has_label(&harness, "Hi, I'm Sprig"), "the welcome page greets the person");
    assert!(harness.query_all_by_label("Sprig").next().is_some(), "Sprig's picture is labelled");

    harness.get_by_label("Skip for now").click();
    wait(&mut harness, "the identity page", |h| has_label(h, "Confirm your Git details"));

    harness.get_by_label("Skip for now").click();
    wait(&mut harness, "onboarding to finish", |h| h.state().onboarding.is_none());
    assert!(harness.state().settings.onboarding_done, "the tour is marked as finished");
    settle(&mut harness);
    assert!(has_label(&harness, "Welcome to NiceGit"), "the welcome screen shows afterwards");
}

#[test]
fn identity_page_is_prefilled_from_the_settings_file() {
    let (_dir, identity) = settings_file("[user]\n\tname = Ada Old\n\temail = old@example.com\n");
    let mut harness = onboarding_app(egui::vec2(1440.0, 900.0), &identity, None);
    harness.get_by_label("Start the tour").click();
    wait(&mut harness, "the identity page", |h| has_label(h, "Confirm your Git details"));
    match &harness.state().onboarding {
        Some(Onboarding::Identity { name, email, .. }) => {
            assert_eq!(name, "Ada Old");
            assert_eq!(email, "old@example.com");
        }
        _ => panic!("the identity page is not showing"),
    }
}

#[test]
fn saving_the_identity_writes_the_settings_file() {
    let (_dir, identity) = settings_file("");
    let mut harness = onboarding_app(egui::vec2(1440.0, 900.0), &identity, None);
    // Skip the tour here, so no practice repository is made: this test is about the identity.
    harness.get_by_label("Skip for now").click();
    wait(&mut harness, "the identity page", |h| has_label(h, "Confirm your Git details"));

    type_into_field(&mut harness, "Name", "Ada Lovelace");
    type_into_field(&mut harness, "Email", "ada@example.com");
    harness.get_by_label("Use these for my commits").click();

    wait(&mut harness, "the identity to be saved", |_| config_value(&identity, "user.email") == "ada@example.com");
    assert_eq!(config_value(&identity, "user.name"), "Ada Lovelace");
    wait(&mut harness, "onboarding to finish", |h| h.state().onboarding.is_none());
}

#[test]
fn an_email_without_an_at_sign_is_refused_and_nothing_is_saved() {
    let (_dir, identity) = settings_file("");
    let mut harness = onboarding_app(egui::vec2(1440.0, 900.0), &identity, None);
    harness.get_by_label("Skip for now").click();
    wait(&mut harness, "the identity page", |h| has_label(h, "Confirm your Git details"));

    type_into_field(&mut harness, "Name", "Ada Lovelace");
    type_into_field(&mut harness, "Email", "ada.example.com");
    harness.get_by_label("Use these for my commits").click();
    wait(&mut harness, "the error", |h| has_label(h, "Enter your name and an email address."));

    assert!(matches!(harness.state().onboarding, Some(Onboarding::Identity { .. })), "the page stays open");
    assert_eq!(config_value(&identity, "user.email"), "", "nothing is saved");
}

#[test]
fn the_tour_walks_through_staging_and_committing_in_the_practice_repository() {
    let parent = temp_dir();
    let (_dir, identity) = settings_file("");
    let mut harness = onboarding_app(egui::vec2(1440.0, 900.0), &identity, Some(parent.path()));

    harness.get_by_label("Start the tour").click();
    wait(&mut harness, "the identity page", |h| has_label(h, "Confirm your Git details"));
    type_into_field(&mut harness, "Name", "Tour Tester");
    type_into_field(&mut harness, "Email", "tester@example.com");
    harness.get_by_label("Use these for my commits").click();

    wait(&mut harness, "the first step", |h| tour_step(h) == Some(TourStep::History) && has_label(h, "Your history"));
    assert!(harness.state().repo().is_some_and(|r| r.path.join("hello.txt").exists()), "the practice repository is open");

    harness.get_by_label("Next").click();
    wait(&mut harness, "the sidebar step", |h| has_label(h, "Branches and more"));
    harness.get_by_label("Next").click();
    wait(&mut harness, "the changes step", |h| has_label(h, "Stage a change"));

    harness.get_by_label("Stage All Changes").click();
    wait(&mut harness, "the commit step", |h| has_label(h, "Commit it"));

    type_into_field(&mut harness, "Commit summary", "Add a greeting change");
    wait(&mut harness, "the commit button", |h| h.query_all_by_label_contains("Commit 1 file to main").next().is_some());
    harness.get_by_label_contains("Commit 1 file to main").click();
    wait(&mut harness, "the toolbar step", |h| has_label(h, "Share your work"));

    harness.get_by_label("Next").click();
    wait(&mut harness, "the palette step", |h| has_label(h, "Find anything"));
    harness.get_by_label("Next").click();
    wait(&mut harness, "the closing card", |h| has_label(h, "You're ready"));

    harness.get_by_label("Finish").click();
    wait(&mut harness, "onboarding to finish", |h| h.state().onboarding.is_none());
    assert!(harness.state().settings.onboarding_done);
}

#[test]
fn skip_tour_in_the_middle_of_the_tour_finishes_onboarding() {
    let parent = temp_dir();
    let (_dir, identity) = settings_file("[user]\n\tname = Ada\n\temail = ada@example.com\n");
    let mut harness = onboarding_app(egui::vec2(1440.0, 900.0), &identity, Some(parent.path()));

    harness.get_by_label("Start the tour").click();
    wait(&mut harness, "the identity page", |h| has_label(h, "Confirm your Git details"));
    harness.get_by_label("Use these for my commits").click();
    wait(&mut harness, "the first step", |h| tour_step(h) == Some(TourStep::History) && has_label(h, "Your history"));

    harness.get_by_label("Skip tour").click();
    wait(&mut harness, "onboarding to finish", |h| h.state().onboarding.is_none());
    assert!(harness.state().settings.onboarding_done);
}

#[test]
fn the_welcome_and_identity_pages_fit_a_small_window() {
    let (_dir, identity) = settings_file("");
    let mut harness = onboarding_app(egui::vec2(900.0, 560.0), &identity, None);
    assert!(has_label(&harness, "Hi, I'm Sprig"));
    harness.get_by_label("Start the tour").click();
    wait(&mut harness, "the identity page", |h| has_label(h, "Confirm your Git details"));
    harness.get_by_label("Use these for my commits");
    harness.get_by_label("Skip for now");
}

#[test]
fn onboarding_starts_only_at_a_first_run_with_no_repository_named() {
    assert!(at_start(false, true).is_some(), "a first run starts the welcome");
    assert!(at_start(false, false).is_none(), "a repository named on the command line opens directly");
    assert!(at_start(true, true).is_none(), "a finished onboarding never returns");
}
