//! Interface tests for branch checks: a branch with an open pull request shows its checks as a dot on
//! its graph label and beside it in the sidebar, and clicking the dot opens the pull request's Checks
//! tab. The open requests are put in place directly, so nothing reaches GitHub.

use std::collections::HashMap;
use std::sync::Arc;

use egui_kittest::kittest::Queryable;
use nicegit_core::github::{Checks, GitHubRepository};

use crate::git_ext::Forge;
use crate::tools::Task;
use crate::ui::branch_checks::{OpenRequests, RequestCheck};
use crate::ui_tests::*;

/// Puts an open request for `branch` in place, with the given checks, as a read would have left it.
/// The read time is set too, so no read starts while the test runs.
fn put_request(harness: &mut egui_kittest::Harness<'static, crate::app::NiceGitApp>, branch: &str, checks: Checks) {
    let github = GitHubRepository { host: "github.com".into(), owner: "acme".into(), name: "app".into() };
    let mut by_ref = HashMap::new();
    by_ref.insert(
        format!("refs/heads/{branch}"),
        RequestCheck { number: 12, title: "Add the status bar".into(), url: "https://github.com/acme/app/pull/12".into(), checks },
    );
    let requests = Arc::new(OpenRequests { forge: Forge::GitHub(github), checks: by_ref });
    let app = harness.state_mut();
    let active = app.active;
    let repo = &mut app.repos[active];
    repo.open_requests = Some(Task::ready(Some(requests)));
    repo.open_requests_at = Some(std::time::Instant::now());
}

#[test]
fn a_failed_check_shows_a_dot_on_the_branch_in_the_graph_and_sidebar() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    idle(&mut harness);
    let current = harness.state().snapshot().expect("the repository is loaded").current_branch.clone();
    put_request(&mut harness, &current, Checks { passed: 2, failed: 1, pending: 0 });
    settle(&mut harness);

    // One dot on the graph's label, and one beside the branch in the sidebar.
    assert_eq!(harness.query_all_by_label("Checks: 1 failed").count(), 2, "the graph and the sidebar both show the dot");
    assert!(has_label(&harness, &format!("Branch label {current}")), "the branch's graph label is still there");
}

#[test]
fn clicking_a_check_dot_opens_the_pull_request_window() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    idle(&mut harness);
    let current = harness.state().snapshot().expect("the repository is loaded").current_branch.clone();
    put_request(&mut harness, &current, Checks { passed: 2, failed: 1, pending: 0 });
    settle(&mut harness);

    harness.get_all_by_label("Checks: 1 failed").next().expect("the dot is shown").click();
    wait(&mut harness, "the pull request window", |h| h.state().tools.iter().any(|t| t.title() == "Pull request #12"));
}

#[test]
fn no_check_dot_is_shown_when_the_setting_is_off() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    idle(&mut harness);
    let current = harness.state().snapshot().expect("the repository is loaded").current_branch.clone();
    put_request(&mut harness, &current, Checks { passed: 0, failed: 1, pending: 0 });
    harness.state_mut().settings.show_branch_checks = false;
    settle(&mut harness);

    assert_eq!(harness.query_all_by_label("Checks: 1 failed").count(), 0, "the setting hides every dot");
}
