//! Interface tests for the review comments of a pull request's Files tab. The detail, threads, and
//! diff are decoded from fixtures and handed to the window, so nothing reaches GitHub. The tests
//! click and type through the window, and stop at each confirmation: no write is ever run.

use egui::accesskit::Role;
use egui::Key;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use nicegit_core::github::{decode_pull_request_detail, GitHubRepository};
use nicegit_core::github_review::{decode_review_threads, file_diffs};

use crate::app::NiceGitApp;
use crate::tools::pull_request_detail::PullRequestDetailWindow;
use crate::ui_tests::*;

fn shows(harness: &Harness<'static, NiceGitApp>, text: &str) -> bool {
    harness.query_all_by_label_contains(text).next().is_some()
}

/// Pull request 12 as `gh pr view` prints it, with one changed file.
const PULL_REQUEST_JSON: &str = r#"{"number": 12, "title": "Add the status bar", "body": "",
    "author": {"login": "octocat"}, "state": "OPEN", "isDraft": false, "baseRefName": "main",
    "headRefName": "status-bar", "headRefOid": "0123456789abcdef0123456789abcdef01234567",
    "url": "https://github.com/acme/app/pull/12",
    "createdAt": "2026-01-02T03:04:05Z", "updatedAt": "2026-01-02T03:04:05Z", "reviewDecision": null,
    "mergeable": "MERGEABLE", "additions": 1, "deletions": 0, "changedFiles": 1, "commits": [],
    "files": [{"path": "src/status_bar.rs", "additions": 1, "deletions": 0}],
    "reviews": [], "comments": [], "statusCheckRollup": [], "labels": []}"#;

/// The diff of the one changed file: the new line 2 is the added helper.
const DIFF: &str = "diff --git a/src/status_bar.rs b/src/status_bar.rs\n--- a/src/status_bar.rs\n+++ b/src/status_bar.rs\n\
     @@ -1 +1,2 @@\n fn draw() {}\n+fn helper() {}\n";

/// An unresolved thread on the added line 2, and a resolved one on line 1 with a reply.
const THREADS_JSON: &str = r#"{"data": {"repository": {"pullRequest": {"reviewThreads": {"nodes": [
    {"id": "PRRT_kwDOTest1", "isResolved": false, "isOutdated": false, "path": "src/status_bar.rs",
     "line": 2, "originalLine": 2, "startLine": null, "diffSide": "RIGHT",
     "comments": {"nodes": [{"id": "PRRC_kwDOTest1", "databaseId": 1, "author": {"login": "carol"},
       "body": "Please rename the helper.", "createdAt": "2026-01-02T04:00:00Z"}]}},
    {"id": "PRRT_kwDOTest2", "isResolved": true, "isOutdated": false, "path": "src/status_bar.rs",
     "line": 1, "originalLine": 1, "startLine": null, "diffSide": "RIGHT",
     "comments": {"nodes": [{"id": "PRRC_kwDOTest2", "databaseId": 2, "author": {"login": "bob"},
       "body": "Keep the draw call first.", "createdAt": "2026-01-02T05:00:00Z"}]}}
]}}}}}"#;

/// Opens the window of pull request 12 with its detail, threads, and diff injected, read as the
/// signed-in user "daniaal", and waits until it is shown.
fn open_review(harness: &mut Harness<'static, NiceGitApp>, threads: &str) {
    let github = GitHubRepository { host: "github.com".into(), owner: "acme".into(), name: "app".into() };
    let detail = decode_pull_request_detail(PULL_REQUEST_JSON, &github, 12).expect("the detail fixture decodes");
    let threads = decode_review_threads(threads).expect("the threads fixture decodes");
    let diff = file_diffs(DIFF);
    harness.state_mut().open_tool(Box::new(PullRequestDetailWindow::loaded_with_review(
        github,
        12,
        detail,
        Some("daniaal"),
        threads,
        diff,
    )));
    wait(harness, "the detail window", |h| h.state().tools.iter().any(|t| t.title() == "Pull request #12"));
    settle(harness);
}

/// Opens the Files tab and chooses the changed file, whose diff and threads are then shown.
fn open_file(harness: &mut Harness<'static, NiceGitApp>) {
    harness.get_by_label_contains("Files (").click();
    settle(harness);
    wait(harness, "the changed file in the list", |h| h.query_by_label("src/status_bar.rs").is_some());
    harness.get_by_label("src/status_bar.rs").click();
    settle(harness);
}

#[test]
fn a_review_thread_shows_under_its_line_and_a_reply_waits_for_confirmation() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    settle(&mut harness);
    open_review(&mut harness, THREADS_JSON);
    open_file(&mut harness);
    wait(&mut harness, "the thread under its line", |h| shows(h, "Please rename the helper."));

    let line_y = harness.get_by_label_contains("fn helper()").rect().min.y;
    let thread_y = harness.get_by_label_contains("Please rename the helper.").rect().min.y;
    assert!(thread_y > line_y, "the thread is drawn under the line it is on");
    assert!(shows(&harness, "carol"), "the thread names its author");

    harness.get_by_label_contains("Reply").click();
    settle(&mut harness);
    harness.get_by_role_and_label(Role::MultilineTextInput, "Reply text").focus();
    harness.step();
    harness.get_by_role_and_label(Role::MultilineTextInput, "Reply text").type_text("Thanks, done.");
    harness.step();
    // The reply box sits low in the window, so the button is scrolled into view before it is pressed.
    harness.get_by_label_contains("Post reply").scroll_to_me();
    harness.run_steps(3);
    harness.get_by_label_contains("Post reply").click();
    wait(&mut harness, "the reply confirmation", |h| shows(h, "Reply to review thread"));

    assert!(shows(&harness, "github.com/acme/app"), "the repository is named");
    assert!(shows(&harness, "#12"), "the number is named");
    assert!(shows(&harness, "Add the status bar"), "the title is named");
    assert!(shows(&harness, "src/status_bar.rs:2"), "the file and line are named");
    assert!(shows(&harness, "Thanks, done."), "the text to post is shown");

    // Escape closes the confirmation without posting it.
    harness.key_press(Key::Escape);
    settle(&mut harness);
    assert!(!shows(&harness, "Reply to review thread"), "closing the confirmation runs nothing");
    assert!(shows(&harness, "Please rename the helper."), "the thread is still there");
}

#[test]
fn resolved_threads_collapse_and_the_conversation_counts_them_with_a_link_to_files() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    settle(&mut harness);
    open_review(&mut harness, THREADS_JSON);

    harness.get_by_label_contains("Conversation (").click();
    settle(&mut harness);
    assert!(shows(&harness, "2 review threads (1 unresolved)"), "the count is shown in the conversation");
    harness.get_by_label_contains("2 review threads").click();
    settle(&mut harness);
    open_file_row(&mut harness);
    wait(&mut harness, "the resolved thread", |h| shows(h, "Resolved thread by bob"));
    assert!(!shows(&harness, "Keep the draw call first."), "a resolved thread is collapsed");
    harness.get_by_label("Show").click();
    settle(&mut harness);
    assert!(shows(&harness, "Keep the draw call first."), "a resolved thread opens when shown");
}

/// Chooses the changed file, once the Files tab is already open.
fn open_file_row(harness: &mut Harness<'static, NiceGitApp>) {
    wait(harness, "the changed file in the list", |h| h.query_by_label("src/status_bar.rs").is_some());
    harness.get_by_label("src/status_bar.rs").click();
    settle(harness);
}
