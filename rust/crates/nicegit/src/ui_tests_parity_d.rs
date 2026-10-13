//! Interface tests for the remaining details of Mac app features (round two), built on the helpers in
//! `ui_tests`.

#![allow(unused_imports)]

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::kittest::{By, NodeT, Queryable};
use egui_kittest::Harness;
use egui_phosphor::regular as icon;

use crate::app::NiceGitApp;
use crate::ui_tests::*;

type App = Harness<'static, NiceGitApp>;

/// Whether any widget's label contains `text`.
fn shows(harness: &App, text: &str) -> bool {
    harness.query_all_by_label_contains(text).next().is_some()
}

/// Opens a collapsed sidebar header by its disclosure triangle, which sits just left of its label.
fn open_header(harness: &mut App, label: &str) {
    let rect = harness.get_by_label_contains(label).rect();
    let pos = egui::pos2(rect.min.x - 8.0, rect.center().y);
    harness.hover_at(pos);
    harness.step();
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    }
    settle(harness);
}

/// Replaces the text in a field: selects everything in it, then types over the selection.
fn replace_text(harness: &mut App, label: &str, text: &str) {
    harness.get_by_role_and_label(Role::TextInput, label).focus();
    harness.step();
    harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
    harness.step();
    harness.get_by_role_and_label(Role::TextInput, label).type_text(text);
    harness.run_steps(3);
}

/// A bare repository that stands in for a remote server.
fn bare_remote() -> tempfile::TempDir {
    let remote = temp_dir();
    git(remote.path(), &["init", "-q", "--bare", "-b", "main"]);
    remote
}

#[test]
fn create_a_repository_from_the_empty_window() {
    let parent = temp_dir();
    let folder = parent.path().join("fresh-project");
    std::fs::create_dir(&folder).unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_eframe(|cc| NiceGitApp::new(cc, None));
    settle(&mut harness);
    assert!(harness.state().repos.is_empty(), "no repository is open at first");

    crate::file_dialog::answer_next(&folder);
    harness.get_by_label_contains("New repository").click();
    wait(&mut harness, "the new repository to open", |h| h.state().snapshot().is_some() && h.state().busy.is_none());
    assert!(folder.join(".git").is_dir(), "Git initialised the chosen folder");
    assert_eq!(git(&folder, &["rev-parse", "--is-inside-work-tree"]), "true");
    assert_eq!(harness.state().repos.len(), 1, "the new repository opened in a tab");
    assert_eq!(harness.state().snapshot().map(|s| s.name.clone()), Some(folder_name(&folder)));
}

#[test]
fn clone_from_a_file_url_through_the_dialog() {
    let source = repository();
    let remote = bare_remote();
    let remote_path = remote.path().to_string_lossy().into_owned();
    git(source.path(), &["push", "-q", &remote_path, "main"]);
    let url = format!("file://{remote_path}");
    let target_parent = temp_dir();
    let destination = target_parent.path().join("from-url");
    let mut harness = open(source.path());
    loaded(&mut harness);

    repository_menu(&mut harness, &folder_name(source.path()), "Clone repository");
    wait(&mut harness, "the clone dialog", |h| h.state().dialog.is_some());
    type_into(&mut harness, "Repository URL or local path", &url);
    type_into(&mut harness, "Destination folder", &destination.display().to_string());
    harness.get_by_label("OK").click();
    wait(&mut harness, "the clone to open", |h| {
        h.state().repo().is_some_and(|r| r.snapshot.as_ref().is_some_and(|s| s.name == "from-url")) && h.state().busy.is_none()
    });
    assert_eq!(git(&destination, &["log", "-1", "--format=%s"]), "Add a second line");
    assert_eq!(git(&destination, &["config", "--get", "remote.origin.url"]), url, "the clone remembers the file URL");
    assert_eq!(harness.state().repos.len(), 2, "the clone opens in a new tab");
}

#[test]
fn rename_a_branch_from_its_menu() {
    let repo = repository();
    let path = repo.path();
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label("feature").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Rename").click();
    wait(&mut harness, "the rename dialog", |h| h.state().dialog.is_some());
    replace_text(&mut harness, "New name", "renamed-feature");
    harness.get_by_label("OK").click();
    idle(&mut harness);

    wait(&mut harness, "the rename", |_| git(path, &["branch", "--list", "--format=%(refname:short)"]) == "main\nrenamed-feature");
    assert!(
        harness.state().snapshot().is_some_and(|s| s.branches.iter().any(|b| b.name == "renamed-feature")),
        "the sidebar shows the new name"
    );
}

