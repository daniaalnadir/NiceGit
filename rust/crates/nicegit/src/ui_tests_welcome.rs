//! Interface tests for the welcome screen shown while no repository is open, and for the tips it
//! and the command palette show. They use the helpers in `ui_tests`.

use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use egui_phosphor::regular as icon;

use crate::app::NiceGitApp;
use crate::ui::tips::{self, TIPS};
use crate::ui_tests::*;

/// The app with no repository open, at the size the other interface tests use.
fn empty_app() -> Harness<'static, NiceGitApp> {
    arm_watchdog();
    let mut harness = Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_eframe(|cc| NiceGitApp::new(cc, None));
    settle(&mut harness);
    assert!(harness.state().repos.is_empty(), "no repository is open at first");
    harness
}

#[test]
fn the_welcome_screen_shows_its_heading_and_three_ways_to_start() {
    let harness = empty_app();
    assert!(harness.query_by_label_contains("Welcome to NiceGit").is_some(), "the heading is shown");
    assert!(harness.query_by_label_contains("Open repository").is_some(), "Open repository is offered");
    assert!(harness.query_by_label_contains("Clone repository").is_some(), "Clone repository is offered");
    assert!(harness.query_by_label_contains("Create repository").is_some(), "Create repository is offered");
}

#[test]
fn open_repository_on_the_welcome_screen_opens_the_chosen_folder() {
    let chosen = repository();
    let mut harness = empty_app();
    crate::file_dialog::answer_next(chosen.path());
    harness.get_by_label(&format!("{}  Open repository…", icon::FOLDER_OPEN)).click();
    wait(&mut harness, "the chosen repository", |h| h.state().snapshot().is_some() && h.state().busy.is_none());
    assert_eq!(harness.state().repos.len(), 1, "the repository opened in a tab");
    assert_eq!(harness.state().repo().map(|r| r.path.canonicalize().unwrap()), Some(chosen.path().canonicalize().unwrap()));
}

#[test]
fn clone_on_the_welcome_screen_opens_the_clone_dialog() {
    let mut harness = empty_app();
    harness.get_by_label(&format!("{}  Clone repository…", icon::DOWNLOAD_SIMPLE)).click();
    wait(&mut harness, "the clone dialog", |h| h.state().dialog.is_some());
    harness.get_by_role_and_label(Role::TextInput, "Repository URL or local path");
}

#[test]
fn create_on_the_welcome_screen_opens_the_new_repository_dialog() {
    let mut harness = empty_app();
    harness.get_by_label(&format!("{}  Create repository…", icon::PLUS)).click();
    wait(&mut harness, "the new repository dialog", |h| h.state().dialog.is_some());
    harness.get_by_role_and_label(Role::TextInput, "Folder");
}

#[test]
fn a_recent_repository_that_exists_is_listed_and_opens_on_click() {
    let existing = repository();
    let parent = temp_dir();
    let gone = parent.path().join("moved-away");
    let mut harness = empty_app();
    harness.state_mut().settings.recent = vec![existing.path().to_path_buf(), gone.clone()];
    settle(&mut harness);

    let name = folder_name(existing.path());
    let row = format!("Open recent repository {name}");
    assert!(harness.query_by_label(&row).is_some(), "the existing repository is listed");
    let gone_row = format!("Open recent repository {}", folder_name(&gone));
    assert!(harness.query_by_label(&gone_row).is_none(), "a folder that no longer exists is not listed");

    harness.get_by_label(&row).click();
    loaded(&mut harness);
    assert_eq!(harness.state().repos.len(), 1, "the recent repository opened in a tab");
    assert_eq!(harness.state().snapshot().map(|s| s.name.clone()), Some(name));
}

#[test]
fn next_tip_changes_the_shown_tip() {
    let mut harness = empty_app();
    let before = harness.state().welcome_tip;
    assert!(harness.query_by_label_contains(&tips::text(before)).is_some(), "the starting tip is shown");

    harness.get_by_label("Next tip").click();
    settle(&mut harness);
    let after = harness.state().welcome_tip;
    assert_ne!(before, after, "the tip changed");
    assert!(harness.query_by_label_contains(&tips::text(after)).is_some(), "the next tip is shown");
}

#[test]
fn show_a_tip_in_the_command_palette_posts_the_next_tip_as_a_notice() {
    let mut harness = empty_app();
    let before = harness.state().welcome_tip;

    harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::P);
    wait(&mut harness, "the palette", |h| h.state().palette.is_some());
    harness.event(egui::Event::Text("show a tip".into()));
    wait(&mut harness, "the tip command", |h| h.query_by_label_contains("Show a tip").is_some());
    harness.key_press(Key::Enter);
    settle(&mut harness);

    let after = harness.state().welcome_tip;
    assert_eq!(after, (before + 1) % TIPS.len(), "the next tip is chosen");
    assert_eq!(harness.state().notice.as_ref().map(|notice| notice.text.clone()), Some(tips::text(after)));
}

/// The welcome screen fits the smallest window the app allows, with its content scrolling rather
/// than clipping.
#[test]
fn the_welcome_screen_lays_out_at_the_smallest_window() {
    arm_watchdog();
    let mut harness = Harness::builder().with_size(egui::vec2(900.0, 560.0)).build_eframe(|cc| NiceGitApp::new(cc, None));
    settle(&mut harness);
    assert!(harness.query_by_label_contains("Welcome to NiceGit").is_some());
    harness.get_by_label("Next tip");
    harness.get_by_label_contains("Open repository");
}
