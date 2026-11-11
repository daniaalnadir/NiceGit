//! Interface tests for the update notice, the update controls in Settings, and the feedback links.
//! No test reaches the network: a check is answered by a stand-in, and links are only recorded.

use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;

use crate::ui_tests::*;
use crate::updates::RELEASES_PAGE;

fn opened() -> Vec<String> {
    crate::ui::changes::OPENED_URLS.with(|opened| opened.borrow().clone())
}

fn clear_opened() {
    crate::ui::changes::OPENED_URLS.with(|opened| opened.borrow_mut().clear());
}

fn notice_shown(harness: &egui_kittest::Harness<'static, crate::app::NiceGitApp>) -> bool {
    harness.query_by_label_contains("is available").is_some()
}

#[test]
fn a_newer_release_shows_the_notice_and_download_opens_the_releases_page() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    assert!(!notice_shown(&harness), "no notice before a check has found a newer version");

    harness.state_mut().receive_latest_release("v2.1.0");
    wait(&mut harness, "the update notice", |h| h.query_by_label("NiceGit 2.1.0 is available").is_some());

    clear_opened();
    harness.get_by_label("Download").click();
    settle(&mut harness);
    assert_eq!(opened(), vec![RELEASES_PAGE.to_string()], "Download opens the releases page");
    assert!(notice_shown(&harness), "Download leaves the notice in place");
}

#[test]
fn dismiss_hides_the_notice_and_remembers_only_that_version() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    harness.state_mut().receive_latest_release("v2.1.0");
    wait(&mut harness, "the update notice", |h| h.query_by_label("NiceGit 2.1.0 is available").is_some());

    harness.get_by_label("Dismiss").click();
    settle(&mut harness);
    assert!(!notice_shown(&harness), "dismissed");
    assert_eq!(harness.state().settings.dismissed_update.as_deref(), Some("2.1.0"));

    // The same version found again stays hidden; a later one shows again.
    harness.state_mut().receive_latest_release("v2.1.0");
    settle(&mut harness);
    assert!(!notice_shown(&harness), "the dismissed version stays hidden");
    harness.state_mut().receive_latest_release("v2.2.0");
    wait(&mut harness, "the later version", |h| h.query_by_label("NiceGit 2.2.0 is available").is_some());
}

#[test]
fn an_equal_or_older_release_shows_nothing_and_is_reported_as_up_to_date() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);

    for tag in ["v2.0.0", "1.9.9"] {
        harness.state_mut().receive_latest_release(tag);
        settle(&mut harness);
        assert!(!notice_shown(&harness), "{tag} is not newer than this build");
        assert_eq!(harness.state().pending_update(), None);
        assert_eq!(harness.state().update_status, Some(crate::updates::UpdateStatus::UpToDate));
    }
}

#[test]
fn a_tag_without_a_plain_version_is_reported_and_shows_no_notice() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);

    harness.state_mut().receive_latest_release("nightly");
    settle(&mut harness);
    assert!(!notice_shown(&harness));
    assert!(matches!(harness.state().update_status, Some(crate::updates::UpdateStatus::Failed(_))));
}

#[test]
fn the_check_for_new_versions_box_turns_checks_on_and_off() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    assert!(harness.state().settings.check_for_updates, "checks start on");

    harness.state_mut().show_settings = true;
    wait(&mut harness, "settings", |h| h.query_by_label("Check for new versions").is_some());
    harness.get_by_label("Check for new versions").click();
    settle(&mut harness);
    assert!(!harness.state().settings.check_for_updates);

    harness.get_by_label("Check for new versions").click();
    settle(&mut harness);
    assert!(harness.state().settings.check_for_updates);
}

#[test]
fn check_now_runs_in_the_background_and_reports_why_it_could_not_check() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    harness.state_mut().show_settings = true;
    wait(&mut harness, "settings", |h| h.query_by_label("Check now").is_some());

    clear_opened();
    harness.get_by_label("Check now").click();
    // Tests make no network request: the stand-in check fails at once, and Settings says so.
    wait(&mut harness, "the check's answer", |h| h.query_by_label_contains("Could not check:").is_some());
    assert!(harness.state().update_check.is_none(), "the check has finished");
    assert!(opened().is_empty(), "nothing was opened");
}

#[test]
fn report_a_problem_and_request_a_feature_open_their_issue_forms() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    harness.state_mut().show_settings = true;
    wait(&mut harness, "settings", |h| h.query_by_label("Report a problem").is_some());

    clear_opened();
    harness.get_by_label("Report a problem").click();
    settle(&mut harness);
    let report = opened();
    assert_eq!(report.len(), 1);
    assert!(report[0].starts_with("https://github.com/daniaalnadir/NiceGit/issues/new?template=bug_report.md&body="));
    assert!(report[0].contains(env!("CARGO_PKG_VERSION")), "the report names this version");
    assert!(!report[0].contains('\n'));

    clear_opened();
    harness.get_by_label("Request a feature").click();
    settle(&mut harness);
    assert_eq!(opened(), vec!["https://github.com/daniaalnadir/NiceGit/issues/new?template=feature_request.md".to_string()]);
}

#[test]
fn the_palette_lists_report_a_problem() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);

    clear_opened();
    harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::P);
    wait(&mut harness, "the palette", |h| h.state().palette.is_some());
    harness.event(egui::Event::Text("report a problem".into()));
    harness.step();
    harness.key_press(Key::Enter);
    settle(&mut harness);
    let urls = opened();
    assert_eq!(urls.len(), 1, "{urls:?}");
    assert!(urls[0].contains("template=bug_report.md"));
}
