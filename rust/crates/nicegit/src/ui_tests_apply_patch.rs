//! Interface tests for the Apply patch window. They open it from the command palette or by dropping
//! a patch file, paste or preview a patch, and click through to the repository Git holds. They use
//! the helpers in `ui_tests`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;

use crate::app::NiceGitApp;
use crate::ui_tests::*;

/// A file dropped onto the window, as the system reports it.
#[derive(Debug)]
struct Dropped(PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|error| error.to_string())
    }
}

/// A diff that changes the first line of notes.txt, which holds "first" and "second".
const CHANGE_FIRST_LINE: &str =
    "diff --git a/notes.txt b/notes.txt\n--- a/notes.txt\n+++ b/notes.txt\n@@ -1,2 +1,2 @@\n-first\n+one\n second\n";

/// The same change, against a first line the notes do not have.
const WRONG_CONTEXT: &str =
    "diff --git a/notes.txt b/notes.txt\n--- a/notes.txt\n+++ b/notes.txt\n@@ -1,2 +1,2 @@\n-missing\n+one\n second\n";

fn apply_patch_window_is_open(harness: &Harness<'static, NiceGitApp>) -> bool {
    harness.state().tools.iter().any(|tool| tool.id() == "apply-patch")
}

#[test]
fn apply_a_pasted_patch_from_the_palette() {
    let repo = repository();
    let path = repo.path();
    let mut harness = open(path);
    loaded(&mut harness);

    harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::P);
    wait(&mut harness, "the palette", |h| h.state().palette.is_some());
    harness.event(egui::Event::Text("apply patch".into()));
    harness.step();
    harness.key_press(Key::Enter);
    wait(&mut harness, "the Apply patch window", apply_patch_window_is_open);

    harness.get_by_role_and_label(Role::MultilineTextInput, "Patch text").focus();
    harness.step();
    harness.event(egui::Event::Paste(CHANGE_FIRST_LINE.to_string()));
    harness.step();
    harness.get_by_role_and_label(Role::Button, "Preview").click();
    wait(&mut harness, "the preview", |h| h.query_by_label_contains("Applies cleanly").is_some());

    harness.get_by_role_and_label(Role::Button, "Apply").click();
    idle(&mut harness);
    assert_eq!(git(path, &["diff", "--cached", "--name-only"]), "", "the changes are unstaged");
    assert_eq!(git(path, &["diff", "--name-only"]), "notes.txt");
    assert_eq!(std::fs::read_to_string(path.join("notes.txt")).unwrap(), "one\nsecond\n");
    assert!(!apply_patch_window_is_open(&harness), "the window closes after Apply");
    wait(&mut harness, "the changed file in Changes", |h| h.query_by_label_contains("notes.txt").is_some());
}

#[test]
fn dropping_a_patch_that_does_not_apply_shows_why_and_changes_nothing() {
    let repo = repository();
    let path = repo.path();
    let patch_dir = temp_dir();
    let patch = patch_dir.path().join("wrong-context.patch");
    std::fs::write(&patch, WRONG_CONTEXT).unwrap();
    let mut harness = open(path);
    loaded(&mut harness);

    harness.input_mut().dropped_files.push(Arc::new(Dropped(patch)));
    wait(&mut harness, "the Apply patch window", apply_patch_window_is_open);
    harness.get_by_role_and_label(Role::Button, "Preview").click();
    wait(&mut harness, "the preview", |h| h.query_by_label_contains("does not apply").is_some());

    // The Apply button is disabled, so the click does nothing.
    harness.get_by_role_and_label(Role::Button, "Apply").click();
    settle(&mut harness);
    assert!(apply_patch_window_is_open(&harness), "the window stays open");
    assert_eq!(git(path, &["diff", "--name-only"]), "");
    assert_eq!(std::fs::read_to_string(path.join("notes.txt")).unwrap(), "first\nsecond\n");
}
