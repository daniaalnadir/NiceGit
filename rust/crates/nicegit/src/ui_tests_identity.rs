//! Interface tests for choosing a saved identity profile from a repository's Identity menu: the
//! profile is saved in Repository settings, applied from the menu, and shown in the commit box. They
//! use the helpers in `ui_tests`.

use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use egui_phosphor::regular as icon;

use crate::app::NiceGitApp;
use crate::ui_tests::*;

/// Replaces the text of the field with this accessible label: selects all of it, then types.
fn replace_text(harness: &mut Harness<'static, NiceGitApp>, label: &str, text: &str) {
    harness.get_by_role_and_label(Role::TextInput, label).focus();
    harness.step();
    harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
    harness.step();
    type_into(harness, label, text);
}

/// Whether the button or field with this label is enabled.
fn enabled(harness: &Harness<'static, NiceGitApp>, label: &str) -> bool {
    harness.query_by_label_contains(label).is_some_and(|node| !format!("{node:?}").contains("disabled: true"))
}

/// Saves the repository's identity as a profile, changed to `name` and `email`, in Repository settings.
fn save_profile(harness: &mut Harness<'static, NiceGitApp>, name: &str, email: &str) {
    let folder = folder_name(&harness.state().repo().expect("repository").path);
    repository_menu(harness, &folder, "Repository settings");
    // The form opens on the Identity tab and reads the identity in the background; its fields are
    // editable once it has.
    wait(harness, "the identity to load", |h| {
        h.query_by_role_and_label(Role::TextInput, "Name").is_some_and(|field| !format!("{field:?}").contains("disabled: true"))
    });
    settle(harness);
    replace_text(harness, "Name", name);
    replace_text(harness, "Email", email);
    harness.get_by_label_contains("Profiles").click();
    settle(harness);
    wait(harness, "the profile to be savable", |h| enabled(h, "Save as profile"));
    harness.get_by_label_contains("Save as profile").click();
    settle(harness);
    harness.state_mut().tools.clear();
}

#[test]
fn a_profile_saved_in_repository_settings_is_chosen_from_the_identity_menu() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    let folder = folder_name(repo.path());

    save_profile(&mut harness, "Ana Day", "ana@day.example");

    repository_menu(&mut harness, &folder, "Identity");
    wait(&mut harness, "the profile entry", |h| h.query_by_label_contains("Ana Day <ana@day.example>").is_some());
    harness.get_by_label_contains("Ana Day <ana@day.example>").click();
    idle(&mut harness);
    assert_eq!(git(repo.path(), &["config", "--local", "user.name"]), "Ana Day");
    assert_eq!(git(repo.path(), &["config", "--local", "user.email"]), "ana@day.example");
    wait(&mut harness, "the identity read after the switch", |h| {
        h.state().repo().and_then(|r| r.identity.clone()) == Some(("Ana Day".into(), "ana@day.example".into()))
    });
    assert!(has_label(&harness, "Committing as Ana Day <ana@day.example>"), "the commit box names the new author");

    // The Identity menu now marks the profile the repository uses.
    repository_menu(&mut harness, &folder, "Identity");
    wait(&mut harness, "the check mark", |h| h.query_by_label_contains(&format!("{}  Ana Day <ana@day.example>", icon::CHECK)).is_some());
    harness.key_press(Key::Escape);
    settle(&mut harness);
}

#[test]
fn manage_identities_opens_repository_settings_on_its_profiles() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    let folder = folder_name(repo.path());

    repository_menu(&mut harness, &folder, "Identity");
    wait(&mut harness, "the manage entry", |h| h.query_by_label_contains("Manage identities").is_some());
    harness.get_by_label_contains("Manage identities").click();
    wait(&mut harness, "the profiles section", |h| h.query_by_label_contains("Save as profile").is_some());
    assert!(has_label(&harness, "No saved profiles yet"), "the profiles section opens first");
}