#[test]
fn delete_a_merged_branch_from_its_menu() {
    let repo = repository();
    let path = repo.path();
    // `done` points at main's tip, so every commit on it is already in main.
    git(path, &["branch", "done"]);
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label("done").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Delete…").click();
    wait(&mut harness, "the delete confirmation", |h| h.state().dialog.is_some());
    assert!(shows(&harness, "merged into the current branch"), "the confirmation says the branch is merged");
    harness.get_by_label("Delete").click();
    idle(&mut harness);

    wait(&mut harness, "the branch to be deleted", |_| git(path, &["branch", "--list", "--format=%(refname:short)"]) == "feature\nmain");
    assert!(git(path, &["branch", "--list", "done"]).is_empty(), "git no longer lists the branch");
}

#[test]
fn check_out_a_remote_branch_creates_a_tracking_branch() {
    let repo = repository();
    let path = repo.path();
    let remote = bare_remote();
    let remote_path = remote.path().to_string_lossy().into_owned();
    git(path, &["remote", "add", "origin", &remote_path]);
    git(path, &["push", "-q", "origin", "main"]);
    // Another clone publishes a branch that this repository has never checked out.
    let other = temp_dir();
    let clone = other.path().join("publisher");
    git(other.path(), &["clone", "-q", &remote_path, &clone.to_string_lossy()]);
    git(&clone, &["config", "user.name", "Test"]);
    git(&clone, &["config", "user.email", "test@example.invalid"]);
    git(&clone, &["config", "commit.gpgsign", "false"]);
    git(&clone, &["switch", "-q", "-c", "review"]);
    std::fs::write(clone.join("review.txt"), "for review\n").unwrap();
    git(&clone, &["add", "review.txt"]);
    git(&clone, &["commit", "-q", "-m", "Ready for review"]);
    git(&clone, &["push", "-q", "origin", "review"]);
    git(path, &["fetch", "-q", "origin"]);
    assert!(git(path, &["branch", "--list", "review"]).is_empty(), "the repository has no local review branch yet");

    let mut harness = open(path);
    loaded(&mut harness);
    open_header(&mut harness, "  origin");
    wait(&mut harness, "the remote branch in the sidebar", |h| h.query_by_label("review").is_some());

    // The harness does not produce egui's double-click reliably, so this uses the row's
    // "Check out" item, which runs the same checkout.
    harness.get_by_label("review").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Check out").click();
    idle(&mut harness);
    wait(&mut harness, "the new local branch to be checked out", |_| git(path, &["branch", "--show-current"]) == "review");
    assert_eq!(git(path, &["rev-parse", "--abbrev-ref", "review@{upstream}"]), "origin/review", "the local branch tracks the remote");
    assert_eq!(git(path, &["config", "--get", "branch.review.remote"]), "origin");
    assert!(path.join("review.txt").exists(), "the remote branch's commit is checked out");
}

#[test]
fn rename_a_remote_and_change_its_fetch_address_in_settings() {
    let repo = repository();
    let path = repo.path();
    let original = bare_remote();
    let moved = bare_remote();
    let moved_path = moved.path().to_string_lossy().into_owned();
    git(path, &["remote", "add", "origin", &original.path().to_string_lossy()]);
    let mut harness = open(path);
    loaded(&mut harness);

    repository_menu(&mut harness, &folder_name(path), "Repository settings");
    wait(&mut harness, "the settings window", |h| shows(h, "  Remotes"));
    harness.get_by_label_contains("  Remotes").click();
    wait(&mut harness, "the origin card", |h| h.query_by_label(&format!("{}  Edit", icon::PENCIL_SIMPLE)).is_some());
    settle(&mut harness);
    harness.get_by_label(&format!("{}  Edit", icon::PENCIL_SIMPLE)).click();
    wait(&mut harness, "the edit form", |h| h.query_by_role_and_label(Role::TextInput, "Edit remote name").is_some());
    settle(&mut harness);

    replace_text(&mut harness, "Edit remote name", "upstream");
    replace_text(&mut harness, "Edit fetch URL", &moved_path);
    settle(&mut harness);
    harness.get_by_label("Save remote").click();
    idle(&mut harness);

    wait(&mut harness, "the rename and new address", |_| {
        git(path, &["remote"]) == "upstream" && git(path, &["remote", "get-url", "upstream"]) == moved_path
    });
    assert_eq!(git(path, &["remote", "get-url", "--push", "upstream"]), moved_path, "pushes go to the new address too");
    wait(&mut harness, "the renamed remote in the snapshot", |h| {
        h.state().snapshot().is_some_and(|s| s.remotes.iter().any(|r| r == "upstream") && !s.remotes.iter().any(|r| r == "origin"))
    });
}

