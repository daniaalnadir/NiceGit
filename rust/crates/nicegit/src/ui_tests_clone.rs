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

/// A plain clone through the dialog opens the new repository without any error, including the
/// background reads that follow it. Set NICEGIT_CLONE_URL to clone a real address instead of a
/// local repository; the Windows clone check runs this under several Git signing setups.
#[test]
fn a_plain_clone_opens_without_an_error() {
    let source = folders_repository();
    let parent = temp_dir();
    let destination = parent.path().join("cloned");
    let address = std::env::var("NICEGIT_CLONE_URL").unwrap_or_else(|_| source.path().display().to_string());
    let mut harness = open(source.path());
    loaded(&mut harness);

    repository_menu(&mut harness, &folder_name(source.path()), "Clone repository");
    wait(&mut harness, "the clone dialog", |h| h.state().dialog.is_some());
    type_into(&mut harness, "Repository URL or local path", &address);
    type_into(&mut harness, "Destination folder", &destination.display().to_string());
    harness.get_by_label("OK").click();
    wait(&mut harness, "the clone to finish", |h| {
        h.state().busy.is_none() && (h.state().notice.as_ref().is_some_and(|n| n.is_error) || h.state().repos.len() == 2)
    });
    let failure = |h: &egui_kittest::Harness<'static, crate::app::NiceGitApp>| {
        h.state().notice.as_ref().filter(|n| n.is_error).map(|n| format!("{}\n{}", n.text, n.detail.clone().unwrap_or_default()))
    };
    assert_eq!(failure(&harness), None, "cloning showed an error");
    wait(&mut harness, "the clone to load", |h| h.state().repo().is_some_and(|r| r.snapshot.is_some()) && h.state().busy.is_none());
    // Background reads (history, signatures, snapshots) run after the first frame shows.
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_secs(5) {
        harness.step();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(failure(&harness), None, "opening the clone showed an error");
    assert!(destination.join(".git").exists(), "the clone exists");
}

/// Git rejects an empty `gpg.format` whenever it signs or checks a signature. The error offers to remove that setting from the file Git named, and its text can
/// be copied from a right-click menu.
#[test]
fn an_empty_gpg_format_can_be_removed_from_the_error() {
    let source = folders_repository();
    let settings = temp_dir();
    let file = settings.path().join("gitconfig");
    fs::write(&file, "[user]\n\tname = Someone\n[gpg]\n\tformat =\n").unwrap();
    let error = format!(
        "Git could not run `git log`: error: invalid value for 'gpg.format': ''\nfatal: bad config variable 'gpg.format' in file '{}' at line 4",
        file.display()
    );
    let mut harness = open(source.path());
    loaded(&mut harness);
    {
        let state = harness.state_mut();
        let active = state.active;
        state.repos[active].snapshot = None;
        state.repos[active].error = Some(error.clone());
    }
    harness.step();

    harness.get_by_label_contains("bad config variable").click_secondary();
    wait(&mut harness, "the Copy menu", |h| h.query_by_label("Copy").is_some());
    harness.get_by_label("Copy").click();
    let mut copied = None;
    for _ in 0..5 {
        harness.step();
        for command in &harness.output().platform_output.commands {
            if let egui::OutputCommand::CopyText(text) = command {
                copied = Some(text.clone());
            }
        }
        if copied.is_some() {
            break;
        }
    }
    assert_eq!(copied.as_deref(), Some(error.as_str()));

    wait(&mut harness, "the menu to close", |h| h.query_by_label("Copy").is_none());
    harness.get_by_label("Remove the empty setting").click();
    wait(&mut harness, "the repository to load again", |h| h.state().repo().is_some_and(|r| r.snapshot.is_some() && r.error.is_none()));
    let text = fs::read_to_string(&file).unwrap();
    assert!(text.contains("name = Someone") && !text.contains("format"), "{text}");
}
