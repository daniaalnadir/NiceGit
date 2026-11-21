//! Interface tests for Gitea and Forgejo: the sidebar lists a repository's pull requests and the status
//! bar shows the current branch's pull request with its checks. The remote is on a `.test` host that
//! is listed in Settings, and the lists are put in place directly, so no request reaches a server.

use std::collections::HashMap;
use std::sync::Arc;

use egui_kittest::kittest::Queryable;
use egui_phosphor::regular as icon;
use nicegit_core::github::{Checks, GitHubItem, OpenPullRequest, PrState, PullRequestStatus};
use nicegit_core::hosting::Host;

use crate::git_ext::Forge;
use crate::tools::Task;
use crate::ui::branch_checks::{OpenRequests, RequestCheck};
use crate::ui::github_section::GitHubList;
use crate::ui_tests::*;

/// The host of the test remote. It is listed as a Gitea host, so its pull requests are read as Gitea's.
const HOST: &str = "forge.example.test";

/// A repository whose `origin` is on the listed Gitea host, with that host added to Settings.
fn gitea_repository() -> tempfile::TempDir {
    let repo = repository();
    git(repo.path(), &["remote", "add", "origin", &format!("https://{HOST}/acme/app.git")]);
    repo
}

/// Reads of the open requests and the status are stopped for the test, so only the injected answers show.
fn stop_reads(harness: &mut egui_kittest::Harness<'static, crate::app::NiceGitApp>) {
    let app = harness.state_mut();
    app.settings.gitea_hosts = vec![HOST.to_string()];
    let active = app.active;
    let repo = &mut app.repos[active];
    let now = std::time::Instant::now();
    repo.open_requests = Some(Task::ready(None));
    repo.open_requests_at = Some(now);
    repo.pr_checked_at = Some(now);
}

/// The Gitea pull request #7 as the sidebar and status bar show it, with two passing checks.
fn pull_request() -> PullRequestStatus {
    PullRequestStatus {
        kind: Host::Gitea,
        number: 7,
        title: "Add the Gitea status".into(),
        state: PrState::Open,
        url: format!("https://{HOST}/acme/app/pulls/7"),
        checks: Checks { passed: 2, failed: 0, pending: 0 },
    }
}

#[test]
fn a_gitea_pull_request_shows_in_the_sidebar_and_its_checks_in_the_status_bar() {
    let repo = gitea_repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    idle(&mut harness);
    stop_reads(&mut harness);
    settle(&mut harness);

    // Before anything is loaded, the sidebar offers to load from the Gitea host, not from GitHub.
    harness.get_by_label_contains("PULL REQUESTS").click();
    settle(&mut harness);
    assert!(harness.query_all_by_label_contains("Load from Gitea / Forgejo").next().is_some(), "the menu names the Gitea host");

    // The listing the sidebar reads, and the status of the current branch, as a read would leave them.
    let item = GitHubItem {
        number: 7,
        title: "Add the Gitea status".into(),
        url: format!("https://{HOST}/acme/app/pulls/7"),
        author: Some("ada".into()),
        state: Some("OPEN".into()),
        is_draft: false,
        labels: Vec::new(),
    };
    harness.state_mut().repos[0].github[0] = Some(GitHubList::loaded("origin", vec![item]));
    harness.state_mut().repos[0].pr_status = Some(("main".into(), Task::ready(Ok(Some(pull_request())))));
    settle(&mut harness);

    assert!(harness.query_all_by_label_contains("Add the Gitea status").next().is_some(), "the sidebar lists the pull request");
    assert!(harness.query_all_by_label_contains(HOST).next().is_some(), "the list says which host it reads");
    assert!(harness.query_all_by_label_contains("PR #7").next().is_some(), "the status bar names the pull request");
    assert!(harness.query_all_by_label_contains(&format!("{} 2", icon::CHECK)).next().is_some(), "and its passed checks");
    assert!(harness.query_by_label("Open GitHub window").is_none(), "the GitHub-only window is not offered for Gitea");
}

#[test]
fn clicking_the_gitea_pull_request_in_the_status_bar_opens_its_page() {
    let repo = gitea_repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    idle(&mut harness);
    stop_reads(&mut harness);
    harness.state_mut().repos[0].pr_status = Some(("main".into(), Task::ready(Ok(Some(pull_request())))));
    settle(&mut harness);

    crate::ui::changes::OPENED_URLS.with(|opened| opened.borrow_mut().clear());
    harness.get_by_label_contains("PR #7").click();
    settle(&mut harness);
    let opened = crate::ui::changes::OPENED_URLS.with(|opened| opened.borrow().clone());
    assert_eq!(opened, vec![format!("https://{HOST}/acme/app/pulls/7")], "clicking opens the pull request's page");
}

#[test]
fn a_gitea_check_dot_is_shown_on_the_branch_and_opens_the_request_page() {
    let repo = gitea_repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    idle(&mut harness);
    stop_reads(&mut harness);
    let forge = Forge::Gitea(nicegit_core::hosting::HostedRepository {
        host: HOST.into(),
        kind: Host::Gitea,
        owner_path: "acme".into(),
        name: "app".into(),
    });
    let request = OpenPullRequest {
        number: 7,
        title: "Add the Gitea status".into(),
        url: format!("https://{HOST}/acme/app/pulls/7"),
        author: Some("ada".into()),
        head_branch: "main".into(),
        is_cross_repository: false,
        is_draft: false,
        review_decision: None,
        review_requests: Vec::new(),
        updated_at: 0,
        checks: Checks { passed: 1, failed: 1, pending: 0 },
    };
    let check = RequestCheck { number: 7, title: request.title.clone(), url: request.url.clone(), checks: request.checks, is_draft: false };
    let mut by_ref = HashMap::new();
    by_ref.insert("refs/heads/main".to_string(), check);
    let requests = Arc::new(OpenRequests { forge, checks: by_ref });
    harness.state_mut().repos[0].open_requests = Some(Task::ready(Some(requests)));
    settle(&mut harness);

    assert!(harness.query_all_by_label("Checks: 1 failed").count() >= 1, "the dot shows the failed check");
    crate::ui::changes::OPENED_URLS.with(|opened| opened.borrow_mut().clear());
    harness.get_all_by_label("Checks: 1 failed").next().expect("the dot is shown").click();
    settle(&mut harness);
    let opened = crate::ui::changes::OPENED_URLS.with(|opened| opened.borrow().clone());
    assert_eq!(opened, vec![format!("https://{HOST}/acme/app/pulls/7")], "the dot opens the request's page");
}

#[test]
fn the_gitea_settings_offer_a_token_row_for_each_public_host_and_the_listed_ones() {
    let repo = gitea_repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    idle(&mut harness);
    stop_reads(&mut harness);
    harness.state_mut().show_settings = true;
    settle(&mut harness);
    assert!(harness.query_all_by_label("Token for codeberg.org").next().is_some(), "codeberg.org has a token row");
    assert!(harness.query_all_by_label("Token for gitea.com").next().is_some(), "gitea.com has a token row");
    assert!(harness.query_all_by_label(&format!("Token for {HOST}")).next().is_some(), "a listed host has a token row");
    assert!(harness.query_all_by_label("Gitea and Forgejo hosts").next().is_some(), "the host list is offered");
}