#[test]
fn push_a_tag_to_a_remote_then_delete_it_there() {
    let repo = repository();
    let path = repo.path();
    let remote = bare_remote();
    git(path, &["remote", "add", "origin", &remote.path().to_string_lossy()]);
    git(path, &["push", "-q", "origin", "main"]);
    git(path, &["tag", "v1.0"]);
    let remote_tags = || git(path, &["ls-remote", "--tags", "origin"]);
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label_contains("TAGS").click();
    settle(&mut harness);
    harness.get_by_label("v1.0").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Push to origin").click();
    idle(&mut harness);
    wait(&mut harness, "the tag on the remote", |_| remote_tags().contains("refs/tags/v1.0"));

    harness.get_by_label("v1.0").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Delete from origin").click();
    wait(&mut harness, "the remote delete confirmation", |h| h.state().dialog.is_some());
    settle(&mut harness);
    harness.get_by_label("Delete from remote").click();
    idle(&mut harness);

    wait(&mut harness, "the tag to leave the remote", |_| !remote_tags().contains("refs/tags/v1.0"));
    assert!(git(path, &["tag", "--list"]).contains("v1.0"), "the local tag is kept");
}

#[test]
fn delete_a_branch_from_its_remote() {
    let repo = repository();
    let path = repo.path();
    let remote = crate::ui_tests_more::publish_to_bare(path);
    git(path, &["push", "-q", "origin", "feature"]);
    // Without its local copy, the sidebar shows one row named feature, under origin.
    git(path, &["branch", "-d", "feature"]);
    let remote_has_feature = || !git(remote.path(), &["for-each-ref", "--format=%(refname)", "refs/heads/feature"]).is_empty();
    assert!(remote_has_feature(), "the remote has feature before the delete");

    let mut harness = open(path);
    loaded(&mut harness);
    open_header(&mut harness, "  origin");
    wait(&mut harness, "the remote branch in the sidebar", |h| h.query_by_label("feature").is_some());

    harness.get_by_label("feature").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Delete origin/feature").click();
    wait(&mut harness, "the remote branch delete confirmation", |h| h.state().dialog.is_some());
    settle(&mut harness);
    harness.get_by_label("Delete from remote").click();
    idle(&mut harness);

    wait(&mut harness, "the branch to leave the remote", |_| !remote_has_feature());
    assert!(git(path, &["branch", "--remotes", "--list", "origin/feature"]).is_empty(), "the remote-tracking branch is gone too");
}

/// Presses and releases the primary button at `at`, as one click.
fn click_at(harness: &mut Harness<'static, NiceGitApp>, at: egui::Pos2) {
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    }
}

#[test]
fn double_click_a_branch_in_the_sidebar_to_check_it_out() {
    let repo = repository();
    let path = repo.path();
    // The harness gives each input event its own frame, so frames must be short for two clicks
    // to land within the system's double-click interval, as a person's would.
    let owned = path.to_path_buf();
    let mut harness =
        Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(0.02).build_eframe(move |cc| NiceGitApp::new(cc, Some(owned)));
    loaded(&mut harness);

    let at = harness.get_by_label("feature").rect().center();
    harness.hover_at(at);
    harness.step();
    click_at(&mut harness, at);
    harness.step();
    click_at(&mut harness, at);
    harness.step();
    idle(&mut harness);
    wait(&mut harness, "the checkout", |_| git(path, &["branch", "--show-current"]) == "feature");
}

