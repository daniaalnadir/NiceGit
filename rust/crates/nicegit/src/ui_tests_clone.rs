//! Interface tests for the Clone dialog's Options section and the Repository menu items a shallow
//! or sparse checkout offers. They use the helpers in `ui_tests`.

use std::fs;
use std::path::Path;

use egui_kittest::kittest::{NodeT, Queryable};

use crate::ui_tests::*;

/// A repository on `main` with two commits and two folders, `a` and `b`.
fn folders_repository() -> tempfile::TempDir {
    let dir = temp_dir();
    let path = dir.path();
    git(path, &["init", "-q", "-b", "main"]);
    for (key, value) in [("user.name", "Test"), ("user.email", "test@example.invalid"), ("commit.gpgsign", "false")] {
        git(path, &["config", key, value]);
    }
    fs::create_dir_all(path.join("a")).unwrap();
    fs::create_dir_all(path.join("b")).unwrap();
    fs::write(path.join("a").join("one.txt"), "one\n").unwrap();
    fs::write(path.join("b").join("two.txt"), "two\n").unwrap();
    git(path, &["add", "."]);
    git(path, &["commit", "-q", "-m", "Add two folders"]);
    fs::write(path.join("a").join("one.txt"), "one\nmore\n").unwrap();
    git(path, &["commit", "-qam", "Edit the first folder"]);
    dir
}

fn commit_count(directory: &Path) -> usize {
    git(directory, &["rev-list", "--count", "HEAD"]).parse().expect("a commit count")
}

#[test]
fn clone_options_make_a_shallow_sparse_clone_that_can_be_completed() {
    let source = folders_repository();
    let parent = temp_dir();
    let destination = parent.path().join("cloned");
    let mut harness = open(source.path());
    loaded(&mut harness);

    repository_menu(&mut harness, &folder_name(source.path()), "Clone repository");
    wait(&mut harness, "the clone dialog", |h| h.state().dialog.is_some());
    type_into(&mut harness, "Repository URL or local path", &source.path().display().to_string());
    type_into(&mut harness, "Destination folder", &destination.display().to_string());

    // The Options section is collapsed until it is opened.
    harness.get_by_label("Options").click();
    settle(&mut harness);
    harness.get_by_label_contains("Only the latest N commits").click();
    settle(&mut harness);
    harness.get_by_label_contains("Only these folders").click();
    settle(&mut harness);
    type_into(&mut harness, "Folders, one per line", "a");
    settle(&mut harness);

    harness.get_by_label("OK").click();
    wait(&mut harness, "the clone to open", |h| {
        h.state().repo().is_some_and(|r| r.snapshot.as_ref().is_some_and(|s| s.name == "cloned")) && h.state().busy.is_none()
    });
    wait(&mut harness, "the new checkout's shape to be read", |h| {
        h.state().repo().and_then(|r| r.shape).is_some_and(|shape| shape.shallow && shape.sparse)
    });

    assert_eq!(harness.state().repos.len(), 2, "the clone opens in a new tab");
    assert_eq!(git(&destination, &["rev-parse", "--is-shallow-repository"]), "true");
    assert_eq!(commit_count(&destination), 1, "only the latest commit is present");
    assert!(destination.join("a").join("one.txt").is_file(), "the chosen folder is checked out");
    assert!(!destination.join("b").exists(), "the other folder is not checked out");

    // The shallow checkout offers the full history, which brings the earlier commit in.
    repository_menu(&mut harness, "cloned", "Fetch full history");
    idle(&mut harness);
    assert_eq!(commit_count(&destination), 2, "the full history is fetched");
    wait(&mut harness, "the checkout to stop being shallow", |h| {
        h.state().repo().and_then(|r| r.shape).is_some_and(|shape| !shape.shallow)
    });
}

#[test]
fn the_clone_options_explain_themselves_and_block_an_invalid_folder() {
    let source = folders_repository();
    let parent = temp_dir();
    let destination = parent.path().join("refused");
    let mut harness = open(source.path());
    loaded(&mut harness);

    repository_menu(&mut harness, &folder_name(source.path()), "Clone repository");
    wait(&mut harness, "the clone dialog", |h| h.state().dialog.is_some());
    type_into(&mut harness, "Repository URL or local path", &source.path().display().to_string());
    type_into(&mut harness, "Destination folder", &destination.display().to_string());
    harness.get_by_label("Options").click();
    settle(&mut harness);
    assert!(
        harness.query_by_label_contains("Faster for large projects. You can fetch the full history later.").is_some(),
        "the history choice explains itself"
    );
    assert!(
        harness.query_by_label_contains("Smaller and faster; Git downloads older file versions").is_some(),
        "the file contents choice explains itself"
    );

    harness.get_by_label_contains("Only these folders").click();
    settle(&mut harness);
    type_into(&mut harness, "Folders, one per line", "../outside");
    settle(&mut harness);
    assert!(harness.query_by_label_contains("cannot contain").is_some(), "the folder problem is shown beside the field");
    assert!(harness.get_by_label("OK").accesskit_node().is_disabled(), "OK is held back while the folder is invalid");
    assert!(!destination.exists(), "nothing was cloned");
}
