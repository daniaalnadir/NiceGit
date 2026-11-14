//! Interface tests for finding repositories in a folder: from the Repository menu, and by dropping
//! a folder that is not a repository. They drive the real window, with the folder picker's answer
//! supplied by the test hook in `file_dialog`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;

use crate::ui_tests::*;

/// Makes a repository at `path`, creating the folder as needed.
fn init_repository(path: &Path) {
    std::fs::create_dir_all(path).unwrap();
    git(path, &["init", "-q", "-b", "main"]);
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap()
}

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

#[test]
fn repositories_found_in_a_folder_are_added_to_recent_when_checked() {
    let open_now = repository();
    let folder = temp_dir();
    for name in ["alpha", "beta", "parent/gamma"] {
        init_repository(&folder.path().join(name));
    }
    let mut harness = open(open_now.path());
    loaded(&mut harness);

    crate::file_dialog::answer_next(folder.path());
    repository_menu(&mut harness, &folder_name(open_now.path()), "Find repositories in a folder");
    wait(&mut harness, "the results", |h| h.query_by_label_contains("Found 3 repositories").is_some());

    // Unchecking one leaves the other two to add.
    harness.get_by_role_and_label(Role::CheckBox, "beta").click();
    settle(&mut harness);
    harness.get_by_label("Add 2 to Recent").click();
    wait(&mut harness, "the dialog to close", |h| h.state().dialog.is_none());

    let recent: Vec<PathBuf> = harness.state().settings.recent.iter().map(|path| canonical(path)).collect();
    assert!(recent.contains(&canonical(&folder.path().join("alpha"))), "the checked alpha is recent");
    assert!(recent.contains(&canonical(&folder.path().join("parent/gamma"))), "the checked nested gamma is recent");
    assert!(!recent.contains(&canonical(&folder.path().join("beta"))), "the unchecked beta is not added");
    assert_eq!(harness.state().repos.len(), 1, "nothing is opened as a tab unless asked");
}

#[test]
fn repositories_found_in_a_folder_open_as_tabs_when_asked() {
    let open_now = repository();
    let folder = temp_dir();
    for name in ["alpha", "beta"] {
        init_repository(&folder.path().join(name));
    }
    let mut harness = open(open_now.path());
    loaded(&mut harness);

    crate::file_dialog::answer_next(folder.path());
    repository_menu(&mut harness, &folder_name(open_now.path()), "Find repositories in a folder");
    wait(&mut harness, "the results", |h| h.query_by_label_contains("Found 2 repositories").is_some());

    harness.get_by_role_and_label(Role::CheckBox, "Also open them as tabs").click();
    settle(&mut harness);
    harness.get_by_label("Add 2 to Recent").click();
    wait(&mut harness, "the tabs to open", |h| h.state().repos.len() == 3 && h.state().busy.is_none() && h.state().dialog.is_none());

    let tabs: Vec<PathBuf> = harness.state().repos.iter().map(|repo| canonical(&repo.path)).collect();
    assert!(tabs.contains(&canonical(&folder.path().join("alpha"))), "alpha opened as a tab");
    assert!(tabs.contains(&canonical(&folder.path().join("beta"))), "beta opened as a tab");
}

#[test]
fn dropping_a_folder_that_is_not_a_repository_offers_to_find_repositories_in_it() {
    let open_now = repository();
    let folder = temp_dir();
    init_repository(&folder.path().join("inner/project"));
    let mut harness = open(open_now.path());
    loaded(&mut harness);

    harness.input_mut().dropped_files.push(Arc::new(Dropped(folder.path().to_path_buf())));
    wait(&mut harness, "the offer", |h| h.query_by_label_contains("isn't a repository").is_some());
    assert_eq!(harness.state().repos.len(), 1, "the folder is not opened as a repository");

    harness.get_by_label("Find").click();
    wait(&mut harness, "the results", |h| h.query_by_label_contains("Found 1 repository").is_some());
    // The one repository found is checked, so it is added as the only choice.
    harness.get_by_label("Add 1 to Recent").click();
    wait(&mut harness, "the dialog to close", |h| h.state().dialog.is_none());
    let recent: Vec<PathBuf> = harness.state().settings.recent.iter().map(|path| canonical(path)).collect();
    assert!(recent.contains(&canonical(&folder.path().join("inner/project"))), "the repository inside the dropped folder is recent");
}