#[test]
fn enterprise_remote_offers_open_on_github_once_listed() {
    let repo = repository();
    let path = repo.path();
    git(path, &["remote", "add", "origin", "https://git.example.com/acme/app.git"]);
    let hash = git(path, &["rev-parse", "HEAD"]);
    let mut harness = open(path);
    loaded(&mut harness);
    crate::ui::changes::OPENED_URLS.with(|opened| opened.borrow_mut().clear());

    // Until its host is listed in Settings, the enterprise remote gets no GitHub link.
    harness.get_by_label("Add a second line").click_secondary();
    settle(&mut harness);
    assert!(!shows(&harness, "Open on GitHub"), "an unlisted host gives no link");
    harness.key_press(Key::Escape);
    settle(&mut harness);

    harness.state_mut().settings.github_enterprise_hosts = vec!["git.example.com".to_string()];
    harness.get_by_label("Add a second line").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Open on GitHub").click();
    settle(&mut harness);
    let opened = crate::ui::changes::OPENED_URLS.with(|opened| opened.borrow().clone());
    assert_eq!(opened, vec![format!("https://git.example.com/acme/app/commit/{hash}")]);
}

#[test]
fn settings_window_rejects_an_invalid_enterprise_host() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    harness.state_mut().show_settings = true;
    settle(&mut harness);

    type_into(&mut harness, "GitHub Enterprise hosts", "Git.Example.com");
    harness.key_press(Key::Enter);
    type_into(&mut harness, "GitHub Enterprise hosts", "not a host");
    settle(&mut harness);

    // The valid hostname is saved, lower-cased; the invalid line stays in the box and is named.
    assert_eq!(harness.state().settings.github_enterprise_hosts, vec!["git.example.com".to_string()]);
    assert!(shows(&harness, "Not saved"), "the invalid line gets a warning");
    assert!(shows(&harness, "not a host"), "the warning names the invalid line");
}

// MARK: Go to file

/// The shared repository plus `src/guide.md`, committed on top of its two commits.
fn repository_with_a_nested_file() -> tempfile::TempDir {
    let repo = repository();
    std::fs::create_dir(repo.path().join("src")).unwrap();
    std::fs::write(repo.path().join("src").join("guide.md"), "# Guide\n").unwrap();
    git(repo.path(), &["add", "src/guide.md"]);
    git(repo.path(), &["commit", "-q", "-m", "Add a guide"]);
    repo
}

/// Opens the Go to file finder with Command-P, types `query`, and waits until the listing shows `file`.
fn open_finder_and_type(harness: &mut App, query: &str, file: &str) {
    harness.key_press_modifiers(Modifiers::COMMAND, Key::P);
    wait(harness, "the file finder", |h| h.state().go_to_file.is_some());
    harness.event(egui::Event::Text(query.into()));
    wait(harness, "the file listing", |h| shows(h, file));
}

#[test]
fn go_to_file_opens_history() {
    let repo = repository_with_a_nested_file();
    let mut harness = open(repo.path());
    loaded(&mut harness);

    open_finder_and_type(&mut harness, "guide", "guide.md");
    harness.key_press(Key::Enter);
    wait(&mut harness, "the file history window", |h| h.state().tools.iter().any(|t| t.id() == "file-history\0src/guide.md"));
    assert!(harness.state().go_to_file.is_none(), "choosing a file closes the finder");
}

#[test]
fn go_to_file_command_enter_opens_blame() {
    let repo = repository_with_a_nested_file();
    let mut harness = open(repo.path());
    loaded(&mut harness);

    open_finder_and_type(&mut harness, "guide", "guide.md");
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    wait(&mut harness, "the blame window", |h| h.state().tools.iter().any(|t| t.id() == "blame\0src/guide.md\0"));
    assert!(!harness.state().tools.iter().any(|t| t.id().starts_with("file-history")), "blame opens instead of history");
}

#[test]
fn go_to_file_escape_closes() {
    let repo = repository_with_a_nested_file();
    let mut harness = open(repo.path());
    loaded(&mut harness);

    open_finder_and_type(&mut harness, "guide", "guide.md");
    harness.key_press(Key::Escape);
    wait(&mut harness, "the finder to close", |h| h.state().go_to_file.is_none());
    assert!(harness.state().tools.is_empty(), "closing the finder opens nothing");
}

#[test]
fn go_to_file_does_nothing_without_a_repository() {
    let mut harness = Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_eframe(|cc| NiceGitApp::new(cc, None));
    settle(&mut harness);
    harness.key_press_modifiers(Modifiers::COMMAND, Key::P);
    settle(&mut harness);
    assert!(harness.state().go_to_file.is_none());
}
