//! Interface tests for Bitbucket and Azure DevOps: the sidebar lists a Bitbucket repository's pull requests,
//! the status bar shows an Azure DevOps pull request with its checks, the notices read as notices rather than
//! errors, Azure DevOps hides its issues, and Settings offers the credential rows. The remotes are on the
//! fixed hosts, and the lists are put in place directly, so no request reaches a server.

use egui_kittest::kittest::Queryable;
use egui_phosphor::regular as icon;
use nicegit_core::forge_api::{NEEDS_TOKEN, NO_ISSUE_TRACKER};
use nicegit_core::github::{Checks, GitHubItem, PrState, PullRequestStatus};
use nicegit_core::hosting::Host;

use crate::tools::Task;
use crate::ui::github_section::GitHubList;
use crate::ui_tests::*;

const BITBUCKET_REMOTE: &str = "https://bitbucket.org/team/tool.git";
const AZURE_REMOTE: &str = "https://dev.azure.com/acme/Web/_git/shop";

/// A repository whose `origin` is the given address.
fn repository_with_origin(address: &str) -> tempfile::TempDir {
    let repo = repository();
    git(repo.path(), &["remote", "add", "origin", address]);
    repo
}

/// Reads of the open requests and the status are stopped for the test, so only the injected answers show.
fn stop_reads(harness: &mut egui_kittest::Harness<'static, crate::app::NiceGitApp>) {
    let app = harness.state_mut();
    let active = app.active;
    let repo = &mut app.repos[active];
    let now = std::time::Instant::now();
    repo.open_requests = Some(Task::ready(None));
    repo.open_requests_at = Some(now);
    repo.pr_checked_at = Some(now);
}

#[test]
fn a_bitbucket_pull_request_shows_in_the_sidebar_with_its_host() {
    let repo = repository_with_origin(BITBUCKET_REMOTE);
    let mut harness = open(repo.path());
    loaded(&mut harness);
    idle(&mut harness);
    stop_reads(&mut harness);
    settle(&mut harness);

    harness.get_by_label_contains("PULL REQUESTS").click();
    settle(&mut harness);
    assert!(harness.query_all_by_label_contains("Load from Bitbucket").next().is_some(), "the menu names Bitbucket");

    let item = GitHubItem {
        number: 3,
        title: "Fix the login form".into(),
        url: "https://bitbucket.org/team/tool/pull-requests/3".into(),
        author: Some("ada".into()),
        state: Some("OPEN".into()),
        is_draft: false,
        labels: Vec::new(),
    };
    harness.state_mut().repos[0].github[0] = Some(GitHubList::loaded("origin", vec![item]));
    settle(&mut harness);

    assert!(harness.query_all_by_label_contains("Fix the login form").next().is_some(), "the sidebar lists the pull request");
    assert!(harness.query_all_by_label_contains("bitbucket.org").next().is_some(), "the list says which host it reads");
    assert!(harness.query_by_label("Open GitHub window").is_none(), "the GitHub-only window is not offered for Bitbucket");
}

#[test]
fn an_azure_devops_pull_request_shows_its_checks_in_the_status_bar() {
    let repo = repository_with_origin(AZURE_REMOTE);
    let mut harness = open(repo.path());
    loaded(&mut harness);
    idle(&mut harness);
    stop_reads(&mut harness);
    let status = PullRequestStatus {
        kind: Host::AzureDevOps,
        number: 12,
        title: "Add the Azure status".into(),
        state: PrState::Open,
        url: "https://dev.azure.com/acme/Web/_git/shop/pullrequest/12".into(),
        checks: Checks { passed: 2, failed: 0, pending: 0 },
    };
    harness.state_mut().repos[0].pr_status = Some(("main".into(), Task::ready(Ok(Some(status)))));
    settle(&mut harness);

    assert!(harness.query_all_by_label_contains("PR #12").next().is_some(), "the status bar names the pull request");
    assert!(harness.query_all_by_label_contains(&format!("{} 2", icon::CHECK)).next().is_some(), "and its passed checks");
    crate::ui::changes::OPENED_URLS.with(|opened| opened.borrow_mut().clear());
    harness.get_by_label_contains("PR #12").click();
    settle(&mut harness);
    let opened = crate::ui::changes::OPENED_URLS.with(|opened| opened.borrow().clone());
    assert_eq!(opened, vec!["https://dev.azure.com/acme/Web/_git/shop/pullrequest/12".to_string()], "clicking opens its page");
}

#[test]
fn an_azure_listing_without_a_token_is_a_notice_and_its_issues_are_not_shown() {
    let repo = repository_with_origin(AZURE_REMOTE);
    let mut harness = open(repo.path());
    loaded(&mut harness);
    idle(&mut harness);
    stop_reads(&mut harness);
    settle(&mut harness);

    harness.get_by_label_contains("PULL REQUESTS").click();
    settle(&mut harness);
    assert!(harness.query_all_by_label_contains("Load from Azure DevOps").next().is_some(), "the menu names Azure DevOps");
    assert!(harness.query_by_label_contains("ISSUES").is_none(), "Azure DevOps work items are not listed, so no issues section");

    let mut list = GitHubList::loaded("origin", Vec::new());
    list.notice = Some(NEEDS_TOKEN);
    harness.state_mut().repos[0].github[0] = Some(list);
    settle(&mut harness);
    assert!(
        harness.query_all_by_label("Add a personal access token in Settings to see pull requests.").next().is_some(),
        "the listing says what to add, in muted words"
    );
}

#[test]
fn a_switched_off_bitbucket_issue_tracker_is_a_notice_not_an_error() {
    let repo = repository_with_origin(BITBUCKET_REMOTE);
    let mut harness = open(repo.path());
    loaded(&mut harness);
    idle(&mut harness);
    stop_reads(&mut harness);
    settle(&mut harness);

    harness.get_by_label_contains("ISSUES").click();
    settle(&mut harness);
    let mut list = GitHubList::loaded("origin", Vec::new());
    list.notice = Some(NO_ISSUE_TRACKER);
    harness.state_mut().repos[0].github[1] = Some(list);
    settle(&mut harness);
    assert!(
        harness.query_all_by_label("This repository has no issue tracker on Bitbucket.").next().is_some(),
        "the list says the tracker is off"
    );
    assert!(harness.query_all_by_label_contains("No open issues").next().is_none(), "and does not also say there are none");
}

#[test]
fn the_settings_offer_a_credential_row_for_bitbucket_and_an_organization_for_azure() {
    let repo = repository_with_origin(BITBUCKET_REMOTE);
    let mut harness = open(repo.path());
    loaded(&mut harness);
    idle(&mut harness);
    stop_reads(&mut harness);
    harness.state_mut().show_settings = true;
    settle(&mut harness);
    assert!(harness.query_all_by_label("Username for bitbucket.org").next().is_some(), "the Bitbucket username field");
    assert!(
        harness.query_all_by_label("App password or API token for bitbucket.org").next().is_some(),
        "the Bitbucket secret field is a password"
    );
    assert!(harness.query_all_by_label("Save the credentials for bitbucket.org").next().is_some(), "and its Save button");
    assert!(harness.query_all_by_label("Organization").next().is_some(), "the Azure DevOps organization field");
    assert!(harness.query_all_by_label("Personal access token").next().is_some(), "and its token field");
}
