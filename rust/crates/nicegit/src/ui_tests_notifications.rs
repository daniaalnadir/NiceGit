//! Desktop notifications, driven through the window: what is announced when the window is in the
//! background, and what is kept quiet.

use std::path::Path;

use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;

use crate::app::NiceGitApp;
use crate::ui_tests::*;

/// Puts the window in or out of focus, as the operating system reports it.
fn set_focus(harness: &mut Harness<'static, NiceGitApp>, focused: bool) {
    harness.input_mut().viewports.entry(egui::ViewportId::ROOT).or_default().focused = Some(focused);
    harness.run_steps(3);
}

/// Commits one new file on the checkout, so it is ahead of its upstream.
fn commit_ahead_of_upstream(path: &Path) {
    std::fs::write(path.join("ahead.txt"), "ahead of origin\n").unwrap();
    git(path, &["add", "--", "ahead.txt"]);
    git(path, &["commit", "-q", "-m", "Ahead of origin"]);
}

#[test]
fn a_push_that_finishes_in_the_background_is_announced() {
    let repo = repository();
    let path = repo.path();
    let _remote = crate::ui_tests_more::publish_to_bare(path);
    commit_ahead_of_upstream(path);
    let mut harness = open(path);
    loaded(&mut harness);
    set_focus(&mut harness, false);

    // The button names how many commits it will push.
    toolbar_button(&mut harness, "Push 1");
    idle(&mut harness);

    assert_eq!(git(path, &["rev-parse", "HEAD"]), git(path, &["rev-parse", "origin/main"]), "the push reached the remote");
    assert_eq!(
        harness.state().sent_notifications,
        vec![("NiceGit".to_string(), format!("Push finished in {}", folder_name(path)))],
        "one notification names the finished push and its repository"
    );
}

#[test]
fn a_push_in_the_focused_window_is_not_announced() {
    let repo = repository();
    let path = repo.path();
    let _remote = crate::ui_tests_more::publish_to_bare(path);
    commit_ahead_of_upstream(path);
    let mut harness = open(path);
    loaded(&mut harness);
    set_focus(&mut harness, true);

    toolbar_button(&mut harness, "Push 1");
    idle(&mut harness);

    assert_eq!(git(path, &["rev-parse", "HEAD"]), git(path, &["rev-parse", "origin/main"]), "the push reached the remote");
    assert!(harness.state().sent_notifications.is_empty(), "nothing is announced while the window is focused");
}

#[test]
fn a_push_is_not_announced_when_the_setting_is_off() {
    let repo = repository();
    let path = repo.path();
    let _remote = crate::ui_tests_more::publish_to_bare(path);
    commit_ahead_of_upstream(path);
    let mut harness = open(path);
    loaded(&mut harness);
    harness.state_mut().settings.notify_when_finished = false;
    set_focus(&mut harness, false);

    toolbar_button(&mut harness, "Push 1");
    idle(&mut harness);

    assert_eq!(git(path, &["rev-parse", "HEAD"]), git(path, &["rev-parse", "origin/main"]), "the push reached the remote");
    assert!(harness.state().sent_notifications.is_empty(), "the setting keeps every notification quiet");
}

#[test]
fn a_clone_that_finishes_in_the_background_is_announced() {
    let source = repository();
    let target_parent = temp_dir();
    let destination = target_parent.path().join("cloned");
    let mut harness = open(source.path());
    loaded(&mut harness);
    set_focus(&mut harness, false);

    repository_menu(&mut harness, &folder_name(source.path()), "Clone repository");
    wait(&mut harness, "the clone dialog", |h| h.state().dialog.is_some());
    type_into(&mut harness, "Repository URL or local path", &source.path().display().to_string());
    type_into(&mut harness, "Destination folder", &destination.display().to_string());
    harness.get_by_label("OK").click();
    wait(&mut harness, "the clone to open", |h| {
        h.state().repo().is_some_and(|r| r.snapshot.as_ref().is_some_and(|s| s.name == "cloned")) && h.state().busy.is_none()
    });

    assert_eq!(harness.state().sent_notifications, vec![("NiceGit".to_string(), "Clone finished in cloned".to_string())]);
}
